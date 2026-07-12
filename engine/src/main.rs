// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod convert;
mod dict;
mod learn;
mod matrix;
mod pos;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};

use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;

/// named pipe の既定名 (Windows では \\.\pipe\quicklime-engine になる)
const DEFAULT_PIPE_NAME: &str = "quicklime-engine";

/// パイプ名。テストで衝突しないよう環境変数 QUICKLIME_PIPE_NAME で上書きできる
fn pipe_name() -> String {
    std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| DEFAULT_PIPE_NAME.to_string())
}

/// 変換に必要なデータ一式 (全接続スレッドで共有する)
struct EngineData {
    dictionary: Dictionary,
    matrix: ConnectionMatrix,
    functional: FunctionalIds,
    /// 学習データ (LEARN で書き込むため Mutex で保護)
    learning: Mutex<LearningStore>,
}

fn main() -> std::io::Result<()> {
    let pipe = pipe_name();

    // 既に別のエンジンが同じパイプで待機していれば二重起動しない
    // (TSF 側の自動起動が複数アプリから同時に走った場合の保険)
    if Stream::connect(pipe.clone().to_ns_name::<GenericNamespaced>()?).is_ok() {
        eprintln!("既にエンジンが起動しているため終了します");
        return Ok(());
    }

    let data = Arc::new(EngineData {
        dictionary: load_dictionary(),
        matrix: load_matrix(),
        functional: load_functional_ids(),
        learning: Mutex::new(LearningStore::load_default()),
    });

    let name = pipe.clone().to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    eprintln!("quicklime-engine: \\\\.\\pipe\\{pipe} で待機中");

    // クライアント (アプリごとの TSF DLL) を1接続=1スレッドで処理する
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let data = Arc::clone(&data);
                thread::spawn(move || handle_client(stream, &data));
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

/// 連接行列を読み込む。失敗しても連接コスト0で起動を続行する
fn load_matrix() -> ConnectionMatrix {
    let Some(dir) = dictionary_dir() else {
        return ConnectionMatrix::empty();
    };
    let path = dir.join("connection_single_column.txt");
    if !path.exists() {
        eprintln!("連接行列がありません ({}), 連接コスト0で動作します", path.display());
        return ConnectionMatrix::empty();
    }
    let started = Instant::now();
    match ConnectionMatrix::load(&path) {
        Ok(matrix) => {
            eprintln!("連接行列を読み込みました ({:.1}秒)", started.elapsed().as_secs_f64());
            matrix
        }
        Err(e) => {
            eprintln!("連接行列の読み込みに失敗しました ({e})。連接コスト0で動作します");
            ConnectionMatrix::empty()
        }
    }
}

/// 品詞ID表 (id.def) を読み込む。無くても単語=文節として動作を続行する
fn load_functional_ids() -> FunctionalIds {
    let Some(dir) = dictionary_dir() else {
        return FunctionalIds::empty();
    };
    let path = dir.join("id.def");
    match FunctionalIds::load(&path) {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("品詞ID表の読み込みに失敗しました ({e})。単語単位の文節になります");
            FunctionalIds::empty()
        }
    }
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream, data: &EngineData) {
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);

    for line in reader.lines() {
        let Ok(line) = line else {
            break; // 読み取りエラー = 切断とみなす
        };
        let response = handle_request(&line, data);
        if send.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
}

/// CONVSEG 応答で文節内のフィールドを区切る文字 (ASCII Unit Separator)
const FIELD_SEPARATOR: char = '\x1f';

/// 1行の要求を解釈して1行の応答を作る
fn handle_request(line: &str, data: &EngineData) -> String {
    let mut fields = line.split('\t');
    match fields.next() {
        Some("CONVERT") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let candidates = convert::candidates(kana, &data.dictionary, &data.matrix);
                format!("OK\t{}\n", candidates.join("\t"))
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVSEG") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let learning = data.learning.lock().expect("learning lock");
                let segments = convert::convert_segments(
                    kana,
                    &data.dictionary,
                    &data.matrix,
                    &data.functional,
                    &learning,
                );
                let body = segments
                    .iter()
                    .map(|s| {
                        let mut fields = vec![s.reading.as_str()];
                        fields.extend(s.candidates.iter().map(String::as_str));
                        fields.join(&FIELD_SEPARATOR.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join("\t");
                format!("OK\t{body}\n")
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("LEARN") => {
            // LEARN\t読み\x1f表記\t読み\x1f表記... : 文節ごとの確定結果を記録する
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for pair in fields {
                if let Some((reading, surface)) = pair.split_once(FIELD_SEPARATOR) {
                    learning.record(reading, surface);
                    count += 1;
                }
            }
            if count > 0 {
                "OK\n".to_string()
            } else {
                "ERR\t記録する内容がありません\n".to_string()
            }
        }
        _ => "ERR\t不明なコマンドです\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_data() -> EngineData {
        EngineData {
            dictionary: Dictionary::empty(),
            matrix: ConnectionMatrix::empty(),
            functional: FunctionalIds::empty(),
            learning: Mutex::new(LearningStore::in_memory()),
        }
    }

    fn sample_data() -> EngineData {
        let mut dictionary = Dictionary::empty();
        dictionary
            .load_from("きょう\t1\t1\t2000\t今日\nは\t2\t2\t500\tは\n".as_bytes())
            .unwrap();
        let functional = FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n".as_bytes()).unwrap();
        EngineData {
            dictionary,
            matrix: ConnectionMatrix::empty(),
            functional,
            learning: Mutex::new(LearningStore::in_memory()),
        }
    }

    #[test]
    fn convert要求に候補を返す() {
        assert_eq!(
            handle_request("CONVERT\tにほん", &empty_data()),
            "OK\tニホン\tにほん\n"
        );
    }

    #[test]
    fn かなが空ならエラー() {
        let data = empty_data();
        assert!(handle_request("CONVERT\t", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVERT", &data).starts_with("ERR\t"));
    }

    #[test]
    fn 不明なコマンドはエラー() {
        assert!(handle_request("FOO\tbar", &empty_data()).starts_with("ERR\t"));
    }

    #[test]
    fn convseg要求に文節列を返す() {
        // 文節はタブ区切り、文節内は US (\x1f) 区切りで「読み 候補1 候補2...」
        let response = handle_request("CONVSEG\tきょうは", &sample_data());
        assert_eq!(
            response,
            "OK\tきょうは\x1f今日は\x1fキョウハ\x1fきょうは\n"
        );
    }

    #[test]
    fn learnで記録した表記が次のconvsegで先頭に来る() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN\tきょうは\x1fキョウハ", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert_eq!(
            response,
            "OK\tきょうは\x1fキョウハ\x1f今日は\x1fきょうは\n"
        );
    }

    #[test]
    fn learnの内容が空ならエラー() {
        assert!(handle_request("LEARN", &empty_data()).starts_with("ERR\t"));
        assert!(handle_request("LEARN\t読みだけ", &empty_data()).starts_with("ERR\t"));
    }
}
