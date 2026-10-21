// 実験 0c: LLM による候補の生成の計測 (docs/design/experiment-0c-llm-gen.md)
//
// テストセットの各件を、辞書をプロセス内で読み込んだ統計変換 (convert_nbest_rerank) と、プロセス内の
// llama.cpp で次の方式にかけ、精度・速さ・副作用を標準出力へ Markdown の表で書く。
// llama.cpp のログは標準エラーに出る。
//   - 並べ替えだけ: N-best の上位 10 件を製品 (quicklime-llm) と同じプロンプト・尤度で並べ替える
//   - A 貪欲法・A beam: zenz に読みから表記を生成させ、後処理で検査する
//   - C: 統計変換をドラフトにして LLM の予測と照合し、ずれたら表記の接頭辞の制約を付けた Viterbi で
//     ドラフトを作り直す (azooKey の Zenzai 方式)
//   - A+C: A 貪欲法の出力を使い、後処理で捨てたときと読みから外れたときは C の結果に代える
// 生成の方式の結果は、並べ替えの結果の前に置いた並び (合わせた並び) でも数える。
//
// 使い方:
//   cargo run --release --example llm_gen_eval --features llm-bench -- \
//     --model ../models/zenz-v3.2-xsmall-Q5_K_M.gguf --backend cpu --threads 4 \
//     --cases ../references/AJIMEE-Bench/ajimee_cases.tsv
//   (Vulkan は --features llm-bench-vulkan と --backend vulkan)
//
// オプション:
//   --threads N    llama.cpp のスレッド数 (既定 4。製品の子プロセスと同じ上限)
//   --limit N      C の評価の回数の上限 (既定 10)
//   --methods a,b  測る方式 (rerank,a-greedy,a-beam,c,a+c。既定は全部)。合わせた並びの土台になるので、
//                  並べ替えは指定によらず常に行う。a+c は代えるかによらず A 貪欲法と C を全件で求める
//   --no-cases     件ごとの表を出さない
//   --rerank-eos E 並べ替えの尤度の終わりの項 (2: 語彙の EOS、3: </s> = 製品と同じ、none: 足さない。既定 3)
//   --rerank-min-chars N  並べ替える読みの最小文字数 (既定は製品の RERANK_MIN_CHARS。0 で全件)
//
// 辞書はエンジンと同じ規則で探す (QUICKLIME_DICT_DIR、無ければ開発レイアウトの Mozc 辞書)。
// 学習・ユーザ辞書は空、前文脈 (統計変換の直前文節) は無し

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
#[path = "../src/llm.rs"]
mod llm;
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
#[path = "../src/typo.rs"]
mod typo;
#[allow(dead_code)]
#[path = "../src/userdict.rs"]
mod userdict;

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

use convert::{NBest, SentenceCandidate};
use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

// プロンプトの区切り (AzooKeyKanaKanjiConverter の ZenzPromptBuilder.swift、zenz-v3 の書式)
const INPUT_TAG: char = '\u{EE00}';
const OUTPUT_TAG: char = '\u{EE01}';
const CONTEXT_TAG: char = '\u{EE02}';
/// 左文脈は末尾 40 文字だけを使う (azooKey の既定値)
const MAX_LEFT_CONTEXT: usize = 40;

/// 並べ替えのコンテキストは製品 (quicklime-llm) と同じ大きさにする。プロンプトと候補が収まらない
/// 件は製品と同じく並べ替えない
const RERANK_N_CTX: u32 = 512;
/// 生成と照合のコンテキスト。AJIMEE-Bench の長い読み (100 かな超) では、beam の 3 本の仮説が
/// 生成の上限まで伸びると 512 に収まらないため大きくとる
const GEN_N_CTX: u32 = 1024;
/// 候補を複数シーケンスに並べて1回で評価するため、最大候補数 + 1 (プロンプト) を確保する。
/// beam は仮説の数 x 2 (作り直す前と後) を使う
const N_SEQ_MAX: u32 = 16;
const BEAM_WIDTH: usize = 3;

/// 生成のしきい値の格子で試す、生成をかける読みの最小文字数
const GEN_MIN_CHARS_GRID: [usize; 7] = [0, 3, 5, 7, 9, 12, 15];

/// 読みの長さ別の区切り (速さの表)
const LENGTH_BUCKETS: [(&str, usize, usize); 3] = [("9 未満", 0, 9), ("9〜19", 9, 20), ("20 以上", 20, usize::MAX)];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Cpu,
    Vulkan,
}

/// 並べ替えの尤度で、候補のトークンの後ろに足す終わりの項
#[derive(Clone, Copy, PartialEq, Eq)]
enum RerankEos {
    /// 語彙の EOS (zenz では ID 2 の <s>)
    VocabEos,
    /// zenz が出力の終わりに出す </s>。製品と同じ
    CloseTag,
    None,
}

impl RerankEos {
    fn label(self) -> &'static str {
        match self {
            RerankEos::VocabEos => "2",
            RerankEos::CloseTag => "3",
            RerankEos::None => "none",
        }
    }
}

struct Args {
    model: PathBuf,
    backend: Backend,
    threads: i32,
    cases: PathBuf,
    limit: usize,
    greedy: bool,
    beam: bool,
    c: bool,
    ac: bool,
    no_cases: bool,
    rerank_eos: RerankEos,
    rerank_min_chars: usize,
}

fn parse_args() -> BoxResult<Args> {
    let mut model = None;
    let mut backend = Backend::Cpu;
    let mut threads = 4;
    let mut cases = None;
    let mut limit = 10;
    let mut methods =
        vec!["rerank".to_string(), "a-greedy".to_string(), "a-beam".to_string(), "c".to_string(), "a+c".to_string()];
    let mut no_cases = false;
    let mut rerank_eos = RerankEos::CloseTag;
    let mut rerank_min_chars = llm::RERANK_MIN_CHARS;
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
            "--threads" => threads = value()?.parse()?,
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--limit" => limit = value()?.parse()?,
            "--methods" => methods = value()?.split(',').map(str::to_string).collect(),
            "--no-cases" => no_cases = true,
            "--rerank-eos" => {
                rerank_eos = match value()?.as_str() {
                    "2" => RerankEos::VocabEos,
                    "3" => RerankEos::CloseTag,
                    "none" => RerankEos::None,
                    other => return Err(format!("未対応の --rerank-eos: {other}").into()),
                }
            }
            "--rerank-min-chars" => rerank_min_chars = value()?.parse()?,
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    for m in &methods {
        if !["rerank", "a-greedy", "a-beam", "c", "a+c"].contains(&m.as_str()) {
            return Err(format!("不明な方式: {m}").into());
        }
    }
    if backend == Backend::Vulkan && !cfg!(feature = "llm-bench-vulkan") {
        return Err("--backend vulkan には feature llm-bench-vulkan でのビルドが必要です".into());
    }
    let has = |m: &str| methods.iter().any(|x| x == m);
    Ok(Args {
        model: model.ok_or("--model が必要です")?,
        backend,
        threads,
        cases: cases.ok_or("--cases が必要です")?,
        limit: limit.max(1),
        greedy: has("a-greedy"),
        beam: has("a-beam"),
        c: has("c"),
        ac: has("a+c"),
        no_cases,
        rerank_eos,
        rerank_min_chars,
    })
}

// ---------------------------------------------------------------------------
// テストセット

struct Case {
    left_context: String,
    reading: String,
    /// 正解の列そのまま (表示用)
    answer: String,
    answers: Vec<String>,
}

impl Case {
    fn is_answer(&self, surface: &str) -> bool {
        self.answers.iter().any(|a| a == surface)
    }

