// 学習の再生評価 (docs/design/eval-sets.md の B)
//
// 実ユーザの learning_stats.tsv を入力に、2 つのモードを測る。
// - 学習が空: すべての p の行 (句の学習) の読みを入力全体の候補 (CONVNBEST。補正なし) にし、行の表記が
//   1 位・3 位以内に入る割合。1 位を外した行は学習ありと同じ分類でも数え、学習によらない変換の弱さの割合を
//   学習ありより多い行で見る
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
// 読みに ASCII 英字を含む p の行は、どちらのモードでも変換の測定から除く。モードレス入力の英字区間は
// (読み = 表記) の句として学習に入るが、実際の入力では変換に渡らないため。学習の取り込みと既出・初出の判定には
// 含める (エンジンは英字区間の学習も持つ)。予測は、読みを打鍵にできないので測れた行に入らない。
// learning_stats.tsv は追記ログなので、同じ件の行は後の行だけを使う (エンジンの読み込みと同じ)。
// ファイルは読むだけで、学習はメモリ上にだけ作る。
// 1 位を外した行は、学習が空のときの 1 位と比べて内訳を出す (件数だけ)。あわせて、外れが語の学習の割引の
// 上限の例外と前文脈のどちらで直りうるかを見るため、正解と 1 位の自立語の違いで分類する: 固定 (1 位が読み全体の
// 学習表記)・分けられない・自立語が同じ・同じ区間・区切り・同音 (違う語の今の度合いが正解の方が大きい / どちらも学習あり /
// 正解に学習なし)・その他 (1 位を分けられない)。固定の得失を見るため、1 位が読み全体の学習表記だった行は、
// その表記と経路の 1-best の表記のどちらが正解かでも数える。
// 辞書に無い語がどんな語かを見るため、分けられない行は正解の表記の文字の種類 (カタカナだけ・英字を含む・
// 漢字を含む・その他) でも数え、カタカナだけの行のうち正解が読み全体のカタカナの行は正解の順位の中央値も出す。
// 区切りの外れが辞書や連接のコストのどこで起きているかを見るため、区切りの行は向き・差の読みの長さ・正解の順位でも
// 数え、向きごとに差に Mozc 辞書 (UT 辞書を積まない) に無い語がある行も出す。
// 区切りの位置が合っているのに学習で直らない理由を見るため、同じ区間の行は向きで分け、正解の差の語が語の学習に
// 数えない語・学習あり・学習なしの行を数える。
// 文節の p の行で同じ (読み, 表記) の q の件があるものは、前の文節を前文脈にして変換する (両方のモード)。
// 実際の入力では前に語があり、BOS から変換すると入力の先頭にしか起きない外れを数えてしまうため。文脈は q の件の
// d の大きい (並べば t の新しい) ものの表記で、読みは同じ表記の文節の p の件から引く。引けない行は前文脈なしで
// 変換する。予測は今までどおり q の行の文脈の表記だけを前文脈にする。
// 入力の先頭の扱いを変えたときの得失を見るため、学習が空のモードは p の行の位置 (読み全体・前文脈を付けた文節・
// q の件はあるが文脈の読みを引けない文節・q の件が無い文節) でも分ける。
// --next-word を指定したときは、後ろの q の行の表記が読みなしの後続語予測 (NEXTWORD) の 9 位以内・1 位に入る割合を、
// 学習なし・あり × 後続語の統計の表なし・ありの 4 つの組で出す (docs/design/next-word-ngram.md)。バックオフの表を引く
// 前文脈の最後の自立語は、文脈の表記と同じ表記の文節の p の件の読みで文脈を辞書の語に分けて求める (q の件の文脈は
// 表記だけのため)。引けない行はバックオフの表を引かない。
// 結果は件数と割合だけを Markdown で標準出力へ書く (読み・表記は書かない)。
//
// 使い方:
//   cargo run --release --example learning_eval -- --stats <learning_stats.tsv>
//       [--weights "half=5000,ww=1500"]... [--empty] [--dump-same-span <file>]
//       [--dump-boundary <file>] [--dump-pinned <file>] [--next-word <next_word.tsv>]
//
// オプション:
//   --stats <tsv>      learning_stats.tsv
//   --weights <設定>   定数の組 (カンマ区切りの key=value)。複数指定でき、指定ごとに 1 行の結果を出す。
//                      無ければ既定値 (learn::WEIGHTS) だけ。key は half (HALF_LIFE)・ww・wm (WORD_WEIGHT・
//                      WORD_MAX)・cw・cm (CONTEXT_*)・pw・pm・pb (PHRASE_WEIGHT・PHRASE_MAX・PHRASE_BASE)
//   --empty            学習ありのモードの表に、学習が空のときの行も出す
//   --dict <dir>       Mozc 辞書のディレクトリ (既定は references/mozc/src/data/dictionary_oss)
//   --ut <file>        UT 辞書 (既定は references/mozcdic-ut/dictionary_ut.txt。無ければ使わない)
//   --dump-same-span <file>
//                      同じ区間の行を 1 行ずつ TSV で file に書く (両方のモード)。列はモード・向き・学習の区分・
//                      正解の順位・正解の経路・1 位の経路。実際の入力の読み・表記が入るので、リポジトリの外に置く
//   --dump-boundary <file>
//                      区切りの行を 1 行ずつ TSV で file に書く (両方のモード)。列はモード・向き・差の読みの長さ・
//                      正解の順位・1 位の差に Mozc 辞書に無い語・正解の差に Mozc 辞書に無い語・正解の経路・1 位の経路。
//                      実際の入力の読み・表記が入るので、リポジトリの外に置く
//   --dump-pinned <file>
//                      読み全体の学習表記で 1 位を固定した行のうち、同じ・当たり以外の行を 1 行ずつ TSV で file に書く
//                      (学習ありのモード)。列はモード・区分・行の種類・読み・正解の表記・固定の表記・経路の 1-best の表記・
//                      正解の順位・固定の表記の句の学習・正解の表記の句の学習 (種類:今の度合い)・読みの句の学習の表記の数。
//                      実際の入力の読み・表記が入るので、リポジトリの外に置く
//   --next-word <tsv>  後続語の統計の表 (next_word.tsv)。指定すると NEXTWORD の評価を出す

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
use convert::{Context, SplitWord};
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
    dump_same_span: Option<PathBuf>,
    dump_boundary: Option<PathBuf>,
    dump_pinned: Option<PathBuf>,
    next_word: Option<PathBuf>,
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
        dump_same_span: None,
        dump_boundary: None,
        dump_pinned: None,
        next_word: None,
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
            "--dump-same-span" => args.dump_same_span = Some(PathBuf::from(value()?)),
            "--dump-boundary" => args.dump_boundary = Some(PathBuf::from(value()?)),
            "--dump-pinned" => args.dump_pinned = Some(PathBuf::from(value()?)),
            "--next-word" => args.next_word = Some(PathBuf::from(value()?)),
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
    /// 句の学習 (p) の種類が読み全体 (whole)。ほかは false
    whole: bool,
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
        let (kind, context, reading, surface, whole, d, t) = match f.as_slice() {
            ["w", r, s, d, t] if filled(&[r, s]) => (Kind::Word, "", *r, *s, false, *d, *t),
            ["p", r, s, d, t, k @ ("seg" | "whole")] if filled(&[r, s]) => {
                (Kind::Phrase, "", *r, *s, *k == "whole", *d, *t)
            }
            ["q", c, r, s, d, t] if filled(&[c, r, s]) => (Kind::CtxPhrase, *c, *r, *s, false, *d, *t),
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
            whole,
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
    /// p の行の種類が読み全体 (whole)。q の行は false
    whole: bool,
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
    SameSpan,
    Boundary,
    HomophoneMore,
    HomophoneBoth,
    HomophoneUnlearned,
    Other,
}

impl MissClass {
    const ALL: [MissClass; 9] = [
        MissClass::Pinned,
        MissClass::Unsplit,
        MissClass::SameWords,
        MissClass::SameSpan,
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
            MissClass::SameSpan => "同じ区間 (区切りは同じで、自立語と付属語の別が違う)",
            MissClass::Boundary => "区切り",
            MissClass::HomophoneMore => "同音・正解の方が多く使われている",
            MissClass::HomophoneBoth => "同音・どちらも学習あり",
            MissClass::HomophoneUnlearned => "同音・正解に学習なし",
            MissClass::Other => "その他 (1 位を辞書の語に分けられない)",
        }
    }
}

/// 正解と 1 位の自立語の多重集合の差 [正解にだけある語, 1 位にだけある語] (語の順を保つ)
fn word_diff(right: &[(String, String)], top: &[(String, String)]) -> (Vec<(String, String)>, Vec<(String, String)>) {
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
    (only(right, top), only(top, right))
}

/// split_path の経路の自立語の (読み, 表記)
fn independent(path: &[SplitWord]) -> Vec<(String, String)> {
    path.iter().filter(|w| !w.functional).map(|w| (w.reading.clone(), w.surface.clone())).collect()
}

