// エンジンを実際に起動して named pipe 経由の応答を確認する統合テスト

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

/// テスト用エンジンを起動する。
/// 固定の小さな辞書 (tests/fixtures) を使い、実辞書の有無に依存しないようにする。
/// パイプ名は呼び出し側で一意にして、起動しっぱなしの開発用エンジンと衝突しないようにする
fn spawn_engine(pipe_name: &str) -> Child {
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    Command::new(env!("CARGO_BIN_EXE_quicklime-engine"))
        .env("QUICKLIME_DICT_DIR", fixtures)
        .env("QUICKLIME_PIPE_NAME", pipe_name)
        .spawn()
        .expect("エンジンを起動できない")
}

#[test]
fn エンジンにconvert要求を送ると候補が返る() {
    let pipe_name = format!("quicklime-engine-test-{}", std::process::id());
    let mut server = spawn_engine(&pipe_name);

    // テスト本体はクロージャで実行し、失敗してもエンジンを必ず終了させる
    let result = (|| -> std::io::Result<(String, String, String)> {
        // エンジンの起動 (pipe 作成) を接続リトライで待つ
        let mut stream = None;
        for _ in 0..50 {
            let name = pipe_name.clone().to_ns_name::<GenericNamespaced>()?;
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

        // 文節ごとの変換
        send.write_all("CONVSEG\tきょうははれです\n".as_bytes())?;
        let mut segments = String::new();
        reader.read_line(&mut segments)?;

        Ok((word, sentence, segments))
    })();

    server.kill().ok();

    let (word, sentence, segments) = result.expect("パイプ通信に失敗");
    // 辞書候補 (コスト順) → カタカナ → ひらがな
    assert_eq!(word, "OK\t日本語\tニホンゴ\tにほんご\n");
    // 文変換 → カタカナ → ひらがな
    assert_eq!(sentence, "OK\t今日は晴れです\tキョウハハレデス\tきょうははれです\n");
    // 文節: きょうは (今日+は) / はれです (晴れ+です)
    assert_eq!(
        segments,
        "OK\tきょうは\x1f今日は\x1fキョウハ\x1fきょうは\
         \tはれです\x1f晴れです\x1fハレデス\x1fはれです\n"
    );
}

#[test]
fn 同じパイプ名のエンジンは二重起動しない() {
    let pipe_name = format!("quicklime-engine-test-dup-{}", std::process::id());
    let mut first = spawn_engine(&pipe_name);

    // 1台目の起動 (パイプ作成) を接続の成功で確認する
    let mut connected = false;
    for _ in 0..50 {
        let name = pipe_name.clone().to_ns_name::<GenericNamespaced>().unwrap();
        if Stream::connect(name).is_ok() {
            connected = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // 2台目は「既に起動している」と判断して自分から終了するはず
    let mut second = spawn_engine(&pipe_name);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut second_exited_ok = false;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = second.try_wait() {
            second_exited_ok = status.success();
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    second.kill().ok();
    first.kill().ok();

    assert!(connected, "1台目のエンジンに接続できない");
    assert!(second_exited_ok, "2台目のエンジンが自動終了しない");
}
