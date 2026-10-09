// 入力全体の N-best (CONVNBEST) の計測 (docs/design/nbest.md の「N-best の計測」)
//
// 同じビルドのエンジン (quicklime-engine.exe) を、学習・ユーザ辞書・インポート辞書・設定を
// 一時ディレクトリに隔離して起動し、テストセットの読みを前文脈なしで CONVNBEST に送る。
// 正解が 1 位・上位3件・上位10件に入る割合と、1回の応答時間 (中央値・最大) を
// 標準出力へ Markdown の表で書く。
//
// 使い方:
//   cargo build --release
//   cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv [<読み>...]
//
// オプション:
//   --cases <tsv>  テストセット (llm_bench と同じ形式: 左文脈<TAB>読み<TAB>候補<TAB>正解)
//   --iters N      1つの読みあたりの計測回数 (既定 5)
//   <読み>...      時間だけを測る追加の読み (長い入力の確認用)
//
// 辞書はエンジンと同じ規則で探す (QUICKLIME_DICT_DIR、無ければ開発レイアウトの Mozc 辞書)

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, RecvHalf, SendHalf, Stream, ToNsName};

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    cases: PathBuf,
    iters: usize,
    extra: Vec<String>,
}

fn parse_args() -> BoxResult<Args> {
    let mut cases = None;
    let mut iters = 5;
    let mut extra = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--iters" => iters = value()?.parse()?,
            other if other.starts_with("--") => return Err(format!("不明な引数: {other}").into()),
            reading => extra.push(reading.to_string()),
        }
    }
    Ok(Args { cases: cases.ok_or("--cases が必要です")?, iters: iters.max(1), extra })
}

struct Case {
    reading: String,
    answer: String,
}

fn load_cases(path: &PathBuf) -> BoxResult<Vec<Case>> {
    let text = std::fs::read_to_string(path)?;
    let mut cases = Vec::new();
    for (no, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 4 {
            return Err(format!("{}:{}: 列数が 4 ではありません", path.display(), no + 1).into());
        }
        cases.push(Case { reading: f[1].to_string(), answer: f[3].to_string() });
    }
    Ok(cases)
}

/// 計測用に隔離して起動したエンジン。drop で終了させ、一時ディレクトリを消す
struct Engine {
    child: Child,
    dir: PathBuf,
    send: SendHalf,
    recv: BufReader<RecvHalf>,
}

