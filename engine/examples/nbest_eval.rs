// 入力全体の N-best (CONVNBEST) の計測 (docs/design/nbest.md の「N-best の計測」)
//
// 同じビルドのエンジン (quicklime-engine.exe) を、学習・ユーザ辞書・インポート辞書・設定を
// 一時ディレクトリに隔離して起動し、テストセットの読みを前文脈なしで CONVNBEST に送る。
// 正解が 1 位・上位3件・上位10件に入る割合と、1回の応答時間 (中央値・最大) を
// 標準出力へ Markdown の表で書く。
// --llm を付けると、LLM による並べ替え (docs/design/llm-rerank.md) も測る。テストセットの左文脈を
// RERANK の LLM 文脈に渡し、並べ替え後の正解の順位と、RERANK から DONE までの時間を出す。
//
// 使い方:
//   cargo build --release
//   cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv [<読み>...]
//   cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv --llm cpu
//
// オプション:
//   --cases <tsv>  テストセット (llm_bench と同じ形式: 左文脈<TAB>読み<TAB>候補<TAB>正解)
//   --iters N      1つの読みあたりの計測回数 (既定 5)
//   --llm <cpu|vulkan>  LLM による並べ替えを測る (設定 llm_backend。llm の exe とモデルは
//                       エンジンと同じ規則で探す)
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
    /// 並べ替えを測るときのバックエンド (cpu / vulkan)
    llm: Option<String>,
    extra: Vec<String>,
}

fn parse_args() -> BoxResult<Args> {
    let mut cases = None;
    let mut iters = 5;
    let mut llm = None;
    let mut extra = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--iters" => iters = value()?.parse()?,
            "--llm" => {
                let backend = value()?;
                if backend != "cpu" && backend != "vulkan" {
                    return Err(format!("未対応のバックエンド: {backend}").into());
                }
                llm = Some(backend);
            }
            other if other.starts_with("--") => return Err(format!("不明な引数: {other}").into()),
            reading => extra.push(reading.to_string()),
        }
    }
    Ok(Args { cases: cases.ok_or("--cases が必要です")?, iters: iters.max(1), llm, extra })
}

struct Case {
    left_context: String,
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
        cases.push(Case {
            left_context: f[0].to_string(),
            reading: f[1].to_string(),
            answer: f[3].to_string(),
        });
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
    /// llm は並べ替えを測るときのバックエンド。設定ファイルに llm 1 と llm_backend を書いて起動する
    fn spawn(llm: Option<&str>) -> BoxResult<Self> {
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
        if let Some(backend) = llm {
            std::fs::write(dir.join("config.tsv"), format!("llm\t1\nllm_backend\t{backend}\n"))?;
        }
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
        Ok((candidate_surfaces(body), elapsed))
    }

    /// RERANK を送り、受け付けた ID を返す (0 なら並べ替えをしない)
    fn rerank(&mut self, left_context: &str, kana: &str) -> BoxResult<u64> {
        let response = self.request(&format!("RERANK\t{left_context}\t\t{kana}"))?;
        let id = response.strip_prefix("OK\t").and_then(|id| id.parse().ok());
        Ok(id.ok_or_else(|| format!("応答が不正: {response}"))?)
    }

    /// RERANK の後、DONE になるまで RERANKGET を送り続け、(並べ替えた候補の表記, RERANK から
    /// DONE までの時間) を返す。NONE なら None
    fn rerank_result(&mut self, left_context: &str, kana: &str) -> BoxResult<Option<(Vec<String>, f64)>> {
        let started = Instant::now();
        let id = self.rerank(left_context, kana)?;
        if id == 0 {
            return Err("RERANK が受け付けられません (ID 0)".into());
        }
        loop {
            let response = self.request(&format!("RERANKGET\t{id}"))?;
            if let Some(body) = response.strip_prefix("OK\tDONE\t") {
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                return Ok(Some((candidate_surfaces(body), elapsed)));
            }
            match response.as_str() {
                "OK\tPENDING" => std::thread::sleep(Duration::from_millis(1)),
                "OK\tNONE" => return Ok(None),
                _ => return Err(format!("応答が不正: {response}").into()),
            }
        }
    }