/// 経路の自立語の多重集合の差 [正解にだけある語, 1 位にだけある語]。word_diff と同じく先に出た語から相殺し、
/// 残った語は経路の中の区間を持ったまま返す
fn path_diff<'a>(right: &'a [SplitWord], top: &'a [SplitWord]) -> (Vec<&'a SplitWord>, Vec<&'a SplitWord>) {
    fn only<'a>(a: &'a [SplitWord], b: &'a [SplitWord]) -> Vec<&'a SplitWord> {
        let mut rest: HashMap<(&str, &str), usize> = HashMap::new();
        for w in b.iter().filter(|w| !w.functional) {
            *rest.entry((w.reading.as_str(), w.surface.as_str())).or_default() += 1;
        }
        let mut out: Vec<&SplitWord> = Vec::new();
        for w in a.iter().filter(|w| !w.functional) {
            match rest.get_mut(&(w.reading.as_str(), w.surface.as_str())) {
                Some(n) if *n > 0 => *n -= 1,
                _ => out.push(w),
            }
        }
        out
    }
    (only(right, top), only(top, right))
}

/// 差の語のすべてについて、もう片方の経路 other に読みの開始と終了の位置が同じ語 (付属語を含む) がある
fn same_span(diff: &[&SplitWord], other: &[SplitWord]) -> bool {
    diff.iter().all(|d| other.iter().any(|w| w.start == d.start && w.end == d.end))
}

/// 外した行を分類する。right・top は正解と 1 位を split_path で分けた経路 (分けられなければ None)、
/// degree は語の学習の今の度合い (無ければ 0)
fn classify_miss(
    pinned: bool,
    right: Option<&[SplitWord]>,
    top: Option<&[SplitWord]>,
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
    let (d_right, d_top) = word_diff(&independent(right), &independent(top));
    if d_right.is_empty() && d_top.is_empty() {
        return MissClass::SameWords;
    }
    fn readings(list: &[(String, String)]) -> Vec<&str> {
        let mut r: Vec<&str> = list.iter().map(|(r, _)| r.as_str()).collect();
        r.sort_unstable();
        r
    }
    if readings(&d_right) != readings(&d_top) {
        let (p_right, p_top) = path_diff(right, top);
        if same_span(&p_right, top) && same_span(&p_top, right) {
            return MissClass::SameSpan;
        }
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

/// 1 位が読み全体の学習表記 (固定) だった p の行の、固定の表記と経路の 1-best の表記の比較
#[derive(Default)]
struct PinTally {
    total: usize,
    /// 固定の表記が正解で、経路の 1-best は正解でない
    hit: usize,
    /// 固定の表記が正解でなく、経路の 1-best が正解
    lost: usize,
    /// 固定の表記と経路の 1-best が同じ表記 [当たり, 外れ]
    same: [usize; 2],
    /// どちらも正解でなく、両者が違う (経路の 1-best が無い行を含む)
    both_miss: usize,
}

/// 固定した行の、固定の表記と経路の 1-best の表記の比較の区分 (PinTally の数え方)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PinClass {
    Hit,
    Lost,
    /// 固定の表記と経路の 1-best が同じ表記。値は当たりか
    Same(bool),
    BothMiss,
}

impl PinClass {
    /// --dump-pinned の区分の名前。同じ・当たりは書き出さないので None
    fn dump_label(self) -> Option<&'static str> {
        match self {
            PinClass::Hit => Some("固定で当てた"),
            PinClass::Lost => Some("固定で外した"),
            PinClass::Same(true) => None,
            PinClass::Same(false) => Some("同じ・外れ"),
            PinClass::BothMiss => Some("どちらも外れ"),
        }
    }
}

/// pinned は固定の表記、path は経路の 1-best の表記 (無ければ None)、surface は正解の表記
fn pin_class(pinned: &str, path: Option<&str>, surface: &str) -> PinClass {
    let pinned_hit = pinned == surface;
    if path == Some(pinned) {
        PinClass::Same(pinned_hit)
    } else if pinned_hit {
        PinClass::Hit
    } else if path == Some(surface) {
        PinClass::Lost
    } else {
        PinClass::BothMiss
    }
}

impl PinTally {
    fn add(&mut self, class: PinClass) {
        self.total += 1;
        match class {
            PinClass::Hit => self.hit += 1,
            PinClass::Lost => self.lost += 1,
            PinClass::Same(hit) => self.same[usize::from(!hit)] += 1,
            PinClass::BothMiss => self.both_miss += 1,
        }
    }
}

/// --dump-pinned の 1 行 (改行を含む)。同じ・当たりの行は None。mode はモードの名前、whole は p の行の種類が
/// 読み全体か、rank は正解の順位 (0 始まり)、store は評価に使う学習
fn pin_dump_line(
    mode: &str,
    class: PinClass,
    whole: bool,
    reading: &str,
    surface: &str,
    pinned: &str,
    path: Option<&str>,
    rank: Option<usize>,
    store: &LearningStore,
) -> Option<String> {
    let label = class.dump_label()?;
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let phrase = |s: &str| match (store.phrase_stat(reading, s), store.phrase_degree(reading, s)) {
        (Some((_, kind)), Some(degree)) => {
            let kind = match kind {
                learn::PhraseKind::Segment => "seg",
                learn::PhraseKind::Whole => "whole",
            };
            format!("{kind}:{degree:.2}")
        }
        _ => "-".to_string(),
    };
    Some(format!(
        "{mode}\t{label}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        if whole { "whole" } else { "seg" },
        clean(reading),
        clean(surface),
        clean(pinned),
        path.map_or("-".to_string(), clean),
        rank.map_or("-".to_string(), |r| (r + 1).to_string()),
        phrase(pinned),
        phrase(surface),
        store.phrase_surfaces(reading).len(),
    ))
}

/// 分類ごとの [行, 3 位以内, 前文脈の学習あり]
#[derive(Default)]
struct ClassTally {
    counts: HashMap<MissClass, [usize; 3]>,
}

/// 正解の表記の文字の種類 (表の行の順。上から順に最初に当てはまるもの)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CharClass {
    Katakana,
    Latin,
    Kanji,
    Other,
}

impl CharClass {
    const ALL: [CharClass; 4] = [CharClass::Katakana, CharClass::Latin, CharClass::Kanji, CharClass::Other];

    fn label(self) -> &'static str {
        match self {
            CharClass::Katakana => "カタカナだけ",
            CharClass::Latin => "英字を含む",
            CharClass::Kanji => "漢字を含む",
            CharClass::Other => "その他",
        }
    }
}

fn char_class(surface: &str) -> CharClass {
    if !surface.is_empty() && surface.chars().all(|c| ('\u{30A0}'..='\u{30FF}').contains(&c)) {
        return CharClass::Katakana;
    }
    if surface.chars().any(|c| c.is_ascii_alphabetic() || ('Ａ'..='Ｚ').contains(&c) || ('ａ'..='ｚ').contains(&c)) {
        return CharClass::Latin;
    }
    if surface.chars().any(|c| c == '々' || ('\u{3400}'..='\u{4DBF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c)) {
        return CharClass::Kanji;
    }
    CharClass::Other
}

/// 「分けられない」の行の文字の種類ごとの [行, 3 位以内, 候補にある]
#[derive(Default)]
struct UnsplitTally {
    counts: [[usize; 3]; 4],
    /// カタカナだけで、正解が読み全体のカタカナの行の正解の順位 (1 始まり。候補に無い行は除く)
    whole_ranks: Vec<usize>,
    whole_rows: usize,
}

impl UnsplitTally {
    fn add(&mut self, reading: &str, surface: &str, rank: Option<usize>) {
        let class = char_class(surface);
        let slot = &mut self.counts[CharClass::ALL.iter().position(|&c| c == class).expect("区分")];
        slot[0] += 1;
        slot[1] += usize::from(rank.is_some_and(|r| r < 3));
        slot[2] += usize::from(rank.is_some());
        if class == CharClass::Katakana && convert::to_katakana(reading) == surface {
            self.whole_rows += 1;
            self.whole_ranks.extend(rank.map(|r| r + 1));
        }
    }

    /// 分類の表の後に足す内訳。unsplit は「分けられない」の行の数
    fn render(&mut self, unsplit: usize) -> String {
        let sum = |i: usize| self.counts.iter().map(|c| c[i]).sum::<usize>();
        assert_eq!(sum(0), unsplit, "文字の種類の合計が「分けられない」の行の数と一致しない");
        let mut out = String::from(
            "\n「分けられない」の内訳 (正解の表記の文字の種類)。\n\n| 区分 | 行 | 3 位以内 | 候補にある |\n|---|---|---|---|\n",
        );
        for (class, [n, top3, any]) in CharClass::ALL.iter().zip(self.counts) {
            out.push_str(&format!("| {} | {n} | {top3} | {any} |\n", class.label()));
        }
        out.push_str(&format!("| 合計 | {} | {} | {} |\n", sum(0), sum(1), sum(2)));
        self.whole_ranks.sort_unstable();
        let ranks = &self.whole_ranks;
        let median = match ranks.len() {
            0 => "-".to_string(),
            n if n % 2 == 1 => ranks[n / 2].to_string(),
            n => format!("{}", (ranks[n / 2 - 1] + ranks[n / 2]) as f64 / 2.0),
        };
        out.push_str(&format!(
            "\n- カタカナだけのうち正解が読み全体のカタカナ: {} 行 (正解の順位の中央値 {median}。候補にある {} 行の中央値)",
            self.whole_rows,
            ranks.len(),
        ));
        out
    }
}

/// 「区切り」の行の向き (表の行の順。上から順に最初に当てはまるもの)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BoundaryDir {
    RightEmpty,
    TopEmpty,
    Finer,
    Coarser,
    Shifted,
}

