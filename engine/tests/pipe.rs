// エンジンを実際に起動して named pipe 経由の応答を確認する統合テスト

use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

#[test]
fn エンジンにconvert要求を送ると候補が返る() {
    // 固定の小さな辞書 (tests/fixtures) を使い、実辞書の有無に依存しないようにする
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let mut server = Command::new(env!("CARGO_BIN_EXE_quicklime-engine"))
        .env("QUICKLIME_DICT_DIR", fixtures)
        .spawn()
        .expect("エンジンを起動できない");

    // テスト本体はクロージャで実行し、失敗してもエンジンを必ず終了させる
    let result = (|| -> std::io::Result<(String, String)> {
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
        let mut reader = BufReader::new(recv);

        // 単語の変換
        send.write_all("CONVERT\tにほんご\n".as_bytes())?;
        let mut word = String::new();
        reader.read_line(&mut word)?;

        // 文の変換 (Viterbi)
        send.write_all("CONVERT\tきょうははれです\n".as_bytes())?;
        let mut sentence = String::new();
        reader.read_line(&mut sentence)?;

        Ok((word, sentence))
    })();

    server.kill().ok();

    let (word, sentence) = result.expect("パイプ通信に失敗");
    // 辞書候補 (コスト順) → カタカナ → ひらがな
    assert_eq!(word, "OK\t日本語\tニホンゴ\tにほんご\n");
    // 文変換 → カタカナ → ひらがな
    assert_eq!(sentence, "OK\t今日は晴れです\tキョウハハレデス\tきょうははれです\n");
}
