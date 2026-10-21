// 学習の再生評価 (docs/design/eval-sets.md の B)
//
// 実ユーザの learning_stats.tsv を入力に、2 つのモードを測る。
// - 学習が空: すべての p の行 (句の学習) の読みを入力全体の候補 (CONVNBEST。補正なし) にし、行の表記が
//   1 位・3 位以内に入る割合
// - 学習あり: w・p・q の件を最後に使った通し番号 t の順 (確定の順の近似) に並べ、件の数の 8 割の位置にある件の
//   t を分け目の t とする。t が分け目の t 以下の件を前、大きい件を後ろとし、前の件を d と t のまま取り込み、
//   後ろの p と q の件を測る
//   - 同じ t の件は前と後ろに分けない (1 回の確定で数えた件や、古い learning_context.tsv の取り込みの件は
//     同じ t を持つ)
//   - 後ろの件のうち d - 1 が MIN_KEEP 以上の件は、分け目より前にも使っていたものとみなし、d - 1 と分け目の t で
//     前の学習にも入れる。同じ件は後の行にまとめるので、こうしないと繰り返し使った件だけが自分の学習を失う
//   - 変換 (p の行): CONVNBEST と同じ関数で 1 位・3 位以内に入る割合
//   - 予測 (p と q の行): 読みを標準のローマ字にした打鍵の先頭 50% (切り上げ) で SUGGEST を引き、行の表記が
//     9 位以内に入る割合。q の行は文脈の表記を前文脈にする (文脈の読みはファイルに無いので空)
//   - 「既出」は前の学習に入れた件か (読み, 表記) が前の p か q の件にもある行、「初出」はそれ以外の行
// learning_stats.tsv は追記ログなので、同じ件の行は後の行だけを使う (エンジンの読み込みと同じ)。
// ファイルは読むだけで、学習はメモリ上にだけ作る。
// 1 位を外した行は、学習が空のときの 1 位と比べて内訳を出す (件数だけ)。あわせて、外れが語の学習の割引の
// 上限の例外と前文脈のどちらで直りうるかを見るため、正解と 1 位の自立語の違いで分類する: 固定 (1 位が読み全体の
// 学習表記)・分けられない・自立語が同じ・区切り・同音 (違う語の今の度合いが正解の方が大きい / どちらも学習あり /
// 正解に学習なし)・その他 (1 位を分けられない)。
// 結果は件数と割合だけを Markdown で標準出力へ書く (読み・表記は書かない)。
//
// 使い方:
//   cargo run --release --example learning_eval -- --stats <learning_stats.tsv>
//       [--weights "half=5000,ww=1500"]... [--empty]
//
// オプション:
//   --stats <tsv>      learning_stats.tsv
//   --weights <設定>   定数の組 (カンマ区切りの key=value)。複数指定でき、指定ごとに 1 行の結果を出す。
//                      無ければ既定値 (learn::WEIGHTS) だけ。key は half (HALF_LIFE)・ww・wm (WORD_WEIGHT・
//                      WORD_MAX)・cw・cm (CONTEXT_*)・pw・pm・pb (PHRASE_WEIGHT・PHRASE_MAX・PHRASE_BASE)
//   --empty            学習ありのモードの表に、学習が空のときの行も出す
//   --dict <dir>       Mozc 辞書のディレクトリ (既定は references/mozc/src/data/dictionary_oss)
//   --ut <file>        UT 辞書 (既定は references/mozcdic-ut/dictionary_ut.txt。無ければ使わない)

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
#[path = "../src/typo.rs"]
mod typo;
#[allow(dead_code)]
#[path = "../src/userdict.rs"]
mod userdict;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use config::Config;
use convert::Context;
use dict::Dictionary;
use learn::{LearningStore, MIN_KEEP, Weights};
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use typo::Keys;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    stats: PathBuf,
    weights: Vec<String>,
    empty: bool,
    dict: PathBuf,
    ut: PathBuf,
}