impl BoundaryDir {
    const ALL: [BoundaryDir; 5] = [
        BoundaryDir::RightEmpty,
        BoundaryDir::TopEmpty,
        BoundaryDir::Finer,
        BoundaryDir::Coarser,
        BoundaryDir::Shifted,
    ];

    fn label(self) -> &'static str {
        match self {
            BoundaryDir::RightEmpty => "正解の差が空 (正解では付属語の部分を 1 位が自立語にした)",
            BoundaryDir::TopEmpty => "1 位の差が空",
            BoundaryDir::Finer => "正解の方が語が少ない (1 位が細かく切った)",
            BoundaryDir::Coarser => "正解の方が語が多い (1 位がまとめた)",
            BoundaryDir::Shifted => "同じ数 (境界の位置のずれ)",
        }
    }
}

/// d_right・d_top は word_diff の [正解にだけある語, 1 位にだけある語]
fn boundary_dir(d_right: &[(String, String)], d_top: &[(String, String)]) -> BoundaryDir {
    if d_right.is_empty() {
        BoundaryDir::RightEmpty
    } else if d_top.is_empty() {
        BoundaryDir::TopEmpty
    } else if d_right.len() < d_top.len() {
        BoundaryDir::Finer
    } else if d_right.len() > d_top.len() {
        BoundaryDir::Coarser
    } else {
        BoundaryDir::Shifted
    }
}

/// 「区切り」の行の向き・差の読みの長さ (正解の差と 1 位の差の、読みの文字数の合計の大きい方)・
/// [1 位の差, 正解の差] に Mozc 辞書に無い語があるか (in_mozc が None なら None)。p_right・p_top は path_diff の差
fn boundary_class(
    p_right: &[&SplitWord],
    p_top: &[&SplitWord],
    in_mozc: Option<&dyn Fn(&str, &str) -> bool>,
) -> (BoundaryDir, usize, Option<[bool; 2]>) {
    let pairs = |list: &[&SplitWord]| -> Vec<(String, String)> {
        list.iter().map(|w| (w.reading.clone(), w.surface.clone())).collect()
    };
    let (d_right, d_top) = (pairs(p_right), pairs(p_top));
    let dir = boundary_dir(&d_right, &d_top);
    let chars = |list: &[(String, String)]| list.iter().map(|(r, _)| r.chars().count()).sum::<usize>();
    let len = chars(&d_right).max(chars(&d_top));
    let missing = in_mozc.map(|in_mozc| {
        let missing = |list: &[&SplitWord]| list.iter().any(|w| !in_mozc(&w.reading, &w.surface));
        [missing(p_top), missing(p_right)]
    });
    (dir, len, missing)
}

/// --dump-boundary の 1 行 (改行を含む)。mode はモードの名前、rank は正解の順位 (0 始まり)
fn boundary_dump_line(
    mode: &str,
    right: &[SplitWord],
    top: &[SplitWord],
    rank: Option<usize>,
    in_mozc: Option<&dyn Fn(&str, &str) -> bool>,
) -> String {
    let (p_right, p_top) = path_diff(right, top);
    let (dir, len, missing) = boundary_class(&p_right, &p_top, in_mozc);
    let mozc = |i: usize| missing.map_or("-", |m| if m[i] { "あり" } else { "なし" });
    format!(
        "{mode}\t{}\t{len}\t{}\t{}\t{}\t{}\t{}\n",
        dir.label(),
        rank.map_or("-".to_string(), |r| (r + 1).to_string()),
        mozc(0),
        mozc(1),
        path_text(right, &p_right),
        path_text(top, &p_top),
    )
}

/// 「区切り」の行の向きごとの [行, 長さ 1–2, 3–4, 5 以上, 2 位, 3 位, 4–10 位, 11 位以下, 候補に無い,
/// 1 位の差に Mozc 辞書に無い語, 正解の差に Mozc 辞書に無い語]
#[derive(Default)]
struct BoundaryTally {
    counts: [[usize; 11]; 5],
}

impl BoundaryTally {
    /// right・top は正解と 1 位を split_path で分けた経路。rank は正解の順位 (0 始まり)。in_mozc は
    /// (読み, 表記) が Mozc 辞書にあるか (UT 辞書が無いときは None で、Mozc 辞書に無い語の列は数えない)
    fn add(
        &mut self,
        right: &[SplitWord],
        top: &[SplitWord],
        rank: Option<usize>,
        in_mozc: Option<&dyn Fn(&str, &str) -> bool>,
    ) {
        let (p_right, p_top) = path_diff(right, top);
        let (dir, len, missing) = boundary_class(&p_right, &p_top, in_mozc);
        let slot = &mut self.counts[BoundaryDir::ALL.iter().position(|&d| d == dir).expect("向き")];
        slot[0] += 1;
        slot[match len {
            0..=2 => 1,
            3..=4 => 2,
            _ => 3,
        }] += 1;
        slot[match rank {
            Some(0) => panic!("1 位が正解の行は外れに入らない"),
            Some(1) => 4,
            Some(2) => 5,
            Some(3..=9) => 6,
            Some(_) => 7,
            None => 8,
        }] += 1;
        if let Some([top_missing, right_missing]) = missing {
            slot[9] += usize::from(top_missing);
            slot[10] += usize::from(right_missing);
        }
    }

    /// 分類の表の後に足す内訳。boundary は「区切り」の行の数、mozc は Mozc 辞書に無い語の列を数えたか
    /// (false のときは `-` を出す)
    fn render(&self, boundary: usize, mozc: bool) -> String {
        let sum = |i: usize| self.counts.iter().map(|c| c[i]).sum::<usize>();
        assert_eq!(sum(0), boundary, "向きの合計が「区切り」の行の数と一致しない");
        let cell = |i: usize, n: usize| if i >= 9 && !mozc { "-".to_string() } else { n.to_string() };
        let mut out = String::from(
            "\n\n「区切り」の内訳 (向き・差の読みの長さ・正解の順位)。\n\n\
             | 向き | 行 | 長さ 1–2 | 3–4 | 5 以上 | 2 位 | 3 位 | 4–10 位 | 11 位以下 | 候補に無い | 1 位の差に Mozc 辞書に無い語 | 正解の差に Mozc 辞書に無い語 |\n\
             |---|---|---|---|---|---|---|---|---|---|---|---|\n",
        );
        for (dir, c) in BoundaryDir::ALL.iter().zip(self.counts) {
            assert_eq!(c[1..4].iter().sum::<usize>(), c[0], "長さの区分の合計が向きの行の数と一致しない");
            assert_eq!(c[4..9].iter().sum::<usize>(), c[0], "順位の区分の合計が向きの行の数と一致しない");
            assert!(c[9..11].iter().all(|&n| n <= c[0]), "Mozc 辞書に無い語の行が向きの行の数を超える");
            let cells: Vec<String> = c.iter().enumerate().map(|(i, &n)| cell(i, n)).collect();
            out.push_str(&format!("| {} | {} |\n", dir.label(), cells.join(" | ")));
        }
        let totals: Vec<String> = (0..11).map(|i| cell(i, sum(i))).collect();
        out.push_str(&format!("| 合計 | {} |", totals.join(" | ")));
        out
    }
}

/// 「同じ区間」の行の向き (表の行の順。上から順に最初に当てはまるもの)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SameSpanDir {
    TopEmpty,
    RightEmpty,
    Other,
}

impl SameSpanDir {
    const ALL: [SameSpanDir; 3] = [SameSpanDir::TopEmpty, SameSpanDir::RightEmpty, SameSpanDir::Other];

    fn label(self) -> &'static str {
        match self {
            SameSpanDir::TopEmpty => "1 位の差が空 (正解の側が自立語)",
            SameSpanDir::RightEmpty => "正解の差が空 (1 位の側が自立語)",
            SameSpanDir::Other => "その他",
        }
    }
}

/// 「同じ区間」の行の正解の差の語の学習の区分 (表の列の順)
const SAME_SPAN_LEARNING: [&str; 3] = ["数えない語", "学習あり", "学習なし"];

/// 「同じ区間」の行の向きと、正解の差の語の学習の区分 (SAME_SPAN_LEARNING の位置。正解の差が空なら None)。
/// p_right・p_top は path_diff の差、degree は語の学習の今の度合い (無ければ 0)
fn same_span_class(
    p_right: &[&SplitWord],
    p_top: &[&SplitWord],
    degree: impl Fn(&str, &str) -> f64,
) -> (SameSpanDir, Option<usize>) {
    let dir = if p_top.is_empty() {
        SameSpanDir::TopEmpty
    } else if p_right.is_empty() {
        SameSpanDir::RightEmpty
    } else {
        SameSpanDir::Other
    };
    let learning = (!p_right.is_empty()).then(|| {
        if p_right.iter().any(|w| !learn::counts_as_word(&w.reading, &w.surface)) {
            0
        } else if p_right.iter().any(|w| degree(&w.reading, &w.surface) > 0.0) {
            1
        } else {
            2
        }
    });
    (dir, learning)
}

