// 候補の統一した並び (SUGGEST) と打鍵のファジィ照合の評価 (docs/spec/engine.md「計測の道具」)
//
// テストセットの正解の読みを標準のローマ字 (訓令式とヘボン式を交互) にし、次の入力を作って SUGGEST にかける。
// - 打ち終えた入力: 崩さない打鍵と、B の誤り (隣のキー・余分・抜け・入れ替え) を擬似乱数で 1〜3 か所
//   入れた打鍵 (先頭のかなの打鍵は崩さない)
// - 途中までの入力: 文の打鍵の先頭から 50%・75% (崩さないものと、その範囲に誤りを 1 か所入れたもの)。
//   語 (文の 1-best の文節の先頭の、3 文字以上の読みの辞書の語) の打鍵の先頭から 50%・75% も同じく作る
// - 打ちかけの入力: 崩さない文の打鍵を先頭から 1 打ずつ増やした途中の状態のうち、確定済みのかなが
//   MIN_TYPO_CHARS 文字以上のもの (最後の 1 打まで打った状態は除く)。候補バーは 1 打ごとに SUGGEST を
//   引くので、打ちかけの読みで補正の候補・ずれのある予測が誤って上位に出ないかを数える
// 打鍵は TSF 層の RomajiComposer と同じ規則でかなと打鍵のフィールドにする (typo::compose)。
// 正解は、崩さない読みを補正なしで変換した 1-best の表記 (examples/typo_eval.rs と同じ)。
// 途中までの文の正解は、1-best の文節のうち打った読みを覆うところまでの表記 (打った所が文節の途中なら
// その文節の終わりまで)。途中までの語の正解は語の表記。
// 学習・ユーザ辞書は空、前文脈は使わない。エンジンは lib を持たないため、使うモジュールをソースごと
// 取り込む (typo_eval と同じ方式)。結果は Markdown で標準出力へ書く。
//
// 使い方:
//   cargo run --release --example fuzzy_eval -- --cases examples/data/llm_cases.tsv \
//       --cases examples/data/typo_cases.tsv [--config "bias=2000,kpe=5"]... [--timing]
//
// オプション:
//   --cases <tsv>     テストセット (typo_eval と同じ形式)。複数指定できる
//   --config <設定>   調整値の組 (カンマ区切りの key=value)。複数指定でき、指定ごとに 1 行の結果を出す。
//                     無ければ既定値 (typo::TUNING) だけ。key は adj・extra・miss・swap (EDIT_*)・
//                     edit (EDIT_* をすべて同じ値に)・kpe (KEYS_PER_EDIT)・max (MAX_EDITS)・
//                     show (TYPO_SHOW)・top (TYPO_TOP)・bias (PREDICT_BIAS)・tail (全体変換の補正で
//                     変えない読みの末尾の文字数)・ptail (予測の照合で誤りを認めない末尾のかなのかたまりの数)・
//                     words (補正の語を区間と補正した読みの組ごとに入れる語数)・
//                     level (ファジィ入力の段階 -1〜3。既定 0。段階の調整値はほかの key を当てた後に重ねる)
//   --timing          最初の --config で、文の打鍵を 1 打ずつ打ったときの SUGGEST の処理時間 (中央値・p95・最大) も測る
//   --detail          既定値 (または最初の --config) で、崩さない入力で先頭が変わった例などを出す
//   --dict <dir>      Mozc 辞書のディレクトリ (既定は references/mozc/src/data/dictionary_oss)
//   --ut <file>       UT 辞書 (既定は references/mozcdic-ut/dictionary_ut.txt。無ければ使わない)

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

use std::path::PathBuf;
use std::time::Instant;

use config::Config;
use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use typo::{Keys, Tuning};
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    cases: Vec<PathBuf>,
    configs: Vec<String>,
    timing: bool,
    detail: bool,
    dict: PathBuf,
    ut: PathBuf,
}

