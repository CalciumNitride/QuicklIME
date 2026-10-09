// quicklime-llm: zenz 系かな漢字変換モデルで変換候補の対数尤度を求める子プロセス
// (docs/design/llm-rerank.md)。エンジン (quicklime-engine) が起動し、標準入出力の行プロトコルで使う。
//
// 起動: quicklime-llm.exe --model <gguf> --threads <N>
//   読み込みと試しの推論に成功したら READY、失敗したら ERR\t<メッセージ> を出して終了する
// 要求: SCORE\t<文脈>\t<読み>\t<候補1>\t<候補2>...   (文脈・読みはエンジンで前処理済み)
// 応答: OK\t<対数尤度1>\t<対数尤度2>... / ERR\t<メッセージ>
// 標準入力が閉じたら終了する。llama.cpp のログは標準エラーに出る
//
// プロンプトの書式・前処理・推論の方式 (候補を1バッチ) は計測 (engine/examples/llm_bench.rs) と同じ

use std::io::{BufRead, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;

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
/// 候補を複数シーケンスに並べて1回で評価するため、最大候補数 + 1 (プロンプト) を確保する
const N_SEQ_MAX: u32 = 16;

/// READY の前に行う試しの推論の入力 (engine/examples/llm_bench.rs の計測用データと同じ)。
/// Vulkan 版はドライバのシェーダキャッシュが無いと最初の推論に数秒かかるため、その時間を
/// エンジンの依頼の応答待ち (LLM_TIMEOUT_MS) に持ち込まないよう、読み込みの一部として済ませる
const WARMUP_CONTEXT: &str = "先週の話ですが、";
const WARMUP_READING: &str = "キノウハアメガフッタ";
/// 候補1件と、エンジンの RERANK_TOP と同じ 10 件で試す
const WARMUP_CANDIDATES: [&str; 10] = [
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
];

/// 3つめの試し: RERANK_TOP 件の各候補を長くし、プロンプトと候補の合計を N_CTX 近くにする。
/// シェーダは処理の大きさの区分ごとに用意されるので、短い入力だけでは長い入力の初回が遅いまま残る
/// (計測で 16 秒)。読みは基の読みを2回、候補は基の候補を繰り返して同じ文字数に切り、
/// N_CTX に収まる最大の文字数にする (llm_bench の 20 かなの計測用データ)
const WARMUP_LONG_READING: &str = "キノウハアメガフッタノデイエデネテイタ";
const WARMUP_LONG_CANDIDATES: [&str; 10] = [
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
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Vulkan,
}

struct Args {
    model: PathBuf,
    threads: i32,
}

fn parse_args() -> BoxResult<Args> {
    let mut model = None;
    let mut threads = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--model" => model = Some(PathBuf::from(value()?)),
            "--threads" => threads = Some(value()?.parse()?),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    Ok(Args {
        model: model.ok_or("--model が必要です")?,
        threads: threads.ok_or("--threads が必要です")?,
    })
}

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

/// logits 上での token の対数確率 (log softmax)
fn log_prob(logits: &[f32], token: LlamaToken) -> f64 {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let sum: f64 = logits.iter().map(|&l| (l as f64 - max).exp()).sum();
    logits[token.0 as usize] as f64 - max - sum.ln()
}

struct Scorer<'a> {
    model: &'a LlamaModel,
    ctx: LlamaContext<'a>,
    batch: LlamaBatch<'static>,
}