fn parse_args() -> BoxResult<Args> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut stats = None;
    let mut args = Args {
        stats: PathBuf::new(),
        weights: Vec::new(),
        empty: false,
        dict: root.join("references/mozc/src/data/dictionary_oss"),
        ut: root.join("references/mozcdic-ut/dictionary_ut.txt"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--stats" => stats = Some(PathBuf::from(value()?)),
            "--weights" => args.weights.push(value()?),
            "--empty" => args.empty = true,
            "--dict" => args.dict = PathBuf::from(value()?),
            "--ut" => args.ut = PathBuf::from(value()?),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    args.stats = stats.ok_or("--stats が必要です")?;
    if args.weights.is_empty() {
        args.weights.push(String::new());
    }
    Ok(args)
}

/// 定数の組の文字列 (key=value のカンマ区切り) を既定値に当てる
fn parse_weights(spec: &str) -> BoxResult<Weights> {
    let mut w = learn::WEIGHTS;
    for item in spec.split(',').filter(|s| !s.is_empty()) {
        let (key, value) = item.split_once('=').ok_or_else(|| format!("key=value ではありません: {item}"))?;
        match key {
            "half" => w.half_life = value.parse()?,
            "ww" => w.word_weight = value.parse()?,
            "wm" => w.word_max = value.parse()?,
            "cw" => w.context_weight = value.parse()?,
            "cm" => w.context_max = value.parse()?,
            "pw" => w.phrase_weight = value.parse()?,
            "pm" => w.phrase_max = value.parse()?,
            "pb" => w.phrase_base = value.parse()?,
            other => return Err(format!("不明な key: {other}").into()),
        }
    }
    Ok(w)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Word,
    Phrase,
    CtxPhrase,
}

/// learning_stats.tsv の 1 件
struct Entry {
    kind: Kind,
    /// 文脈の句の学習 (q) の文脈の表記。ほかは空
    context: String,
    reading: String,
    surface: String,
    d: f64,
    t: u64,
    /// 取り込みに渡す元の行
    line: String,
}

/// learning_stats.tsv の w・p・q の行を読む。同じ件は後の行が勝ち、件の位置も後の行のものにする。
/// 戻り値は (件, 読んだ w・p・q の有効な行の数)
fn load_entries(text: &str) -> (Vec<Entry>, usize) {
    let mut slots: Vec<Option<Entry>> = Vec::new();
    let mut index: HashMap<(Kind, String, String, String), usize> = HashMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let filled = |fields: &[&str]| fields.iter().all(|f| !f.is_empty());
        let (kind, context, reading, surface, d, t) = match f.as_slice() {
            ["w", r, s, d, t] if filled(&[r, s]) => (Kind::Word, "", *r, *s, *d, *t),
            ["p", r, s, d, t, "seg" | "whole"] if filled(&[r, s]) => (Kind::Phrase, "", *r, *s, *d, *t),
            ["q", c, r, s, d, t] if filled(&[c, r, s]) => (Kind::CtxPhrase, *c, *r, *s, *d, *t),
            _ => continue,
        };
        let (Ok(d), Ok(t)) = (d.parse::<f64>(), t.parse::<u64>()) else {
            continue;
        };
        let entry = Entry {
            kind,
            context: context.to_string(),
            reading: reading.to_string(),
            surface: surface.to_string(),
            d,
            t,
            line: line.to_string(),
        };
        let key = (kind, entry.context.clone(), entry.reading.clone(), entry.surface.clone());
        if let Some(old) = index.insert(key, slots.len()) {
            slots[old] = None;
        }
        slots.push(Some(entry));
    }
    let lines = slots.len();
    (slots.into_iter().flatten().collect(), lines)
}

/// 学習ありの分け方
struct Split {
    /// 前の件の数 (t の順に並べた件の先頭から)
    front: usize,
    /// 分け目の t
    split_t: u64,
    /// 前の学習にも入れた後ろの件 (並べた件の位置)
    carried: HashSet<usize>,
    /// 分け目の時点の学習として load_stats_from に渡す行
    train_text: String,
}

/// 行の d と t を置き換える
fn with_stat(line: &str, kind: Kind, d: f64, t: u64) -> String {
    let mut f: Vec<String> = line.split('\t').map(str::to_string).collect();
    let at = if kind == Kind::CtxPhrase { 4 } else { 3 };
    f[at] = d.to_string();
    f[at + 1] = t.to_string();
    f.join("\t")
}

