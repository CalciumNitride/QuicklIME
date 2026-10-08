// 実験 0b: zenz 系かな漢字変換モデルによる順位補正の計測 (docs/design/experiment-0b-llm.md)
//
// llama.cpp でモデルを読み込み、生成・順位付けの遅延、読み込み時間、メモリ、
// テストセットでの精度を測って標準出力へ Markdown の表で書く。llama.cpp のログは標準エラーに出る。
//
// 使い方:
//   cargo run --release --example llm_bench --features llm-bench -- \
//     --model <gguf> --backend cpu --threads 6 --cases examples/data/llm_cases.tsv
//   cargo run --release --example llm_bench --features llm-bench-vulkan -- \
//     --model <gguf> --backend vulkan --cases examples/data/llm_cases.tsv
//
// オプション:
//   --mode bench|accuracy|all  (既定 all)
//   --iters N / --warmup N     (既定 30 / 5)
//   --load-iters N             (読み込み時間の反復回数。既定 5)
//   --engine                   (精度確認で、起動中のエンジンから統計変換の 1-best を取る。
//                               パイプ名は QUICKLIME_PIPE_NAME)

use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Instant;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::params::kv_overrides::ParamOverrideValue;
use llama_cpp_2::token::LlamaToken;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

// プロンプトの区切り (AzooKeyKanaKanjiConverter の ZenzPromptBuilder.swift、zenz-v3 の書式)
const INPUT_TAG: char = '\u{EE00}';
const OUTPUT_TAG: char = '\u{EE01}';
const CONTEXT_TAG: char = '\u{EE02}';
/// 左文脈は末尾 40 文字だけを使う (azooKey の既定値)
const MAX_LEFT_CONTEXT: usize = 40;

const N_CTX: u32 = 512;
/// 候補を複数シーケンスに並べて1回で評価する形 (reuse-batch) のため、最大候補数 + 1 を確保する
const N_SEQ_MAX: u32 = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Cpu,
    Vulkan,
}

struct Args {
    model: PathBuf,
    backend: Backend,
    threads: i32,
    cases: Option<PathBuf>,
    mode: String,
    iters: usize,
    warmup: usize,
    load_iters: usize,
    engine: bool,
}

fn parse_args() -> BoxResult<Args> {
    let mut model = None;
    let mut backend = Backend::Cpu;
    let mut threads = None;
    let mut cases = None;
    let mut mode = "all".to_string();
    let mut iters = 30;
    let mut warmup = 5;
    let mut load_iters = 5;
    let mut engine = false;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--model" => model = Some(PathBuf::from(value()?)),
            "--backend" => {
                backend = match value()?.as_str() {
                    "cpu" => Backend::Cpu,
                    "vulkan" => Backend::Vulkan,
                    other => return Err(format!("未対応のバックエンド: {other}").into()),
                }
            }
            "--threads" => threads = Some(value()?.parse()?),
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--mode" => mode = value()?,
            "--iters" => iters = value()?.parse()?,
            "--warmup" => warmup = value()?.parse()?,
            "--load-iters" => load_iters = value()?.parse()?,
            "--engine" => engine = true,
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    if backend == Backend::Vulkan && !cfg!(feature = "llm-bench-vulkan") {
        return Err("--backend vulkan には feature llm-bench-vulkan でのビルドが必要です".into());
    }
    let threads = threads.unwrap_or_else(|| {
        std::thread::available_parallelism().map_or(4, |n| n.get() as i32)
    });
    Ok(Args {
        model: model.ok_or("--model が必要です")?,
        backend,
        threads,
        cases,
        mode,
        iters,
        warmup,
        load_iters: load_iters.max(1),
        engine,
    })
}

// ---------------------------------------------------------------------------
// メモリ計測

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

// エンジンの windows-sys に feature を足すと通常ビルドが変わるため、ここだけ直接宣言する
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
}

struct MemSnapshot {
    working_set: usize,
    peak_working_set: usize,
    private: usize,
    peak_private: usize,
    /// GPU の専用メモリ (このプロセス分)。Vulkan のときだけ測る
    vram: Option<u64>,
}

