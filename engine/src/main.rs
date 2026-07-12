// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod convert;
mod dict;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};

use dict::Dictionary;

/// named pipe の名前 (Windows では \\.\pipe\quicklime-engine になる)
const PIPE_NAME: &str = "quicklime-engine";

fn main() -> std::io::Result<()> {
    let dictionary = Arc::new(load_dictionary());

    let name = PIPE_NAME.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    eprintln!("quicklime-engine: \\\\.\\pipe\\{PIPE_NAME} で待機中");

    // クライアント (アプリごとの TSF DLL) を1接続=1スレッドで処理する
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let dictionary = Arc::clone(&dictionary);
                thread::spawn(move || handle_client(stream, &dictionary));
            }
            Err(e) => eprintln!("接続の受け付けに失敗: {e}"),
        }
    }
    Ok(())
}

/// 辞書ディレクトリの決定。優先順:
/// 1. 環境変数 QUICKLIME_DICT_DIR
/// 2. プロジェクトルートの references/mozc/src/data/dictionary_oss
///    (exe が engine/target/{debug,release}/ にある前提で相対解決)
fn dictionary_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QUICKLIME_DICT_DIR") {
        return Some(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().ok()?;
    let project_root = exe.parent()?.parent()?.parent()?.parent()?;
    Some(project_root.join("references/mozc/src/data/dictionary_oss"))
}

/// 辞書を読み込む。失敗しても空の辞書で起動を続行する (カタカナ/ひらがな候補のみになる)
fn load_dictionary() -> Dictionary {
    let Some(dir) = dictionary_dir() else {
        eprintln!("辞書ディレクトリを特定できません。辞書なしで起動します");
        return Dictionary::empty();
    };
    let started = Instant::now();
    match Dictionary::load(&dir) {
        Ok(dict) => {
            eprintln!(
                "辞書を読み込みました: {} エントリ ({:.1}秒) [{}]",
                dict.entry_count(),
                started.elapsed().as_secs_f64(),
                dir.display()
            );
            dict
        }
        Err(e) => {
            eprintln!("辞書の読み込みに失敗しました ({e})。辞書なしで起動します");
            Dictionary::empty()
        }
    }
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream, dictionary: &Dictionary) {
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);

    for line in reader.lines() {
        let Ok(line) = line else {
            break; // 読み取りエラー = 切断とみなす
        };
        let response = handle_request(&line, dictionary);
        if send.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
}

/// 1行の要求を解釈して1行の応答を作る
fn handle_request(line: &str, dictionary: &Dictionary) -> String {
    let mut fields = line.split('\t');
    match fields.next() {
        Some("CONVERT") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let candidates = convert::candidates(kana, dictionary);
                format!("OK\t{}\n", candidates.join("\t"))
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        _ => "ERR\t不明なコマンドです\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert要求に候補を返す() {
        assert_eq!(
            handle_request("CONVERT\tにほん", &Dictionary::empty()),
            "OK\tニホン\tにほん\n"
        );
    }

    #[test]
    fn かなが空ならエラー() {
        let dict = Dictionary::empty();
        assert!(handle_request("CONVERT\t", &dict).starts_with("ERR\t"));
        assert!(handle_request("CONVERT", &dict).starts_with("ERR\t"));
    }

    #[test]
    fn 不明なコマンドはエラー() {
        assert!(handle_request("FOO\tbar", &Dictionary::empty()).starts_with("ERR\t"));
    }
}
