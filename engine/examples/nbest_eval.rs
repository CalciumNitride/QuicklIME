// 入力全体の N-best (CONVNBEST) の計測 (docs/spec/engine.md「計測の道具」)
//
// 同じビルドのエンジン (quicklime-engine.exe) を、学習・ユーザ辞書・インポート辞書・設定を
// 一時ディレクトリに隔離して起動し、テストセットの読みを前文脈なしで CONVNBEST に送る。
// 正解が 1 位・上位3件・上位10件に入る割合と、1回の応答時間 (中央値・最大) を
// 標準出力へ Markdown の表で書く。
// --llm を付けると、LLM による並べ替え (docs/spec/engine.md「LLM の並べ替え」) も測る。テストセットの左文脈を
// RERANK の LLM 文脈に渡し、並べ替え後の正解の順位と、RERANK から DONE までの時間を出す。
//
// 使い方:
//   cargo build --release
//   cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv [<読み>...]
//   cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv --llm cpu
//
// オプション:
//   --cases <tsv>  テストセット (llm_bench と同じ形式: 左文脈<TAB>読み<TAB>候補<TAB>正解)。正解は | 区切りで
//                  複数書け、どれかが入れば当たりとする
//   --iters N      1つの読みあたりの計測回数 (既定 5)
//   --llm <cpu|vulkan>  LLM による並べ替えを測る (設定 llm_backend。llm の exe とモデルは
//                       エンジンと同じ規則で探す)
//   --grid         --llm と一緒に使う。「読みの文字数 >= L かつ統計変換の 1 位と 2 位のコスト差
//                  (読み 1 文字あたり) <= G」のときだけ並べ替えを掛けた場合の 1 位・3 位以内の割合を、
//                  L と G の格子の表で出す。コスト差は example の中で辞書を読み込み、前文脈なし・学習が空・
//                  補正なしの convert_nbest_scored で求める。全件を並べ替えて測るため、エンジンには
//                  QUICKLIME_RERANK_MIN_CHARS=0 を渡して読みの長さのしきい値を外す
//   --no-cases     件ごとの表 (読み・正解・候補) と、応答時間の表の最大の読みを出さない
//   <読み>...      時間だけを測る追加の読み (長い入力の確認用)
//
// 辞書はエンジンと同じ規則で探す (QUICKLIME_DICT_DIR、無ければ開発レイアウトの Mozc 辞書)

#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
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
#[path = "../src/number.rs"]
mod number;
#[allow(dead_code)]
#[path = "../src/pos.rs"]
mod pos;
#[allow(dead_code)]
#[path = "../src/predict.rs"]
mod predict;
#[allow(dead_code)]
#[path = "../src/suggest.rs"]
mod suggest;
#[allow(dead_code)]
#[path = "../src/textfile.rs"]
mod textfile;
#[allow(dead_code)]
#[path = "../src/typo.rs"]
mod typo;
#[allow(dead_code)]
#[path = "../src/userdict.rs"]
mod userdict;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::{GenericNamespaced, RecvHalf, SendHalf, Stream, ToNsName};

use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    cases: PathBuf,
    iters: usize,
    /// 並べ替えを測るときのバックエンド (cpu / vulkan)
    llm: Option<String>,
    grid: bool,
    no_cases: bool,
    extra: Vec<String>,
}

fn parse_args() -> BoxResult<Args> {
    let mut cases = None;
    let mut iters = 5;
    let mut llm = None;
    let mut grid = false;
    let mut no_cases = false;
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
            "--grid" => grid = true,
            "--no-cases" => no_cases = true,
            other if other.starts_with("--") => return Err(format!("不明な引数: {other}").into()),
            reading => extra.push(reading.to_string()),
        }
    }
    if grid && llm.is_none() {
        return Err("--grid は --llm と一緒に使います".into());
    }
    Ok(Args { cases: cases.ok_or("--cases が必要です")?, iters: iters.max(1), llm, grid, no_cases, extra })
}

struct Case {
    left_context: String,
    reading: String,
    /// 正解の列そのまま (表示用)
    answer: String,
    answers: Vec<String>,
}