/// t の順に並べた件を前と後ろに分ける
fn split_entries(entries: &[Entry]) -> Split {
    let head = entries.len() * 8 / 10;
    let split_t = head.checked_sub(1).map_or(0, |i| entries[i].t);
    let front = if head == 0 { 0 } else { entries.partition_point(|e| e.t <= split_t) };
    // 確定 1 回で度合いは 1 増えるので、後ろの件の最後の確定の前の度合いは d - 1
    let carried: HashSet<usize> = (front..entries.len()).filter(|&i| entries[i].d - 1.0 >= MIN_KEEP).collect();
    let mut carried_sorted: Vec<usize> = carried.iter().copied().collect();
    carried_sorted.sort_unstable();
    // 古さは n と t の差で決まるため、分け目の時点の通し番号を後ろの件の最小の t にする
    let serial = entries.get(front).map_or(split_t, |e| e.t);
    let train_text: String = entries[..front]
        .iter()
        .map(|e| format!("{}\n", e.line))
        .chain(carried_sorted.iter().map(|&i| {
            let e = &entries[i];
            format!("{}\n", with_stat(&e.line, e.kind, e.d - 1.0, split_t))
        }))
        .chain([format!("n\t{serial}\n")])
        .collect();
    Split { front, split_t, carried, train_text }
}

/// 測る 1 行 (後ろの p と q の件)
struct Row {
    /// q の行の文脈の表記。p の行は None (変換も測る)
    context: Option<String>,
    reading: String,
    surface: String,
    /// 既出 (前の学習にも入れた件か、(読み, 表記) が前の p か q の件にもある)
    seen: bool,
    /// 学習が空のときの変換の 1 位が行の表記 (p の行だけ)
    empty_top: bool,
}

/// 割合を数える。予測は行の種類 [p, q] ごと
#[derive(Default, Clone, Copy)]
struct Tally {
    /// 変換を測った行 (p の行)
    conv_total: usize,
    conv1: usize,
    conv3: usize,
    /// 変換で 1 位に入らなかった行のうち、1 位が読み全体の学習表記だった行
    miss_pinned: usize,
    /// 予測を測れた行 (読みを打鍵にできた行)
    pred_total: [usize; 2],
    pred9: [usize; 2],
}

/// 変換の 1 位を外した行の内訳 (学習が空のときとの比較)
#[derive(Default)]
struct Misses {
    /// 学習が空なら 1 位だったのに外れた行 [1 位が読み全体の学習表記, それ以外 (経路)]
    lost: [usize; 2],
    /// 学習が空では外れ、学習で 1 位になった行
    gained: usize,
    /// 1 位が読み全体の学習表記で外れた行 (学習が空でも外れていた行を含む)
    pinned: usize,
    /// そのうち行の表記が、その読みの学習にある (前にも選んだことがある)
    pinned_known: usize,
}

/// 変換の 1 位を外した行の分類 (表の行の順)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum MissClass {
    Pinned,
    Unsplit,
    SameWords,
    Boundary,
    HomophoneMore,
    HomophoneBoth,
    HomophoneUnlearned,
    Other,
}

impl MissClass {
    const ALL: [MissClass; 8] = [
        MissClass::Pinned,
        MissClass::Unsplit,
        MissClass::SameWords,
        MissClass::Boundary,
        MissClass::HomophoneMore,
        MissClass::HomophoneBoth,
        MissClass::HomophoneUnlearned,
        MissClass::Other,
    ];

    fn label(self) -> &'static str {
        match self {
            MissClass::Pinned => "固定 (1 位が読み全体の学習表記)",
            MissClass::Unsplit => "分けられない (正解を辞書の語に分けられない)",
            MissClass::SameWords => "自立語が同じ",
            MissClass::Boundary => "区切り",
            MissClass::HomophoneMore => "同音・正解の方が多く使われている",
            MissClass::HomophoneBoth => "同音・どちらも学習あり",
            MissClass::HomophoneUnlearned => "同音・正解に学習なし",
            MissClass::Other => "その他 (1 位を辞書の語に分けられない)",
        }
    }
}