/// 経路を `表記/読み` の ` | ` 区切りで書く。付属語は `[` `]` で囲み、diff (path の中の語への参照) の語は頭に `*` を付ける。
/// TSV の 1 フィールドにするので、タブと改行は空白にする
fn path_text(path: &[SplitWord], diff: &[&SplitWord]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    path.iter()
        .map(|w| {
            let mark = if diff.iter().any(|d| std::ptr::eq(*d, w)) { "*" } else { "" };
            let word = format!("{}/{}", clean(&w.surface), clean(&w.reading));
            if w.functional { format!("{mark}[{word}]") } else { format!("{mark}{word}") }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// --dump-same-span の 1 行 (改行を含む)。mode はモードの名前、rank は正解の順位 (0 始まり)
fn same_span_dump_line(
    mode: &str,
    right: &[SplitWord],
    top: &[SplitWord],
    rank: Option<usize>,
    degree: impl Fn(&str, &str) -> f64,
) -> String {
    let (p_right, p_top) = path_diff(right, top);
    let (dir, learning) = same_span_class(&p_right, &p_top, degree);
    format!(
        "{mode}\t{}\t{}\t{}\t{}\t{}\n",
        dir.label(),
        learning.map_or("-", |i| SAME_SPAN_LEARNING[i]),
        rank.map_or("-".to_string(), |r| (r + 1).to_string()),
        path_text(right, &p_right),
        path_text(top, &p_top),
    )
}

/// 「同じ区間」の行の向きごとの [行, 数えない語, 学習あり, 学習なし]
#[derive(Default)]
struct SameSpanTally {
    counts: [[usize; 4]; 3],
}

impl SameSpanTally {
    /// right・top は正解と 1 位を split_path で分けた経路。degree は語の学習の今の度合い (無ければ 0)
    fn add(&mut self, right: &[SplitWord], top: &[SplitWord], degree: impl Fn(&str, &str) -> f64) {
        let (p_right, p_top) = path_diff(right, top);
        let (dir, learning) = same_span_class(&p_right, &p_top, degree);
        let slot = &mut self.counts[SameSpanDir::ALL.iter().position(|&d| d == dir).expect("向き")];
        slot[0] += 1;
        if let Some(i) = learning {
            slot[1 + i] += 1;
        }
    }

    /// 分類の表の後に足す内訳。same_span は「同じ区間」の行の数
    fn render(&self, same_span: usize) -> String {
        let sum = |i: usize| self.counts.iter().map(|c| c[i]).sum::<usize>();
        assert_eq!(sum(0), same_span, "向きの合計が「同じ区間」の行の数と一致しない");
        let mut out = String::from(
            "\n\n「同じ区間」の内訳 (向き・正解の差の語の学習)。\n\n\
             | 向き | 行 | 数えない語 | 学習あり | 学習なし |\n\
             |---|---|---|---|---|\n",
        );
        for (dir, c) in SameSpanDir::ALL.iter().zip(self.counts) {
            let learned: usize = c[1..4].iter().sum();
            if *dir == SameSpanDir::RightEmpty {
                assert_eq!(learned, 0, "正解の差が空の行を正解の差の語の学習で数えた");
            } else {
                assert_eq!(learned, c[0], "正解の差の語の学習の区分の合計が向きの行の数と一致しない");
            }
            let cells: Vec<String> = c.iter().map(usize::to_string).collect();
            out.push_str(&format!("| {} | {} |\n", dir.label(), cells.join(" | ")));
        }
        let totals: Vec<String> = (0..4).map(|i| sum(i).to_string()).collect();
        out.push_str(&format!("| 合計 | {} |", totals.join(" | ")));
        out
    }
}

/// 学習が空のモードで測る p の行の位置
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PhrasePos {
    Whole,
    AfterSegmentWithContext,
    AfterSegmentNoReading,
    FirstSegment,
}

impl PhrasePos {
    const ALL: [PhrasePos; 4] = [
        PhrasePos::Whole,
        PhrasePos::AfterSegmentWithContext,
        PhrasePos::AfterSegmentNoReading,
        PhrasePos::FirstSegment,
    ];

    fn label(self) -> &'static str {
        match self {
            PhrasePos::Whole => "読み全体",
            PhrasePos::AfterSegmentWithContext => "文節で、同じ (読み, 表記) の q の件があり、前文脈を付けた",
            PhrasePos::AfterSegmentNoReading => "文節で、同じ (読み, 表記) の q の件があるが、文脈の読みを引けない",
            PhrasePos::FirstSegment => "文節で、q の件が無い",
        }
    }
}

/// q の件は前に文節があった文節にだけできるので、同じ (読み, 表記) の q の件の有無で、前に文節があったかを近似する。
/// contexts は segment_contexts の結果
fn phrase_pos(e: &Entry, contexts: &HashMap<(&str, &str), Option<Context>>) -> PhrasePos {
    if e.whole {
        return PhrasePos::Whole;
    }
    match contexts.get(&(e.reading.as_str(), e.surface.as_str())) {
        Some(Some(_)) => PhrasePos::AfterSegmentWithContext,
        Some(None) => PhrasePos::AfterSegmentNoReading,
        None => PhrasePos::FirstSegment,
    }
}

/// 文節の p の行を変換するときの前文脈。キーはファイル全体の q の件の (読み, 表記) で、文脈の読みを引けなければ None。
/// 実際の入力では前に文節があり、BOS から変換すると入力の先頭にしか起きない外れを数えてしまうため。
/// q の件の文脈は表記だけなので、読みは同じ表記の文節の p の件から引く (resolve_context_id は読みが無いと
/// 前文脈なしになる)。どちらも複数あれば d の大きいもの、並べば t の新しいものを使う
fn segment_contexts(entries: &[Entry]) -> HashMap<(&str, &str), Option<Context>> {
    fn pick<'a, K: std::hash::Hash + Eq>(map: &mut HashMap<K, &'a Entry>, key: K, e: &'a Entry) {
        let slot = map.entry(key).or_insert(e);
        if (e.d, e.t) > (slot.d, slot.t) {
            *slot = e;
        }
    }
    let mut chosen: HashMap<(&str, &str), &Entry> = HashMap::new();
    let mut readings: HashMap<&str, &Entry> = HashMap::new();
    for e in entries {
        match e.kind {
            Kind::CtxPhrase => pick(&mut chosen, (e.reading.as_str(), e.surface.as_str()), e),
            Kind::Phrase if !e.whole => pick(&mut readings, e.surface.as_str(), e),
            _ => {}
        }
    }
    chosen
        .into_iter()
        .map(|(key, q)| {
            let ctx = readings
                .get(q.context.as_str())
                .map(|p| Context { reading: p.reading.clone(), surface: q.context.clone() });
            (key, ctx)
        })
        .collect()
}

/// 文脈の表記 → 読み。q の件の文脈は表記だけなので、segment_contexts と同じく同じ表記の文節の p の件から
/// 引く (複数あれば d の大きいもの、並べば t の新しいもの)
fn context_readings(entries: &[Entry]) -> HashMap<&str, &str> {
    let mut chosen: HashMap<&str, &Entry> = HashMap::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Phrase && !e.whole) {
        let slot = chosen.entry(e.surface.as_str()).or_insert(e);
        if (e.d, e.t) > (slot.d, slot.t) {
            *slot = e;
        }
    }
    chosen.into_iter().map(|(surface, e)| (surface, e.reading.as_str())).collect()
}

/// NEXTWORD の評価の 1 つの組の数
#[derive(Default, Clone, Copy)]
struct NextTally {
    total: usize,
    top1: usize,
    top9: usize,
}

impl NextTally {
    fn add(&mut self, rank: Option<usize>) {
        self.total += 1;
        self.top1 += usize::from(rank == Some(0));
        self.top9 += usize::from(rank.is_some_and(|r| r < 9));
    }
}

/// 文節の p の行を変換するときの前文脈 (segment_contexts で引けたときだけ)
fn segment_ctx<'a>(
    contexts: &'a HashMap<(&'a str, &'a str), Option<Context>>,
    whole: bool,
    reading: &'a str,
    surface: &'a str,
) -> Option<&'a Context> {
    if whole { None } else { contexts.get(&(reading, surface)).and_then(Option::as_ref) }
}