impl<'a> Scorer<'a> {
    fn new(model: &'a LlamaModel, backend: &LlamaBackend, threads: i32) -> BoxResult<Self> {
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(N_CTX))
            .with_n_batch(N_CTX)
            .with_n_ubatch(N_CTX)
            .with_n_seq_max(N_SEQ_MAX)
            .with_kv_unified(true)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let ctx = model.new_context(backend, params)?;
        Ok(Self { model, ctx, batch: LlamaBatch::new(N_CTX as usize, N_SEQ_MAX as i32) })
    }

    /// score が KV キャッシュに置くトークン数 (プロンプト + 各候補の EOS を除くトークン)
    fn kv_tokens(&self, left_context: &str, reading: &str, candidates: &[&str]) -> usize {
        let vocab = self.model.vocab();
        let prompt = vocab.tokenize(build_prompt(left_context, reading).as_bytes(), true, false).len();
        let cands: usize =
            candidates.iter().map(|c| vocab.tokenize(preprocess(c).as_bytes(), false, false).len()).sum();
        prompt + cands
    }

    /// 3つめの試しの入力: 候補を同じ文字数に切りそろえ、N_CTX に収まる最大の文字数にする
    fn long_warmup_input(&self) -> (String, Vec<String>) {
        let reading = WARMUP_LONG_READING.repeat(2);
        let repeated: Vec<Vec<char>> =
            WARMUP_LONG_CANDIDATES.iter().map(|c| c.repeat(4).chars().collect()).collect();
        let max_len = repeated.iter().map(Vec::len).min().unwrap_or(0);
        let mut best: Vec<String> = Vec::new();
        for len in 1..=max_len {
            let candidates: Vec<String> = repeated.iter().map(|c| c[..len].iter().collect()).collect();
            let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
            if self.kv_tokens(WARMUP_CONTEXT, &reading, &refs) > N_CTX as usize {
                break;
            }
            best = candidates;
        }
        (reading, best)
    }

    /// 各候補の対数尤度 (候補の全トークン + 出力終端の EOS) を返す。プロンプトを1回だけ評価し、
    /// そのキャッシュを候補数分のシーケンスへ複製して、全候補を1回の decode で評価する
    fn score(&mut self, left_context: &str, reading: &str, candidates: &[&str]) -> BoxResult<Vec<f64>> {
        if candidates.len() + 1 > N_SEQ_MAX as usize {
            return Err("候補が多すぎます".into());
        }
        // azooKey と同じく BOS を付け、特殊トークンの解釈はしない
        let prompt = self.model.vocab().tokenize(build_prompt(left_context, reading).as_bytes(), true, false);
        let eos = self.model.vocab().eos();
        let cand_tokens: Vec<Vec<LlamaToken>> = candidates
            .iter()
            .map(|c| {
                let mut t = self.model.vocab().tokenize(preprocess(c).as_bytes(), false, false);
                t.push(eos);
                t
            })
            .collect();
        let p = prompt.len();
        // 最後の EOS の次の予測は不要なので、EOS 自体はバッチに入れない
        let batch_tokens: usize = cand_tokens.iter().map(|t| t.len() - 1).sum();
        // KV キャッシュはシーケンス間で共有 (kv_unified) するため、プロンプトと全候補が収まる必要がある
        if p == 0 || p + batch_tokens > N_CTX as usize {
            return Err("入力が長すぎます".into());
        }

        self.ctx.clear_kv_cache();
        self.batch.clear();
        for (i, &t) in prompt.iter().enumerate() {
            self.batch.add(t, i as i32, &[0], i == p - 1)?;
        }
        self.ctx.decode(&mut self.batch)?;
        let prompt_logits = self.ctx.get_logits_ith((p - 1) as i32).to_vec();

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
        let mut scores = Vec::with_capacity(cand_tokens.len());
        for (k, toks) in cand_tokens.iter().enumerate() {
            let mut s = log_prob(&prompt_logits, toks[0]);
            for (j, &t) in toks[1..].iter().enumerate() {
                s += log_prob(self.ctx.get_logits_ith(offsets[k] + j as i32), t);
            }
            scores.push(s);
        }
        // 次の要求に複製したシーケンスを残さない
        for k in 0..cand_tokens.len() {
            self.ctx.clear_kv_cache_seq(Some((k + 1) as u32), None, None)?;
        }
        Ok(scores)
    }
}

