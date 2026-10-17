// 変換へのタイプミス補正の計測 (docs/design/typo-conversion.md の「計測」)
//
// 打ち間違いの無い文 (テストセットの読み) と、その読みのローマ字に 1 打の誤り (余分・抜け・
// 隣のキー・入れ替え) を 1 種類ずつ機械的に入れた読みを、入力全体の候補 (convert_nbest) にかける。
// 1. しきい値の表: 補正の最良経路と通常の最良経路のコストの差 (TYPO_COST を含めない) から、
//    TYPO_COST・TYPO_SHOW・TYPO_TOP の組ごとに、打ち間違いの無い文で補正の候補が出る・先頭になる件数と、
//    打ち間違いのある文で補正の候補が出て正しい文になった割合・先頭になった割合を出す
// 2. 今の定数での convert_nbest の結果 (補正の候補の有無と位置) を同じ数え方で出す
// 3. convert_nbest の応答時間 (中央値・最大) を補正なしと比べる
// 4. 文節 UI の変換 (convert_segments。kinds 付きの CONVSEG) の応答時間を補正なしと比べる
//    (docs/design/typo-segment-ui.md)
// 正しい文は、打ち間違いの無い読みの補正なしの 1-best の表記とする。
// 学習・ユーザ辞書は空 (メモリ上のみ) で、前文脈は使わない。エンジンは lib を持たないため、
// 変換に使うモジュールをソースごと取り込む (ut_diff と同じ方式)。結果は Markdown で標準出力へ書く。
//
// 使い方:
//   cargo run --release --example typo_eval -- --cases examples/data/llm_cases.tsv \
//       --cases examples/data/typo_cases.tsv
//
// オプション:
//   --cases <tsv>  テストセット。複数指定できる。4 列 (llm_cases.tsv の形式) なら 2 列目が読み・4 列目が
//                  正解、2 列なら 1 列目が読み・2 列目が正解。# で始まる行と空行は読み飛ばす
//   --dict <dir>   Mozc 辞書のディレクトリ (既定は references/mozc/src/data/dictionary_oss)
//   --ut <file>    UT 辞書 (既定は references/mozcdic-ut/dictionary_ut.txt。無ければ使わない)
//   --iters N      応答時間の 1 つの読みあたりの計測回数 (既定 5)

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
#[path = "../src/typo.rs"]
mod typo;
#[allow(dead_code)]
#[path = "../src/userdict.rs"]
mod userdict;

use std::path::PathBuf;
use std::time::Instant;

use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Args {
    cases: Vec<PathBuf>,
    dict: PathBuf,
    ut: PathBuf,
    iters: usize,
}

fn parse_args() -> BoxResult<Args> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut args = Args {
        cases: Vec::new(),
        dict: root.join("references/mozc/src/data/dictionary_oss"),
        ut: root.join("references/mozcdic-ut/dictionary_ut.txt"),
        iters: 5,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--cases" => args.cases.push(PathBuf::from(value()?)),
            "--dict" => args.dict = PathBuf::from(value()?),
            "--ut" => args.ut = PathBuf::from(value()?),
            "--iters" => args.iters = value()?.parse::<usize>()?.max(1),
            other => return Err(format!("不明な引数: {other}").into()),
        }
    }
    if args.cases.is_empty() {
        return Err("--cases が必要です".into());
    }
    Ok(args)
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

const ERROR_KINDS: [&str; 4] = ["余分", "抜け", "隣のキー", "入れ替え"];