    /// 候補の表記の列で、最初に正解のどれかに当たった順位
    fn rank(&self, surfaces: &[String]) -> Option<usize> {
        surfaces.iter().position(|s| self.is_answer(s))
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

// ---------------------------------------------------------------------------
// 統計変換

/// example の中の統計変換 (前文脈なし・学習とユーザ辞書は空)
struct Converter {
    dictionary: Dictionary,
    user: UserDict,
    matrix: ConnectionMatrix,
    functional: FunctionalIds,
    learning: LearningStore,
}

impl Converter {
    /// 辞書をエンジンと同じ規則で読み込む (nbest_eval と同じ)
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
        Ok(Converter { dictionary, user: UserDict::empty(), matrix, functional, learning: LearningStore::in_memory() })
    }

    /// RERANK と同じ入力全体の候補。補正は設定の既定 (有効) で、打鍵が無いのでかなを戻した近似
    fn nbest(&self, kana: &str) -> NBest {
        convert::convert_nbest_rerank(
            kana, None, &self.dictionary, &self.user, &self.matrix, &self.functional, &self.learning,
            convert::TypoSource::Kana, &typo::TUNING,
        )
    }

    /// 表記の接頭辞 (exact なら表記全体) の制約を満たす最小コストの経路の表記
    fn constrained(&self, kana: &str, prefix: &[u8], exact: bool) -> Option<String> {
        convert::constrained_path(
            kana, None, &self.dictionary, &self.user, &self.matrix, &self.functional, &self.learning, prefix, exact,
        )
        .map(|segments| segments.into_iter().map(|(_, surface)| surface).collect())
    }
}

// ---------------------------------------------------------------------------
// プロンプトと推論 (llm_bench.rs・quicklime-llm と同じ書式)

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

/// logits 上での全トークンの対数確率 (log softmax)
fn log_softmax(logits: &[f32]) -> Vec<f64> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let sum: f64 = logits.iter().map(|&l| (l as f64 - max).exp()).sum();
    let norm = max + sum.ln();
    logits.iter().map(|&l| l as f64 - norm).collect()
}

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

/// 対数確率の高い順に k 個の (トークン, 対数確率)。同点はトークン ID の小さい順
fn top_k(logits: &[f32], k: usize) -> Vec<(LlamaToken, f64)> {
    let lp = log_softmax(logits);
    let mut idx: Vec<usize> = (0..lp.len()).collect();
    idx.sort_by(|&a, &b| lp[b].total_cmp(&lp[a]).then(a.cmp(&b)));
    idx.into_iter().take(k).map(|i| (LlamaToken(i as i32), lp[i])).collect()
}

/// 生成の 1 件
struct Generated {
    text: String,
    /// EOS を出さずに上限まで続いた
    limit: bool,
}

/// C の照合の 1 回の結果
enum Verdict {
    Pass,
    /// 新しい制約 (表記の接頭辞のバイト列, 完全一致か)
    Fix(Vec<u8>, bool),
}

struct Llm<'a> {
    model: &'a LlamaModel,
    ctx: LlamaContext<'a>,
    batch: LlamaBatch<'static>,
    n_ctx: u32,
    /// 並べ替えの尤度の終わりの項に使うトークン (None は足さない)
    rerank_end: Option<LlamaToken>,
}

impl<'a> Llm<'a> {
    fn new(model: &'a LlamaModel, backend: &LlamaBackend, threads: i32, n_ctx: u32) -> BoxResult<Self> {
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(n_ctx)
            .with_n_ubatch(n_ctx)
            .with_n_seq_max(N_SEQ_MAX)
            .with_kv_unified(true)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let ctx = model.new_context(backend, params)?;
        Ok(Self { model, ctx, batch: LlamaBatch::new(n_ctx as usize, N_SEQ_MAX as i32), n_ctx, rerank_end: None })
    }

    fn tokenize_prompt(&self, prompt: &str) -> Vec<LlamaToken> {
        // azooKey と同じく BOS を付け、特殊トークンの解釈はしない
        self.model.vocab().tokenize(prompt.as_bytes(), true, false)
    }

    fn tokenize_text(&self, text: &str) -> Vec<LlamaToken> {
        self.model.vocab().tokenize(preprocess(text).as_bytes(), false, false)
    }

    fn piece(&self, token: LlamaToken) -> Vec<u8> {
        self.model.vocab().token_to_piece(token, false, None)
    }

    fn is_eog(&self, token: LlamaToken) -> bool {
        self.model.vocab().is_eog(token)
    }

    /// キャッシュを捨ててプロンプトをシーケンス 0 に置き、最後の位置の logits を返す
    fn decode_prompt(&mut self, prompt: &[LlamaToken]) -> BoxResult<Vec<f32>> {
        self.ctx.clear_kv_cache();
        self.batch.clear();
        let last = prompt.len() - 1;
        for (i, &t) in prompt.iter().enumerate() {
            self.batch.add(t, i as i32, &[0], i == last)?;
        }
        self.ctx.decode(&mut self.batch)?;
        Ok(self.ctx.get_logits_ith(last as i32).to_vec())
    }

    /// 並べ替えの尤度の終わりの項のトークン。</s> は語彙の EOS (<s>) とは別の EOG トークンで、
    /// llama.cpp の API に特殊トークンとして引く手段が無いため、語彙の文字列から引く
    fn resolve_rerank_end(&self, eos: RerankEos) -> BoxResult<Option<LlamaToken>> {
        let vocab = self.model.vocab();
        Ok(match eos {
            RerankEos::VocabEos => Some(vocab.eos()),
            RerankEos::CloseTag => Some(
                vocab
                    .tokens()
                    .find(|&t| vocab.text(t).is_some_and(|s| s.to_bytes() == b"</s>"))
                    .ok_or("語彙に </s> がありません")?,
            ),
            RerankEos::None => None,
        })
    }

    /// 製品の並べ替え (quicklime-llm の Scorer::score) と同じ計算の、各候補の対数尤度。
    /// 終わりの項は rerank_end (既定は製品と同じ </s>)。
    /// プロンプトと候補がコンテキストに収まらなければ None (製品では ERR になり並べ替えない)
    fn score(&mut self, left_context: &str, reading: &str, candidates: &[String]) -> BoxResult<Option<Vec<f64>>> {
        if candidates.len() + 1 > N_SEQ_MAX as usize {
            return Err("候補が多すぎます".into());
        }
        let prompt = self.tokenize_prompt(&build_prompt(left_context, reading));
        let end = self.rerank_end;
        let cand_tokens: Vec<Vec<LlamaToken>> = candidates
            .iter()
            .map(|c| {
                let mut t = self.tokenize_text(c);
                t.extend(end);
                t
            })
            .collect();
        let p = prompt.len();
        // 収まるかは終わりの項によらず製品と同じ (候補のトークン数) で判定し、並べ替える件を条件間でそろえる
        let text_tokens: usize = cand_tokens.iter().map(|t| t.len() - usize::from(end.is_some())).sum();
        if p == 0 || p + text_tokens > self.n_ctx as usize {
            return Ok(None);
        }
        let prompt_logits = self.decode_prompt(&prompt)?;
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
        for k in 0..cand_tokens.len() {
            self.ctx.clear_kv_cache_seq(Some((k + 1) as u32), None, None)?;
        }
        Ok(Some(scores))
    }

    /// 貪欲法 (llm_bench の generate と同じ)。EOS か上限 (読みの文字数 x 2 + 8 トークン) で止める
    fn greedy(&mut self, left_context: &str, reading: &str) -> BoxResult<Generated> {
        let prompt = self.tokenize_prompt(&build_prompt(left_context, reading));
        let max_new = reading.chars().count() * 2 + 8;
        let mut logits = self.decode_prompt(&prompt)?;
        let mut pos = prompt.len() as i32;
        let mut out = Vec::new();
        for _ in 0..max_new {
            let next = argmax(&logits);
            if self.is_eog(next) {
                return Ok(Generated { text: String::from_utf8_lossy(&out).into_owned(), limit: false });
            }
            out.extend(self.piece(next));
            self.batch.clear();
            self.batch.add(next, pos, &[0], true)?;
            self.ctx.decode(&mut self.batch)?;
            logits = self.ctx.get_logits_ith(0).to_vec();
            pos += 1;
        }
        Ok(Generated { text: String::from_utf8_lossy(&out).into_owned(), limit: true })
    }