/// 外した行を分類する。right・top は正解と 1 位を split_words で分けた自立語 (分けられなければ None)、
/// degree は語の学習の今の度合い (無ければ 0)
fn classify_miss(
    pinned: bool,
    right: Option<&[(String, String)]>,
    top: Option<&[(String, String)]>,
    degree: impl Fn(&str, &str) -> f64,
) -> MissClass {
    if pinned {
        return MissClass::Pinned;
    }
    let Some(right) = right else {
        return MissClass::Unsplit;
    };
    let Some(top) = top else {
        return MissClass::Other;
    };
    // 多重集合の差 (語の順を保つ)
    let only = |a: &[(String, String)], b: &[(String, String)]| {
        let mut rest: HashMap<&(String, String), usize> = HashMap::new();
        for w in b {
            *rest.entry(w).or_default() += 1;
        }
        let mut out: Vec<(String, String)> = Vec::new();
        for w in a {
            match rest.get_mut(w) {
                Some(n) if *n > 0 => *n -= 1,
                _ => out.push(w.clone()),
            }
        }
        out
    };
    let d_right = only(right, top);
    let d_top = only(top, right);
    if d_right.is_empty() && d_top.is_empty() {
        return MissClass::SameWords;
    }
    fn readings(list: &[(String, String)]) -> Vec<&str> {
        let mut r: Vec<&str> = list.iter().map(|(r, _)| r.as_str()).collect();
        r.sort_unstable();
        r
    }
    if readings(&d_right) != readings(&d_top) {
        return MissClass::Boundary;
    }
    let mut used = vec![false; d_top.len()];
    let mut best: Option<(f64, f64)> = None;
    for (reading, surface) in &d_right {
        let Some(j) = (0..d_top.len()).find(|&j| !used[j] && d_top[j].0 == *reading) else {
            continue;
        };
        used[j] = true;
        let pair = (degree(reading, surface), degree(&d_top[j].0, &d_top[j].1));
        if best.is_none_or(|(r, t)| pair.0 - pair.1 > r - t) {
            best = Some(pair);
        }
    }
    let (r, t) = best.expect("読みが一致する差の語は 1 組以上ある");
    if r > t {
        MissClass::HomophoneMore
    } else if r > 0.0 {
        MissClass::HomophoneBoth
    } else {
        MissClass::HomophoneUnlearned
    }
}

/// 分類ごとの [行, 3 位以内, 前文脈の学習あり]
#[derive(Default)]
struct ClassTally {
    counts: HashMap<MissClass, [usize; 3]>,
}

fn percent(n: usize, total: usize) -> String {
    format!("{:.1}%", 100.0 * n as f64 / total.max(1) as f64)
}