impl Case {
    /// 候補の表記の列で、最初に正解のどれかに当たった順位
    fn rank(&self, surfaces: &[String]) -> Option<usize> {
        surfaces.iter().position(|s| self.answers.contains(s))
    }
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
            answers: f[3].split('|').map(str::to_string).collect(),
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
    /// llm は並べ替えを測るときのバックエンド。設定ファイルに llm 1 と llm_backend を書いて起動する。
    /// rerank_all なら読みの長さによらず並べ替えさせる
    fn spawn(llm: Option<&str>, rerank_all: bool) -> BoxResult<Self> {
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
        let mut command = Command::new(&exe);
        if rerank_all {
            command.env("QUICKLIME_RERANK_MIN_CHARS", "0");
        }
        let child = command
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

    /// RERANK の後、DONE になるまで RERANKGET を送り続けて結果を返す。READY を待った後の ID 0 は
    /// 読みが短くて並べ替えなかったものとみなす
    fn rerank_result(&mut self, left_context: &str, kana: &str) -> BoxResult<Rerank> {
        let started = Instant::now();
        let id = self.rerank(left_context, kana)?;
        if id == 0 {
            return Ok(Rerank::Short);
        }
        loop {
            let response = self.request(&format!("RERANKGET\t{id}"))?;
            if let Some(body) = response.strip_prefix("OK\tDONE\t") {
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                return Ok(Rerank::Done(candidate_surfaces(body), elapsed));
            }
            match response.as_str() {
                "OK\tPENDING" => std::thread::sleep(Duration::from_millis(1)),
                "OK\tNONE" => return Ok(Rerank::None),
                _ => return Err(format!("応答が不正: {response}").into()),
            }
        }
    }

    /// 子プロセスの読み込みが終わり、RERANK が受け付けられるまで待つ
    fn wait_llm_ready(&mut self) -> BoxResult<()> {
        let started = Instant::now();
        loop {
            // エンジンの既定のしきい値 (RERANK_MIN_CHARS) より長くして、しきい値によらず受け付けさせる
            let id = self.rerank("", "あいうえおかきくけこさしすせそ")?;
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

/// RERANK の結果
enum Rerank {
    /// 並べ替えた候補の表記と、RERANK から DONE までの時間 (ms)
    Done(Vec<String>, f64),
    /// RERANKGET が NONE (推論の失敗・タイムアウトなど)
    None,
    /// ID 0 (読みが短くて並べ替えない)
    Short,
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
    let converter = if args.grid { Some(Converter::load()?) } else { None };
    let mut engine = Engine::spawn(args.llm.as_deref(), args.grid)?;
    if args.llm.is_some() {
        return measure_llm(&mut engine, &cases, args.iters, converter.as_ref(), args.no_cases);
    }

    println!("## 候補 ({} 件、前文脈なし)\n", cases.len());
    if !args.no_cases {
        println!("| # | 読み | 正解 | 順位 | 件数 | 上位3件 |");
        println!("|---|---|---|---|---|---|");
    }
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
        let rank = case.rank(&surfaces);
        count_hits(&mut hits, rank);
        if args.no_cases {
            continue;
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
    if args.no_cases {
        println!("| 中央値 (ms) | 最大 (ms) |");
        println!("|---|---|");
        println!("| {:.2} | {:.2} |", median(&mut times), max);
    } else {
        println!("| 中央値 (ms) | 最大 (ms) | 最大の読み |");
        println!("|---|---|---|");
        println!("| {:.2} | {:.2} | {} |", median(&mut times), max, slowest.1);
    }
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
fn measure_llm(
    engine: &mut Engine,
    cases: &[Case],
    iters: usize,
    converter: Option<&Converter>,
    no_cases: bool,
) -> BoxResult<()> {
    engine.wait_llm_ready()?;
    println!("## 並べ替え ({} 件、左文脈あり、前文脈なし)\n", cases.len());
    if !no_cases {
        println!("| # | 左文脈 | 読み | 正解 | 順位 (前) | 順位 (後) | 上位3件 (後) |");
        println!("|---|---|---|---|---|---|---|");
    }
    let mut before = [0usize; 3];
    let mut after = [0usize; 3];
    let mut none = 0;
    let mut short = 0;
    let mut times: Vec<f64> = Vec::new();
    let mut grid_cases: Vec<GridCase> = Vec::new();
    let mut top_mismatches = 0;
    for (i, case) in cases.iter().enumerate() {
        let (surfaces, _) = engine.nbest(&case.reading)?;
        let rank_before = case.rank(&surfaces);
        count_hits(&mut before, rank_before);
        let scored = converter.map(|c| c.top_and_gap(&case.reading));
        if let Some((top, _)) = &scored {
            top_mismatches += usize::from(top.as_deref() != surfaces.first().map(String::as_str));
        }
        let mut grid_case = |rank_after| {
            if let Some((_, gap)) = scored {
                grid_cases.push(GridCase {
                    chars: case.reading.chars().count(),
                    gap,
                    before: rank_before,
                    after: rank_after,
                });
            }
        };
        let reranked = match engine.rerank_result(&case.left_context, &case.reading)? {
            Rerank::Done(reranked, _) => reranked,
            skipped => {
                if matches!(skipped, Rerank::Short) {
                    short += 1;
                } else {
                    none += 1;
                }
                count_hits(&mut after, rank_before);
                grid_case(rank_before);
                continue;
            }
        };
        for _ in 0..iters {
            if let Rerank::Done(_, ms) = engine.rerank_result(&case.left_context, &case.reading)? {
                times.push(ms);
            }
        }
        let rank_after = case.rank(&reranked);
        count_hits(&mut after, rank_after);
        grid_case(rank_after);
        if no_cases {
            continue;
        }
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
    if short > 0 {
        println!("\n- 短い読みで並べ替えなし: {short} 件 (並べ替え前の順位で数えた)");
    }
    if !times.is_empty() {
        let max = times.iter().copied().fold(0.0f64, f64::max);
        println!("\n## RERANK から DONE までの時間 ({} 回/読み)\n", iters);
        println!("| 中央値 (ms) | p95 (ms) | 最大 (ms) |");
        println!("|---|---|---|");
        println!("| {:.2} | {:.2} | {:.2} |", median(&mut times), p95(&mut times), max);
    }
    if converter.is_some() {
        print_grid(&grid_cases, top_mismatches);
    }
    Ok(())
}

/// 並べ替えの条件を決めるための、example の中の統計変換 (前文脈なし・学習が空・補正なし)
struct Converter {
    dictionary: Dictionary,
    user: UserDict,
    matrix: ConnectionMatrix,
    functional: FunctionalIds,
    learning: LearningStore,
}

impl Converter {
    /// 辞書をエンジンと同じ規則で読み込む
    fn load() -> BoxResult<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let isolated = std::env::var_os("QUICKLIME_DICT_DIR");
        let dir = isolated
            .as_ref()
            .map_or_else(|| root.join("references/mozc/src/data/dictionary_oss"), PathBuf::from);
        // エンジンは QUICKLIME_DICT_DIR の指定があるとプロジェクトの UT 辞書を見ない
        let ut = [
            Some(dir.join("dictionary_ut.txt")),
            isolated.is_none().then(|| root.join("references/mozcdic-ut/dictionary_ut.txt")),
        ]
        .into_iter()
        .flatten()
        .find(|p| p.is_file());
        let extra: Vec<PathBuf> = ut.into_iter().collect();
        let started = Instant::now();
        let mut dictionary = Dictionary::load(&dir, &extra)?;
        let parent = dir.parent().map(PathBuf::from).unwrap_or_default();
        if let Some(path) = [dir.join("symbol.tsv"), parent.join("symbol/symbol.tsv")].iter().find(|p| p.is_file()) {
            let _ = dictionary.load_symbols(path);
        }
        if let Some(path) =
            [dir.join("single_kanji.tsv"), parent.join("single_kanji/single_kanji.tsv")].iter().find(|p| p.is_file())
        {
            let _ = dictionary.load_single_kanji(path);
        }
        let matrix = ConnectionMatrix::load(&dir.join("connection_single_column.txt"))?;
        let functional = FunctionalIds::load(&dir.join("id.def"))?;
        eprintln!("辞書の読み込み: {:.1} 秒", started.elapsed().as_secs_f64());
        Ok(Converter {
            dictionary,
            user: UserDict::empty(),
            matrix,
            functional,
            learning: LearningStore::in_memory(),
        })
    }

    /// (1 位の表記, 読み 1 文字あたりの 1 位と 2 位のコスト差)
    fn top_and_gap(&self, kana: &str) -> (Option<String>, f64) {
        let nbest = convert::convert_nbest_scored(
            kana, None, &self.dictionary, &self.user, &self.matrix, &self.functional, &self.learning,
            convert::TypoSource::Off, &typo::TUNING,
        );
        let top = nbest.candidates.first().map(|c| c.segments.iter().map(|s| s.1.as_str()).collect());
        (top, cost_gap(&nbest.costs, kana.chars().count()))
    }
}

/// 経路のコストを持つ候補のうち、最小と 2 番目に小さいコストの差を文字数で割った値。
/// 経路のコストを持つ候補が 1 つ以下なら無限大
fn cost_gap(costs: &[Option<i64>], chars: usize) -> f64 {
    let mut costs: Vec<i64> = costs.iter().flatten().copied().collect();
    costs.sort_unstable();
    match costs.as_slice() {
        [first, second, ..] => (second - first) as f64 / chars.max(1) as f64,
        _ => f64::INFINITY,
    }
}

/// 格子の評価に使う 1 件
#[derive(Clone, Copy)]
struct GridCase {
    chars: usize,
    gap: f64,
    /// 並べ替え前と後の正解の順位 (NONE の件は後 = 前)
    before: Option<usize>,
    after: Option<usize>,
}

#[derive(Debug, PartialEq)]
struct GridCell {
    top1: usize,
    top3: usize,
    /// 条件を満たして並べ替えた件
    reranked: usize,
}

const GRID_CHARS: [usize; 7] = [0, 3, 5, 7, 9, 12, 15];
const GRID_GAPS: [f64; 6] = [f64::INFINITY, 3000.0, 2000.0, 1000.0, 500.0, 250.0];

/// 文字数 >= min_chars かつコスト差 <= max_gap の件だけ並べ替え後の順位、ほかは前の順位で数える
fn grid_cell(cases: &[GridCase], min_chars: usize, max_gap: f64) -> GridCell {
    let mut cell = GridCell { top1: 0, top3: 0, reranked: 0 };
    for case in cases {
        let rerank = case.chars >= min_chars && case.gap <= max_gap;
        let rank = if rerank { case.after } else { case.before };
        cell.reranked += usize::from(rerank);
        cell.top1 += usize::from(rank.is_some_and(|r| r < 1));
        cell.top3 += usize::from(rank.is_some_and(|r| r < 3));
    }
    cell
}

fn print_grid(cases: &[GridCase], top_mismatches: usize) {
    let total = cases.len().max(1) as f64;
    let pct = |n: usize| 100.0 * n as f64 / total;
    let show = |cell: &GridCell| {
        format!("{:.1}% / {:.1}% ({:.0}%)", pct(cell.top1), pct(cell.top3), pct(cell.reranked))
    };
    println!("\n## 並べ替えの条件ごとの正解率 ({} 件)\n", cases.len());
    println!("- 並べ替え前: {}", show(&grid_cell(cases, usize::MAX, f64::INFINITY)));
    println!("- 全件並べ替え: {}", show(&grid_cell(cases, 0, f64::INFINITY)));
    println!(
        "- example の統計変換の 1 位と CONVNBEST の 1 位が食い違う件: {top_mismatches} / {}",
        cases.len()
    );
    println!(
        "\nセルは「1 位 / 3 位以内 (並べ替えた件の割合)」。行 L は読みの文字数の下限、列 G は 1 位と 2 位の\
         コスト差 (読み 1 文字あたり) の上限\n"
    );
    let header: Vec<String> = GRID_GAPS
        .iter()
        .map(|&g| if g.is_infinite() { "無制限".to_string() } else { format!("G <= {g:.0}") })
        .collect();
    println!("| L | {} |", header.join(" | "));
    println!("|---|{}", "---|".repeat(GRID_GAPS.len()));
    for &l in &GRID_CHARS {
        let cells: Vec<String> = GRID_GAPS.iter().map(|&g| show(&grid_cell(cases, l, g))).collect();
        println!("| >= {l} | {} |", cells.join(" | "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_gap_uses_two_smallest_path_costs() {
        assert_eq!(cost_gap(&[None, Some(5000), Some(3000), Some(3000)], 2), 0.0);
        assert_eq!(cost_gap(&[Some(1000), None, Some(3000)], 4), 500.0);
        assert!(cost_gap(&[None, Some(1000)], 3).is_infinite());
        assert!(cost_gap(&[], 3).is_infinite());
    }

    #[test]
    fn grid_cell_reranks_only_cases_meeting_both_conditions() {
        let case = |chars, gap, before, after| GridCase { chars, gap, before, after };
        let cases = [
            case(2, 100.0, Some(0), Some(1)),
            case(10, 4000.0, Some(2), Some(0)),
            case(10, 200.0, Some(1), Some(0)),
            case(4, f64::INFINITY, None, Some(0)),
        ];
        let cell = |top1, top3, reranked| GridCell { top1, top3, reranked };
        // 並べ替え前と全件並べ替え
        assert_eq!(grid_cell(&cases, usize::MAX, f64::INFINITY), cell(1, 3, 0));
        assert_eq!(grid_cell(&cases, 0, f64::INFINITY), cell(3, 4, 4));
        assert_eq!(grid_cell(&cases, 9, f64::INFINITY), cell(3, 3, 2));
        // コスト差が無限大の件は上限があると並べ替えない
        assert_eq!(grid_cell(&cases, 0, 1000.0), cell(1, 3, 2));
        assert_eq!(grid_cell(&cases, 9, 250.0), cell(2, 3, 1));
        // 上限ちょうどは並べ替える
        assert_eq!(grid_cell(&cases, 0, 200.0), cell(1, 3, 2));
        assert_eq!(grid_cell(&cases, 0, 199.0), cell(0, 3, 1));
    }
}
