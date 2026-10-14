// UT 辞書の有無による変換の差分 (docs/design/dict-expansion.md の「検証方法」)
//
// 読みの一覧を、Mozc 辞書だけの辞書と UT 辞書を足した辞書の 2 つで convert_segments にかけ、
// 第1候補 (文節表記の連結) が変わったものだけを「読み\tUT なし\tUT あり」で標準出力へ書く。
// 学習・ユーザ辞書・前文脈は使わない。エンジンは lib を持たないため、変換に使うモジュールを
// ソースごと取り込む (src/bin の単語登録ツールと同じ方式)。
//
// 使い方:
//   cargo run --release --example ut_diff -- --mozc <dictionary_oss> --ut <dictionary_ut.txt> \
//       --cases examples/data/llm_cases.tsv --cases examples/data/ut_diff_cases.txt
//
// --cases は複数指定できる。# で始まる行と空行は読み飛ばし、タブを含む行は 2 列目
// (llm_cases.tsv の読みの列) を、含まない行は行全体を読みとする

#[allow(dead_code)]
#[path = "../src/convert.rs"]
mod convert;
#[allow(dead_code)]
#[path = "../src/dict.rs"]
mod dict;
#[allow(dead_code)]
#[path = "../src/english.rs"]
mod english;
#[allow(dead_code)]
#[path = "../src/learn.rs"]
mod learn;
#[allow(dead_code)]
#[path = "../src/matrix.rs"]
mod matrix;
#[allow(dead_code)]
#[path = "../src/pos.rs"]
mod pos;
#[allow(dead_code)]
#[path = "../src/userdict.rs"]
mod userdict;

use std::path::PathBuf;

use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    mozc: PathBuf,
    ut: PathBuf,
    cases: Vec<PathBuf>,
}

fn parse_args() -> BoxResult<Args> {
    let (mut mozc, mut ut) = (None, None);
    let mut cases = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--mozc" => mozc = Some(PathBuf::from(value()?)),
            "--ut" => ut = Some(PathBuf::from(value()?)),
            "--cases" => cases.push(PathBuf::from(value()?)),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    let usage = "使い方: ut_diff --mozc <dictionary_oss> --ut <dictionary_ut.txt> --cases <file>...";
    if cases.is_empty() {
        return Err(usage.into());
    }
    Ok(Args { mozc: mozc.ok_or(usage)?, ut: ut.ok_or(usage)?, cases })
}

fn load_readings(paths: &[PathBuf]) -> BoxResult<Vec<String>> {
    let mut readings = Vec::new();
    for path in paths {
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let reading = match line.split_once('\t') {
                Some((_, rest)) => rest.split('\t').next().unwrap_or(""),
                None => line,
            };
            if !reading.is_empty() && !readings.iter().any(|r| r == reading) {
                readings.push(reading.to_string());
            }
        }
    }
    Ok(readings)
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    if !args.ut.is_file() {
        return Err(format!("UT 辞書がありません: {}", args.ut.display()).into());
    }
    let readings = load_readings(&args.cases)?;
    let matrix = ConnectionMatrix::load(&args.mozc.join("connection_single_column.txt"))?;
    let functional = FunctionalIds::load(&args.mozc.join("id.def"))?;
    let without = Dictionary::load(&args.mozc, &[])?;
    let with = Dictionary::load(&args.mozc, &[args.ut.clone()])?;
    let user = UserDict::empty();
    let learning = LearningStore::in_memory();

    let best = |kana: &str, dict: &Dictionary| -> String {
        convert::convert_segments(kana, None, dict, &user, &matrix, &functional, &learning)
            .iter()
            .map(|s| s.candidates[0].as_str())
            .collect()
    };
    let mut changed = 0;
    for kana in &readings {
        let (a, b) = (best(kana, &without), best(kana, &with));
        if a != b {
            changed += 1;
            println!("{kana}\t{a}\t{b}");
        }
    }
    eprintln!("第1候補が変わった読み: {changed} / {}", readings.len());
    Ok(())
}