    /// 子プロセスの読み込みが終わり、RERANK が受け付けられるまで待つ
    fn wait_llm_ready(&mut self) -> BoxResult<()> {
        let started = Instant::now();
        loop {
            let id = self.rerank("", "あ")?;
            if id != 0 {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(60) {
                return Err("LLM の子プロセスが READY になりません (exe とモデルの場所を確かめる)".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// CONVNBEST 形式の候補列 (タブ区切り、文節は \x1e 区切り) を表記の列にする
fn candidate_surfaces(body: &str) -> Vec<String> {
    body.split('\t')
        .map(|candidate| {
            candidate
                .split('\x1e')
                .map(|seg| seg.split_once('\x1f').map_or("", |s| s.1))
                .collect()
        })
        .collect()
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

/// p95 (nearest-rank 法)
fn p95(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    let n = samples.len();
    let rank = ((0.95 * n as f64).ceil() as usize).clamp(1, n);
    samples[rank - 1]
}

/// 正解が 1 位・上位3件・上位10件に入ったかを数える
fn count_hits(hits: &mut [usize; 3], rank: Option<usize>) {
    for (k, limit) in [1usize, 3, 10].into_iter().enumerate() {
        hits[k] += usize::from(rank.is_some_and(|r| r < limit));
    }
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let cases = load_cases(&args.cases)?;
    let mut engine = Engine::spawn(args.llm.as_deref())?;
    if args.llm.is_some() {
        return measure_llm(&mut engine, &cases, args.iters);
    }

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
        count_hits(&mut hits, rank);
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

/// LLM による並べ替えの計測。各読みを左文脈つきで RERANK し、並べ替え前 (CONVNBEST) と後の
/// 正解の順位を比べる。時間は iters 回の RERANK から DONE まで (初回は計測に入れない)
fn measure_llm(engine: &mut Engine, cases: &[Case], iters: usize) -> BoxResult<()> {
    engine.wait_llm_ready()?;
    println!("## 並べ替え ({} 件、左文脈あり、前文脈なし)\n", cases.len());
    println!("| # | 左文脈 | 読み | 正解 | 順位 (前) | 順位 (後) | 上位3件 (後) |");
    println!("|---|---|---|---|---|---|---|");
    let mut before = [0usize; 3];
    let mut after = [0usize; 3];
    let mut none = 0;
    let mut times: Vec<f64> = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let (surfaces, _) = engine.nbest(&case.reading)?;
        let rank_before = surfaces.iter().position(|s| *s == case.answer);
        count_hits(&mut before, rank_before);
        let Some((reranked, _)) = engine.rerank_result(&case.left_context, &case.reading)? else {
            none += 1;
            count_hits(&mut after, rank_before);
            continue;
        };
        for _ in 0..iters {
            if let Some((_, ms)) = engine.rerank_result(&case.left_context, &case.reading)? {
                times.push(ms);
            }
        }
        let rank_after = reranked.iter().position(|s| *s == case.answer);
        count_hits(&mut after, rank_after);
        let rank = |r: Option<usize>| r.map_or("-".to_string(), |r| (r + 1).to_string());
        println!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            i + 1,
            if case.left_context.is_empty() { "-" } else { &case.left_context },
            case.reading,
            case.answer,
            rank(rank_before),
            rank(rank_after),
            reranked.iter().take(3).cloned().collect::<Vec<_>>().join(" / ")
        );
    }
    let total = cases.len().max(1);
    let pct = |h: usize| format!("{}/{} ({:.0}%)", h, cases.len(), 100.0 * h as f64 / total as f64);
    println!("\n## 正解の順位\n");
    println!("| 並び | 1位 | 上位3件 | 上位10件 |");
    println!("|---|---|---|---|");
    println!("| 並べ替え前 | {} | {} | {} |", pct(before[0]), pct(before[1]), pct(before[2]));
    println!("| 並べ替え後 | {} | {} | {} |", pct(after[0]), pct(after[1]), pct(after[2]));
    if none > 0 {
        println!("\n- 並べ替えの結果が NONE だった読み: {none} 件 (並べ替え前の順位で数えた)");
    }
    if !times.is_empty() {
        let max = times.iter().copied().fold(0.0f64, f64::max);
        println!("\n## RERANK から DONE までの時間 ({} 回/読み)\n", iters);
        println!("| 中央値 (ms) | p95 (ms) | 最大 (ms) |");
        println!("|---|---|---|");
        println!("| {:.2} | {:.2} | {:.2} |", median(&mut times), p95(&mut times), max);
    }
    Ok(())
}