fn mem_snapshot(with_vram: bool) -> MemSnapshot {
    let mut c = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ..Default::default()
    };
    unsafe {
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
    }
    MemSnapshot {
        working_set: c.working_set_size,
        peak_working_set: c.peak_working_set_size,
        private: c.pagefile_usage,
        peak_private: c.peak_pagefile_usage,
        vram: if with_vram { query_vram() } else { None },
    }
}

/// Windows のパフォーマンスカウンタ「GPU Process Memory」から、自プロセスの専用 GPU メモリを
/// 全アダプタ分合計して返す。DXGI を FFI で呼ぶより簡単なため PowerShell を介する
fn query_vram() -> Option<u64> {
    let pid = std::process::id();
    let script = format!(
        "((Get-Counter '\\GPU Process Memory(pid_{pid}_*)\\Dedicated Usage').CounterSamples \
         | Measure-Object -Property CookedValue -Sum).Sum"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().ok().map(|v| v as u64)
}

fn mb(bytes: usize) -> String {
    format!("{:.1}", bytes as f64 / 1024.0 / 1024.0)
}

// ---------------------------------------------------------------------------
// 統計

struct Stats {
    median: f64,
    p95: f64,
    min: f64,
    max: f64,
}

fn stats(samples_ms: &mut [f64]) -> Stats {
    samples_ms.sort_by(|a, b| a.total_cmp(b));
    let n = samples_ms.len();
    let median = if n % 2 == 1 {
        samples_ms[n / 2]
    } else {
        (samples_ms[n / 2 - 1] + samples_ms[n / 2]) / 2.0
    };
    // p95 は nearest-rank 法
    let rank = ((0.95 * n as f64).ceil() as usize).clamp(1, n);
    Stats {
        median,
        p95: samples_ms[rank - 1],
        min: samples_ms[0],
        max: samples_ms[n - 1],
    }
}

fn measure<F: FnMut() -> BoxResult<()>>(warmup: usize, iters: usize, mut f: F) -> BoxResult<Stats> {
    for _ in 0..warmup {
        f()?;
    }
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let started = Instant::now();
        f()?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(stats(&mut samples))
}

// ---------------------------------------------------------------------------
// プロンプトと推論

fn to_katakana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3041}'..='\u{3096}' => char::from_u32(c as u32 + 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// zenz のトークナイザは半角スペースと改行を扱えないため、azooKey と同じく置き換える
fn preprocess(s: &str) -> String {
    s.replace(' ', "\u{3000}").replace('\n', "")
}

fn build_prompt(left_context: &str, reading: &str) -> String {
    let mut prompt = String::new();
    if !left_context.is_empty() {
        let n = left_context.chars().count();
        let trimmed: String = left_context.chars().skip(n.saturating_sub(MAX_LEFT_CONTEXT)).collect();
        prompt.push(CONTEXT_TAG);
        prompt.push_str(&trimmed);
    }
    prompt.push(INPUT_TAG);
    prompt.push_str(&to_katakana(reading));
    prompt.push(OUTPUT_TAG);
    preprocess(&prompt)
}

struct Llm<'a> {
    model: &'a LlamaModel,
    ctx: LlamaContext<'a>,
    batch: LlamaBatch<'static>,
}

/// logits 上での token の対数確率 (log softmax)
fn log_prob(logits: &[f32], token: LlamaToken) -> f64 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let sum: f64 = logits.iter().map(|&l| (l as f64 - max).exp()).sum();
    logits[token.0 as usize] as f64 - max - sum.ln()
}

fn argmax(logits: &[f32]) -> LlamaToken {
    let mut best = 0;
    for (i, &l) in logits.iter().enumerate() {
        if l > logits[best] {
            best = i;
        }
    }
    LlamaToken(best as i32)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RankMode {
    /// 候補ごとにキャッシュを捨て、プロンプト + 候補を丸ごと評価する
    NoReuse,
    /// プロンプトを1回だけ評価し、候補ごとにプロンプト以降のキャッシュを捨てて順に評価する
    ReuseSeq,
    /// プロンプトを1回だけ評価し、そのキャッシュを候補数分のシーケンスへ複製して1回で評価する
    ReuseBatch,
}

impl RankMode {
    fn label(self) -> &'static str {
        match self {
            RankMode::NoReuse => "使い回さない",
            RankMode::ReuseSeq => "使い回す (候補を順に)",
            RankMode::ReuseBatch => "使い回す (候補を1バッチ)",
        }
    }
}