/// romaji の head 打目より後ろに、kind の誤りを 1 打入れる
fn inject(romaji: &[u8], head: usize, kind: usize, rng: &mut Rng) -> Option<Vec<u8>> {
    let n = romaji.len();
    let mut v = romaji.to_vec();
    match kind {
        // 余分: 隣のキーか同じキーを 1 打多く打つ
        0 => {
            let i = head + rng.below(n - head + 1);
            let base = romaji[i.min(n - 1)];
            let mut keys = typo::neighbor_keys(base);
            keys.push(base);
            v.insert(i, keys[rng.below(keys.len())]);
        }
        1 => {
            if n <= head {
                return None;
            }
            v.remove(head + rng.below(n - head));
        }
        2 => {
            if n <= head {
                return None;
            }
            let i = head + rng.below(n - head);
            let keys = typo::neighbor_keys(romaji[i]);
            if keys.is_empty() {
                return None;
            }
            v[i] = keys[rng.below(keys.len())];
        }
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

/// 読みの補正の結果 (補正の最良経路の差、文字数、補正の表記)
struct Sample {
    gain: Option<(i64, String)>,
    chars: usize,
}

/// 打ち間違いのある読み
struct TypoCase {
    kind: usize,
    reading: String,
    /// 正しい文 (元の読みの補正なしの 1-best)
    target: String,
    sample: Sample,
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
    eprintln!("辞書の読み込み: {:.1} 秒", started.elapsed().as_secs_f64());

    let nbest = |kana: &str, typo: bool| {
        convert::convert_nbest(kana, None, &dictionary, &user, &matrix, &functional, &learning, typo)
    };
    let sample = |kana: &str| Sample {
        gain: convert::typo_gain(kana, &dictionary, &user, &matrix, &functional, &learning),
        chars: kana.chars().count(),
    };

    let short = readings.iter().filter(|r| r.chars().count() < typo::MIN_TYPO_CHARS).count();
    let clean: Vec<(String, String, Sample)> = readings
        .iter()
        .filter(|r| r.chars().count() >= typo::MIN_TYPO_CHARS)
        .map(|r| (r.clone(), nbest(r, false)[0].surface(), sample(r)))
        .collect();

    // 打ち間違いのある読み: 元の読みごとに 4 種類の誤りを 1 つずつ。ローマ字は訓令式とヘボン式を
    // 交互に使い、先頭のかなの打鍵は変えない。かなにならない・元と同じになる誤りは位置を変えて
    // 試し直し、20 回で作れなければ数えない
    let mut typos: Vec<TypoCase> = Vec::new();
    let mut skipped = 0;
    for (index, (reading, target, _)) in clean.iter().enumerate() {
        let Some(romaji) = typo::to_romaji(reading, index % 2 == 1) else {
            skipped += ERROR_KINDS.len();
            continue;
        };
        let first: String = reading.chars().take(1).collect();
        let head = typo::to_romaji(&first, index % 2 == 1).map_or(1, |r| r.len());
        for kind in 0..ERROR_KINDS.len() {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((index as u64) << 8 | kind as u64));
            let made = (0..20).find_map(|_| {
                let typed = inject(romaji.as_bytes(), head, kind, &mut rng)?;
                let typed = typo::romaji_to_kana(std::str::from_utf8(&typed).ok()?)?;
                (typed != *reading && typed.chars().next() == reading.chars().next()).then_some(typed)
            });
            match made {
                Some(typed) => {
                    let sample = sample(&typed);
                    typos.push(TypoCase { kind, reading: typed, target: target.clone(), sample });
                }
                None => skipped += 1,
            }
        }
    }

    println!("## テストセット\n");
    println!("- 打ち間違いの無い文: {} 件 (3 文字未満の読み {short} 件は除いた)", clean.len());
    println!("- 打ち間違いのある文: {} 件 (作れなかった {skipped} 件は除いた)", typos.len());
    for (kind, name) in ERROR_KINDS.iter().enumerate() {
        println!("  - {name}: {} 件", typos.iter().filter(|t| t.kind == kind).count());
    }

    // 1 文字あたりの差の分布 (TYPO_COST を含めない)
    let per_char = |s: &Sample, cost: i64| s.gain.as_ref().map(|(g, _)| (g - cost) as f64 / s.chars as f64);
    println!("\n## 打ち間違いの無い文の Δ (TYPO_COST = 0) の上位 10 件\n");
    println!("| Δ | 読み | 補正の表記 |");
    println!("|---|---|---|");
    let mut ranked: Vec<&(String, String, Sample)> = clean.iter().filter(|(_, _, s)| s.gain.is_some()).collect();
    ranked.sort_by(|a, b| per_char(&b.2, 0).unwrap().total_cmp(&per_char(&a.2, 0).unwrap()));
    for (reading, _, s) in ranked.iter().take(10) {
        println!("| {:.0} | {} | {} |", per_char(s, 0).unwrap(), reading, s.gain.as_ref().unwrap().1);
    }
    let correct: Vec<f64> = typos
        .iter()
        .filter(|t| t.sample.gain.as_ref().is_some_and(|(_, s)| *s == t.target))
        .filter_map(|t| per_char(&t.sample, 0))
        .collect();
    println!(
        "\n- 打ち間違いのある文で補正の最良経路が正しい文になる件数: {}/{} (Δ の条件なし)",
        correct.len(),
        typos.len()
    );
    println!("\n## 補正の最良経路が正しい文にならなかった例 (種類ごとに先頭の 8 件)\n");
    println!("| 誤りの種類 | 打ったかな | 正しい文 | 補正の最良経路 |");
    println!("|---|---|---|---|");
    for kind in 0..ERROR_KINDS.len() {
        let wrong = typos
            .iter()
            .filter(|t| t.kind == kind && t.sample.gain.as_ref().is_none_or(|(_, s)| *s != t.target));
        for t in wrong.take(8) {
            let surface = t.sample.gain.as_ref().map_or("-", |(_, s)| s.as_str());
            println!("| {} | {} | {} | {surface} |", ERROR_KINDS[kind], t.reading, t.target);
        }
    }

    // TYPO_COST ごとの、打ち間違いの無い文の Δ の最大と、それを超える Δ で正しい文になる割合
    println!("\n## TYPO_COST ごとの打ち間違いの無い文の Δ の最大\n");
    println!("| TYPO_COST | Δ の最大 | 2 番目 | 3 番目 | 最大を超える Δ で正しい文になる割合 |");
    println!("|---|---|---|---|---|");
    for cost in [0i64, 1000, 2000, 3000, 3500, 4000, 4500, 5000] {
        let mut deltas: Vec<f64> = clean.iter().filter_map(|(_, _, s)| per_char(s, cost)).collect();
        deltas.sort_by(|a, b| b.total_cmp(a));
        let max = deltas.first().copied().unwrap_or(f64::MIN);
        let above = typos
            .iter()
            .filter(|t| t.sample.gain.as_ref().is_some_and(|(_, s)| *s == t.target))
            .filter(|t| per_char(&t.sample, cost).is_some_and(|d| d > max))
            .count();
        let at = |i: usize| deltas.get(i).map_or("-".to_string(), |d| format!("{d:.0}"));
        println!(
            "| {cost} | {} | {} | {} | {:.0}% |",
            at(0),
            at(1),
            at(2),
            100.0 * above as f64 / typos.len().max(1) as f64
        );
    }

    // しきい値の表。Δ = (差 − TYPO_COST) / 文字数
    let costs = [2000i64, 3000, 3500, 4000, 4500, 5000];
    let shows = [-150.0f64, -100.0, -50.0, 0.0, 50.0, 100.0];
    let tops = [-50.0f64, 0.0, 50.0, 100.0, 200.0, 400.0];
    println!("\n## しきい値ごとの件数\n");
    println!("無し = 打ち間違いの無い文、有り = 打ち間違いのある文。");
    println!("出る = 補正の候補が出た件数、先頭 = 補正の候補が先頭になった件数、");
    println!("正 = 補正の候補が正しい文で出た割合、正先頭 = 正しい文で先頭になった割合、誤先頭 = 正しくない文で先頭になった件数\n");
    println!("| TYPO_COST | TYPO_SHOW | TYPO_TOP | 無し: 出る | 無し: 先頭 | 有り: 正 | 有り: 正先頭 | 有り: 誤先頭 |");
    println!("|---|---|---|---|---|---|---|---|");
    let total = typos.len().max(1) as f64;
    for &cost in &costs {
        for &show in &shows {
            for &top in &tops {
                if top < show {
                    continue;
                }
                let delta_of = |s: &Sample| per_char(s, cost);
                let clean_shown = clean.iter().filter(|(_, _, s)| delta_of(s).is_some_and(|d| d >= show)).count();
                let clean_top = clean.iter().filter(|(_, _, s)| delta_of(s).is_some_and(|d| d >= show && d >= top)).count();
                let mut ok = 0;
                let mut ok_top = 0;
                let mut bad_top = 0;
                for t in &typos {
                    let Some(d) = delta_of(&t.sample) else { continue };
                    if d < show {
                        continue;
                    }
                    let right = t.sample.gain.as_ref().is_some_and(|(_, s)| *s == t.target);
                    ok += usize::from(right);
                    if d >= top {
                        ok_top += usize::from(right);
                        bad_top += usize::from(!right);
                    }
                }
                println!(
                    "| {cost} | {show} | {top} | {clean_shown} | {clean_top} | {:.0}% | {:.0}% | {bad_top} |",
                    100.0 * ok as f64 / total,
                    100.0 * ok_top as f64 / total
                );
            }
        }
    }

    // 今の定数での convert_nbest の結果
    let position = |kana: &str| nbest(kana, true).iter().position(|c| c.typo).map(|p| (p, nbest(kana, true)[p].surface()));
    let mut clean_counts = [0usize; 2];
    let mut clean_listed: Vec<(usize, String, String)> = Vec::new();
    for (reading, _, _) in &clean {
        if let Some((p, surface)) = position(reading) {
            clean_counts[usize::from(p != 0)] += 1;
            clean_listed.push((p, reading.clone(), surface));
        }
    }
    println!("\n## 今の定数での convert_nbest\n");
    println!(
        "- 打ち間違いの無い文: 補正の候補が先頭 {} 件、2 番目以降 {} 件 (全 {} 件)",
        clean_counts[0],
        clean_counts[1],
        clean.len()
    );
    for (p, reading, surface) in &clean_listed {
        println!("  - {} 番目: {reading} → {surface}", p + 1);
    }
    println!("\n| 誤りの種類 | 件数 | 正しい文で出た | 正しい文で先頭 | 正しくない文で先頭 |");
    println!("|---|---|---|---|---|");
    for kind in 0..=ERROR_KINDS.len() {
        let cases: Vec<&TypoCase> = typos.iter().filter(|t| kind == ERROR_KINDS.len() || t.kind == kind).collect();
        let mut counts = [0usize; 3];
        for t in &cases {
            if let Some((p, surface)) = position(&t.reading) {
                let right = surface == t.target;
                counts[0] += usize::from(right);
                counts[1] += usize::from(right && p == 0);
                counts[2] += usize::from(!right && p == 0);
            }
        }
        let n = cases.len().max(1) as f64;
        let name = ERROR_KINDS.get(kind).copied().unwrap_or("計");
        println!(
            "| {name} | {} | {} ({:.0}%) | {} ({:.0}%) | {} |",
            cases.len(),
            counts[0],
            100.0 * counts[0] as f64 / n,
            counts[1],
            100.0 * counts[1] as f64 / n,
            counts[2]
        );
    }

    // 応答時間
    let all: Vec<&str> = clean.iter().map(|(r, _, _)| r.as_str()).chain(typos.iter().map(|t| t.reading.as_str())).collect();
    let segments = |kana: &str, typo: bool| {
        convert::convert_segments(kana, None, &dictionary, &user, &matrix, &functional, &learning, typo)
    };
    let timings: [(&str, &dyn Fn(&str, bool)); 2] = [
        ("convert_nbest", &|kana, typo| drop(nbest(kana, typo))),
        ("convert_segments", &|kana, typo| drop(segments(kana, typo))),
    ];
    for (name, run) in timings {
        println!("\n## 応答時間 ({name} の 1 回、{} 回/読み、{} 読み)\n", args.iters, all.len());
        println!("| 補正 | 中央値 (ms) | 最大 (ms) | 最大の読み |");
        println!("|---|---|---|---|");
        for typo in [false, true] {
            let mut times: Vec<f64> = Vec::new();
            let mut slowest = (0.0f64, "");
            for kana in &all {
                run(kana, typo);
                for _ in 0..args.iters {
                    let started = Instant::now();
                    run(kana, typo);
                    let ms = started.elapsed().as_secs_f64() * 1000.0;
                    if ms > slowest.0 {
                        slowest = (ms, kana);
                    }
                    times.push(ms);
                }
            }
            times.sort_by(|a, b| a.total_cmp(b));
            let median = times[times.len() / 2];
            println!("| {} | {median:.2} | {:.2} | {} |", if typo { "あり" } else { "なし" }, slowest.0, slowest.1);
        }
    }
    Ok(())
}
