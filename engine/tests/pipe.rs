// エンジンを実際に起動して named pipe 経由の応答を確認する統合テスト

use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

#[test]
fn エンジンにconvert要求を送ると候補が返る() {
    let mut server = Command::new(env!("CARGO_BIN_EXE_quicklime-engine"))
        .spawn()
        .expect("エンジンを起動できない");

    // テスト本体はクロージャで実行し、失敗してもエンジンを必ず終了させる
    let result = (|| -> std::io::Result<String> {
        // エンジンの起動 (pipe 作成) を接続リトライで待つ
        let mut stream = None;
        for _ in 0..50 {
            let name = "quicklime-engine".to_ns_name::<GenericNamespaced>()?;
            if let Ok(s) = Stream::connect(name) {
                stream = Some(s);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let stream = stream
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "接続できない"))?;

        let (recv, mut send) = stream.split();
        send.write_all("CONVERT\tにほんご\n".as_bytes())?;

        let mut line = String::new();
        BufReader::new(recv).read_line(&mut line)?;
        Ok(line)
    })();

    server.kill().ok();

    assert_eq!(result.expect("パイプ通信に失敗"), "OK\tニホンゴ\tにほんご\n");
}