/// 変換の測定から除く読み。モードレス入力の英字区間は (読み = 表記) の句として学習に入るが、実際の入力では
/// 変換せずに続くので、読みに ASCII 英字を含む句が CONVNBEST に渡ることはない
fn latin_reading(reading: &str) -> bool {
    reading.chars().any(|c| c.is_ascii_alphabetic())
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
    let path = |r: &str, s: &str| convert::split_path(r, s, &dictionary, &user, &matrix, &functional);
    // 区切りの差の語が UT 辞書由来かの目安にするので、UT 辞書を読み込んだときだけ別に持つ
    let mozc = if extra.is_empty() { None } else { Some(Dictionary::load(&args.dict, &[])?) };
    let in_mozc = |r: &str, s: &str| mozc.as_ref().is_some_and(|d| d.lookup(r).iter().any(|e| e.surface == s));
    let in_mozc: Option<&dyn Fn(&str, &str) -> bool> = if mozc.is_some() { Some(&in_mozc) } else { None };

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
    let mut empty_classes = ClassTally::default();
    let mut empty_unsplit = UnsplitTally::default();
    let mut empty_boundary = BoundaryTally::default();
    let mut empty_same_span = SameSpanTally::default();
    let dump = args.dump_same_span.is_some();
    let dump_boundary = args.dump_boundary.is_some();
    let dump_pinned = args.dump_pinned.is_some();
    let contexts = segment_contexts(&entries);
    // 位置の区分ごとの [行, 変換 1, 変換 3]
    let mut empty_pos = [[0usize; 3]; PhrasePos::ALL.len()];
    let mut dump_text = String::from("モード	向き	学習の区分	正解の順位	正解の経路	1 位の経路
");
    let mut boundary_text = String::from(
        "モード	向き	差の読みの長さ	正解の順位	1 位の差に Mozc 辞書に無い語	正解の差に Mozc 辞書に無い語	正解の経路	1 位の経路
",
    );
    let mut pinned_text = String::from(
        "モード\t区分\t行の種類\t読み\t正解の表記\t固定の表記\t経路の 1-best の表記\t正解の順位\t\
         固定の表記の句の学習\t正解の表記の句の学習\t読みの句の学習の表記の数\n",
    );
    for e in entries.iter().filter(|e| e.kind == Kind::Phrase && !latin_reading(&e.reading)) {
        let ctx = segment_ctx(&contexts, e.whole, &e.reading, &e.surface);
        let t = Instant::now();
        let nbest = convert::convert_nbest_scored(
            &e.reading, ctx, &dictionary, &user, &matrix, &functional, &empty,
            convert::TypoSource::Off, &typo::TUNING,
        );
        empty_times.push(t.elapsed().as_secs_f64() * 1000.0);
        let rank = nbest.candidates.iter().position(|c| c.surface() == e.surface);
        empty_tally.conv_total += 1;
        empty_tally.conv1 += usize::from(rank == Some(0));
        empty_tally.conv3 += usize::from(rank.is_some_and(|r| r < 3));
        empty_top.insert((&e.reading, &e.surface), rank == Some(0));
        let pos = &mut empty_pos[PhrasePos::ALL.iter().position(|&p| p == phrase_pos(e, &contexts)).expect("区分")];
        pos[0] += 1;
        pos[1] += usize::from(rank == Some(0));
        pos[2] += usize::from(rank.is_some_and(|r| r < 3));
        if rank != Some(0) {
            let (right, top) = if nbest.learned {
                (None, None)
            } else {
                let top = nbest.candidates.first().and_then(|c| path(&e.reading, &c.surface()));
                (path(&e.reading, &e.surface), top)
            };
            let class = classify_miss(nbest.learned, right.as_deref(), top.as_deref(), |r, s| {
                empty.word_degree(r, s).unwrap_or(0.0)
            });
            if class == MissClass::Unsplit {
                empty_unsplit.add(&e.reading, &e.surface, rank);
            }
            if let (MissClass::Boundary, Some(right), Some(top)) = (class, &right, &top) {
                empty_boundary.add(right, top, rank, in_mozc);
                if dump_boundary {
                    boundary_text.push_str(&boundary_dump_line("学習が空", right, top, rank, in_mozc));
                }
            }
            if let (MissClass::SameSpan, Some(right), Some(top)) = (class, &right, &top) {
                empty_same_span.add(right, top, |r, s| empty.word_degree(r, s).unwrap_or(0.0));
                if dump {
                    dump_text.push_str(&same_span_dump_line("学習が空", right, top, rank, |r, s| {
                        empty.word_degree(r, s).unwrap_or(0.0)
                    }));
                }
            }
            let slot = empty_classes.counts.entry(class).or_default();
            slot[0] += 1;
            slot[1] += usize::from(rank.is_some_and(|r| r < 3));
        }
    }

    let rows: Vec<Row> = (front..entries.len())
        .filter(|&i| entries[i].kind != Kind::Word)
        .map(|i| {
            let e = &entries[i];
            Row {
                context: (e.kind == Kind::CtxPhrase).then(|| e.context.clone()),
                reading: e.reading.clone(),
                surface: e.surface.clone(),
                whole: e.whole,
                seen: carried.contains(&i) || known.contains(&(e.reading.as_str(), e.surface.as_str())),
                empty_top: e.kind == Kind::Phrase
                    && !latin_reading(&e.reading)
                    && empty_top[&(e.reading.as_str(), e.surface.as_str())],
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
    let phrases = entries.iter().filter(|e| e.kind == Kind::Phrase);
    let test_phrases = rows.iter().filter(|r| r.context.is_none());
    println!(
        "- 変換の測定から除いた読みに英字を含む p の行: 学習が空 {} 行 (全 p の件 {} 件のうち)・学習あり {} 行 (後ろの p の行 {} 行のうち)",
        phrases.clone().filter(|e| latin_reading(&e.reading)).count(),
        phrases.count(),
        test_phrases.clone().filter(|r| latin_reading(&r.reading)).count(),
        test_phrases.count(),
    );
    // [q の件がある文節の行, そのうち文脈の読みを引けない行]
    let count_ctx = |n: &mut [usize; 2], whole: bool, r: &str, s: &str| {
        if whole || latin_reading(r) {
            return;
        }
        if let Some(ctx) = contexts.get(&(r, s)) {
            n[0] += 1;
            n[1] += usize::from(ctx.is_none());
        }
    };
    let mut empty_ctx = [0usize; 2];
    for e in entries.iter().filter(|e| e.kind == Kind::Phrase) {
        count_ctx(&mut empty_ctx, e.whole, &e.reading, &e.surface);
    }
    let mut test_ctx = [0usize; 2];
    for r in rows.iter().filter(|r| r.context.is_none()) {
        count_ctx(&mut test_ctx, r.whole, &r.reading, &r.surface);
    }
    println!(
        "- 前文脈を付けて変換した文節の p の行: 学習が空 {} 行 (q の件がある文節の行 {} 行のうち、文脈の読みを引けない行 {} 行)・\
         学習あり {} 行 (後ろの p の行のうち、q の件がある文節の行 {} 行のうち、文脈の読みを引けない行 {} 行)",
        empty_ctx[0] - empty_ctx[1],
        empty_ctx[0],
        empty_ctx[1],
        test_ctx[0] - test_ctx[1],
        test_ctx[0],
        test_ctx[1],
    );
    {
        let mut store = LearningStore::in_memory();
        store.load_stats_from(train_text.as_bytes());
        store.refresh_phrases(&split);
        let unsplit = rows.iter().filter(|r| split(&r.reading, &r.surface).is_none()).count();
        println!("- 取り込んだ学習: {} 件 (確定の通し番号 {})", store.entry_count(), store.serial());
        println!("- 測る行のうち辞書の語に分けられない (読み, 表記): {unsplit} 行");
    }

    println!("\n## 学習が空 (読みに英字を含まないすべての p の行)\n");
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
    for i in 0..3 {
        let sum: usize = empty_pos.iter().map(|c| c[i]).sum();
        let all = [empty_tally.conv_total, empty_tally.conv1, empty_tally.conv3][i];
        assert_eq!(sum, all, "位置の区分の合計が学習が空の行の数と一致しない");
    }
    println!(
        "\np の行の位置で分けた変換 1/3 (件数と割合)。q の件は、同じ (読み, 表記) の q の件がファイル全体にあるか。\n"
    );
    println!("| 位置 | 行 | 変換 1 | 変換 3 |");
    println!("|---|---|---|---|");
    for (pos, c) in PhrasePos::ALL.iter().zip(&empty_pos) {
        println!(
            "| {} | {} | {} ({}) | {} ({}) |",
            pos.label(),
            c[0],
            c[1],
            percent(c[1], c[0]),
            c[2],
            percent(c[2], c[0]),
        );
    }
    let empty_class_total: usize = empty_classes.counts.values().map(|c| c[0]).sum();
    assert_eq!(
        empty_class_total,
        empty_tally.conv_total - empty_tally.conv1,
        "分類の合計が外れの数と一致しない"
    );
    println!("\n変換の 1 位を外した行の分類 (学習が空なので、同音の行はすべて「正解に学習なし」に入る)。\n");
    println!("| 分類 | 行 | 3 位以内 |");
    println!("|---|---|---|");
    for class in MissClass::ALL {
        let [n, top3, _] = empty_classes.counts.get(&class).copied().unwrap_or_default();
        println!("| {} | {n} | {top3} |", class.label());
    }
    let empty_sum = |i: usize| empty_classes.counts.values().map(|c| c[i]).sum::<usize>();
    println!("| 合計 | {} | {} |", empty_sum(0), empty_sum(1));
    let empty_unsplit_rows = empty_classes.counts.get(&MissClass::Unsplit).map_or(0, |c| c[0]);
    let empty_boundary_rows = empty_classes.counts.get(&MissClass::Boundary).map_or(0, |c| c[0]);
    let empty_same_span_rows = empty_classes.counts.get(&MissClass::SameSpan).map_or(0, |c| c[0]);
    println!(
        "{}{}{}",
        empty_unsplit.render(empty_unsplit_rows),
        empty_boundary.render(empty_boundary_rows, mozc.is_some()),
        empty_same_span.render(empty_same_span_rows)
    );

    println!("\n## 学習あり (後ろの p と q の行)\n");
    println!("変換 1/3 = p の行 (読みに英字を含む行を除く) の CONVNBEST で 1 位・3 位以内の割合、予測 9 = 打鍵の先頭 50% の SUGGEST で 9 位以内の割合");
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
        let mut pins = PinTally::default();
        let mut unsplit = UnsplitTally::default();
        let mut boundary = BoundaryTally::default();
        let mut same_span = SameSpanTally::default();
        for row in &rows {
            let kind = usize::from(row.context.is_some());
            let ctx = row.context.as_ref().map(|c| Context { reading: String::new(), surface: c.clone() });
            // 文脈の句の学習は変換の経路に効かず、q の行の変換は同じ件の p の行と変わらないので、予測だけを測る
            let conversion = (row.context.is_none() && !latin_reading(&row.reading)).then(|| {
                let conv_ctx = segment_ctx(&contexts, row.whole, &row.reading, &row.surface);
                let t = Instant::now();
                let nbest = convert::convert_nbest_scored(
                    &row.reading, conv_ctx, &dictionary, &user, &matrix, &functional, &store,
                    convert::TypoSource::Off, &typo::TUNING,
                );
                nbest_times.push(t.elapsed().as_secs_f64() * 1000.0);
                let rank = nbest.candidates.iter().position(|c| c.surface() == row.surface);
                if nbest.learned {
                    let pinned = nbest.candidates[0].surface();
                    let path = nbest.best.map(|i| nbest.candidates[i].surface());
                    let class = pin_class(&pinned, path.as_deref(), &row.surface);
                    pins.add(class);
                    if dump_pinned {
                        pinned_text.extend(pin_dump_line(
                            &format!("学習あり ({name})"),
                            class,
                            row.whole,
                            &row.reading,
                            &row.surface,
                            &pinned,
                            path.as_deref(),
                            rank,
                            &store,
                        ));
                    }
                }
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
                        let top = nbest.candidates.first().and_then(|c| path(&row.reading, &c.surface()));
                        (path(&row.reading, &row.surface), top)
                    };
                    let class = classify_miss(nbest.learned, right.as_deref(), top.as_deref(), |r, s| {
                        store.word_degree(r, s).unwrap_or(0.0)
                    });
                    if class == MissClass::Unsplit {
                        unsplit.add(&row.reading, &row.surface, rank);
                    }
                    if let (MissClass::Boundary, Some(right), Some(top)) = (class, &right, &top) {
                        boundary.add(right, top, rank, in_mozc);
                        if dump_boundary {
                            boundary_text.push_str(&boundary_dump_line(&format!("学習あり ({name})"), right, top, rank, in_mozc));
                        }
                    }
                    if let (MissClass::SameSpan, Some(right), Some(top)) = (class, &right, &top) {
                        same_span.add(right, top, |r, s| store.word_degree(r, s).unwrap_or(0.0));
                        if dump {
                            dump_text.push_str(&same_span_dump_line(&format!("学習あり ({name})"), right, top, rank, |r, s| {
                                store.word_degree(r, s).unwrap_or(0.0)
                            }));
                        }
                    }
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
        table.push_str(&format!("| 合計 | {} | {} | {} |\n", sum(0), sum(1), sum(2)));
        let unsplit_rows = classes.counts.get(&MissClass::Unsplit).map_or(0, |c| c[0]);
        table.push_str(&unsplit.render(unsplit_rows));
        let boundary_rows = classes.counts.get(&MissClass::Boundary).map_or(0, |c| c[0]);
        table.push_str(&boundary.render(boundary_rows, mozc.is_some()));
        let same_span_rows = classes.counts.get(&MissClass::SameSpan).map_or(0, |c| c[0]);
        table.push_str(&same_span.render(same_span_rows));
        assert_eq!(
            pins.hit + pins.lost + pins.same[0] + pins.same[1] + pins.both_miss,
            pins.total,
            "固定の得失の合計が固定した行の数と一致しない"
        );
        table.push_str(&format!(
            "\n\n- 固定した行 {} 行: 固定で当てた {}・固定で外した {}・同じ {} (当たり {} / 外れ {})・どちらも外れ {}",
            pins.total,
            pins.hit,
            pins.lost,
            pins.same[0] + pins.same[1],
            pins.same[0],
            pins.same[1],
            pins.both_miss,
        ));
        class_tables.push(table);
    }
    if let Some(file) = &args.next_word {
        let started = Instant::now();
        let table = predict::NextWordTable::load(file)?;
        eprintln!("後続語の統計の読み込み: {} 件 ({:.2} 秒)", table.len(), started.elapsed().as_secs_f64());
        let mut store = LearningStore::in_memory();
        store.load_stats_from(train_text.as_bytes());
        store.refresh_phrases(&split);
        let readings = context_readings(&entries);
        let q_rows: Vec<&Row> = rows.iter().filter(|r| r.context.is_some()).collect();
        // 前文脈の最後の自立語 (バックオフの表を引く文脈)
        let heads: Vec<Option<String>> = q_rows
            .iter()
            .map(|r| {
                let context = r.context.as_deref().expect("q の行");
                let reading = readings.get(context)?;
                split(reading, context)?.pop().map(|(_, surface)| surface)
            })
            .collect();
        println!("\n## 後続語の予測 (NEXTWORD。後ろの q の行)\n");
        println!(
            "q の行の表記が、文脈の表記を前文脈にした NEXTWORD の 9 位以内・1 位に入る割合 (既出・初出は学習ありと同じ分け方)。\n\
             学習ありは既定の定数で前の学習を取り込んだもの。q の行 {} 行のうち、前文脈の最後の自立語を引けた行 {} 行 \
             (引けない行はバックオフの表を引かない)。表は {} 件。\n",
            q_rows.len(),
            heads.iter().filter(|h| h.is_some()).count(),
            table.len(),
        );
        println!("| 学習 | 表 | 9 位以内 | 1 位 | 既出 9 位以内 | 既出 1 位 | 初出 9 位以内 | 初出 1 位 |");
        println!("|---|---|---|---|---|---|---|---|");
        let empty = LearningStore::in_memory();
        for (learning_name, learning) in [("なし", &empty), ("あり", &store)] {
            for (table_name, table) in [("なし", None), ("あり", Some(&table))] {
                let (mut all, mut seen, mut unseen) = (NextTally::default(), NextTally::default(), NextTally::default());
                for (row, head) in q_rows.iter().zip(&heads) {
                    let context = row.context.as_deref().expect("q の行");
                    let list = predict::next_words(context, head.as_deref(), learning, table, &cfg);
                    let rank = list.iter().position(|(_, surface)| *surface == row.surface);
                    all.add(rank);
                    if row.seen { seen.add(rank) } else { unseen.add(rank) }
                }
                println!(
                    "| {learning_name} | {table_name} | {} | {} | {} | {} | {} | {} |",
                    percent(all.top9, all.total),
                    percent(all.top1, all.total),
                    percent(seen.top9, seen.total),
                    percent(seen.top1, seen.total),
                    percent(unseen.top9, unseen.total),
                    percent(unseen.top1, unseen.total),
                );
            }
        }
        println!(
            "\n既出 {} 行・初出 {} 行。",
            q_rows.iter().filter(|r| r.seen).count(),
            q_rows.iter().filter(|r| !r.seen).count()
        );
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
    if let Some(file) = &args.dump_same_span {
        std::fs::write(file, &dump_text)?;
        eprintln!("同じ区間の行を {} 行書き出した: {}", dump_text.lines().count() - 1, file.display());
    }
    if let Some(file) = &args.dump_boundary {
        std::fs::write(file, &boundary_text)?;
        eprintln!("区切りの行を {} 行書き出した: {}", boundary_text.lines().count() - 1, file.display());
    }
    if let Some(file) = &args.dump_pinned {
        std::fs::write(file, &pinned_text)?;
        eprintln!("固定した行を {} 行書き出した: {}", pinned_text.lines().count() - 1, file.display());
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
    fn phrase_pos_by_kind_and_ctx_phrase() {
        let text = "p\ta\tA\t1\t1\twhole\np\tb\tB\t1\t2\tseg\np\tc\tC\t1\t3\tseg\np\td\tD\t1\t4\tseg\n\
                    q\tB\td\tD\t1\t5\nq\tx\tb\tB\t1\t6\nq\tx\tc\tC2\t1\t7\nq\tx\ta\tA\t1\t8\n";
        let (entries, _) = load_entries(text);
        let ctx = segment_contexts(&entries);
        let pos: Vec<PhrasePos> =
            entries.iter().filter(|e| e.kind == Kind::Phrase).map(|e| phrase_pos(e, &ctx)).collect();
        // 読み全体は q の件があっても読み全体。q の件は表記まで同じものだけを数える
        assert_eq!(
            pos,
            [
                PhrasePos::Whole,
                PhrasePos::AfterSegmentNoReading,
                PhrasePos::FirstSegment,
                PhrasePos::AfterSegmentWithContext,
            ]
        );
    }

    #[test]
    fn segment_contexts_picks_max_d_then_newest_t() {
        // b は d の大きい文脈 X、c は d が並ぶので t の新しい文脈 Y。文脈の読みも同じ表記の文節の p の件で
        // d の大きい (並べば t の新しい) もの。読み全体の p の件からは引かない
        let text = "p\tえっくす1\tX\t1\t1\tseg\np\tえっくす2\tX\t3\t2\tseg\np\tわい1\tY\t2\t3\tseg\n\
                    p\tわい2\tY\t2\t4\tseg\np\tわい3\tY\t9\t5\twhole\n\
                    q\tX\tb\tB\t2\t6\nq\tY\tb\tB\t1\t7\nq\tX\tc\tC\t1\t8\nq\tY\tc\tC\t1\t9\n\
                    q\tZ\td\tD\t1\t10\nq\tY\td\tD\t1\t9\n";
        let (entries, _) = load_entries(text);
        let ctx = segment_contexts(&entries);
        let get = |r: &str, s: &str| ctx[&(r, s)].as_ref().map(|c| format!("{}/{}", c.reading, c.surface));
        assert_eq!(get("b", "B").as_deref(), Some("えっくす2/X"));
        assert_eq!(get("c", "C").as_deref(), Some("わい2/Y"));
        // 文脈 Z の読みは引けないので、引ける Y があっても選んだ文脈のまま前文脈なし
        assert_eq!(get("d", "D"), None);
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn context_readings_picks_max_d_then_newest_t_of_segment_phrases() {
        let text = "p\tえっくす1\tX\t1\t1\tseg\np\tえっくす2\tX\t3\t2\tseg\np\tわい1\tY\t2\t3\tseg\n\
                    p\tわい2\tY\t2\t4\tseg\np\tぜっと\tZ\t9\t5\twhole\nq\tX\tb\tB\t2\t6\n";
        let (entries, _) = load_entries(text);
        let readings = context_readings(&entries);
        assert_eq!(readings.get("X"), Some(&"えっくす2"));
        assert_eq!(readings.get("Y"), Some(&"わい2"));
        // 読み全体の p の件と q の件からは引かない
        assert_eq!(readings.get("Z"), None);
        assert_eq!(readings.get("B"), None);
    }

    #[test]
    fn next_tally_counts_top1_and_top9() {
        let mut tally = NextTally::default();
        for rank in [Some(0), Some(8), Some(9), None] {
            tally.add(rank);
        }
        assert_eq!((tally.total, tally.top1, tally.top9), (4, 1, 2));
    }

    #[test]
    fn classify_miss_by_independent_words() {
        // 付属語の無い経路 (区間は読みの文字数を順に足して付ける)
        let words = |list: &[(&str, &str)]| -> Vec<SplitWord> {
            path_of(&list.iter().map(|&(r, s)| (r, s, false)).collect::<Vec<_>>())
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

        // 正解の自立語 I (1–2) を 1 位は付属語にした: 読みの組は違うが、区間はどれももう片方の経路にある
        let right = path_of(&[("あ", "A", false), ("い", "I", false)]);
        let top = path_of(&[("あ", "B", false), ("い", "い", true)]);
        assert_eq!(classify_miss(false, Some(&right), Some(&top), degrees), MissClass::SameSpan);
        assert_eq!(classify_miss(false, Some(&top), Some(&right), degrees), MissClass::SameSpan);
        // 片方向だけ同じ区間: 1 位の差 AI (0–2) は正解の経路に無い
        let coarse = path_of(&[("あい", "AI", false)]);
        let fine = path_of(&[("あ", "A", false), ("い", "い", true)]);
        assert_eq!(classify_miss(false, Some(&fine), Some(&coarse), degrees), MissClass::Boundary);
        assert_eq!(classify_miss(false, Some(&coarse), Some(&fine), degrees), MissClass::Boundary);
    }

    #[test]
    fn boundary_dir_by_first_match() {
        let words = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(r, s)| (r.to_string(), s.to_string())).collect()
        };
        let dir = |right: &[(&str, &str)], top: &[(&str, &str)]| {
            let (d_right, d_top) = word_diff(&words(right), &words(top));
            boundary_dir(&d_right, &d_top)
        };
        assert_eq!(dir(&[("x", "X")], &[("x", "X"), ("y", "Y")]), BoundaryDir::RightEmpty);
        assert_eq!(dir(&[("x", "X"), ("y", "Y")], &[("x", "X")]), BoundaryDir::TopEmpty);
        assert_eq!(dir(&[("xy", "XY")], &[("x", "X"), ("y", "Y")]), BoundaryDir::Finer);
        assert_eq!(dir(&[("x", "X"), ("y", "Y")], &[("xy", "XY")]), BoundaryDir::Coarser);
        assert_eq!(dir(&[("xy", "XY"), ("z", "Z")], &[("x", "X"), ("yz", "YZ")]), BoundaryDir::Shifted);
        // 共通の語は差に入らない
        assert_eq!(dir(&[("a", "A"), ("xy", "XY")], &[("a", "A"), ("x", "X"), ("y", "Y")]), BoundaryDir::Finer);
    }

    /// (読み, 表記, 付属語か) を先頭から並べた経路 (区間は読みの文字数を順に足して付ける)
    fn path_of(list: &[(&str, &str, bool)]) -> Vec<SplitWord> {
        let mut start = 0;
        list.iter()
            .map(|&(r, s, functional)| {
                let end = start + r.chars().count();
                let w = SplitWord { start, end, reading: r.to_string(), surface: s.to_string(), functional };
                start = end;
                w
            })
            .collect()
    }

    #[test]
    fn path_diff_keeps_spans_and_matches_word_diff() {
        let right = path_of(&[("x", "X", false), ("y", "Y", false), ("は", "は", true), ("x", "X", false)]);
        let top = path_of(&[("x", "X", false), ("yは", "YH", false), ("x", "Z", false)]);
        let (d_right, d_top) = path_diff(&right, &top);
        // 先に出た x から相殺するので、正解の差には後ろの x (3–4) が残る
        let spans = |list: &[&SplitWord]| list.iter().map(|w| (w.start, w.end)).collect::<Vec<_>>();
        assert_eq!(spans(&d_right), [(1, 2), (3, 4)]);
        assert_eq!(spans(&d_top), [(1, 3), (3, 4)]);
        let pairs = |list: &[&SplitWord]| {
            list.iter().map(|w| (w.reading.clone(), w.surface.clone())).collect::<Vec<_>>()
        };
        assert_eq!((pairs(&d_right), pairs(&d_top)), word_diff(&independent(&right), &independent(&top)));
    }

    #[test]
    fn same_span_needs_every_diff_word_in_other_path() {
        let right = path_of(&[("あ", "A", false), ("い", "い", true), ("う", "U", false)]);
        let top = path_of(&[("あ", "B", false), ("い", "I", false), ("う", "U", false)]);
        let (d_right, d_top) = path_diff(&right, &top);
        // 正解の差 A (0–1) と 1 位の差 B (0–1)・I (1–2) は、もう片方に同じ区間の語 (付属語を含む) がある
        assert!(same_span(&d_right, &top));
        assert!(same_span(&d_top, &right));
        // 正解の差 A (0–1) と 1 位の差 AI (0–2) は、もう片方の経路に同じ区間の語が無い
        let coarse = path_of(&[("あい", "AI", false), ("う", "U", false)]);
        let (d_right, d_top) = path_diff(&right, &coarse);
        assert!(!same_span(&d_right, &coarse));
        assert!(!same_span(&d_top, &right));
        assert!(same_span(&[], &coarse));
    }

    #[test]
    fn boundary_tally_counts_length_rank_and_mozc() {
        let in_mozc = |_: &str, s: &str| !matches!(s, "AI" | "S" | "MI");
        let mut tally = BoundaryTally::default();
        let mut add = |right: &[(&str, &str, bool)], top: &[(&str, &str, bool)], rank: Option<usize>| {
            tally.add(&path_of(right), &path_of(top), rank, Some(&in_mozc));
        };
        // 差の読みの長さ: 2 (1 + 1 と 2 の大きい方)。1 位の差 AI が Mozc 辞書に無い
        add(&[("あ", "A", false), ("い", "I", false)], &[("あい", "AI", false)], Some(1));
        // 3 (差は 1 位の「かきく」だけ)
        add(&[("う", "U", false), ("かきく", "かきく", true)], &[("う", "U", false), ("かきく", "KKK", false)], Some(2));
        // 5 (「さしす」+「せそ」)。正解の差 S が Mozc 辞書に無い
        add(&[("さしす", "S", false), ("せそ", "T", false)], &[("さしすせそ", "ST", false)], Some(3));
        add(&[("たち", "T", false), ("つ", "U", false)], &[("た", "T", false), ("ちつ", "C", false)], Some(9));
        add(&[("な", "N", false), ("に", "M", false)], &[("なに", "NM", false)], Some(10));
        add(&[("は", "H", false), ("ひ", "I", false)], &[("はひ", "HI", false)], None);
        // 正解の差 MI が Mozc 辞書に無い
        add(&[("ま", "M", false), ("み", "MI", false)], &[("ま", "M2", false), ("み", "み", true)], Some(1));
        assert_eq!(tally.counts[0], [1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0]);
        assert_eq!(tally.counts[1], [0; 11]);
        assert_eq!(tally.counts[3], [5, 4, 0, 1, 2, 0, 1, 1, 1, 1, 2]);
        assert_eq!(tally.counts[4], [1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0]);
        let out = tally.render(7, true);
        assert!(out.contains("| 正解の方が語が多い (1 位がまとめた) | 5 | 4 | 0 | 1 | 2 | 0 | 1 | 1 | 1 | 1 | 2 |"));
        assert!(out.ends_with("| 合計 | 7 | 4 | 2 | 1 | 2 | 1 | 2 | 1 | 1 | 1 | 2 |"));

        // UT 辞書が無いときは Mozc 辞書に無い語の列を数えず `-` を出す
        let mut tally = BoundaryTally::default();
        tally.add(&path_of(&[("あ", "A", false), ("い", "I", false)]), &path_of(&[("あい", "AI", false)]), Some(1), None);
        assert_eq!(tally.counts[3], [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert!(tally.render(1, false).ends_with("| 合計 | 1 | 1 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | - | - |"));
    }

    #[test]
    fn boundary_dump_line_has_dir_length_rank_and_mozc() {
        let in_mozc = |_: &str, s: &str| s != "AI";
        let right = path_of(&[("あ", "A", false), ("い", "I", false), ("は", "は", true)]);
        let top = path_of(&[("あい", "AI", false), ("は", "は", true)]);
        assert_eq!(
            boundary_dump_line("学習が空", &right, &top, Some(1), Some(&in_mozc)),
            "学習が空\t正解の方が語が多い (1 位がまとめた)\t2\t2\tあり\tなし\t*A/あ | *I/い | [は/は]\t*AI/あい | [は/は]\n"
        );
        // UT 辞書が無いときは Mozc 辞書に無い語の列を `-` にし、候補に無い順位も `-` にする
        let line = boundary_dump_line("学習あり", &right, &top, None, None);
        assert_eq!(line.matches('\t').count(), 7);
        assert!(line.starts_with("学習あり\t正解の方が語が多い (1 位がまとめた)\t2\t-\t-\t-\t"));
    }

    #[test]
    fn same_span_tally_counts_dir_and_learning() {
        let degree = |r: &str, s: &str| if (r, s) == ("いう", "IU") { 2.0 } else { 0.0 };
        let mut tally = SameSpanTally::default();
        let mut add = |right: &[(&str, &str, bool)], top: &[(&str, &str, bool)]| {
            tally.add(&path_of(right), &path_of(top), degree);
        };
        // 1 位の差が空。正解の差 い (1 文字) は数えない語
        add(&[("あ", "A", false), ("い", "い", false)], &[("あ", "A", false), ("い", "い", true)]);
        // 1 位の差が空。正解の差 IU に学習あり
        add(&[("あ", "A", false), ("いう", "IU", false)], &[("あ", "A", false), ("いう", "いう", true)]);
        // 正解の差が空。学習の列には数えない
        add(&[("か", "K", false), ("きく", "きく", true)], &[("か", "K", false), ("きく", "KK", false)]);
        // その他: 正解の差 SK・SS は学習なし
        add(
            &[("さく", "SK", false), ("しす", "SS", false), ("せ", "せ", true)],
            &[("さく", "SK2", false), ("しす", "しす", true), ("せ", "せ", true)],
        );
        assert_eq!(tally.counts[0], [2, 1, 1, 0]);
        assert_eq!(tally.counts[1], [1, 0, 0, 0]);
        assert_eq!(tally.counts[2], [1, 0, 0, 1]);
        let out = tally.render(4);
        assert!(out.contains("| 1 位の差が空 (正解の側が自立語) | 2 | 1 | 1 | 0 |"));
        assert!(out.ends_with("| 合計 | 4 | 1 | 1 | 1 |"));
    }

    #[test]
    fn path_text_marks_functional_and_diff_words() {
        let right = path_of(&[("あ", "A", false), ("い", "I", false), ("は", "は", true)]);
        let top = path_of(&[("あ", "A", false), ("い", "い", true), ("は", "は", true)]);
        let (p_right, p_top) = path_diff(&right, &top);
        assert_eq!(path_text(&right, &p_right), "A/あ | *I/い | [は/は]");
        assert_eq!(path_text(&top, &p_top), "A/あ | [い/い] | [は/は]");
        // 付属語が差に入ることは無いが、印は括弧の外に付ける
        let all: Vec<&SplitWord> = top.iter().collect();
        assert_eq!(path_text(&top, &all), "*A/あ | *[い/い] | *[は/は]");
        // 同じ語が 2 つあっても、差に残った方 (後ろ) だけに印を付ける
        let right = path_of(&[("x", "X", false), ("y", "Y", false), ("x", "X", false)]);
        let top = path_of(&[("x", "X", false), ("yx", "YX", false)]);
        let (p_right, _) = path_diff(&right, &top);
        assert_eq!(path_text(&right, &p_right), "X/x | *Y/y | *X/x");
        // タブと改行は空白にする
        let odd = path_of(&[("a\tb", "S\nT", false)]);
        assert_eq!(path_text(&odd, &[]), "S T/a b");

        let line = same_span_dump_line("学習あり", &right, &top, None, |_, _| 0.0);
        assert_eq!(line.matches('\t').count(), 5);
        assert!(line.ends_with('\n'));
        let right = path_of(&[("あ", "A", false), ("いう", "IU", false)]);
        let top = path_of(&[("あ", "A", false), ("いう", "いう", true)]);
        assert_eq!(
            same_span_dump_line("学習が空", &right, &top, Some(1), |_, _| 1.0),
            "学習が空\t1 位の差が空 (正解の側が自立語)\t学習あり\t2\tA/あ | *IU/いう\tA/あ | [いう/いう]\n"
        );
    }

    #[test]
    fn pin_class_matches_pin_tally() {
        assert_eq!(pin_class("A", Some("B"), "A"), PinClass::Hit);
        assert_eq!(pin_class("A", None, "A"), PinClass::Hit);
        assert_eq!(pin_class("A", Some("B"), "B"), PinClass::Lost);
        assert_eq!(pin_class("A", Some("A"), "A"), PinClass::Same(true));
        assert_eq!(pin_class("A", Some("A"), "B"), PinClass::Same(false));
        assert_eq!(pin_class("A", Some("B"), "C"), PinClass::BothMiss);
        assert_eq!(pin_class("A", None, "C"), PinClass::BothMiss);

        let mut pins = PinTally::default();
        for class in [
            PinClass::Hit,
            PinClass::Lost,
            PinClass::Lost,
            PinClass::Same(true),
            PinClass::Same(false),
            PinClass::BothMiss,
        ] {
            pins.add(class);
        }
        assert_eq!((pins.total, pins.hit, pins.lost, pins.same, pins.both_miss), (6, 1, 2, [1, 1], 1));
        assert_eq!(PinClass::Same(true).dump_label(), None);
        assert_eq!(PinClass::Same(false).dump_label(), Some("同じ・外れ"));
    }

    #[test]
    fn pin_dump_line_has_class_surfaces_and_phrase_learning() {
        let mut store = LearningStore::in_memory();
        store.learn_phrase("きょうは", "京は", learn::PhraseKind::Whole, false);
        store.learn_phrase("きょうは", "京は", learn::PhraseKind::Whole, false);
        store.learn_phrase("きょうは", "今日は", learn::PhraseKind::Segment, false);
        let degree = |s: &str| store.phrase_degree("きょうは", s).expect("句の学習");
        assert_eq!(
            pin_dump_line("学習あり (既定)", PinClass::Lost, true, "きょうは", "今日は", "京は", Some("今日は"), Some(1), &store),
            Some(format!(
                "学習あり (既定)\t固定で外した\twhole\tきょうは\t今日は\t京は\t今日は\t2\twhole:{:.2}\tseg:{:.2}\t2\n",
                degree("京は"),
                degree("今日は"),
            ))
        );
        // 正解の表記に句の学習が無く、経路の 1-best も候補の正解も無い行は `-` にする。タブと改行は空白にする
        let line = pin_dump_line("m", PinClass::BothMiss, false, "きょうは", "教\tは", "京は", None, None, &store)
            .expect("書き出す行");
        assert_eq!(line.matches('\t').count(), 10);
        assert!(line.starts_with("m\tどちらも外れ\tseg\tきょうは\t教 は\t京は\t-\t-\twhole:"));
        assert!(line.ends_with("\t-\t2\n"));
        assert_eq!(
            pin_dump_line("m", PinClass::Same(true), false, "きょうは", "京は", "京は", Some("京は"), Some(0), &store),
            None
        );
    }

    #[test]
    fn char_class_by_first_match() {
        assert_eq!(char_class("テスト"), CharClass::Katakana);
        assert_eq!(char_class("コーヒー・ブレイク"), CharClass::Katakana);
        assert_eq!(char_class("テストA"), CharClass::Latin);
        assert_eq!(char_class("ａｂｃ"), CharClass::Latin);
        assert_eq!(char_class("Ｚ形"), CharClass::Latin);
        assert_eq!(char_class("日本"), CharClass::Kanji);
        assert_eq!(char_class("人々"), CharClass::Kanji);
        assert_eq!(char_class("カタカナ漢字"), CharClass::Kanji);
        assert_eq!(char_class("ひらがな"), CharClass::Other);
        assert_eq!(char_class("テストです"), CharClass::Other);
        assert_eq!(char_class("123"), CharClass::Other);
        assert_eq!(char_class("！？"), CharClass::Other);
    }

    #[test]
    fn latin_reading_has_ascii_letter() {
        assert!(latin_reading("abc"));
        assert!(latin_reading("Word"));
        assert!(latin_reading("てすとa"));
        assert!(latin_reading("x1"));
        assert!(!latin_reading("てすと"));
        assert!(!latin_reading("１２３"));
        assert!(!latin_reading("123"));
        assert!(!latin_reading("ａｂｃ"));
        assert!(!latin_reading(""));
    }

    #[test]
    fn unsplit_tally_counts_and_whole_katakana_median() {
        let mut tally = UnsplitTally::default();
        tally.add("てすと", "テスト", Some(4));
        tally.add("こーひー", "コーヒー", Some(1));
        tally.add("あいう", "アイウ", None);
        tally.add("あ", "カ", Some(0));
        tally.add("に", "日", Some(2));
        tally.add("え", "A", None);
        assert_eq!(tally.counts, [[4, 2, 3], [1, 0, 0], [1, 1, 1], [0, 0, 0]]);
        assert_eq!(tally.whole_rows, 3);
        let out = tally.render(6);
        assert!(out.contains("| カタカナだけ | 4 | 2 | 3 |"));
        assert!(out.contains("| 合計 | 6 | 3 | 4 |"));
        // 候補にある 2 行の順位 2 と 5 の中央値
        assert!(out.contains("3 行 (正解の順位の中央値 3.5。候補にある 2 行の中央値)"));
    }
}
