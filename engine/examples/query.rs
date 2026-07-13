// 開発用: 起動中のエンジンに CONVERT 要求を送って応答を表示する
//
// 使い方: cargo run --example query -- きょうははれです [ほかの読み...]

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("使い方: query <読み> [<読み>...]");
        std::process::exit(1);
    }

    let name = "quicklime-engine".to_ns_name::<GenericNamespaced>()?;
    let stream = Stream::connect(name)?;
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);

    for kana in args {
        send.write_all(format!("CONVERT\t{kana}\n").as_bytes())?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        println!("{kana} -> {}", line.trim_end().replace('\t', " | "));

        send.write_all(format!("CONVSEG\t{kana}\n").as_bytes())?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        // 文節区切りは [読み: 候補1 候補2 ...] の形で表示する
        let pretty = line
            .trim_end()
            .trim_start_matches("OK\t")
            .split('\t')
            .map(|seg| {
                let fields: Vec<&str> = seg.split('\x1f').collect();
                format!("[{}: {}]", fields[0], fields[1..].join(" "))
            })
            .collect::<Vec<_>>()
            .join(" ");
        println!("  文節: {pretty}");

        send.write_all(format!("CONVSYM\t{kana}\n").as_bytes())?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        println!("  記号: {}", line.trim_end().replace('\t', " | "));
    }
    Ok(())
}