fn parse_args() -> BoxResult<Args> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut args = Args {
        cases: Vec::new(),
        configs: Vec::new(),
        timing: false,
        detail: false,
        dict: root.join("references/mozc/src/data/dictionary_oss"),
        ut: root.join("references/mozcdic-ut/dictionary_ut.txt"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--cases" => args.cases.push(PathBuf::from(value()?)),
            "--config" => args.configs.push(value()?),
            "--timing" => args.timing = true,
            "--detail" => args.detail = true,
            "--dict" => args.dict = PathBuf::from(value()?),
            "--ut" => args.ut = PathBuf::from(value()?),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    if args.cases.is_empty() {
        return Err("--cases が必要です".into());
    }
    if args.configs.is_empty() {
        args.configs.push(String::new());
    }
    Ok(args)
}

/// 調整値の組の文字列 (key=value のカンマ区切り) を既定値に当て、ファジィ入力の段階とともに返す
fn parse_tuning(spec: &str) -> BoxResult<(Tuning, i8)> {
    let mut t = typo::TUNING;
    let mut level: i8 = 0;
    for item in spec.split(',').filter(|s| !s.is_empty()) {
        let (key, value) = item.split_once('=').ok_or_else(|| format!("key=value ではありません: {item}"))?;
        match key {
            "adj" => t.edit_adjacent = value.parse()?,
            "extra" => t.edit_extra = value.parse()?,
            "miss" => t.edit_missing = value.parse()?,
            "swap" => t.edit_swap = value.parse()?,
            "edit" => {
                let v: i32 = value.parse()?;
                t.edit_adjacent = v;
                t.edit_extra = v;
                t.edit_missing = v;
                t.edit_swap = v;
            }
            "kpe" => t.keys_per_edit = value.parse()?,
            "max" => t.max_edits = value.parse()?,
            "show" => t.typo_show = value.parse()?,
            "top" => t.typo_top = value.parse()?,
            "bias" => t.predict_bias = value.parse()?,
            "tail" => t.tail_chars = value.parse()?,
            "ptail" => t.predict_tail = value.parse()?,
            "words" => t.typo_words = value.parse()?,
            "level" => {
                level = value.parse()?;
                if !(typo::MIN_LEVEL..=typo::MAX_LEVEL).contains(&level) {
                    return Err(format!("段階が範囲外です: {level}").into());
                }
            }
            other => return Err(format!("不明な key: {other}").into()),
        }
    }
    Ok((t, level))
}

/// テストセットの読み (重複は除く)
fn load_readings(paths: &[PathBuf]) -> BoxResult<Vec<String>> {
    let mut readings: Vec<String> = Vec::new();
    for path in paths {
        for (no, line) in std::fs::read_to_string(path)?.lines().enumerate() {
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let reading = match f.len() {
                4 => f[1],
                2 => f[0],
                _ => return Err(format!("{}:{}: 列数が 2 か 4 ではありません", path.display(), no + 1).into()),
            };
            if !readings.iter().any(|r| r == reading) {
                readings.push(reading.to_string());
            }
        }
    }
    Ok(readings)
}

/// 決まった並びの擬似乱数 (xorshift64)
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// romaji の head 打目より後ろに、B の誤りを 1 か所入れる (種類は乱数)
fn inject(romaji: &[u8], head: usize, rng: &mut Rng) -> Option<Vec<u8>> {
    let n = romaji.len();
    if n <= head {
        return None;
    }
    let mut v = romaji.to_vec();
    match rng.below(4) {
        // 余分: 隣のキーか同じキーを 1 打多く打つ
        0 => {
            let i = head + rng.below(n - head + 1);
            let base = romaji[i.min(n - 1)];
            let mut keys = typo::neighbor_keys(base);
            keys.push(base);
            v.insert(i, keys[rng.below(keys.len())]);
        }
        // 抜け
        1 => {
            v.remove(head + rng.below(n - head));
        }
        // 隣のキー
        2 => {
            let i = head + rng.below(n - head);
            let keys = typo::neighbor_keys(romaji[i]);
            if keys.is_empty() {
                return None;
            }
            v[i] = keys[rng.below(keys.len())];
        }
        // 入れ替え
        _ => {
            if n < head + 2 {
                return None;
            }
            let i = head + rng.below(n - head - 1);
            if v[i] == v[i + 1] {
                return None;
            }
            v.swap(i, i + 1);
        }
    }
    Some(v)
}

/// romaji に誤りを edits か所入れて、かなと打鍵のフィールドにする。元のかな clean と同じになるもの・
/// かなにならないものは作り直し、20 回で作れなければ None
fn crush(romaji: &str, head: usize, edits: usize, clean: &str, seed: u64) -> Option<(String, String)> {
    let mut rng = Rng(seed | 1);
    for _ in 0..20 {
        let mut v = romaji.as_bytes().to_vec();
        let mut ok = true;
        for _ in 0..edits {
            match inject(&v, head, &mut rng) {
                Some(next) => v = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let Some((kana, field)) = typo::compose(std::str::from_utf8(&v).ok()?) else {
            continue;
        };
        if kana != clean && !kana.is_empty() {
            return Some((kana, field));
        }
    }
    None
}

/// 打鍵の先頭 frac の割合 (切り上げ)
fn cut(romaji: &str, frac: f64) -> &str {
    let n = ((romaji.len() as f64) * frac).ceil() as usize;
    &romaji[..n.min(romaji.len())]
}

/// SUGGEST にかける 1 件
struct Input {
    kana: String,
    field: String,
    /// 正解の表記
    target: String,
}

/// 文のテストケース
struct Sentence {
    reading: String,
    /// 正解 (崩さない読みの補正なしの 1-best)
    target: String,
    /// 正解の文節 (読み, 表記)
    segments: Vec<(String, String)>,
    romaji: String,
    /// 先頭のかなの打鍵の数
    head: usize,
}

/// 評価の入力一式
struct Inputs {
    clean: Vec<Input>,
    /// 崩した打ち終えた入力 (誤りの数, 入力)
    crushed: Vec<(usize, Input)>,
    /// 途中までの文 [50% 崩さない, 75% 崩さない, 50% 崩した, 75% 崩した]
    partial: [Vec<Input>; 4],
    /// 途中までの語 (同じ並び)
    words: [Vec<Input>; 4],
    /// 打ちかけの入力 (打鍵, 入力)。正解はその文の正解
    typing: Vec<(String, Input)>,
}

/// 途中までの打鍵 prefix の正解: 打った読み (未完成のローマ字が残れば次の 1 字まで) を覆う文節までの表記
fn partial_target(prefix: &str, segments: &[(String, String)]) -> Option<String> {
    let (kana, field) = typo::compose(prefix)?;
    let pending = !field.ends_with('\x1e');
    let covered = kana.chars().count() + usize::from(pending);
    let mut length = 0;
    let mut surface = String::new();
    for (reading, s) in segments {
        if length >= covered {
            break;
        }
        length += reading.chars().count();
        surface.push_str(s);
    }
    Some(surface)
}

/// 途中までの入力 (崩さない 50%・75%、崩した 50%・75%) を作る
fn partials(romaji: &str, head: usize, target: impl Fn(&str) -> Option<String>, seed: u64) -> [Option<Input>; 4] {
    let mut result: [Option<Input>; 4] = [None, None, None, None];
    for (i, frac) in [0.5, 0.75].into_iter().enumerate() {
        let prefix = cut(romaji, frac);
        if prefix.len() <= head {
            continue;
        }
        let Some(expected) = target(prefix) else { continue };
        if let Some((kana, field)) = typo::compose(prefix) {
            if !kana.is_empty() {
                result[i] = Some(Input { kana: kana.clone(), field, target: expected.clone() });
                if let Some((k, f)) = crush(prefix, head, 1, &kana, seed ^ (i as u64 + 1) << 20) {
                    result[2 + i] = Some(Input { kana: k, field: f, target: expected });
                }
            }
        }
    }
    result
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let readings = load_readings(&args.cases)?;
    let extra: Vec<PathBuf> = args.ut.is_file().then(|| args.ut.clone()).into_iter().collect();
    let started = Instant::now();
    let mut dictionary = Dictionary::load(&args.dict, &extra)?;
    if let Some(parent) = args.dict.parent() {
        let _ = dictionary.load_symbols(&parent.join("symbol/symbol.tsv"));
        let _ = dictionary.load_single_kanji(&parent.join("single_kanji/single_kanji.tsv"));
    }
    let matrix = ConnectionMatrix::load(&args.dict.join("connection_single_column.txt"))?;
    let functional = FunctionalIds::load(&args.dict.join("id.def"))?;
    let user = UserDict::empty();
    let learning = LearningStore::in_memory();
    let cfg = Config::default();
    eprintln!("辞書の読み込み: {:.1} 秒", started.elapsed().as_secs_f64());

    // 文のテストケース
    let mut sentences: Vec<Sentence> = Vec::new();
    for (index, reading) in readings.iter().enumerate() {
        let hepburn = index % 2 == 1;
        let Some(romaji) = typo::to_romaji(reading, hepburn) else { continue };
        let first: String = reading.chars().take(1).collect();
        let head = typo::to_romaji(&first, hepburn).map_or(1, |r| r.len());
        let best = convert::convert_nbest(reading, None, &dictionary, &user, &matrix, &functional, &learning, false);
        let Some(top) = best.first() else { continue };
        sentences.push(Sentence {
            reading: reading.clone(),
            target: top.surface(),
            segments: top.segments.clone(),
            romaji,
            head,
        });
    }

    let mut inputs = Inputs {
        clean: Vec::new(),
        crushed: Vec::new(),
        partial: Default::default(),
        words: Default::default(),
        typing: Vec::new(),
    };
    let mut skipped = 0;
    let mut word_list: Vec<(String, String)> = Vec::new();
    for (index, s) in sentences.iter().enumerate() {
        let seed = 0x9E37_79B9_7F4A_7C15 ^ ((index as u64) << 8);
        match typo::compose(&s.romaji) {
            Some((kana, field)) if kana == s.reading => {
                inputs.clean.push(Input { kana, field, target: s.target.clone() });
            }
            _ => skipped += 1,
        }
        for edits in 1..=3 {
            match crush(&s.romaji, s.head, edits, &s.reading, seed ^ edits as u64) {
                Some((kana, field)) => {
                    inputs.crushed.push((edits, Input { kana, field, target: s.target.clone() }))
                }
                None => skipped += 1,
            }
        }
        for n in 1..s.romaji.len() {
            let Some((kana, field)) = typo::compose(&s.romaji[..n]) else { continue };
            if kana.chars().count() < typo::MIN_TYPO_CHARS {
                continue;
            }
            inputs.typing.push((s.romaji[..n].to_string(), Input { kana, field, target: s.target.clone() }));
        }
        let made = partials(&s.romaji, s.head, |p| partial_target(p, &s.segments), seed ^ 0x55);
        for (i, input) in made.into_iter().enumerate() {
            if let Some(input) = input {
                inputs.partial[i].push(input);
            }
        }
        // 文節の先頭の、3 文字以上の読みの辞書の語 (表記が文節の表記の先頭と同じ最長のもの)
        for (reading, surface) in &s.segments {
            let chars: Vec<char> = reading.chars().collect();
            let mut word: Option<(String, String)> = None;
            for (len, entries) in dictionary.common_prefix_search(&chars, 16) {
                if len < 3 {
                    continue;
                }
                if let Some(e) = entries.iter().find(|e| surface.starts_with(e.surface.as_str())) {
                    word = Some((chars[..len].iter().collect(), e.surface.clone()));
                }
            }
            if let Some(w) = word {
                if !word_list.contains(&w) {
                    word_list.push(w);
                }
            }
        }
    }
    for (index, (reading, surface)) in word_list.iter().enumerate() {
        let hepburn = index % 2 == 1;
        let Some(romaji) = typo::to_romaji(reading, hepburn) else { continue };
        let first: String = reading.chars().take(1).collect();
        let head = typo::to_romaji(&first, hepburn).map_or(1, |r| r.len());
        let made = partials(&romaji, head, |_| Some(surface.clone()), 0xABCD ^ ((index as u64) << 8));
        for (i, input) in made.into_iter().enumerate() {
            if let Some(input) = input {
                inputs.words[i].push(input);
            }
        }
    }

    println!("## テストセット\n");
    println!("- 文: {} 件 (崩さない打ち終えた入力 {} 件)", sentences.len(), inputs.clean.len());
    for edits in 1..=3 {
        println!(
            "- 崩した打ち終えた入力 (誤り {edits} か所): {} 件",
            inputs.crushed.iter().filter(|(e, _)| *e == edits).count()
        );
    }
    println!("- 作れなかった入力: {skipped} 件");
    let names = ["50% 崩さない", "75% 崩さない", "50% 崩した", "75% 崩した"];
    for (i, name) in names.iter().enumerate() {
        println!("- 途中までの文 ({name}): {} 件、語 ({name}): {} 件", inputs.partial[i].len(), inputs.words[i].len());
    }
    println!("- 語: {} 語", word_list.len());
    println!("- 打ちかけの入力: {} 件", inputs.typing.len());

    let run = |input: &Input, (tuning, level): &(Tuning, i8)| -> (Vec<suggest::Suggestion>, f64) {
        let keys = Keys::parse(&input.kana, &input.field).expect("打鍵のフィールド");
        let started = Instant::now();
        let list = suggest::suggest(
            &input.kana, None, &keys, &dictionary, &user, &matrix, &functional, &learning, &cfg, tuning,
            *level,
        );
        (list, started.elapsed().as_secs_f64() * 1000.0)
    };
    let rank = |list: &[suggest::Suggestion], target: &str| list.iter().position(|s| s.surface() == target);
    let percentile = |times: &mut Vec<f64>, q: f64| -> f64 {
        times.sort_by(|a, b| a.total_cmp(b));
        times[((times.len() as f64 - 1.0) * q).round() as usize]
    };
    let pct = |n: usize, total: usize| format!("{:.0}%", 100.0 * n as f64 / total.max(1) as f64);

    println!("\n## 調整値ごとの結果\n");
    println!("先頭変化 = 崩さない打ち終えた入力で SUGGEST の先頭が CONVNBEST (打鍵つき。表記が読みのかなと同じ候補と、書き換えの候補を除く) の先頭と変わった件数。");
    println!("崩し 1/3/9 = 崩した打ち終えた入力で正解が 1 位・3 位以内・9 位以内の割合 (誤り 1〜3 か所の合計)。");
    println!("誤補正 = 崩さない打ち終えた入力で、補正の候補かずれのある予測が 3 位以内に出た件数 / 先頭に出た件数。");
    println!("途中誤補正 = 打ちかけの入力 (崩さない文の打鍵を 1 打ずつ増やした途中の状態で、確定済みのかなが {} 文字以上のもの) で、補正の候補かずれのある予測が 3 位以内に出た件数 / 先頭に出た件数 (割合の母数は打ちかけの入力の数)。", typo::MIN_TYPO_CHARS);
    println!("補正=正解 = 崩した打ち終えた入力で、CONVNBEST (打鍵つき) の補正の候補が正解になった割合。");
    println!("括弧内は打鍵を使わずかなを戻した近似 (打鍵の無い CONVNBEST) の同じ割合 (コスト・状態の数・しきい値は同じ調整値)。");
    println!("文・語 = 途中までの入力で正解が 9 位以内の割合 (50%/75%、崩さない | 崩した)。");
    println!("時間 = SUGGEST 1 回 (この表の入力すべて) の中央値 / p95 / 最大 (ms)\n");
    println!("| 調整値 | 先頭変化 | 誤補正 | 途中誤補正 | 崩し 1 | 崩し 3 | 崩し 9 | 崩し 3 (誤り 1/2/3) | 補正=正解 (誤り 1/2/3) (近似) | 文 崩さない | 文 崩した | 語 崩さない | 語 崩した | 時間 |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    let mut details: Vec<String> = Vec::new();
    for (ci, spec) in args.configs.iter().enumerate() {
        let tuning = parse_tuning(spec)?;
        // CONVNBEST は SUGGEST と同じく段階の調整値で引き、段階 -1 なら補正しない
        let level_tuning = tuning.0.at_level(tuning.1);
        let fuzzy = tuning.1 >= 0;
        let mut times: Vec<f64> = Vec::new();
        let mut changed = 0;
        // 崩さない入力で、補正の候補・ずれのある予測が 3 位以内に出た件数と先頭に出た件数
        let mut false_shown = 0;
        let mut false_top = 0;
        for input in &inputs.clean {
            let (list, ms) = run(input, &tuning);
            times.push(ms);
            let is_typo = |s: &suggest::Suggestion| matches!(s.kind, suggest::Kind::Typo | suggest::Kind::PredictTypo);
            false_shown += usize::from(list.iter().take(3).any(is_typo));
            false_top += usize::from(list.first().is_some_and(is_typo));
            if ci == 0 && args.detail {
                if let Some((p, s)) = list.iter().take(3).enumerate().find(|(_, s)| is_typo(s)) {
                    details.push(format!(
                        "- 崩さない入力で補正が {} 位: {} → {} [{}] 正解 {}",
                        p + 1,
                        input.kana,
                        s.surface(),
                        s.kind.name(),
                        input.target
                    ));
                }
            }
            let keys = Keys::parse(&input.kana, &input.field).unwrap();
            let source = if fuzzy { convert::TypoSource::Keys(&keys) } else { convert::TypoSource::Off };
            let nbest = convert::convert_nbest_scored(
                &input.kana, None, &dictionary, &user, &matrix, &functional, &learning, source, &level_tuning,
            );
            // SUGGEST は表記が読みのかなと同じ候補を除き、書き換えの候補 (表記をひらがなに直すと読みと同じ。
            // 最良経路と読み全体の学習表記を除く) を全体変換の最後に置くので、比べる CONVNBEST の先頭は
            // そのどちらでもない最初の候補 (無ければ最初の書き換えの候補) にする
            let shown: Vec<(usize, String)> = nbest
                .candidates
                .iter()
                .enumerate()
                .map(|(i, c)| (i, c.surface()))
                .filter(|(_, s)| *s != input.kana)
                .collect();
            let rewrite = |i: usize, s: &str| {
                i >= nbest.typo_head
                    && nbest.best != Some(i)
                    && !(nbest.learned && i == nbest.typo_head)
                    && suggest::to_hiragana(s) == input.kana
            };
            let top = shown
                .iter()
                .find(|(i, s)| !rewrite(*i, s))
                .or_else(|| shown.first())
                .map(|(_, s)| s.clone());
            if list.first().map(|s| s.surface()) != top {
                changed += 1;
                if ci == 0 && args.detail {
                    details.push(format!(
                        "- 先頭変化: {} → {} (CONVNBEST {})",
                        input.kana,
                        list.first().map_or(String::new(), |s| format!("{} [{}]", s.surface(), s.kind.name())),
                        top.unwrap_or_default()
                    ));
                }
            }
        }
        let mut within = [0usize; 3];
        let mut within3_by_edits = [0usize; 3];
        let mut total_by_edits = [0usize; 3];
        let mut typo_right = [0usize; 3];
        // 打鍵を使わない、かなを戻した近似の補正 (今の CONVNBEST) で補正の候補が正解になった件数
        let mut kana_right = [0usize; 3];
        for (edits, input) in &inputs.crushed {
            let (list, ms) = run(input, &tuning);
            times.push(ms);
            let r = rank(&list, &input.target);
            let keys = Keys::parse(&input.kana, &input.field).unwrap();
            let source = if fuzzy { convert::TypoSource::Keys(&keys) } else { convert::TypoSource::Off };
            let nbest = convert::convert_nbest_scored(
                &input.kana, None, &dictionary, &user, &matrix, &functional, &learning, source, &level_tuning,
            );
            typo_right[edits - 1] += usize::from(
                nbest.candidates.iter().any(|c| c.typo && c.surface() == input.target),
            );
            let source = if fuzzy { convert::TypoSource::Kana } else { convert::TypoSource::Off };
            let approx = convert::convert_nbest_scored(
                &input.kana, None, &dictionary, &user, &matrix, &functional, &learning, source, &level_tuning,
            );
            kana_right[edits - 1] += usize::from(
                approx.candidates.iter().any(|c| c.typo && c.surface() == input.target),
            );
            for (k, limit) in [1, 3, 9].into_iter().enumerate() {
                within[k] += usize::from(r.is_some_and(|r| r < limit));
            }
            total_by_edits[edits - 1] += 1;
            within3_by_edits[edits - 1] += usize::from(r.is_some_and(|r| r < 3));
            if ci == 0 && args.detail && r.is_none_or(|r| r >= 3) && *edits == 1 {
                let shown: Vec<String> = list.iter().take(3).map(|s| s.surface()).collect();
                details.push(format!("- 崩し (誤り 1) で 3 位以内に無い: {} 正解 {} 上位 {:?}", input.kana, input.target, shown));
            }
        }
        let hit = |list: &[Input], times: &mut Vec<f64>| -> usize {
            let mut n = 0;
            for input in list {
                let (got, ms) = run(input, &tuning);
                times.push(ms);
                n += usize::from(rank(&got, &input.target).is_some_and(|r| r < 9));
            }
            n
        };
        let partial: Vec<usize> = (0..4).map(|i| hit(&inputs.partial[i], &mut times)).collect();
        let words: Vec<usize> = (0..4).map(|i| hit(&inputs.words[i], &mut times)).collect();
        let total = inputs.crushed.len();
        let median = percentile(&mut times, 0.5);
        let p95 = percentile(&mut times, 0.95);
        let max = percentile(&mut times, 1.0);
        // 時間の列は既存の入力だけで測るので、打ちかけの入力の時間は数えない
        let mut typing_shown = 0;
        let mut typing_top = 0;
        let mut typing_listed = 0;
        for (typed, input) in &inputs.typing {
            let (list, _) = run(input, &tuning);
            let is_typo = |s: &suggest::Suggestion| matches!(s.kind, suggest::Kind::Typo | suggest::Kind::PredictTypo);
            typing_shown += usize::from(list.iter().take(3).any(is_typo));
            let Some(first) = list.first().filter(|s| is_typo(s)) else { continue };
            typing_top += 1;
            if ci == 0 && args.detail && typing_listed < 40 {
                typing_listed += 1;
                details.push(format!(
                    "- 打ちかけで補正が先頭: 打鍵 {typed} かな {} 先頭 {} [{}] 正解 {}",
                    input.kana,
                    first.surface(),
                    first.kind.name(),
                    input.target
                ));
            }
        }
        let typing_total = inputs.typing.len();
        println!(
            "| {} | {changed} | {false_shown}/{false_top} | 3位内 {typing_shown} ({}) / 先頭 {typing_top} ({}) | {} | {} | {} | {}/{}/{} | {}/{}/{} ({}/{}/{}) | {}/{} | {}/{} | {}/{} | {}/{} | {median:.2} / {p95:.2} / {max:.2} |",
            if spec.is_empty() { "既定" } else { spec },
            pct(typing_shown, typing_total),
            pct(typing_top, typing_total),
            pct(within[0], total),
            pct(within[1], total),
            pct(within[2], total),
            pct(within3_by_edits[0], total_by_edits[0]),
            pct(within3_by_edits[1], total_by_edits[1]),
            pct(within3_by_edits[2], total_by_edits[2]),
            pct(typo_right[0], total_by_edits[0]),
            pct(typo_right[1], total_by_edits[1]),
            pct(typo_right[2], total_by_edits[2]),
            pct(kana_right[0], total_by_edits[0]),
            pct(kana_right[1], total_by_edits[1]),
            pct(kana_right[2], total_by_edits[2]),
            pct(partial[0], inputs.partial[0].len()),
            pct(partial[1], inputs.partial[1].len()),
            pct(partial[2], inputs.partial[2].len()),
            pct(partial[3], inputs.partial[3].len()),
            pct(words[0], inputs.words[0].len()),
            pct(words[1], inputs.words[1].len()),
            pct(words[2], inputs.words[2].len()),
            pct(words[3], inputs.words[3].len()),
        );
    }
    if !details.is_empty() {
        println!("\n## 例 (最初の調整値)\n");
        for line in &details {
            println!("{line}");
        }
    }

    if args.timing {
        // 文の打鍵を 1 打ずつ打ったときの SUGGEST (候補バーの 1 打鍵ぶん)
        let tuning = parse_tuning(&args.configs[0])?;
        let mut times: Vec<f64> = Vec::new();
        let mut slowest = (0.0f64, String::new());
        for s in &sentences {
            for n in 1..=s.romaji.len() {
                let Some((kana, field)) = typo::compose(&s.romaji[..n]) else { continue };
                if kana.chars().count() < cfg.min_suggest_chars {
                    continue;
                }
                let input = Input { kana, field, target: String::new() };
                run(&input, &tuning);
                for _ in 0..3 {
                    let (_, ms) = run(&input, &tuning);
                    if ms > slowest.0 {
                        slowest = (ms, s.romaji[..n].to_string());
                    }
                    times.push(ms);
                }
            }
        }
        let count = times.len();
        let median = percentile(&mut times, 0.5);
        let p95 = percentile(&mut times, 0.95);
        println!("\n## 1 打鍵ぶんの SUGGEST の処理時間\n");
        println!(
            "- {count} 回 (文の打鍵の各接頭辞で、かな {} 文字以上のもの × 3 回): 中央値 {median:.2} ms、p95 {p95:.2} ms、最大 {:.2} ms ({})",
            cfg.min_suggest_chars, slowest.0, slowest.1
        );
    }
    Ok(())
}