/// 中央値 / p95 (ms)
fn time_stat(times: &mut [f64]) -> String {
    if times.is_empty() {
        return "-".to_string();
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| times[((times.len() as f64 - 1.0) * q).round() as usize];
    format!("{:.2} / {:.2}", at(0.5), at(0.95))
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let (mut entries, lines) = load_entries(&std::fs::read_to_string(&args.stats)?);
    // 同じ t の件は記録の位置の順にする (sort_by_key は安定)
    entries.sort_by_key(|e| e.t);

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
    let cfg = Config::default();
    eprintln!("辞書の読み込み: {:.1} 秒", started.elapsed().as_secs_f64());
    let split = |r: &str, s: &str| convert::split_words(r, s, &dictionary, &user, &matrix, &functional);

    let count = |list: &[Entry], kind: Kind| list.iter().filter(|e| e.kind == kind).count();
    let Split { front, split_t, carried, train_text } = split_entries(&entries);
    let (train, test) = entries.split_at(front);
    let known: HashSet<(&str, &str)> = train
        .iter()
        .filter(|e| e.kind != Kind::Word)
        .map(|e| (e.reading.as_str(), e.surface.as_str()))
        .collect();

    println!("## データ\n");
    println!(
        "- learning_stats.tsv: w {} 件・p {} 件・q {} 件 (w・p・q の有効な行 {} 行のうち、同じ件の前の行 {} 行を除く)",
        count(&entries, Kind::Word),
        count(&entries, Kind::Phrase),
        count(&entries, Kind::CtxPhrase),
        lines,
        lines - entries.len(),
    );
    println!(
        "- 学習ありの分け方 (t の順。分け目の t = {split_t}): 前 {} 件 (w {} / p {} / q {})、後ろ {} 件 (w {} / p {} / q {})",
        train.len(),
        count(train, Kind::Word),
        count(train, Kind::Phrase),
        count(train, Kind::CtxPhrase),
        test.len(),
        count(test, Kind::Word),
        count(test, Kind::Phrase),
        count(test, Kind::CtxPhrase),
    );
    let carried_count = |kind: Kind| carried.iter().filter(|&&i| entries[i].kind == kind).count();
    println!(
        "- 前の学習にも入れた後ろの件 (d - 1 が {MIN_KEEP} 以上): {} 件 (w {} / p {} / q {})",
        carried.len(),
        carried_count(Kind::Word),
        carried_count(Kind::Phrase),
        carried_count(Kind::CtxPhrase),
    );

    // 学習が空のモード。後ろの p の行の 1 位は、学習ありの内訳の比較にも使う
    let empty = LearningStore::in_memory();
    let mut empty_tally = Tally::default();
    let mut empty_times: Vec<f64> = Vec::new();
    let mut empty_top: HashMap<(&str, &str), bool> = HashMap::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Phrase) {
        let t = Instant::now();
        let nbest = convert::convert_nbest_scored(
            &e.reading, None, &dictionary, &user, &matrix, &functional, &empty,
            convert::TypoSource::Off, &typo::TUNING,
        );
        empty_times.push(t.elapsed().as_secs_f64() * 1000.0);
        let rank = nbest.candidates.iter().position(|c| c.surface() == e.surface);
        empty_tally.conv_total += 1;
        empty_tally.conv1 += usize::from(rank == Some(0));
        empty_tally.conv3 += usize::from(rank.is_some_and(|r| r < 3));
        empty_top.insert((&e.reading, &e.surface), rank == Some(0));
    }

    let rows: Vec<Row> = (front..entries.len())
        .filter(|&i| entries[i].kind != Kind::Word)
        .map(|i| {
            let e = &entries[i];
            Row {
                context: (e.kind == Kind::CtxPhrase).then(|| e.context.clone()),
                reading: e.reading.clone(),
                surface: e.surface.clone(),
                seen: carried.contains(&i) || known.contains(&(e.reading.as_str(), e.surface.as_str())),
                empty_top: e.kind == Kind::Phrase && empty_top[&(e.reading.as_str(), e.surface.as_str())],
            }
        })
        .collect();
    let rows_of = |seen: Option<bool>, q: bool| {
        rows.iter().filter(|r| seen.is_none_or(|s| r.seen == s) && r.context.is_some() == q).count()
    };
    println!(
        "- 学習ありで測る行: {} 行 (p {} / q {})、うち既出 {} 行 (p {} / q {})・初出 {} 行 (p {} / q {})",
        rows.len(),
        rows_of(None, false),
        rows_of(None, true),
        rows_of(Some(true), false) + rows_of(Some(true), true),
        rows_of(Some(true), false),
        rows_of(Some(true), true),
        rows_of(Some(false), false) + rows_of(Some(false), true),
        rows_of(Some(false), false),
        rows_of(Some(false), true),
    );
    {
        let mut store = LearningStore::in_memory();
        store.load_stats_from(train_text.as_bytes());
        store.refresh_phrases(&split);
        let unsplit = rows.iter().filter(|r| split(&r.reading, &r.surface).is_none()).count();
        println!("- 取り込んだ学習: {} 件 (確定の通し番号 {})", store.entry_count(), store.serial());
        println!("- 測る行のうち辞書の語に分けられない (読み, 表記): {unsplit} 行");
    }

    println!("\n## 学習が空 (すべての p の行)\n");
    println!("変換 1/3 = CONVNBEST で 1 位・3 位以内の割合。時間は CONVNBEST 1 回の中央値 / p95 (ms)。\n");
    println!("| 行 | 変換 1 | 変換 3 | CONVNBEST |");
    println!("|---|---|---|---|");
    println!(
        "| {} | {} | {} | {} |",
        empty_tally.conv_total,
        percent(empty_tally.conv1, empty_tally.conv_total),
        percent(empty_tally.conv3, empty_tally.conv_total),
        time_stat(&mut empty_times),
    );

    println!("\n## 学習あり (後ろの p と q の行)\n");
    println!("変換 1/3 = p の行の CONVNBEST で 1 位・3 位以内の割合、予測 9 = 打鍵の先頭 50% の SUGGEST で 9 位以内の割合");
    println!("(全体 (p / q) の順)。既出は前の学習にも入れた件か (読み, 表記) が前の p か q の件にもある行、初出はそれ以外の行。");
    println!("変換 1 位外 (固定) = 変換で 1 位に入らなかった行の数 (うち 1 位が読み全体の学習表記の行の数)。時間は CONVNBEST・SUGGEST 1 回の中央値 / p95 (ms)。\n");
    println!("| 方式 | 変換 1 | 変換 3 | 予測 9 | 既出 変換 1 | 既出 変換 3 | 既出 予測 9 | 初出 変換 1 | 初出 変換 3 | 初出 予測 9 | 変換 1 位外 (固定) | CONVNBEST | SUGGEST |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");

    let mut details: Vec<String> = Vec::new();
    let mut class_tables: Vec<String> = Vec::new();
    let mut contexts_of: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for r in &rows {
        if let Some(c) = &r.context {
            contexts_of.entry((r.reading.as_str(), r.surface.as_str())).or_default().push(c.as_str());
        }
    }
    let mut configs: Vec<(String, Option<Weights>)> = Vec::new();
    if args.empty {
        configs.push(("学習なし".to_string(), None));
    }
    for spec in &args.weights {
        let name = if spec.is_empty() { "既定".to_string() } else { spec.clone() };
        configs.push((name, Some(parse_weights(spec)?)));
    }
    for (name, weights) in configs {
        let mut store = LearningStore::in_memory();
        if let Some(w) = weights {
            store.set_weights(w);
            store.load_stats_from(train_text.as_bytes());
            store.refresh_phrases(&split);
        }
        let mut all = Tally::default();
        let mut seen = Tally::default();
        let mut unseen = Tally::default();
        let mut nbest_times: Vec<f64> = Vec::new();
        let mut suggest_times: Vec<f64> = Vec::new();
        let mut misses = Misses::default();
        let mut classes = ClassTally::default();
        for row in &rows {
            let kind = usize::from(row.context.is_some());
            let ctx = row.context.as_ref().map(|c| Context { reading: String::new(), surface: c.clone() });
            // 前文脈は変換の経路に効かないので、q の行は予測だけを測る
            let conversion = row.context.is_none().then(|| {
                let t = Instant::now();
                let nbest = convert::convert_nbest_scored(
                    &row.reading, None, &dictionary, &user, &matrix, &functional, &store,
                    convert::TypoSource::Off, &typo::TUNING,
                );
                nbest_times.push(t.elapsed().as_secs_f64() * 1000.0);
                let rank = nbest.candidates.iter().position(|c| c.surface() == row.surface);
                if rank == Some(0) {
                    misses.gained += usize::from(!row.empty_top);
                } else {
                    if row.empty_top {
                        misses.lost[usize::from(!nbest.learned)] += 1;
                    }
                    if nbest.learned {
                        misses.pinned += 1;
                        misses.pinned_known += usize::from(store.has_phrase(&row.reading, &row.surface));
                    }
                    let (right, top) = if nbest.learned {
                        (None, None)
                    } else {
                        let top = nbest.candidates.first().and_then(|c| split(&row.reading, &c.surface()));
                        (split(&row.reading, &row.surface), top)
                    };
                    let class = classify_miss(nbest.learned, right.as_deref(), top.as_deref(), |r, s| {
                        store.word_degree(r, s).unwrap_or(0.0)
                    });
                    let ctx_learned = contexts_of
                        .get(&(row.reading.as_str(), row.surface.as_str()))
                        .is_some_and(|cs| {
                            cs.iter().any(|c| store.ctx_phrase_stat(c, &row.reading, &row.surface).is_some())
                        });
                    let slot = classes.counts.entry(class).or_default();
                    slot[0] += 1;
                    slot[1] += usize::from(rank.is_some_and(|r| r < 3));
                    slot[2] += usize::from(ctx_learned);
                }
                (rank, nbest.learned)
            });
            let prediction = typo::to_romaji(&row.reading, false).and_then(|romaji| {
                let n = romaji.len().div_ceil(2);
                typo::compose(&romaji[..n])
            });
            let predicted = prediction.filter(|(kana, _)| !kana.is_empty()).map(|(kana, field)| {
                let keys = Keys::parse(&kana, &field).expect("打鍵のフィールド");
                let t = Instant::now();
                let list = suggest::suggest(
                    &kana, ctx.as_ref(), &keys, &dictionary, &user, &matrix, &functional, &store, &cfg,
                    &typo::TUNING,
                );
                suggest_times.push(t.elapsed().as_secs_f64() * 1000.0);
                list.iter().take(9).any(|s| s.surface() == row.surface)
            });
            let part = if row.seen { &mut seen } else { &mut unseen };
            for tally in [&mut all, part] {
                if let Some((rank, learned)) = conversion {
                    tally.conv_total += 1;
                    tally.conv1 += usize::from(rank == Some(0));
                    tally.conv3 += usize::from(rank.is_some_and(|r| r < 3));
                    tally.miss_pinned += usize::from(rank != Some(0) && learned);
                }
                if let Some(hit) = predicted {
                    tally.pred_total[kind] += 1;
                    tally.pred9[kind] += usize::from(hit);
                }
            }
        }
        let pred = |t: &Tally| {
            let p = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
            format!(
                "{:.1}% ({:.1} / {:.1})",
                p(t.pred9[0] + t.pred9[1], t.pred_total[0] + t.pred_total[1]),
                p(t.pred9[0], t.pred_total[0]),
                p(t.pred9[1], t.pred_total[1])
            )
        };
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} ({}) | {} | {} |",
            percent(all.conv1, all.conv_total),
            percent(all.conv3, all.conv_total),
            pred(&all),
            percent(seen.conv1, seen.conv_total),
            percent(seen.conv3, seen.conv_total),
            pred(&seen),
            percent(unseen.conv1, unseen.conv_total),
            percent(unseen.conv3, unseen.conv_total),
            pred(&unseen),
            all.conv_total - all.conv1,
            all.miss_pinned,
            time_stat(&mut nbest_times),
            time_stat(&mut suggest_times),
        );
        details.push(format!(
            "- {name}: 学習で外れた {} 行 (1 位が読み全体の学習表記 {}・経路 {})、学習で当たった {} 行。\
             1 位が読み全体の学習表記で外れた {} 行のうち、行の表記を学習で覚えている {} 行",
            misses.lost[0] + misses.lost[1],
            misses.lost[0],
            misses.lost[1],
            misses.gained,
            misses.pinned,
            misses.pinned_known,
        ));
        let class_total: usize = classes.counts.values().map(|c| c[0]).sum();
        assert_eq!(class_total, all.conv_total - all.conv1, "分類の合計が外れの数と一致しない");
        let mut table = format!(
            "\n### {name}\n\n| 分類 | 行 | 3 位以内 | 前文脈の学習あり |\n|---|---|---|---|\n"
        );
        for class in MissClass::ALL {
            let [n, top3, ctx] = classes.counts.get(&class).copied().unwrap_or_default();
            table.push_str(&format!("| {} | {n} | {top3} | {ctx} |\n", class.label()));
        }
        let sum = |i: usize| classes.counts.values().map(|c| c[i]).sum::<usize>();
        table.push_str(&format!("| 合計 | {} | {} | {} |", sum(0), sum(1), sum(2)));
        class_tables.push(table);
    }
    println!("\n## 1 位を外した行の内訳\n");
    for line in details {
        println!("{line}");
    }
    println!(
        "\n分類: 1 位を正解と同じく辞書の自立語に分け、片方にだけある語を比べる。3 位以内 = 正解が 3 位以内、\
         前文脈の学習あり = 同じ (読み, 表記) の後ろの q の行の文脈の学習が、評価に使う学習に入っている行。"
    );
    for table in class_tables {
        println!("{table}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_same_t_and_carries_reused_back_entries() {
        // 8 割の位置 (21 件の 16 件目と 17 件目) が t = 3 のかたまりの中にある
        let mut text = String::from("p\tj\tJ\t1\t4\tseg\nw\ta\tA\t1\t1\np\tb\tB\t1\t2\tseg\n");
        for i in 0..15 {
            text.push_str(&format!("q\tx\tc{i}\tC{i}\t1\t3\n"));
        }
        text.push_str("p\tj\tJ\t2.5\t5\tseg\nw\tk\tK\t1.04\t6\np\tl\tL\t1\t6\twhole\nq\tx\tm\tM\t1.5\t7\n");
        let (mut entries, lines) = load_entries(&text);
        entries.sort_by_key(|e| e.t);
        assert_eq!((entries.len(), lines), (21, 22));

        let split = split_entries(&entries);
        assert_eq!((split.front, split.split_t), (17, 3));
        assert!(entries[..split.front].iter().all(|e| e.t <= 3));
        assert!(entries[split.front..].iter().all(|e| e.t > 3));
        assert_eq!(entries[..split.front].iter().filter(|e| e.kind == Kind::CtxPhrase).count(), 15);

        let mut carried: Vec<&str> = split.carried.iter().map(|&i| entries[i].reading.as_str()).collect();
        carried.sort_unstable();
        assert_eq!(carried, ["j", "m"]);
        let train: Vec<&str> = split.train_text.lines().collect();
        assert_eq!(train.len(), 17 + 2 + 1);
        assert!(train.contains(&"p\tj\tJ\t1.5\t3\tseg"));
        assert!(train.contains(&"q\tx\tm\tM\t0.5\t3"));
        assert!(!train.iter().any(|l| l.contains("\tk\t") || l.contains("\tl\t") || l.contains("\t4\t")));
        assert_eq!(train.last(), Some(&"n\t5"));

        let mut store = LearningStore::in_memory();
        assert_eq!(store.load_stats_from(split.train_text.as_bytes()), 20);
    }

    #[test]
    fn classify_miss_by_independent_words() {
        let words = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(r, s)| (r.to_string(), s.to_string())).collect()
        };
        let degrees = |r: &str, s: &str| match (r, s) {
            ("a", "A1") => 3.0,
            ("a", "A2") => 1.0,
            ("b", "B1") => 1.0,
            ("b", "B2") => 2.0,
            _ => 0.0,
        };
        let classify = |right: Option<&[(&str, &str)]>, top: Option<&[(&str, &str)]>| {
            let right = right.map(words);
            let top = top.map(words);
            classify_miss(false, right.as_deref(), top.as_deref(), degrees)
        };
        let x: &[(&str, &str)] = &[("x", "X")];

        assert_eq!(classify_miss(true, None, None, degrees), MissClass::Pinned);
        assert_eq!(classify(None, Some(x)), MissClass::Unsplit);
        assert_eq!(classify(Some(x), None), MissClass::Other);
        assert_eq!(classify(Some(x), Some(x)), MissClass::SameWords);
        assert_eq!(classify(Some(&[("x", "X"), ("y", "Y")]), Some(&[("xy", "XY")])), MissClass::Boundary);
        // 同じ語が 2 つと 1 つ: 多重集合の差で読みが合わない
        assert_eq!(classify(Some(&[("x", "X"), ("x", "X")]), Some(x)), MissClass::Boundary);
        assert_eq!(classify(Some(&[("x", "X"), ("a", "A1")]), Some(&[("x", "X"), ("a", "A2")])), MissClass::HomophoneMore);
        // (正解 - 1 位) が最大の組で決める: a は 3 - 1、b は 1 - 2
        assert_eq!(
            classify(Some(&[("b", "B1"), ("a", "A1")]), Some(&[("a", "A2"), ("b", "B2")])),
            MissClass::HomophoneMore
        );
        assert_eq!(classify(Some(&[("b", "B1")]), Some(&[("b", "B2")])), MissClass::HomophoneBoth);
        assert_eq!(classify(Some(&[("a", "A3")]), Some(&[("a", "A1")])), MissClass::HomophoneUnlearned);
        assert_eq!(classify(Some(&[("c", "C1")]), Some(&[("c", "C2")])), MissClass::HomophoneUnlearned);
    }
}