/// 1行の要求を処理して応答の行 (改行なし) を返す
fn handle_line(scorer: &mut Scorer, line: &str) -> String {
    let mut fields = line.split('\t');
    if fields.next() != Some("SCORE") {
        return "ERR\t不明なコマンドです".to_string();
    }
    let (Some(context), Some(reading)) = (fields.next(), fields.next()) else {
        return "ERR\t引数が足りません".to_string();
    };
    let candidates: Vec<&str> = fields.collect();
    match scorer.score(context, reading, &candidates) {
        Ok(scores) => {
            let mut response = "OK".to_string();
            for s in scores {
                response.push('\t');
                response.push_str(&s.to_string());
            }
            response
        }
        Err(e) => format!("ERR\t{}", e.to_string().replace(['\t', '\r', '\n'], " ")),
    }
}

fn load(backend: Backend, args: &Args, llama: &LlamaBackend) -> BoxResult<LlamaModel> {
    if backend == Backend::Vulkan
        && !llama_cpp_2::list_llama_ggml_backend_devices()
            .iter()
            .any(|d| d.backend.eq_ignore_ascii_case("vulkan"))
    {
        // デバイスが無くても読み込み自体は CPU で通ってしまうため、ここで失敗にして CPU 版へ譲る
        return Err("Vulkan のデバイスがありません".into());
    }
    let n_gpu_layers = if backend == Backend::Vulkan { 999 } else { 0 };
    let mut model_params = std::pin::pin!(LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers));
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
    Ok(LlamaModel::load_from_file(llama, &args.model, &model_params)?)
}

fn reply(out: &mut impl Write, line: &str) -> bool {
    out.write_all(line.as_bytes()).is_ok() && out.write_all(b"\n").is_ok() && out.flush().is_ok()
}

pub fn run(backend: Backend) -> ExitCode {
    let mut out = std::io::stdout().lock();
    let fail = |out: &mut std::io::StdoutLock, e: &dyn std::fmt::Display| {
        // メッセージにタブ・改行が混ざると行プロトコルが崩れるため空白にする
        let message = e.to_string().replace(['\t', '\r', '\n'], " ");
        reply(out, &format!("ERR\t{message}"));
        ExitCode::FAILURE
    };
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => return fail(&mut out, &e),
    };
    let llama = match LlamaBackend::init() {
        Ok(llama) => llama,
        Err(e) => return fail(&mut out, &e),
    };
    let model = match load(backend, &args, &llama) {
        Ok(model) => model,
        Err(e) => return fail(&mut out, &e),
    };
    let mut scorer = match Scorer::new(&model, &llama, args.threads) {
        Ok(scorer) => scorer,
        Err(e) => return fail(&mut out, &e),
    };
    let (long_reading, long_candidates) = scorer.long_warmup_input();
    let long_refs: Vec<&str> = long_candidates.iter().map(String::as_str).collect();
    let warmups: [(&str, &[&str]); 3] = [
        (WARMUP_READING, &WARMUP_CANDIDATES[..1]),
        (WARMUP_READING, &WARMUP_CANDIDATES),
        (&long_reading, &long_refs),
    ];
    for (reading, candidates) in warmups {
        if let Err(e) = scorer.score(WARMUP_CONTEXT, reading, candidates) {
            return fail(&mut out, &format!("試しの推論に失敗しました: {e}"));
        }
    }
    eprintln!(
        "quicklime-llm: 試しの推論 (長い入力) は候補 {} 件 x {} 文字、KV {} / {} トークン",
        long_refs.len(),
        long_refs.first().map_or(0, |c| c.chars().count()),
        scorer.kv_tokens(WARMUP_CONTEXT, &long_reading, &long_refs),
        N_CTX
    );
    if !reply(&mut out, "READY") {
        return ExitCode::FAILURE;
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let response = handle_line(&mut scorer, line.trim_end_matches('\r'));
        if !reply(&mut out, &response) {
            break;
        }
    }
    ExitCode::SUCCESS
}