    /// beam 幅 BEAM_WIDTH の beam search。EOS で終わった仮説を累積の対数確率の高い順に返し、上限まで
    /// 終わらなかった仮説は limit を立てて後ろに付ける。仮説はシーケンス 1〜3 と 4〜6 を交互に使い、
    /// 親のキャッシュを複製してから新しいトークンを 1 回の decode で足す
    fn beam(&mut self, left_context: &str, reading: &str) -> BoxResult<Vec<Generated>> {
        struct Hyp {
            tokens: Vec<LlamaToken>,
            score: f64,
            finished: bool,
            seq: i32,
            /// 次のトークンの候補 (上位 BEAM_WIDTH 個)。終わった仮説では空
            next: Vec<(LlamaToken, f64)>,
        }
        let prompt = self.tokenize_prompt(&build_prompt(left_context, reading));
        let p = prompt.len();
        let max_new = reading.chars().count() * 2 + 8;
        let root = self.decode_prompt(&prompt)?;
        let mut hyps =
            vec![Hyp { tokens: Vec::new(), score: 0.0, finished: false, seq: 0, next: top_k(&root, BEAM_WIDTH) }];
        let width = BEAM_WIDTH as i32;
        let mut bank = 1;
        for step in 0..max_new {
            // (累積の対数確率, 親, 足すトークン)。足すトークンが None は終わった仮説をそのまま残す
            let mut expansions: Vec<(f64, usize, Option<LlamaToken>)> = Vec::new();
            for (h, hyp) in hyps.iter().enumerate() {
                if hyp.finished {
                    expansions.push((hyp.score, h, None));
                } else {
                    for &(t, lp) in &hyp.next {
                        expansions.push((hyp.score + lp, h, Some(t)));
                    }
                }
            }
            expansions.sort_by(|a, b| b.0.total_cmp(&a.0));
            expansions.truncate(BEAM_WIDTH);
            let mut next_hyps: Vec<Hyp> = Vec::new();
            let mut to_decode: Vec<(usize, LlamaToken)> = Vec::new();
            for &(score, parent, token) in &expansions {
                let base = &hyps[parent];
                match token {
                    None => next_hyps.push(Hyp {
                        tokens: base.tokens.clone(),
                        score,
                        finished: true,
                        seq: -1,
                        next: Vec::new(),
                    }),
                    Some(t) if self.is_eog(t) => next_hyps.push(Hyp {
                        tokens: base.tokens.clone(),
                        score,
                        finished: true,
                        seq: -1,
                        next: Vec::new(),
                    }),
                    Some(t) => {
                        let mut tokens = base.tokens.clone();
                        tokens.push(t);
                        to_decode.push((next_hyps.len(), t));
                        next_hyps.push(Hyp { tokens, score, finished: false, seq: base.seq, next: Vec::new() });
                    }
                }
            }
            let last_step = step + 1 == max_new;
            if !to_decode.is_empty() && !last_step {
                // 親のシーケンスを新しい側のシーケンスへ複製してから、古い側を消す
                let new_base = bank * width + 1;
                for (k, &(h, _)) in to_decode.iter().enumerate() {
                    let seq = new_base + k as i32;
                    self.ctx.clear_kv_cache_seq(Some(seq as u32), None, None)?;
                    self.ctx.copy_kv_cache_seq(next_hyps[h].seq, seq, None, None)?;
                }
                let old_base = (1 - bank) * width + 1;
                for k in 0..width {
                    self.ctx.clear_kv_cache_seq(Some((old_base + k) as u32), None, None)?;
                }
                self.batch.clear();
                for (k, &(h, t)) in to_decode.iter().enumerate() {
                    let seq = new_base + k as i32;
                    next_hyps[h].seq = seq;
                    let pos = (p + next_hyps[h].tokens.len() - 1) as i32;
                    self.batch.add(t, pos, &[seq], true)?;
                }
                self.ctx.decode(&mut self.batch)?;
                for (k, &(h, _)) in to_decode.iter().enumerate() {
                    next_hyps[h].next = top_k(self.ctx.get_logits_ith(k as i32), BEAM_WIDTH);
                }
                bank = 1 - bank;
            }
            hyps = next_hyps;
            if hyps.iter().all(|h| h.finished) {
                break;
            }
        }
        for k in 1..=2 * width {
            self.ctx.clear_kv_cache_seq(Some(k as u32), None, None)?;
        }
        let mut finished: Vec<&Hyp> = hyps.iter().filter(|h| h.finished).collect();
        finished.sort_by(|a, b| b.score.total_cmp(&a.score));
        let text = |h: &Hyp| {
            let bytes: Vec<u8> = h.tokens.iter().flat_map(|&t| self.piece(t)).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let mut out: Vec<Generated> = finished.iter().map(|h| Generated { text: text(h), limit: false }).collect();
        out.extend(hyps.iter().filter(|h| !h.finished).map(|h| Generated { text: text(h), limit: true }));
        Ok(out)
    }

    /// ドラフトを照合する。プロンプト (シーケンス 0 の先頭 prompt_len トークン、最後の logits が
    /// prompt_logits) より後ろのキャッシュだけを作り直し、ドラフトのトークンを 1 回の decode で評価する。
    /// 各位置の argmax がドラフトの次のトークン (最後は EOS) と一致すれば合格
    fn evaluate(&mut self, prompt_len: usize, prompt_logits: &[f32], draft: &str) -> BoxResult<Verdict> {
        let toks = self.tokenize_text(draft);
        self.ctx.clear_kv_cache_seq(Some(0), Some(prompt_len as u32), None)?;
        if !toks.is_empty() {
            self.batch.clear();
            for (j, &t) in toks.iter().enumerate() {
                self.batch.add(t, (prompt_len + j) as i32, &[0], true)?;
            }
            self.ctx.decode(&mut self.batch)?;
        }
        let mut prefix: Vec<u8> = Vec::new();
        for (j, &expected) in toks.iter().enumerate() {
            let top = if j == 0 { argmax(prompt_logits) } else { argmax(self.ctx.get_logits_ith((j - 1) as i32)) };
            if top == expected {
                prefix.extend(self.piece(expected));
                continue;
            }
            if self.is_eog(top) {
                return Ok(Verdict::Fix(prefix, true));
            }
            prefix.extend(self.piece(top));
            return Ok(Verdict::Fix(prefix, false));
        }
        // zenz は出力の終わりに </s> (語彙の EOS の <s> とは別の EOG トークン) を出すので、EOS の位置は
        // 生成の停止 (A) と同じく EOG のどれかを argmax にしていれば合格とする
        let top = if toks.is_empty() { argmax(prompt_logits) } else { argmax(self.ctx.get_logits_ith((toks.len() - 1) as i32)) };
        if !self.is_eog(top) {
            prefix.extend(self.piece(top));
            return Ok(Verdict::Fix(prefix, false));
        }
        Ok(Verdict::Pass)
    }
}

// ---------------------------------------------------------------------------
// A の後処理と副作用

#[derive(Clone, Copy, PartialEq, Eq)]
enum Discard {
    /// 上限まで EOS を出さなかった
    Limit,
    /// 読みより長い
    Length,
    /// 同じ並びの繰り返し
    Repeat,
}

/// 後処理を通した A の出力 1 件
struct Output {
    /// ルビの除去の後の表記
    text: String,
    ruby: bool,
    discard: Option<Discard>,
    /// 完全一致の制約つきの Viterbi で、同じ表記の経路が無い
    off_reading: bool,
    /// 読みの数字・英字の並びが残っていない
    literal_lost: bool,
    echo: bool,
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}' | '\u{30A1}'..='\u{30FA}' | 'ー')
}

fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{3096}')
}

/// 括弧の中がかなだけなら括弧ごと消す (健診(けんしん) → 健診)。読み自体にその括弧があれば消さない
fn strip_ruby(text: &str, reading: &str) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut stripped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, '(' | '（') && !reading.contains(c) {
            let close = chars[i + 1..].iter().position(|&d| matches!(d, ')' | '）')).map(|k| i + 1 + k);
            if let Some(j) = close
                && j > i + 1
                && !reading.contains(chars[j])
                && chars[i + 1..j].iter().all(|&d| is_kana(d))
            {
                stripped = true;
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    (out, stripped)
}

/// 同じ 2 文字以上の並びが 3 回以上続けて現れるか
fn has_tandem_repeat(text: &str) -> bool {
    let v: Vec<char> = text.chars().collect();
    for k in 2..=v.len() / 3 {
        for i in 0..=v.len() - 3 * k {
            if v[i..i + k] == v[i + k..i + 2 * k] && v[i..i + k] == v[i + 2 * k..i + 3 * k] {
                return true;
            }
        }
    }
    false
}

/// 読みの中の数字・英字の連続 (半角・全角)
fn literal_runs(reading: &str) -> Vec<String> {
    let is_literal = |c: char| c.is_ascii_alphanumeric() || matches!(c, '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ');
    let mut runs = Vec::new();
    let mut current = String::new();
    for c in reading.chars() {
        if is_literal(c) {
            current.push(c);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// 出力に左文脈と共通の 4 文字以上のひらがなの並びがあり、それが読みにもあるか
fn is_echo(text: &str, left_context: &str, reading: &str) -> bool {
    let v: Vec<char> = text.chars().collect();
    v.windows(4).any(|w| {
        if !w.iter().all(|&c| is_hiragana(c)) {
            return false;
        }
        let s: String = w.iter().collect();
        left_context.contains(&s) && reading.contains(&s)
    })
}

fn postprocess(generated: &Generated, case: &Case, conv: &Converter) -> Output {
    let (text, ruby) = strip_ruby(&generated.text, &case.reading);
    let discard = if generated.limit {
        Some(Discard::Limit)
    } else if text.chars().count() > case.reading.chars().count() {
        Some(Discard::Length)
    } else if has_tandem_repeat(&text) && !has_tandem_repeat(&case.reading) {
        Some(Discard::Repeat)
    } else {
        None
    };
    let off_reading = discard.is_none() && conv.constrained(&case.reading, text.as_bytes(), true).is_none();
    let literal_lost = literal_runs(&case.reading).iter().any(|r| !text.contains(r.as_str()));
    let echo = is_echo(&text, &case.left_context, &case.reading);
    Output { text, ruby, discard, off_reading, literal_lost, echo }
}

/// A+C で、A 貪欲法の出力を C の結果に代えるか
fn replace_with_c(greedy: &Output) -> bool {
    greedy.discard.is_some() || greedy.off_reading
}

// ---------------------------------------------------------------------------
// C

#[derive(Clone, Copy, PartialEq, Eq)]
enum CEnd {
    Pass,
    /// 新しい制約が前と同じ
    Same,
    /// 評価の回数が上限に達した
    Limit,
    /// 制約を満たす経路が無い
    NoPath,
}

impl CEnd {
    fn label(self) -> &'static str {
        match self {
            CEnd::Pass => "合格",
            CEnd::Same => "同じ制約",
            CEnd::Limit => "上限",
            CEnd::NoPath => "経路なし",
        }
    }
}

struct CResult {
    text: String,
    evals: usize,
    end: CEnd,
}

/// 統計変換をドラフトにした照合。結果は合格したドラフトか、打ち切ったときは最後に評価したドラフト
fn run_c(llm: &mut Llm, conv: &Converter, case: &Case, context: &str, nbest: &NBest, limit: usize) -> BoxResult<CResult> {
    let Some(best) = nbest.best else {
        return Ok(CResult { text: String::new(), evals: 0, end: CEnd::NoPath });
    };
    let prompt = llm.tokenize_prompt(&build_prompt(context, &case.reading));
    let prompt_logits = llm.decode_prompt(&prompt)?;
    // 制約を更新したときに先に探す、最初の N-best の上位件数の候補。補正の候補は打ったかなと違う読みの
    // 語を通り、読みのラティスの経路でないので入れない
    let pool: Vec<String> =
        nbest.candidates.iter().take(llm::RERANK_TOP).filter(|c| !c.typo).map(SentenceCandidate::surface).collect();
    let mut draft = nbest.candidates[best].surface();
    let mut constraint: (Vec<u8>, bool) = (Vec::new(), false);
    let mut evals = 0;
    let end = loop {
        let verdict = llm.evaluate(prompt.len(), &prompt_logits, &draft)?;
        evals += 1;
        let new = match verdict {
            Verdict::Pass => break CEnd::Pass,
            Verdict::Fix(bytes, exact) => (bytes, exact),
        };
        if new == constraint {
            break CEnd::Same;
        }
        if evals >= limit {
            break CEnd::Limit;
        }
        constraint = new;
        let satisfies = |s: &str| {
            if constraint.1 { s.as_bytes() == constraint.0.as_slice() } else { s.as_bytes().starts_with(&constraint.0) }
        };
        if let Some(next) = pool.iter().find(|s| **s != draft && satisfies(s)) {
            draft = next.clone();
            continue;
        }
        match conv.constrained(&case.reading, &constraint.0, constraint.1) {
            Some(next) => draft = next,
            None => break CEnd::NoPath,
        }
    };
    Ok(CResult { text: draft, evals, end })
}

// ---------------------------------------------------------------------------
// 集計

/// 1 件の結果
struct Row {
    chars: usize,
    /// 統計変換の並び (並べ替えの前)
    before: Vec<String>,
    /// 並べ替えの結果 (並べ替えなかった件は統計の並び)
    reranked: Vec<String>,
    rerank_ms: Option<f64>,
    greedy: Option<(Output, f64)>,
    beam: Option<(Vec<Output>, f64)>,
    c: Option<(CResult, f64)>,
}

/// 方式。合わせた並びの先頭に置く出力を返す
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Rerank,
    Greedy,
    Beam,
    C,
    GreedyC,
}

impl Method {
    fn label(self) -> &'static str {
        match self {
            Method::Rerank => "並べ替えだけ",
            Method::Greedy => "A 貪欲法",
            Method::Beam => "A beam",
            Method::C => "C",
            Method::GreedyC => "A+C",
        }
    }
}

impl Row {
    /// 方式の出力 (後処理で捨てたものは除く)。測っていなければ None
    fn outputs(&self, method: Method) -> Option<Vec<String>> {
        let kept = |o: &Output| o.discard.is_none().then(|| o.text.clone());
        match method {
            Method::Rerank => Some(Vec::new()),
            Method::Greedy => self.greedy.as_ref().map(|(o, _)| kept(o).into_iter().collect()),
            Method::Beam => self.beam.as_ref().map(|(os, _)| os.iter().filter_map(kept).take(BEAM_WIDTH).collect()),
            Method::C => self.c.as_ref().map(|(c, _)| vec![c.text.clone()]),
            Method::GreedyC => self.greedy_c().map(|(text, _, _)| vec![text]),
        }
    }

    /// 方式の出力を先頭に置き、並べ替えの結果を続けた並び (同じ表記は先勝ち)
    fn combined(&self, method: Method) -> Option<Vec<String>> {
        let mut list: Vec<String> = Vec::new();
        for s in self.outputs(method)?.into_iter().chain(self.reranked.iter().cloned()) {
            if !s.is_empty() && !list.contains(&s) {
                list.push(s);
            }
        }
        Some(list)
    }

    /// 方式の生成・照合の時間
    fn method_ms(&self, method: Method) -> Option<f64> {
        match method {
            Method::Rerank => self.rerank_ms,
            Method::Greedy => self.greedy.as_ref().map(|g| g.1),
            Method::Beam => self.beam.as_ref().map(|b| b.1),
            Method::C => self.c.as_ref().map(|c| c.1),
            Method::GreedyC => self.greedy_c().map(|(_, _, ms)| ms),
        }
    }

    /// A+C の (出力, C に代えたか, 生成・照合の時間)。計測では A と C を全件で求めているので、
    /// 時間は代えた件だけ C を足す
    fn greedy_c(&self) -> Option<(String, bool, f64)> {
        let (g, g_ms) = self.greedy.as_ref()?;
        let (c, c_ms) = self.c.as_ref()?;
        Some(if replace_with_c(g) { (c.text.clone(), true, g_ms + c_ms) } else { (g.text.clone(), false, *g_ms) })
    }
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

fn stat3(samples: &[f64]) -> String {
    if samples.is_empty() {
        return "- | - | -".to_string();
    }
    let mut v = samples.to_vec();
    let max = v.iter().copied().fold(0.0f64, f64::max);
    format!("{:.1} | {:.1} | {:.1}", median(&mut v), p95(&mut v), max)
}

fn stat2(samples: &[f64]) -> String {
    if samples.is_empty() {
        return "-".to_string();
    }
    let mut v = samples.to_vec();
    format!("{:.1} / {:.1}", median(&mut v), p95(&mut v))
}

/// 表のセル。生成の出力に混ざるプロンプトの区切り (私用領域の文字) は見えないので [U+XXXX] で書く
fn cell(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.chars()
        .map(|c| match c {
            '|' => "\\|".to_string(),
            '\u{E000}'..='\u{F8FF}' => format!("[U+{:04X}]", c as u32),
            _ => c.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let cases = load_cases(&args.cases)?;
    let conv = Converter::load()?;

    let backend = LlamaBackend::init()?;
    let n_gpu_layers = if args.backend == Backend::Vulkan { 999 } else { 0 };
    let mut model_params = std::pin::pin!(LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers));
    // zenz の GGUF は前処理の種類が gpt2-small-japanese-char で、上流の llama.cpp は読めない。
    // azooKey の llama.cpp フォーク (b4846) はこれを GPT-2 と同じ正規表現で扱っているだけなので、
    // メタデータを gpt-2 に上書きすれば同じトークン化になる
    let mut pre = [0 as std::os::raw::c_char; 128];
    for (d, s) in pre.iter_mut().zip(b"gpt-2") {
        *d = *s as std::os::raw::c_char;
    }
    model_params.as_mut().append_kv_override(c"tokenizer.ggml.pre", ParamOverrideValue::Str(pre));
    let model = LlamaModel::load_from_file(&backend, &args.model, &model_params)?;
    let mut rr = Llm::new(&model, &backend, args.threads, RERANK_N_CTX)?;
    rr.rerank_end = rr.resolve_rerank_end(args.rerank_eos)?;
    let mut gen_llm = Llm::new(&model, &backend, args.threads, GEN_N_CTX)?;

    println!("## 条件\n");
    println!("- モデル: {}", args.model.display());
    println!(
        "- バックエンド: {}、スレッド数: {}",
        if args.backend == Backend::Vulkan { "vulkan" } else { "cpu" },
        args.threads
    );
    for d in llama_cpp_2::list_llama_ggml_backend_devices() {
        println!("- デバイス: {} / {} ({})", d.name, d.description, d.backend);
    }
    println!("- テストセット: {} ({} 件、左文脈あり、前文脈なし、学習・ユーザ辞書は空)", args.cases.display(), cases.len());
    println!(
        "- 方式: 並べ替え{}{}{}{}。C の評価の上限 {} 回",
        if args.greedy { "・A 貪欲法" } else { "" },
        if args.beam { "・A beam" } else { "" },
        if args.c { "・C" } else { "" },
        if args.ac { "・A+C" } else { "" },
        args.limit
    );
    println!(
        "- 並べ替えの終わりの項 (--rerank-eos): {}{}",
        args.rerank_eos.label(),
        rr.rerank_end.map_or(String::new(), |t| format!(" (トークン ID {})", t.0))
    );
    println!("- 並べ替える読みの最小文字数 (--rerank-min-chars): {}", args.rerank_min_chars);

    // 空の制約つきの Viterbi が統計変換の最良経路と同じか (C の最初のドラフトの前提) を全件で確かめる
    let mismatches: Vec<&str> = cases
        .iter()
        .filter(|case| {
            let nbest = conv.nbest(&case.reading);
            let best = nbest.best.map(|b| nbest.candidates[b].surface());
            best != conv.constrained(&case.reading, b"", false)
        })
        .map(|case| case.reading.as_str())
        .collect();
    println!("- 検算: 空の制約つきの Viterbi と統計変換の最良経路が違った読み: {} 件 {}", mismatches.len(), mismatches.join("、"));

    // 初回の推論はシェーダの準備などを含むため、先頭の数件と最長の読みの件を計測の前に 1 回ずつ流す
    let longest = (0..cases.len()).max_by_key(|&i| cases[i].reading.chars().count());
    let warmups: Vec<usize> = (0..cases.len().min(3)).chain(longest).collect();
    for &i in &warmups {
        run_case(&args, &cases[i], &conv, &mut rr, &mut gen_llm)?;
    }
    println!("- ウォームアップ: {} 件 (先頭 {} 件と最長の読みの件)", warmups.len(), cases.len().min(3));

    let mut rows: Vec<Row> = Vec::with_capacity(cases.len());
    let mut rerank_none = 0;
    for (i, case) in cases.iter().enumerate() {
        let (row, none) = run_case(&args, case, &conv, &mut rr, &mut gen_llm)?;
        rerank_none += usize::from(none);
        rows.push(row);
        eprintln!("{}/{}", i + 1, cases.len());
    }
    if rerank_none > 0 {
        println!("- プロンプトと候補が並べ替えのコンテキスト ({RERANK_N_CTX}) に収まらず並べ替えなかった件: {rerank_none}");
    }

    let mut methods = vec![Method::Rerank];
    if args.greedy {
        methods.push(Method::Greedy);
    }
    if args.beam {
        methods.push(Method::Beam);
    }
    if args.c {
        methods.push(Method::C);
    }
    if args.ac {
        methods.push(Method::GreedyC);
    }
    if !args.no_cases {
        print_cases(&cases, &rows, &methods);
    }
    print_accuracy(&cases, &rows, &methods);
    print_speed(&rows, &methods, args.rerank_min_chars);
    if args.c || args.ac {
        print_c(&rows);
    }
    if args.greedy || args.beam || args.ac {
        print_side_effects(&cases, &rows);
    }
    if args.ac {
        print_greedy_c(&cases, &rows);
    }
    if args.greedy {
        print_gen_threshold(&cases, &rows);
    }
    Ok(())
}

/// 1 件を全方式にかける。戻り値の bool は、並べ替えるはずの件が長すぎて並べ替えられなかったか
fn run_case(args: &Args, case: &Case, conv: &Converter, rr: &mut Llm, gen_llm: &mut Llm) -> BoxResult<(Row, bool)> {
    let chars = case.reading.chars().count();
    let context = llm::preprocess_context(&case.left_context);
    let nbest = conv.nbest(&case.reading);
    let candidates: Vec<SentenceCandidate> = nbest
        .candidates
        .iter()
        .map(|c| SentenceCandidate {
            segments: c.segments.clone(),
            protected: c.protected,
            user_chosen: c.user_chosen,
            typo: c.typo,
        })
        .collect();

    let before: Vec<String> = candidates.iter().map(SentenceCandidate::surface).collect();
    let mut rerank_ms = None;
    let mut none = false;
    let reranked = if chars < args.rerank_min_chars {
        candidates
    } else {
        let targets = llm::rerank_targets(&candidates);
        if targets.len() < 2 {
            rerank_ms = Some(0.0);
            candidates
        } else {
            let surfaces: Vec<String> = targets.iter().map(|&i| candidates[i].surface()).collect();
            let started = Instant::now();
            let scores = rr.score(&context, &case.reading, &surfaces)?;
            let reordered = match scores {
                Some(scores) => llm::reorder(candidates, &targets, &scores),
                None => {
                    none = true;
                    candidates
                }
            };
            rerank_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            reordered
        }
    };
    let reranked: Vec<String> = reranked.iter().map(SentenceCandidate::surface).collect();

    let greedy = if args.greedy || args.ac {
        let started = Instant::now();
        let generated = gen_llm.greedy(&context, &case.reading)?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        Some((postprocess(&generated, case, conv), ms))
    } else {
        None
    };
    let beam = if args.beam {
        let started = Instant::now();
        let generated = gen_llm.beam(&context, &case.reading)?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        Some((generated.iter().map(|g| postprocess(g, case, conv)).collect(), ms))
    } else {
        None
    };
    let c = if args.c || args.ac {
        let started = Instant::now();
        let result = run_c(gen_llm, conv, case, &context, &nbest, args.limit)?;
        Some((result, started.elapsed().as_secs_f64() * 1000.0))
    } else {
        None
    };
    Ok((Row { chars, before, reranked, rerank_ms, greedy, beam, c }, none))
}

fn output_cell(o: &Output, case: &Case) -> String {
    let mut s = cell(&o.text);
    if case.is_answer(&o.text) {
        s.push_str(" (o)");
    }
    match o.discard {
        Some(Discard::Limit) => s.push_str(" [捨: 上限]"),
        Some(Discard::Length) => s.push_str(" [捨: 長さ]"),
        Some(Discard::Repeat) => s.push_str(" [捨: 繰り返し]"),
        None => {}
    }
    if o.ruby {
        s.push_str(" [ルビ除去]");
    }
    if o.off_reading {
        s.push_str(" [読み外]");
    }
    if o.echo {
        s.push_str(" [エコー]");
    }
    s
}

fn print_cases(cases: &[Case], rows: &[Row], methods: &[Method]) {
    println!("\n## 件ごとの結果\n");
    println!(
        "(o) は正解。[捨: …] は後処理で捨てた出力、[読み外] は同じ表記の経路が辞書のラティスに無い出力。\
         A+C の [C] は A 貪欲法の出力を C の結果に代えた件。順位は合わせた並びでの正解の順位 ({})\n",
        methods.iter().map(|m| m.label()).collect::<Vec<_>>().join(" / ")
    );
    println!("| # | 左文脈 | 読み | 正解 | 並べ替えの1位 | A 貪欲法 | A beam | C | C の評価 | A+C | 順位 |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (i, (case, row)) in cases.iter().zip(rows).enumerate() {
        let top = row.reranked.first().map_or("-".to_string(), |s| {
            let mut c = cell(s);
            if case.is_answer(s) {
                c.push_str(" (o)");
            }
            c
        });
        let greedy = row.greedy.as_ref().map_or("-".to_string(), |(o, _)| output_cell(o, case));
        let beam = row.beam.as_ref().map_or("-".to_string(), |(os, _)| {
            if os.is_empty() { "-".to_string() } else { os.iter().map(|o| output_cell(o, case)).collect::<Vec<_>>().join("<br>") }
        });
        let (c, c_eval) = row.c.as_ref().map_or(("-".to_string(), "-".to_string()), |(c, _)| {
            let mut s = cell(&c.text);
            if case.is_answer(&c.text) {
                s.push_str(" (o)");
            }
            (s, format!("{} 回 / {}", c.evals, c.end.label()))
        });
        let ac = row.greedy_c().map_or("-".to_string(), |(text, replaced, _)| {
            let mut s = cell(&text);
            if case.is_answer(&text) {
                s.push_str(" (o)");
            }
            if replaced {
                s.push_str(" [C]");
            }
            s
        });
        let ranks: Vec<String> = methods
            .iter()
            .map(|&m| {
                row.combined(m)
                    .and_then(|list| case.rank(&list))
                    .map_or("-".to_string(), |r| (r + 1).to_string())
            })
            .collect();
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            i + 1,
            cell(&case.left_context),
            case.reading,
            cell(&case.answer),
            top,
            greedy,
            beam,
            c,
            c_eval,
            ac,
            ranks.join(" / ")
        );
    }
}

fn print_accuracy(cases: &[Case], rows: &[Row], methods: &[Method]) {
    let total = cases.len();
    let pct = |h: usize, n: usize| {
        if n == 0 { "-".to_string() } else { format!("{}/{} ({:.0}%)", h, n, 100.0 * h as f64 / n as f64) }
    };
    let rerank_top1: Vec<bool> = cases.iter().zip(rows).map(|(c, r)| c.rank(&r.reranked) == Some(0)).collect();
    // 並べ替えの上位 10 件に正解が無い件
    let outside: Vec<bool> =
        cases.iter().zip(rows).map(|(c, r)| c.rank(&r.reranked).is_none_or(|k| k >= 10)).collect();
    let n_outside = outside.iter().filter(|&&o| o).count();

    println!("\n## 精度\n");
    println!("### 方式の単独の結果が正解と一致した件数\n");
    println!("| 方式 | 全件 ({total}) | 並べ替えの上位10件に正解が無い件 ({n_outside}) |");
    println!("|---|---|---|");
    let single = |name: &str, hit: &dyn Fn(&Case, &Row) -> Option<bool>| {
        let mut all = 0;
        let mut out = 0;
        let mut measured = false;
        for (k, (c, r)) in cases.iter().zip(rows).enumerate() {
            if let Some(h) = hit(c, r) {
                measured = true;
                all += usize::from(h);
                out += usize::from(h && outside[k]);
            }
        }
        if measured {
            println!("| {name} | {} | {} |", pct(all, total), pct(out, n_outside));
        }
    };
    single("並べ替えの1位", &|c, r| Some(r.reranked.first().is_some_and(|s| c.is_answer(s))));
    single("A 貪欲法", &|c, r| r.greedy.as_ref().map(|(o, _)| o.discard.is_none() && c.is_answer(&o.text)));
    single("A beam (1件目)", &|c, r| {
        r.outputs(Method::Beam).map(|os| os.first().is_some_and(|s| c.is_answer(s)))
    });
    single("A beam (3件のどれか)", &|c, r| r.outputs(Method::Beam).map(|os| os.iter().any(|s| c.is_answer(s))));
    single("C", &|c, r| r.c.as_ref().map(|(x, _)| c.is_answer(&x.text)));
    single("A+C", &|c, r| r.greedy_c().map(|(text, _, _)| c.is_answer(&text)));

    let short = rows.iter().filter(|r| r.chars < llm::RERANK_MIN_CHARS).count();
    println!("\n### 合わせた並び (方式の出力を先頭に置き、並べ替えの結果を続ける)\n");
    println!(
        "得・失は並べ替えだけの 1 位と比べた件数 (得 = 並べ替えで外れ・合わせた並びで当たり)。\
         読みの長さ別は 1 位の件数\n"
    );
    println!(
        "| 方式 | 1位 | 上位3件 | 上位10件 | 得 | 失 | 1位 (9 文字未満, {short} 件) | 1位 (9 文字以上, {} 件) |",
        total - short
    );
    println!("|---|---|---|---|---|---|---|---|");
    let mut before = [0usize; 3];
    for (c, r) in cases.iter().zip(rows) {
        let rank = c.rank(&r.before);
        for (j, limit) in [1usize, 3, 10].into_iter().enumerate() {
            before[j] += usize::from(rank.is_some_and(|x| x < limit));
        }
    }
    println!(
        "| (参考) 並べ替え前 | {} | {} | {} | - | - | - | - |",
        pct(before[0], total),
        pct(before[1], total),
        pct(before[2], total)
    );
    for &m in methods {
        let mut hits = [0usize; 3];
        let (mut gain, mut loss, mut short_hit, mut long_hit) = (0, 0, 0, 0);
        for (k, (c, r)) in cases.iter().zip(rows).enumerate() {
            let rank = r.combined(m).and_then(|list| c.rank(&list));
            for (j, limit) in [1usize, 3, 10].into_iter().enumerate() {
                hits[j] += usize::from(rank.is_some_and(|x| x < limit));
            }
            let top1 = rank == Some(0);
            gain += usize::from(top1 && !rerank_top1[k]);
            loss += usize::from(!top1 && rerank_top1[k]);
            if r.chars < llm::RERANK_MIN_CHARS {
                short_hit += usize::from(top1);
            } else {
                long_hit += usize::from(top1);
            }
        }
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            m.label(),
            pct(hits[0], total),
            pct(hits[1], total),
            pct(hits[2], total),
            gain,
            loss,
            pct(short_hit, short),
            pct(long_hit, total - short)
        );
    }
}

fn print_speed(rows: &[Row], methods: &[Method], min_chars: usize) {
    println!("\n## 速さ (ms、件ごとに 1 回)\n");
    println!(
        "並べ替えは読みが {} 文字以上の件だけ。合計は並べ替えと生成・照合の和 (並べ替えない件は生成・照合だけ)\n",
        min_chars
    );
    println!("| 方式 | 並べ替え 中央値 | p95 | 最大 | 生成・照合 中央値 | p95 | 最大 | 合計 中央値 | p95 | 最大 |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let rerank: Vec<f64> = rows.iter().filter_map(|r| r.rerank_ms).collect();
    for &m in methods {
        let (own, sum): (Vec<f64>, Vec<f64>) = if m == Method::Rerank {
            (Vec::new(), rerank.clone())
        } else {
            rows.iter()
                .filter_map(|r| r.method_ms(m).map(|ms| (ms, ms + r.rerank_ms.unwrap_or(0.0))))
                .unzip()
        };
        println!("| {} | {} | {} | {} |", m.label(), stat3(&rerank), stat3(&own), stat3(&sum));
    }
    println!("\n### 読みの長さ別 (中央値 / p95)\n");
    let header: Vec<String> = LENGTH_BUCKETS
        .iter()
        .map(|(name, lo, hi)| {
            let n = rows.iter().filter(|r| r.chars >= *lo && r.chars < *hi).count();
            format!("{name} ({n} 件)")
        })
        .collect();
    println!("| 方式 | 時間 | {} |", header.join(" | "));
    println!("|---|---|{}", "---|".repeat(LENGTH_BUCKETS.len()));
    for &m in methods {
        let kinds: Vec<(&str, Box<dyn Fn(&Row) -> Option<f64>>)> = if m == Method::Rerank {
            vec![("並べ替え", Box::new(|r: &Row| r.rerank_ms))]
        } else {
            vec![
                ("生成・照合", Box::new(move |r: &Row| r.method_ms(m))),
                ("合計", Box::new(move |r: &Row| r.method_ms(m).map(|ms| ms + r.rerank_ms.unwrap_or(0.0)))),
            ]
        };
        for (kind, f) in kinds {
            let cells: Vec<String> = LENGTH_BUCKETS
                .iter()
                .map(|(_, lo, hi)| {
                    let v: Vec<f64> = rows.iter().filter(|r| r.chars >= *lo && r.chars < *hi).filter_map(|r| f(r)).collect();
                    stat2(&v)
                })
                .collect();
            println!("| {} | {kind} | {} |", m.label(), cells.join(" | "));
        }
    }
}

fn print_c(rows: &[Row]) {
    let results: Vec<&CResult> = rows.iter().filter_map(|r| r.c.as_ref().map(|c| &c.0)).collect();
    println!("\n## C の評価の回数と終わり方 ({} 件)\n", results.len());
    let buckets = [("1", 1, 1), ("2", 2, 2), ("3", 3, 3), ("4〜6", 4, 6), ("7 以上", 7, usize::MAX)];
    println!("| {} |", buckets.iter().map(|b| format!("{} 回", b.0)).collect::<Vec<_>>().join(" | "));
    println!("|{}", "---|".repeat(buckets.len()));
    let counts: Vec<String> = buckets
        .iter()
        .map(|&(_, lo, hi)| results.iter().filter(|c| c.evals >= lo && c.evals <= hi).count().to_string())
        .collect();
    println!("| {} |", counts.join(" | "));
    let ends = [CEnd::Pass, CEnd::Same, CEnd::Limit, CEnd::NoPath];
    println!("\n| {} |", ends.iter().map(|e| e.label()).collect::<Vec<_>>().join(" | "));
    println!("|{}", "---|".repeat(ends.len()));
    let counts: Vec<String> =
        ends.iter().map(|&e| results.iter().filter(|c| c.end == e).count().to_string()).collect();
    println!("| {} |", counts.join(" | "));
    let total: usize = results.iter().map(|c| c.evals).sum();
    println!("\n- 評価の回数の合計: {total} (1 件あたり平均 {:.2})", total as f64 / results.len().max(1) as f64);
}

fn print_side_effects(cases: &[Case], rows: &[Row]) {
    let applicable = cases.iter().filter(|c| !literal_runs(&c.reading).is_empty()).count();
    println!("\n## A の副作用\n");
    println!(
        "貪欲法は件数、beam は出力 (仮説) の数。「読みから外れた」は後処理で捨てなかった出力のうち、\
         完全一致の制約つきの Viterbi で同じ表記の経路が無いもの。読みに数字・英字がある件: {applicable}\n"
    );
    println!("| 項目 | 貪欲法 | beam |");
    println!("|---|---|---|");
    let greedy: Vec<(&Case, &Output)> =
        cases.iter().zip(rows).filter_map(|(c, r)| r.greedy.as_ref().map(|(o, _)| (c, o))).collect();
    let beam: Vec<(&Case, &Output)> = cases
        .iter()
        .zip(rows)
        .flat_map(|(c, r)| r.beam.iter().flat_map(move |(os, _)| os.iter().map(move |o| (c, o))))
        .collect();
    let count = |list: &[(&Case, &Output)], f: &dyn Fn(&Case, &Output) -> bool| {
        list.iter().filter(|(c, o)| f(c, o)).count()
    };
    let line = |name: &str, f: &dyn Fn(&Case, &Output) -> bool| {
        println!("| {name} | {} | {} |", count(&greedy, f), count(&beam, f));
    };
    println!("| 出力の数 | {} | {} |", greedy.len(), beam.len());
    line("上限で止まった (捨てた)", &|_, o| o.discard == Some(Discard::Limit));
    line("ルビを除去した", &|_, o| o.ruby);
    line("長さで捨てた", &|_, o| o.discard == Some(Discard::Length));
    line("繰り返しで捨てた", &|_, o| o.discard == Some(Discard::Repeat));
    line("読みから外れた", &|_, o| o.off_reading);
    line("読みから外れた・正解", &|c, o| o.off_reading && c.is_answer(&o.text));
    line("読みの数字・英字が残っていない", &|_, o| o.literal_lost);
    line("左文脈のエコー", &|_, o| o.echo);
}

fn print_greedy_c(cases: &[Case], rows: &[Row]) {
    let reasons: [(&str, &dyn Fn(&Output) -> bool); 4] = [
        ("上限で捨てた", &|o| o.discard == Some(Discard::Limit)),
        ("長さで捨てた", &|o| o.discard == Some(Discard::Length)),
        ("繰り返しで捨てた", &|o| o.discard == Some(Discard::Repeat)),
        ("読みから外れた", &|o| o.off_reading),
    ];
    println!("\n## A+C で C に代えた件\n");
    println!("「A が正解」は A 貪欲法の出力 (後処理で捨てた出力を含む) が正解だった件\n");
    println!("| 理由 | 件数 | C が正解 | A が正解 | どちらも外れ |");
    println!("|---|---|---|---|---|");
    let mut total = [0usize; 4];
    for (name, matches) in reasons {
        let mut n = [0usize; 4];
        for (case, row) in cases.iter().zip(rows) {
            let (Some((g, _)), Some((c, _))) = (row.greedy.as_ref(), row.c.as_ref()) else { continue };
            if !replace_with_c(g) || !matches(g) {
                continue;
            }
            let c_hit = case.is_answer(&c.text);
            let a_hit = case.is_answer(&g.text);
            n[0] += 1;
            n[1] += usize::from(c_hit);
            n[2] += usize::from(a_hit);
            n[3] += usize::from(!c_hit && !a_hit);
        }
        for k in 0..4 {
            total[k] += n[k];
        }
        println!("| {name} | {} | {} | {} | {} |", n[0], n[1], n[2], n[3]);
    }
    println!("| 計 | {} | {} | {} | {} |", total[0], total[1], total[2], total[3]);
}

/// 生成のしきい値の格子の 1 行
#[derive(Debug, PartialEq, Eq)]
struct GridRow {
    min_chars: usize,
    /// 生成をかけた件
    applied: usize,
    top1: usize,
    top3: usize,
    /// 並べ替えだけの 1 位と比べた得・失
    gain: usize,
    loss: usize,
}

/// 件ごとの (読みの文字数, 並べ替えだけの正解の順位, A 貪欲法の合わせた並びの正解の順位) から、
/// 読みが min_chars 文字以上の件だけ A 貪欲法の合わせた並びを使ったときの集計を下限ごとに出す
fn gen_threshold_grid(items: &[(usize, Option<usize>, Option<usize>)], mins: &[usize]) -> Vec<GridRow> {
    mins.iter()
        .map(|&min_chars| {
            let mut row = GridRow { min_chars, applied: 0, top1: 0, top3: 0, gain: 0, loss: 0 };
            for &(chars, rerank, greedy) in items {
                let applied = chars >= min_chars;
                let rank = if applied { greedy } else { rerank };
                row.applied += usize::from(applied);
                row.top1 += usize::from(rank == Some(0));
                row.top3 += usize::from(rank.is_some_and(|r| r < 3));
                row.gain += usize::from(rank == Some(0) && rerank != Some(0));
                row.loss += usize::from(rank != Some(0) && rerank == Some(0));
            }
            row
        })
        .collect()
}

fn print_gen_threshold(cases: &[Case], rows: &[Row]) {
    let total = cases.len();
    let pct = |h: usize| {
        if total == 0 { "-".to_string() } else { format!("{}/{} ({:.1}%)", h, total, 100.0 * h as f64 / total as f64) }
    };
    let items: Vec<(usize, Option<usize>, Option<usize>)> = cases
        .iter()
        .zip(rows)
        .map(|(c, r)| (r.chars, c.rank(&r.reranked), r.combined(Method::Greedy).and_then(|list| c.rank(&list))))
        .collect();
    let rerank_top1 = items.iter().filter(|(_, r, _)| *r == Some(0)).count();
    let rerank_top3 = items.iter().filter(|(_, r, _)| r.is_some_and(|k| k < 3)).count();
    println!("\n## 生成のしきい値の格子\n");
    println!(
        "読みが L 文字以上の件だけ A 貪欲法の出力を合わせた並びの先頭に置き、それ以外は並べ替えだけの並び。\
         並べ替え自体は上の条件のまま。得・失は並べ替えだけの 1 位と比べた件数\n"
    );
    println!("| L | 生成をかけた件 | 1位 | 上位3件 | 得 | 失 |");
    println!("|---|---|---|---|---|---|");
    println!("| (並べ替えだけ) | - | {} | {} | - | - |", pct(rerank_top1), pct(rerank_top3));
    for g in gen_threshold_grid(&items, &GEN_MIN_CHARS_GRID) {
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            g.min_chars,
            pct(g.applied),
            pct(g.top1),
            pct(g.top3),
            g.gain,
            g.loss
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn しきい値未満の件は並べ替えだけの順位で数える() {
        // (文字数, 並べ替えだけ, A 貪欲法の合わせた並び)
        let items = [
            (2, Some(0), Some(1)),
            (6, Some(1), Some(0)),
            (10, None, Some(2)),
            (10, Some(0), Some(0)),
        ];
        let grid = gen_threshold_grid(&items, &[0, 5, 9, 12]);
        assert_eq!(grid[0], GridRow { min_chars: 0, applied: 4, top1: 2, top3: 4, gain: 1, loss: 1 });
        assert_eq!(grid[1], GridRow { min_chars: 5, applied: 3, top1: 3, top3: 4, gain: 1, loss: 0 });
        assert_eq!(grid[2], GridRow { min_chars: 9, applied: 2, top1: 2, top3: 4, gain: 0, loss: 0 });
        assert_eq!(grid[3], GridRow { min_chars: 12, applied: 0, top1: 2, top3: 3, gain: 0, loss: 0 });
    }

    #[test]
    fn かなだけの括弧は消し読みにある括弧は残す() {
        assert_eq!(strip_ruby("健診(けんしん)を受ける", "けんしんをうける"), ("健診を受ける".to_string(), true));
        assert_eq!(strip_ruby("健診（ケンシン）", "けんしん"), ("健診".to_string(), true));
        assert_eq!(strip_ruby("面白い(笑)", "おもしろい"), ("面白い(笑)".to_string(), false));
        assert_eq!(strip_ruby("関数(かんすう)", "かんすう(かんすう)"), ("関数(かんすう)".to_string(), false));
        assert_eq!(strip_ruby("空()", "から"), ("空()".to_string(), false));
    }

    #[test]
    fn 二文字以上の並びが三回続くと繰り返し() {
        assert!(has_tandem_repeat("分かった分かった分かった"));
        assert!(has_tandem_repeat("あいあいあい"));
        assert!(!has_tandem_repeat("あいあい"));
        assert!(!has_tandem_repeat("ああああ"));
        assert!(!has_tandem_repeat(""));
    }

    #[test]
    fn 左文脈と読みの両方にあるひらがな四文字を写すとエコー() {
        assert!(is_echo("きのうはあめ", "きのうはあめでした", "きのうはあめ"));
        assert!(!is_echo("昨日は雨", "きのうはあめでした", "きのうはあめ"));
        assert!(!is_echo("きのうは", "今日は", "きのうは"));
    }

    #[test]
    fn 捨てた出力と読みから外れた出力だけをcに代える() {
        let output = |discard: Option<Discard>, off_reading: bool| Output {
            text: "昨日は雨".to_string(),
            ruby: false,
            discard,
            off_reading,
            literal_lost: false,
            echo: false,
        };
        assert!(!replace_with_c(&output(None, false)));
        assert!(replace_with_c(&output(None, true)));
        assert!(replace_with_c(&output(Some(Discard::Limit), false)));
        assert!(replace_with_c(&output(Some(Discard::Length), false)));
        assert!(replace_with_c(&output(Some(Discard::Repeat), false)));
    }

    #[test]
    fn 数字と英字の連続を取り出す() {
        assert_eq!(literal_runs("2024ねんのiphone15"), vec!["2024", "iphone15"]);
        assert_eq!(literal_runs("１２じ"), vec!["１２"]);
        assert!(literal_runs("かな").is_empty());
    }
}