impl<'a> Llm<'a> {
    fn new(model: &'a LlamaModel, backend: &LlamaBackend, args: &Args) -> BoxResult<Self> {
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX))
            .with_n_batch(N_CTX)
            .with_n_ubatch(N_CTX)
            .with_n_seq_max(N_SEQ_MAX)
            .with_kv_unified(true)
            .with_n_threads(args.threads)
            .with_n_threads_batch(args.threads);
        let ctx = model.new_context(backend, params)?;
        Ok(Self {
            model,
            ctx,
            batch: LlamaBatch::new(N_CTX as usize, N_SEQ_MAX as i32),
        })
    }

    fn tokenize_prompt(&self, prompt: &str) -> Vec<LlamaToken> {
        // azooKey と同じく BOS を付け、特殊トークンの解釈はしない
        self.model.vocab().tokenize(prompt.as_bytes(), true, false)
    }

    fn tokenize_text(&self, text: &str) -> Vec<LlamaToken> {
        self.model.vocab().tokenize(preprocess(text).as_bytes(), false, false)
    }

    fn eos(&self) -> LlamaToken {
        self.model.vocab().eos()
    }

    /// 貪欲法で出力を生成する
    fn generate(&mut self, left_context: &str, reading: &str) -> BoxResult<String> {
        let prompt = self.tokenize_prompt(&build_prompt(left_context, reading));
        let max_new = reading.chars().count() * 2 + 8;
        self.ctx.clear_kv_cache();
        self.batch.clear();
        let last = prompt.len() - 1;
        for (i, &t) in prompt.iter().enumerate() {
            self.batch.add(t, i as i32, &[0], i == last)?;
        }
        self.ctx.decode(&mut self.batch)?;
        let mut logits_index = last as i32;
        let mut pos = prompt.len() as i32;
        let mut out = Vec::new();
        for _ in 0..max_new {
            let next = argmax(self.ctx.get_logits_ith(logits_index));
            if self.model.vocab().is_eog(next) {
                break;
            }
            out.extend(self.model.vocab().token_to_piece(next, false, None));
            self.batch.clear();
            self.batch.add(next, pos, &[0], true)?;
            self.ctx.decode(&mut self.batch)?;
            logits_index = 0;
            pos += 1;
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// 各候補の対数尤度 (候補の全トークン + 出力終端の EOS) を返す
    fn rank(&mut self, left_context: &str, reading: &str, candidates: &[String], mode: RankMode) -> BoxResult<Vec<f64>> {
        let prompt = self.tokenize_prompt(&build_prompt(left_context, reading));
        let eos = self.eos();
        let cand_tokens: Vec<Vec<LlamaToken>> = candidates
            .iter()
            .map(|c| {
                let mut t = self.tokenize_text(c);
                t.push(eos);
                t
            })
            .collect();
        let p = prompt.len();
        let mut scores = Vec::with_capacity(candidates.len());

        match mode {
            RankMode::NoReuse => {
                for toks in &cand_tokens {
                    self.ctx.clear_kv_cache();
                    self.batch.clear();
                    for (i, &t) in prompt.iter().enumerate() {
                        self.batch.add(t, i as i32, &[0], i == p - 1)?;
                    }
                    // 最後の EOS の次の予測は不要なので、EOS 自体はバッチに入れない
                    for (j, &t) in toks[..toks.len() - 1].iter().enumerate() {
                        self.batch.add(t, (p + j) as i32, &[0], true)?;
                    }
                    self.ctx.decode(&mut self.batch)?;
                    let mut s = 0.0;
                    for (j, &t) in toks.iter().enumerate() {
                        s += log_prob(self.ctx.get_logits_ith((p - 1 + j) as i32), t);
                    }
                    scores.push(s);
                }
            }
            RankMode::ReuseSeq | RankMode::ReuseBatch => {
                self.ctx.clear_kv_cache();
                self.batch.clear();
                for (i, &t) in prompt.iter().enumerate() {
                    self.batch.add(t, i as i32, &[0], i == p - 1)?;
                }
                self.ctx.decode(&mut self.batch)?;
                let prompt_logits = self.ctx.get_logits_ith((p - 1) as i32).to_vec();

                if mode == RankMode::ReuseSeq {
                    for toks in &cand_tokens {
                        self.ctx.clear_kv_cache_seq(Some(0), Some(p as u32), None)?;
                        let mut s = log_prob(&prompt_logits, toks[0]);
                        if toks.len() > 1 {
                            self.batch.clear();
                            for (j, &t) in toks[..toks.len() - 1].iter().enumerate() {
                                self.batch.add(t, (p + j) as i32, &[0], true)?;
                            }
                            self.ctx.decode(&mut self.batch)?;
                            for (j, &t) in toks[1..].iter().enumerate() {
                                s += log_prob(self.ctx.get_logits_ith(j as i32), t);
                            }
                        }
                        scores.push(s);
                    }
                } else {
                    if candidates.len() + 1 > N_SEQ_MAX as usize {
                        return Err("候補数が N_SEQ_MAX を超えています".into());
                    }
                    self.batch.clear();
                    let mut offsets = Vec::with_capacity(cand_tokens.len());
                    for (k, toks) in cand_tokens.iter().enumerate() {
                        let seq = (k + 1) as i32;
                        self.ctx.copy_kv_cache_seq(0, seq, None, None)?;
                        offsets.push(self.batch.n_tokens());
                        for (j, &t) in toks[..toks.len() - 1].iter().enumerate() {
                            self.batch.add(t, (p + j) as i32, &[seq], true)?;
                        }
                    }
                    if self.batch.n_tokens() > 0 {
                        self.ctx.decode(&mut self.batch)?;
                    }
                    for (k, toks) in cand_tokens.iter().enumerate() {
                        let mut s = log_prob(&prompt_logits, toks[0]);
                        for (j, &t) in toks[1..].iter().enumerate() {
                            s += log_prob(self.ctx.get_logits_ith(offsets[k] + j as i32), t);
                        }
                        scores.push(s);
                    }
                    // 次回の呼び出しに複製したシーケンスを残さない
                    for k in 0..cand_tokens.len() {
                        self.ctx.clear_kv_cache_seq(Some((k + 1) as u32), None, None)?;
                    }
                }
            }
        }
        Ok(scores)
    }
}

// ---------------------------------------------------------------------------
// 計測用の固定データ

const BENCH_LEFT_CONTEXT: &str = "先週の話ですが、";

/// 読みの長さ (かな数) ごとの生成の入力
const GEN_READINGS: [(usize, &str); 3] = [
    (5, "あめがふる"),
    (10, "きのうはあめがふった"),
    (20, "きのうはあめがふったのでいえでねていた"),
];

/// 順位付けの入力。候補は先頭 N 個を使う
const RANK_SETS: [(usize, &str, [&str; 10]); 2] = [
    (
        10,
        "きのうはあめがふった",
        [
            "昨日は雨が降った",
            "昨日は飴が降った",
            "機能は雨が降った",
            "昨日は雨が振った",
            "きのうは雨が降った",
            "昨日はあめが降った",
            "帰納は雨が降った",
            "昨日は天が降った",
            "昨日は雨がふった",
            "昨日は飴が振った",
        ],
    ),
    (
        20,
        "きのうはあめがふったのでいえでねていた",
        [
            "昨日は雨が降ったので家で寝ていた",
            "昨日は飴が降ったので家で寝ていた",
            "機能は雨が降ったので家で寝ていた",
            "昨日は雨が振ったので家で寝ていた",
            "昨日は雨が降ったので言えで寝ていた",
            "昨日は雨が降ったので家で練ていた",
            "昨日は雨が降ったので家で寝て居た",
            "きのうは雨が降ったので家で寝ていた",
            "昨日は雨がふったので家でねていた",
            "昨日は天が降ったので家で寝ていた",
        ],
    ),
];

// ---------------------------------------------------------------------------
// テストセットと統計変換

struct Case {
    left_context: String,
    reading: String,
    candidates: Vec<String>,
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
        let candidates: Vec<String> = f[2].split('|').map(str::to_string).collect();
        if !candidates.iter().any(|c| c == f[3]) {
            return Err(format!("{}:{}: 正解が候補に含まれていません", path.display(), no + 1).into());
        }
        cases.push(Case {
            left_context: f[0].to_string(),
            reading: f[1].to_string(),
            candidates,
            answer: f[3].to_string(),
        });
    }
    Ok(cases)
}

