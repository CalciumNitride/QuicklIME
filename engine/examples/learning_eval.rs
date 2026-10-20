// 学習の点数付けの評価 (docs/design/learning-score.md の G)
//
// 実ユーザの learning.tsv (読み\t表記) と learning_context.tsv (文脈\t読み\t表記) を、ファイルごとに記録順の
// 前の 8 割と後ろの 2 割に分け、前の 8 割を取り込んだ学習で後ろの 2 割の各行を測る。
// - 変換: 入力全体の候補 (CONVNBEST。補正なし) で行の表記が 1 位・3 位以内に入る割合。
//   learning_context.tsv の行は文脈表記を前文脈にする (文脈の読みはファイルに無いので空。文頭の文脈 ID は使わない)
// - 予測: 読みを標準のローマ字にした打鍵の先頭 50% (切り上げ) で SUGGEST を引き、行の表記が 9 位以内に入る割合
// - 「既出」は、(読み, 表記) が前の 8 割のどちらかのファイルにもある行だけの割合
// 取り込みはエンジンの learning_stats.tsv が無いときの取り込みと同じ (learning.tsv の行を文節の確定として数え、
// learning_context.tsv の行は文脈の句の学習だけを数える)。ファイルは読むだけで、学習はメモリ上にだけ作る。
// 1 位を外した行は、学習が空のときの 1 位と比べて内訳を出す (件数だけ)。
// 結果は件数と割合だけを Markdown で標準出力へ書く (読み・表記は書かない)。
//
// 使い方:
//   cargo run --release --example learning_eval -- --learn <learning.tsv> --context <learning_context.tsv>
//       [--weights "half=5000,ww=1500"]... [--empty]
//
// オプション:
//   --learn <tsv>      learning.tsv
//   --context <tsv>    learning_context.tsv
//   --weights <設定>   定数の組 (カンマ区切りの key=value)。複数指定でき、指定ごとに 1 行の結果を出す。
//                      無ければ既定値 (learn::WEIGHTS) だけ。key は half (HALF_LIFE)・ww・wm (WORD_WEIGHT・
//                      WORD_MAX)・cw・cm (CONTEXT_*)・pw・pm・pb (PHRASE_WEIGHT・PHRASE_MAX・PHRASE_BASE)
//   --empty            学習が空のときの行も出す
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

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use config::Config;
use convert::Context;
use dict::Dictionary;
use learn::{LearningStore, Weights};
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use typo::Keys;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    learn: PathBuf,
    context: PathBuf,
    weights: Vec<String>,
    empty: bool,
    dict: PathBuf,
    ut: PathBuf,
}