impl Engine {
    fn spawn() -> BoxResult<Self> {
        // examples の exe は target/<profile>/examples/ に置かれる
        let exe = std::env::current_exe()?
            .parent()
            .and_then(|p| p.parent())
            .ok_or("exe の場所を特定できません")?
            .join("quicklime-engine.exe");
        if !exe.is_file() {
            return Err(format!("{} がありません (先に cargo build する)", exe.display()).into());
        }
        let pipe = format!("quicklime-nbest-eval-{}", std::process::id());
        let dir = std::env::temp_dir().join(&pipe);
        std::fs::create_dir_all(&dir)?;
        let child = Command::new(&exe)
            .env("QUICKLIME_PIPE_NAME", &pipe)
            .env("QUICKLIME_LEARN_FILE", dir.join("learning.tsv"))
            .env("QUICKLIME_USER_DICT_FILE", dir.join("userdict.tsv"))
            .env("QUICKLIME_IMPORT_DIR", dir.join("imported"))
            .env("QUICKLIME_CONFIG_FILE", dir.join("config.tsv"))
            .stderr(std::process::Stdio::null())
            .spawn()?;
        // 辞書の読み込みが終わってパイプができるまで待つ
        let started = Instant::now();
        loop {
            if let Ok(stream) = Stream::connect(pipe.clone().to_ns_name::<GenericNamespaced>()?) {
                let (recv, send) = stream.split();
                return Ok(Engine { child, dir, send, recv: BufReader::new(recv) });
            }
            if started.elapsed() > Duration::from_secs(60) {
                let mut child = child;
                let _ = child.kill();
                return Err("エンジンに接続できません".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn request(&mut self, line: &str) -> BoxResult<String> {
        self.send.write_all(format!("{line}\n").as_bytes())?;
        let mut response = String::new();
        self.recv.read_line(&mut response)?;
        Ok(response.trim_end_matches('\n').to_string())
    }

    /// CONVNBEST の候補の表記 (文節の表記の連結) を返す
    fn nbest(&mut self, kana: &str) -> BoxResult<(Vec<String>, f64)> {
        let started = Instant::now();
        let response = self.request(&format!("CONVNBEST\t\t{kana}"))?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        let body = response.strip_prefix("OK\t").ok_or_else(|| format!("応答が不正: {response}"))?;
        let surfaces = body
            .split('\t')
            .map(|candidate| {
                candidate
                    .split('\x1e')
                    .map(|seg| seg.split_once('\x1f').map_or("", |s| s.1))
                    .collect()
            })
            .collect();
        Ok((surfaces, elapsed))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// kana の CONVNBEST を iters 回測り、(候補の表記, 各回の時間) を返す。初回は計測に入れない
fn measure(engine: &mut Engine, kana: &str, iters: usize) -> BoxResult<(Vec<String>, Vec<f64>)> {
    let (surfaces, _) = engine.nbest(kana)?;
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        samples.push(engine.nbest(kana)?.1);
    }
    Ok((surfaces, samples))
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    let n = samples.len();
    if n % 2 == 1 { samples[n / 2] } else { (samples[n / 2 - 1] + samples[n / 2]) / 2.0 }
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let cases = load_cases(&args.cases)?;
    let mut engine = Engine::spawn()?;

    println!("## 候補 ({} 件、前文脈なし)\n", cases.len());
    println!("| # | 読み | 正解 | 順位 | 件数 | 上位3件 |");
    println!("|---|---|---|---|---|---|");
    let mut hits = [0usize; 3];
    let mut times: Vec<f64> = Vec::new();
    let mut slowest = (0.0f64, String::new());
    for (i, case) in cases.iter().enumerate() {
        let (surfaces, samples) = measure(&mut engine, &case.reading, args.iters)?;
        for &ms in &samples {
            if ms > slowest.0 {
                slowest = (ms, case.reading.clone());
            }
        }
        times.extend(samples);
        let rank = surfaces.iter().position(|s| *s == case.answer);
        for (k, limit) in [1usize, 3, 10].into_iter().enumerate() {
            hits[k] += usize::from(rank.is_some_and(|r| r < limit));
        }
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            i + 1,
            case.reading,
            case.answer,
            rank.map_or("-".to_string(), |r| (r + 1).to_string()),
            surfaces.len(),
            surfaces.iter().take(3).cloned().collect::<Vec<_>>().join(" / ")
        );
    }
    let mut extra_times = Vec::new();
    for kana in &args.extra {
        let (surfaces, mut samples) = measure(&mut engine, kana, args.iters)?;
        let max = samples.iter().copied().fold(0.0f64, f64::max);
        extra_times.push((kana.clone(), surfaces, median(&mut samples), max));
    }

    let total = cases.len().max(1);
    let pct = |h: usize| format!("{}/{} ({:.0}%)", h, cases.len(), 100.0 * h as f64 / total as f64);
    println!("\n## 正解の順位\n");
    println!("| 1位 | 上位3件 | 上位10件 |");
    println!("|---|---|---|");
    println!("| {} | {} | {} |", pct(hits[0]), pct(hits[1]), pct(hits[2]));

    println!("\n## 応答時間 (CONVNBEST の1往復、{} 回/読み)\n", args.iters);
    let max = times.iter().copied().fold(0.0f64, f64::max);
    println!("| 中央値 (ms) | 最大 (ms) | 最大の読み |");
    println!("|---|---|---|");
    println!("| {:.2} | {:.2} | {} |", median(&mut times), max, slowest.1);
    if !extra_times.is_empty() {
        println!("\n| 追加の読み | 文字数 | 中央値 (ms) | 最大 (ms) | 件数 | 上位3件 |");
        println!("|---|---|---|---|---|---|");
        for (kana, surfaces, ms, max) in &extra_times {
            println!(
                "| {} | {} | {:.2} | {:.2} | {} | {} |",
                kana,
                kana.chars().count(),
                ms,
                max,
                surfaces.len(),
                surfaces.iter().take(3).cloned().collect::<Vec<_>>().join(" / ")
            );
        }
    }
    Ok(())
}