/// 起動中のエンジンから CONVERT の第1候補と CONVSEG の各文節第1候補の連結を得る
fn engine_one_best(readings: &[&str]) -> BoxResult<Vec<(String, String)>> {
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};

    let pipe = std::env::var("QUICKLIME_PIPE_NAME").map_err(|_| {
        "--engine には QUICKLIME_PIPE_NAME の指定が必要です (常用エンジンに接続しないため)"
    })?;
    let stream = Stream::connect(pipe.to_ns_name::<GenericNamespaced>()?)?;
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);
    let mut out = Vec::new();
    for kana in readings {
        send.write_all(format!("CONVERT\t{kana}\n").as_bytes())?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let convert = line.trim_end().split('\t').nth(1).unwrap_or("").to_string();

        send.write_all(format!("CONVSEG\t{kana}\n").as_bytes())?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let convseg: String = line
            .trim_end()
            .split('\t')
            .skip(1)
            .map(|seg| seg.split('\x1f').nth(1).unwrap_or(""))
            .collect();
        out.push((convert, convseg));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let with_vram = args.backend == Backend::Vulkan;
    let mem_start = mem_snapshot(with_vram);

    let backend = LlamaBackend::init()?;
    println!("## 条件\n");
    println!("- モデル: {}", args.model.display());
    println!(
        "- バックエンド: {}、スレッド数: {}",
        if args.backend == Backend::Vulkan { "vulkan" } else { "cpu" },
        args.threads
    );
    for d in llama_cpp_2::list_llama_ggml_backend_devices() {
        println!(
            "- デバイス: {} / {} ({}) 空き {} MB / 全体 {} MB",
            d.name,
            d.description,
            d.backend,
            d.memory_free / 1024 / 1024,
            d.memory_total / 1024 / 1024
        );
    }

    let n_gpu_layers = if args.backend == Backend::Vulkan { 999 } else { 0 };
    let mut model_params =
        std::pin::pin!(LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers));
    // zenz の GGUF は前処理の種類が gpt2-small-japanese-char で、上流の llama.cpp は読めない。
    // azooKey の llama.cpp フォーク (b4846) はこれを GPT-2 と同じ正規表現で扱っているだけなので、
    // メタデータを gpt-2 に上書きすれば同じトークン化になる
    let mut pre = [0 as std::os::raw::c_char; 128];
    for (d, s) in pre.iter_mut().zip(b"gpt-2") {
        *d = *s as std::os::raw::c_char;
    }
    model_params
        .as_mut()
        .append_kv_override(c"tokenizer.ggml.pre", ParamOverrideValue::Str(pre));

    // 読み込み時間はモデルの読み込み + コンテキスト作成。1回目の直後のメモリを常駐分として記録する。
    // 残りの反復は推論の計測後に行い、反復中の確保がプロセスのピークに混ざらないようにする
    let mut load_ms = Vec::new();
    let started = Instant::now();
    let model = LlamaModel::load_from_file(&backend, &args.model, &model_params)?;
    let mut llm = Llm::new(&model, &backend, &args)?;
    load_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    let loaded = mem_snapshot(with_vram);
    println!(
        "- パラメータ数: {}、ファイル上のサイズ: {} MB、語彙数: {}",
        model.n_params(),
        mb(model.size() as usize),
        model.n_vocab()
    );

    // トークン化の検算: プロンプトを分割して戻したとき元の文字列に戻るか
    let sample = build_prompt(BENCH_LEFT_CONTEXT, GEN_READINGS[1].1);
    let sample_tokens = llm.tokenize_prompt(&sample);
    let restored: Vec<u8> = sample_tokens
        .iter()
        .filter(|&&t| t != model.vocab().bos())
        .flat_map(|&t| model.vocab().token_to_piece(t, false, None))
        .collect();
    println!(
        "- トークン化の検算: BOS の付与 {}、トークン数 {} (文字数 {})、復元一致 {}",
        sample_tokens.first() == Some(&model.vocab().bos()),
        sample_tokens.len(),
        sample.chars().count(),
        String::from_utf8_lossy(&restored) == sample
    );

    // 初回推論はシェーダの準備などを含むため、ウォームアップ前に1回だけ別に測る
    let started = Instant::now();
    llm.generate(BENCH_LEFT_CONTEXT, GEN_READINGS[1].1)?;
    let first_infer = started.elapsed().as_secs_f64() * 1000.0;
    println!("\n- 初回の生成 (読み 10 かな): {first_infer:.1} ms");

    if args.mode == "bench" || args.mode == "all" {
        println!("\n## 生成 (貪欲法、左文脈「{BENCH_LEFT_CONTEXT}」)\n");
        println!("| 読み (かな) | 出力 | 中央値 (ms) | p95 (ms) | 最小 (ms) | 最大 (ms) |");
        println!("|---|---|---|---|---|---|");
        for (len, reading) in GEN_READINGS {
            let output = llm.generate(BENCH_LEFT_CONTEXT, reading)?;
            let s = measure(args.warmup, args.iters, || {
                llm.generate(BENCH_LEFT_CONTEXT, reading).map(|_| ())
            })?;
            println!(
                "| {len} | {output} | {:.2} | {:.2} | {:.2} | {:.2} |",
                s.median, s.p95, s.min, s.max
            );
        }

        println!("\n## 順位付け (候補ごとの対数尤度、左文脈「{BENCH_LEFT_CONTEXT}」)\n");
        println!("| 読み (かな) | N | KV キャッシュ | 中央値 (ms) | p95 (ms) | 最小 (ms) | 最大 (ms) |");
        println!("|---|---|---|---|---|---|---|");
        // 実装の検算として、使い回さない形との対数尤度の最大差と1位の不一致回数を方式ごとに数える。
        // 量子化モデルでは KV セル上の配置 (= 総和の順序) が変わるだけで値が少し動くため、
        // 完全一致はしない場合がある
        let modes = [RankMode::NoReuse, RankMode::ReuseSeq, RankMode::ReuseBatch];
        let mut max_diff = [0.0f64; 3];
        let mut top1_mismatch = [0usize; 3];
        let top1 = |v: &[f64]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, _)| k);
        for (len, reading, all) in RANK_SETS {
            for n in [5usize, 10] {
                let cands: Vec<String> = all[..n].iter().map(|s| s.to_string()).collect();
                let base = llm.rank(BENCH_LEFT_CONTEXT, reading, &cands, RankMode::NoReuse)?;
                for (m, mode) in modes.into_iter().enumerate() {
                    let scores = llm.rank(BENCH_LEFT_CONTEXT, reading, &cands, mode)?;
                    for (a, b) in base.iter().zip(&scores) {
                        max_diff[m] = max_diff[m].max((a - b).abs());
                    }
                    top1_mismatch[m] += usize::from(top1(&base) != top1(&scores));
                    let s = measure(args.warmup, args.iters, || {
                        llm.rank(BENCH_LEFT_CONTEXT, reading, &cands, mode).map(|_| ())
                    })?;
                    println!(
                        "| {len} | {n} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
                        mode.label(),
                        s.median,
                        s.p95,
                        s.min,
                        s.max
                    );
                }
            }
        }
        println!();
        for (m, mode) in modes.into_iter().enumerate().skip(1) {
            println!(
                "- 「{}」と「{}」の対数尤度の最大差: {:.4}、1位の不一致: {} / 4 条件",
                mode.label(),
                RankMode::NoReuse.label(),
                max_diff[m],
                top1_mismatch[m]
            );
        }
    }


    if args.mode == "accuracy" || args.mode == "all" {
        let path = args.cases.as_ref().ok_or("精度確認には --cases が必要です")?;
        let cases = load_cases(path)?;
        let stat = if args.engine {
            let readings: Vec<&str> = cases.iter().map(|c| c.reading.as_str()).collect();
            Some(engine_one_best(&readings)?)
        } else {
            None
        };

        println!("\n## 精度 ({} 件)\n", cases.len());
        println!("| # | 左文脈 | 読み | 正解 | 統計 1-best | LLM 順位1位 | LLM 生成 |");
        println!("|---|---|---|---|---|---|---|");
        // [統計, 順位, 生成] の正解数を、全体・左文脈あり・左文脈なしで数える
        let mut hits = [[0usize; 3]; 3];
        let mut totals = [0usize; 3];
        let mut convseg_mismatch = Vec::new();
        for (i, c) in cases.iter().enumerate() {
            let scores = llm.rank(&c.left_context, &c.reading, &c.candidates, RankMode::ReuseSeq)?;
            let best = scores
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(k, _)| k)
                .unwrap_or(0);
            let ranked = &c.candidates[best];
            let generated = llm.generate(&c.left_context, &c.reading)?;
            let stat_best = stat.as_ref().map(|s| s[i].0.clone());
            if let Some(s) = &stat
                && s[i].0 != s[i].1
            {
                convseg_mismatch.push(format!("{} ({} / {})", c.reading, s[i].0, s[i].1));
            }

            let ok = [
                stat_best.as_deref() == Some(c.answer.as_str()),
                *ranked == c.answer,
                generated == c.answer,
            ];
            let group = if c.left_context.is_empty() { 2 } else { 1 };
            for g in [0, group] {
                totals[g] += 1;
                for k in 0..3 {
                    hits[g][k] += ok[k] as usize;
                }
            }
            let mark = |s: &str, ok: bool| if ok { s.to_string() } else { format!("**{s}** (x)") };
            println!(
                "| {} | {} | {} | {} | {} | {} | {} |",
                i + 1,
                if c.left_context.is_empty() { "-" } else { &c.left_context },
                c.reading,
                c.answer,
                stat_best.as_deref().map_or("未計測".to_string(), |s| mark(s, ok[0])),
                mark(ranked, ok[1]),
                mark(&generated, ok[2]),
            );
        }

        println!("\n| 区分 | 件数 | 統計 1-best | LLM 順位1位 | LLM 生成 |");
        println!("|---|---|---|---|---|");
        for (g, name) in ["全体", "左文脈あり", "左文脈なし"].iter().enumerate() {
            let pct = |h: usize| {
                if totals[g] == 0 {
                    "-".to_string()
                } else {
                    format!("{}/{} ({:.0}%)", h, totals[g], 100.0 * h as f64 / totals[g] as f64)
                }
            };
            println!(
                "| {name} | {} | {} | {} | {} |",
                totals[g],
                if stat.is_some() { pct(hits[g][0]) } else { "未計測".to_string() },
                pct(hits[g][1]),
                pct(hits[g][2])
            );
        }
        if !convseg_mismatch.is_empty() {
            println!("\n- CONVERT と CONVSEG の 1-best が異なった読み: {}", convseg_mismatch.join("、"));
        }
    }

    // 推論 (計測と精度確認) の後のメモリ。プロセスのピークは初回の読み込みと推論の分だけを含む
    let mem_after = mem_snapshot(with_vram);
    println!("\n## メモリ (MB)\n");
    println!("| 時点 | ワーキングセット | プライベート (コミット) | GPU 専用メモリ |");
    println!("|---|---|---|---|");
    let vram = |v: Option<u64>| v.map_or("未計測".to_string(), |b| mb(b as usize));
    println!(
        "| 開始時 (バックエンド初期化前) | {} | {} | {} |",
        mb(mem_start.working_set),
        mb(mem_start.private),
        vram(mem_start.vram)
    );
    println!(
        "| 初回の読み込み直後 (常駐) | {} | {} | {} |",
        mb(loaded.working_set),
        mb(loaded.private),
        vram(loaded.vram)
    );
    println!(
        "| 推論の後 | {} | {} | {} |",
        mb(mem_after.working_set),
        mb(mem_after.private),
        vram(mem_after.vram)
    );
    println!(
        "| プロセスのピーク | {} | {} | - |",
        mb(mem_after.peak_working_set),
        mb(mem_after.peak_private)
    );

    drop(llm);
    drop(model);
    for _ in 1..args.load_iters {
        let started = Instant::now();
        let model = LlamaModel::load_from_file(&backend, &args.model, &model_params)?;
        let llm = Llm::new(&model, &backend, &args)?;
        load_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        drop(llm);
    }
    let first_load = load_ms[0];
    let load_stats = stats(&mut load_ms.clone());
    println!("\n## 読み込み時間 (モデル読み込み + コンテキスト作成)\n");
    println!("| 回数 | 1回目 (ms) | 中央値 (ms) | 最大 (ms) |");
    println!("|---|---|---|---|");
    println!(
        "| {} | {:.1} | {:.1} | {:.1} |",
        load_ms.len(),
        first_load,
        load_stats.median,
        load_stats.max
    );
    Ok(())
}
