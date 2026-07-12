// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod convert;

use std::io::{BufRead, BufReader, Write};
use std::thread;

use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};

/// named pipe の名前 (Windows では \\.\pipe\quicklime-engine になる)
const PIPE_NAME: &str = "quicklime-engine";

fn main() -> std::io::Result<()> {
    let name = PIPE_NAME.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    eprintln!("quicklime-engine: \\\\.\\pipe\\{PIPE_NAME} で待機中");

    // クライアント (アプリごとの TSF DLL) を1接続=1スレッドで処理する
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                thread::spawn(|| handle_client(stream));
            }
            Err(e) => eprintln!("接続の受け付けに失敗: {e}"),
        }
    }
    Ok(())
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream) {
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);

    for line in reader.lines() {
        let Ok(line) = line else {
            break; // 読み取りエラー = 切断とみなす
        };
        let response = handle_request(&line);
        if send.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
}

/// 1行の要求を解釈して1行の応答を作る
fn handle_request(line: &str) -> String {
    let mut fields = line.split('\t');
    match fields.next() {
        Some("CONVERT") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let candidates = convert::candidates(kana);
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
        assert_eq!(handle_request("CONVERT\tにほん"), "OK\tニホン\tにほん\n");
    }

    #[test]
    fn かなが空ならエラー() {
        assert!(handle_request("CONVERT\t").starts_with("ERR\t"));
        assert!(handle_request("CONVERT").starts_with("ERR\t"));
    }

    #[test]
    fn 不明なコマンドはエラー() {
        assert!(handle_request("FOO\tbar").starts_with("ERR\t"));
    }
}