fn parse_args() -> BoxResult<Args> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut learn = None;
    let mut context = None;
    let mut args = Args {
        learn: PathBuf::new(),
        context: PathBuf::new(),
        weights: Vec::new(),
        empty: false,
        dict: root.join("references/mozc/src/data/dictionary_oss"),
        ut: root.join("references/mozcdic-ut/dictionary_ut.txt"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--learn" => learn = Some(PathBuf::from(value()?)),
            "--context" => context = Some(PathBuf::from(value()?)),
            "--weights" => args.weights.push(value()?),
            "--empty" => args.empty = true,
            "--dict" => args.dict = PathBuf::from(value()?),
            "--ut" => args.ut = PathBuf::from(value()?),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    args.learn = learn.ok_or("--learn が必要です")?;
    args.context = context.ok_or("--context が必要です")?;
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

/// 測る 1 行
struct Row {
    /// 直前の文節の表記 (learning_context.tsv の行)
    context: Option<String>,
    reading: String,
    surface: String,
    /// (読み, 表記) が前の 8 割にもある
    seen: bool,
}

/// 割合を数える (行の種類 [文脈なし, 文脈付き] ごと)
#[derive(Default, Clone, Copy)]
struct Tally {
    total: [usize; 2],
    conv1: [usize; 2],
    conv3: [usize; 2],
    /// 予測を測れた行 (読みを打鍵にできた行)
    pred_total: [usize; 2],
    pred9: [usize; 2],
    /// 変換で 1 位に入らなかった行のうち、1 位が読み全体の学習表記だった行
    miss_pinned: [usize; 2],
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
    /// そのうち固定した表記が、前の 8 割の learning.tsv でその読みの最後の表記と同じ (今の方式でも同じ表記を固定する)
    pinned_last: usize,
    /// 経路で外れた行 (学習が空なら 1 位) [文脈なし, 文脈付き]
    lost_path_kind: [usize; 2],
}

/// (読み, 表記) を学習で覚えているか
fn knows(store: &LearningStore, reading: &str, surface: &str) -> bool {
    store.has_phrase(reading, surface)
}

/// 前の 8 割 (記録順)
fn head_count(n: usize) -> usize {
    n * 8 / 10
}

/// 学習を取り込む (エンジンの learning_stats.tsv が無いときの取り込みと同じ)
fn train(
    store: &mut LearningStore,
    plain: &[(String, String)],
    context: &[(String, String, String)],
    split: &learn::Splitter,
) {
    let text: String = plain.iter().map(|(r, s)| format!("{r}\t{s}\n")).collect();
    store.import_plain(text.as_bytes(), split);
    let text: String = context.iter().map(|(c, r, s)| format!("{c}\t{r}\t{s}\n")).collect();
    store.import_context(text.as_bytes());
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let plain: Vec<(String, String)> = std::fs::read_to_string(&args.learn)?
        .lines()
        .filter_map(|line| {
            let (r, s) = line.split_once('\t')?;
            (!r.is_empty() && !s.is_empty() && !s.contains('\t')).then(|| (r.to_string(), s.to_string()))
        })
        .collect();
    let context: Vec<(String, String, String)> = std::fs::read_to_string(&args.context)?
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\t');
            let (c, r, s) = (f.next()?, f.next()?, f.next()?);
            (!c.is_empty() && !r.is_empty() && !s.is_empty()).then(|| (c.to_string(), r.to_string(), s.to_string()))
        })
        .collect();

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

    let (plain_train, plain_test) = plain.split_at(head_count(plain.len()));
    let (ctx_train, ctx_test) = context.split_at(head_count(context.len()));
    let known: HashSet<(&str, &str)> = plain_train
        .iter()
        .map(|(r, s)| (r.as_str(), s.as_str()))
        .chain(ctx_train.iter().map(|(_, r, s)| (r.as_str(), s.as_str())))
        .collect();
    let mut rows: Vec<Row> = Vec::new();
    for (r, s) in plain_test {
        let seen = known.contains(&(r.as_str(), s.as_str()));
        rows.push(Row { context: None, reading: r.clone(), surface: s.clone(), seen });
    }
    for (c, r, s) in ctx_test {
        let seen = known.contains(&(r.as_str(), s.as_str()));
        rows.push(Row { context: Some(c.clone()), reading: r.clone(), surface: s.clone(), seen });
    }

    println!("## データ\n");
    println!("- learning.tsv: {} 行 (取り込み {} 行、測る {} 行)", plain.len(), plain_train.len(), plain_test.len());
    println!(
        "- learning_context.tsv: {} 行 (取り込み {} 行、測る {} 行)",
        context.len(),
        ctx_train.len(),
        ctx_test.len()
    );
    println!("- 測る行のうち既出: {} 行", rows.iter().filter(|r| r.seen).count());
    {
        let mut store = LearningStore::in_memory();
        train(&mut store, plain_train, ctx_train, &split);
        let unsplit = rows.iter().filter(|r| split(&r.reading, &r.surface).is_none()).count();
        println!("- 取り込んだ学習: {} 件 (確定の通し番号 {})", store.entry_count(), store.serial());
        println!("- 測る行のうち辞書の語に分けられない (読み, 表記): {unsplit} 行");
    }

    println!("\n## 結果\n");
    println!("変換 1/3 = CONVNBEST で 1 位・3 位以内の割合、予測 9 = 打鍵の先頭 50% の SUGGEST で 9 位以内の割合。");
    println!("全体 (文脈なし / 文脈付き) の順。既出は (読み, 表記) が前の 8 割にもある行だけ。");
    println!("変換 1 位外 (固定) = 変換で 1 位に入らなかった行の数 (うち 1 位が読み全体の学習表記の行の数)。時間は CONVNBEST・SUGGEST 1 回の中央値 / p95 (ms)。\n");
    println!("| 方式 | 変換 1 | 変換 3 | 予測 9 | 既出 変換 1 | 既出 変換 3 | 既出 予測 9 | 変換 1 位外 (固定) | CONVNBEST | SUGGEST |");
    println!("|---|---|---|---|---|---|---|---|---|---|");

    // 学習が空のときの変換の 1 位
    let empty = LearningStore::in_memory();
    let empty_top: Vec<bool> = rows
        .iter()
        .map(|row| {
            let ctx = row.context.as_ref().map(|c| Context { reading: String::new(), surface: c.clone() });
            let nbest = convert::convert_nbest_scored(
                &row.reading, ctx.as_ref(), &dictionary, &user, &matrix, &functional, &empty,
                convert::TypoSource::Off, &typo::TUNING,
            );
            nbest.candidates.first().is_some_and(|c| c.surface() == row.surface)
        })
        .collect();
    let mut last_plain: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for (r, s) in plain_train {
        last_plain.insert(r, s);
    }
    let mut details: Vec<String> = Vec::new();

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
            train(&mut store, plain_train, ctx_train, &split);
        }
        let mut all = Tally::default();
        let mut seen = Tally::default();
        let mut nbest_times: Vec<f64> = Vec::new();
        let mut suggest_times: Vec<f64> = Vec::new();
        let mut misses = Misses::default();
        for (index, row) in rows.iter().enumerate() {
            let kind = usize::from(row.context.is_some());
            let ctx = row.context.as_ref().map(|c| Context { reading: String::new(), surface: c.clone() });
            let t = Instant::now();
            let nbest = convert::convert_nbest_scored(
                &row.reading, ctx.as_ref(), &dictionary, &user, &matrix, &functional, &store,
                convert::TypoSource::Off, &typo::TUNING,
            );
            nbest_times.push(t.elapsed().as_secs_f64() * 1000.0);
            let rank = nbest.candidates.iter().position(|c| c.surface() == row.surface);
            if rank == Some(0) {
                misses.gained += usize::from(!empty_top[index]);
            } else {
                if empty_top[index] {
                    misses.lost[usize::from(!nbest.learned)] += 1;
                    if !nbest.learned {
                        misses.lost_path_kind[kind] += 1;
                    }
                }
                if nbest.learned {
                    misses.pinned += 1;
                    misses.pinned_known += usize::from(knows(&store, &row.reading, &row.surface));
                    let pinned = nbest.candidates[0].surface();
                    misses.pinned_last += usize::from(last_plain.get(row.reading.as_str()) == Some(&pinned.as_str()));
                }
            }
            let mut tallies = vec![&mut all];
            if row.seen {
                tallies.push(&mut seen);
            }
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
            for tally in tallies {
                tally.total[kind] += 1;
                tally.conv1[kind] += usize::from(rank == Some(0));
                tally.conv3[kind] += usize::from(rank.is_some_and(|r| r < 3));
                tally.miss_pinned[kind] += usize::from(rank != Some(0) && nbest.learned);
                if let Some(hit) = predicted {
                    tally.pred_total[kind] += 1;
                    tally.pred9[kind] += usize::from(hit);
                }
            }
        }
        let pct = |n: [usize; 2], total: [usize; 2]| {
            let p = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
            format!(
                "{:.1}% ({:.1} / {:.1})",
                p(n[0] + n[1], total[0] + total[1]),
                p(n[0], total[0]),
                p(n[1], total[1])
            )
        };
        let stat = |times: &mut Vec<f64>| -> String {
            times.sort_by(|a, b| a.total_cmp(b));
            let at = |q: f64| times[((times.len() as f64 - 1.0) * q).round() as usize];
            format!("{:.2} / {:.2}", at(0.5), at(0.95))
        };
        println!(
            "| {name} | {} | {} | {} | {} | {} | {} | {} ({}) | {} | {} |",
            pct(all.conv1, all.total),
            pct(all.conv3, all.total),
            pct(all.pred9, all.pred_total),
            pct(seen.conv1, seen.total),
            pct(seen.conv3, seen.total),
            pct(seen.pred9, seen.pred_total),
            all.total[0] + all.total[1] - all.conv1[0] - all.conv1[1],
            all.miss_pinned[0] + all.miss_pinned[1],
            stat(&mut nbest_times),
            stat(&mut suggest_times),
        );
        details.push(format!(
            "- {name}: 学習で外れた {} 行 (1 位が読み全体の学習表記 {}・経路 {} [文脈なし {} / 文脈付き {}])、学習で当たった {} 行。\
             1 位が読み全体の学習表記で外れた {} 行のうち、行の表記を学習で覚えている {} 行、固定した表記が learning.tsv の\
             その読みの最後の表記 {} 行",
            misses.lost[0] + misses.lost[1],
            misses.lost[0],
            misses.lost[1],
            misses.lost_path_kind[0],
            misses.lost_path_kind[1],
            misses.gained,
            misses.pinned,
            misses.pinned_known,
            misses.pinned_last,
        ));
    }
    println!("\n## 1 位を外した行の内訳\n");
    for line in details {
        println!("{line}");
    }
    Ok(())
}
