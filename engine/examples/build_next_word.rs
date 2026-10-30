// 後続語の予測の統計 (next_word.tsv) の生成 (docs/spec/suggest.md「文節への分け方」「表」「生成」)
//
// 日本語版ウィキペディアの CirrusSearch のダンプ (jawiki_content-*.json.bz2) の本文を、逆変換の対象の文字の
// 連続ごとに区切って逆変換 (src/reverse.rs) し、変換の group_path と同じ規則で文節にまとめ、隣り合う文節の
// 組と、文節の末尾の「名詞系の語 + 助詞・助動詞の語の連続」を数える。学習とユーザ辞書は空にし、利用者の
// データに依らない表にする。取得と実行は scripts\fetch-wiki-ngram.ps1 が行う。
//
// 出力は 1 行 1 件の「表の区別\t文脈\t読み\t表記\t回数」(UTF-8、BOM なし)。表の区別は c (文節の表。
// 文脈は前の文節の表記の末尾 16 文字)、b (バックオフの表。文脈は前の文節の最後の自立語の表記)、
// s (付属語の表。文脈は名詞系の語の表記、読みと表記はその後ろの助詞・助動詞の語の連続)。
// 文脈ごとにまとめ、回数の多い順に並べる。
//
// 使い方:
//   cargo run --release --example build_next_word -- --out <next_word.tsv> <jawiki_content-*.json.bz2>...
//
// オプション:
//   --out <file>       出力先
//   --dict <dir>       Mozc 辞書のディレクトリ (既定は references/mozc/src/data/dictionary_oss)
//   --ut <file>        UT 辞書 (既定は references/mozcdic-ut/dictionary_ut.txt。無ければ使わない)
//   --min-count N      回数が N 未満の組を捨てる (既定 15)

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
#[path = "../src/reverse.rs"]
mod reverse;
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

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use dict::{Dictionary, SurfaceIndex};
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use reverse::ReversedWord;
use userdict::UserDict;

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

/// 文脈ごとに残す件数 (max_predictions の上限)
const MAX_PER_CONTEXT: usize = 36;

/// 逆変換のスレッドに 1 度に渡す行の数
const BATCH_LINES: usize = 64;

struct Args {
    inputs: Vec<PathBuf>,
    out: PathBuf,
    dict: PathBuf,
    ut: PathBuf,
    min_count: u32,
}

fn parse_args() -> BoxResult<Args> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = None;
    let mut args = Args {
        inputs: Vec::new(),
        out: PathBuf::new(),
        dict: root.join("references/mozc/src/data/dictionary_oss"),
        ut: root.join("references/mozcdic-ut/dictionary_ut.txt"),
        min_count: 15,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} に値がありません"));
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(value()?)),
            "--dict" => args.dict = PathBuf::from(value()?),
            "--ut" => args.ut = PathBuf::from(value()?),
            "--min-count" => args.min_count = value()?.parse()?,
            other if other.starts_with("--") => return Err(format!("不明な引数: {other}").into()),
            other => args.inputs.push(PathBuf::from(other)),
        }
    }
    let usage = "使い方: build_next_word --out <file> [--dict <dir>] [--ut <file>] [--min-count N] <.json.bz2>...";
    args.out = out.ok_or(usage)?;
    if args.inputs.is_empty() {
        return Err(usage.into());
    }
    Ok(args)
}

/// 逆変換した語をまとめた文節
#[derive(Debug, PartialEq)]
struct Bunsetsu {
    reading: String,
    surface: String,
    /// 最後の自立語の表記。付属語だけの文節 (連続の先頭の付属語) は None
    head: Option<String>,
    /// 未知語を含むか
    unknown: bool,
    /// 文節の末尾が「名詞系の語 + 助詞・助動詞の語の連続」の形なら、(名詞系の語の表記, 連続の読み, 連続の表記)
    suffix: Option<(String, String, String)>,
}

/// 語の列を文節にまとめる。変換の group_path と同じく付属語を前の文節にまとめ、先頭の付属語は単独の文節にする
fn group(words: Vec<ReversedWord>, functional: &FunctionalIds) -> Vec<Bunsetsu> {
    let mut groups: Vec<Bunsetsu> = Vec::new();
    // 作っている文節の、末尾の助詞・助動詞の連続の前にある名詞系の語の表記と、連続の (読み, 表記)
    let mut noun: Option<String> = None;
    let mut tail = (String::new(), String::new());
    let finish = |last: Option<&mut Bunsetsu>, noun: &mut Option<String>, tail: &mut (String, String)| {
        if let Some(last) = last {
            let tail = std::mem::take(tail);
            last.suffix = noun.take().filter(|_| !tail.0.is_empty()).map(|n| (n, tail.0, tail.1));
        }
        *noun = None;
    };
    for word in words {
        match groups.last_mut() {
            Some(last) if word.functional => {
                last.reading.push_str(&word.reading);
                last.surface.push_str(&word.surface);
                last.unknown |= word.unknown;
            }
            last => {
                finish(last, &mut noun, &mut tail);
                let head = (!word.functional).then(|| word.surface.clone());
                groups.push(Bunsetsu {
                    reading: word.reading.clone(),
                    surface: word.surface.clone(),
                    head,
                    unknown: word.unknown,
                    suffix: None,
                });
            }
        }
        if functional.is_particle(word.left_id) {
            if noun.is_some() {
                tail.0.push_str(&word.reading);
                tail.1.push_str(&word.surface);
            }
        } else {
            noun = functional.is_noun(word.left_id).then_some(word.surface);
            tail = (String::new(), String::new());
        }
    }
    finish(groups.last_mut(), &mut noun, &mut tail);
    groups
}

/// 本文を逆変換の対象の文字の連続に区切る
fn split_runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !reverse::is_allowed(c)).filter(|run| !run.is_empty())
}

fn has_digit(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

fn is_katakana(c: char) -> bool {
    matches!(c, 'ァ'..='ヶ' | 'ー')
}

/// 逆変換した語の列 words のうち、カタカナ (ー を含む) の 2 文字以上の連続に文字が重なる語に 1 文字の語が
/// あれば、その連続に重なるすべての語を未知語にする。辞書に無いカタカナ語 (ベイカー) は辞書にある 1 文字の語
/// (ー・カタカナ 1 文字) を使って「ベイカ|ー」のように割られ、逆変換の未知語の印が付かないため。
/// 表記が「ー」で始まる語も未知語にする。ひらがなの後の長音 (すごーく → ー|く) はカタカナの連続にならないため
fn mark_fragments(words: &mut [ReversedWord]) {
    for word in words.iter_mut().filter(|w| w.surface.starts_with('ー')) {
        word.unknown = true;
    }
    // 語ごとの表記の文字の範囲
    let mut spans = Vec::with_capacity(words.len());
    let mut chars: Vec<char> = Vec::new();
    for word in words.iter() {
        let start = chars.len();
        chars.extend(word.surface.chars());
        spans.push(start..chars.len());
    }
    let mut start = 0;
    while start < chars.len() {
        if !is_katakana(chars[start]) {
            start += 1;
            continue;
        }
        let end = start + chars[start..].iter().take_while(|&&c| is_katakana(c)).count();
        if end - start >= 2 {
            let overlapping: Vec<usize> =
                (0..words.len()).filter(|&i| spans[i].start < end && spans[i].end > start).collect();
            if overlapping.iter().any(|&i| spans[i].len() == 1) {
                for i in overlapping {
                    words[i].unknown = true;
                }
            }
        }
        start = end;
    }
}

/// 逆変換のスレッドが 1 度に返す結果
#[derive(Default)]
struct Batch {
    docs: u64,
    runs: u64,
    failed: u64,
    /// 連続ごとの文節の列。1 文節の連続も付属語の表に数えるので入れる
    bunsetsu: Vec<Vec<Bunsetsu>>,
}

/// 逆変換に使う辞書一式。学習とユーザ辞書は空
struct Reverser<'a> {
    dict: &'a Dictionary,
    index: &'a SurfaceIndex,
    matrix: &'a ConnectionMatrix,
    functional: &'a FunctionalIds,
    user: UserDict,
    learning: LearningStore,
}

impl<'a> Reverser<'a> {
    fn new(
        dict: &'a Dictionary,
        index: &'a SurfaceIndex,
        matrix: &'a ConnectionMatrix,
        functional: &'a FunctionalIds,
    ) -> Self {
        Reverser { dict, index, matrix, functional, user: UserDict::empty(), learning: LearningStore::in_memory() }
    }

    /// 1 文書の本文を連続に区切って逆変換し、文節にまとめて batch に足す
    fn add_text(&self, text: &str, batch: &mut Batch) {
        batch.docs += 1;
        for run in split_runs(text) {
            batch.runs += 1;
            let words = reverse::reverse_words(
                run,
                self.dict,
                self.index,
                &self.user,
                self.matrix,
                self.functional,
                &self.learning,
            );
            match words {
                Some(mut words) => {
                    mark_fragments(&mut words);
                    let bunsetsu = group(words, self.functional);
                    if !bunsetsu.is_empty() {
                        batch.bunsetsu.push(bunsetsu);
                    }
                }
                None => batch.failed += 1,
            }
        }
    }

    /// ダンプの 1 行を読む。text を持たない行 (index の行) は読み飛ばす
    fn add_line(&self, line: &str, batch: &mut Batch) -> Result<(), String> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("JSON として読めない行があります: {e}"))?;
        if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
            self.add_text(text, batch);
        }
        Ok(())
    }
}

/// 文字列に番号を振る。数が多いので、表と番号から引く列で同じ文字列を共有する
#[derive(Default)]
struct Interner {
    ids: HashMap<Rc<str>, u32>,
    strings: Vec<Rc<str>>,
}

impl Interner {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.strings.len() as u32;
        let rc: Rc<str> = Rc::from(s);
        self.strings.push(Rc::clone(&rc));
        self.ids.insert(rc, id);
        id
    }

    fn get(&self, id: u32) -> &str {
        &self.strings[id as usize]
    }
}

/// 残した 1 つの文脈と、その次の文節 (読み, 表記, 回数) の列
type ContextRows<'a> = (&'a str, Vec<(&'a str, &'a str, u32)>);

/// 文節の組の回数
#[derive(Default)]
struct Counter {
    /// 文脈 (文節の表の文脈とバックオフの表の文脈)
    contexts: Interner,
    /// 次の文節の「読み\t表記」
    nexts: Interner,
    /// (文脈, 次の文節) → 回数
    bunsetsu_table: HashMap<(u32, u32), u32>,
    backoff_table: HashMap<(u32, u32), u32>,
    /// (名詞系の語の表記, 助詞・助動詞の語の連続) → 回数
    suffix_table: HashMap<(u32, u32), u32>,
    pairs: u64,
    /// 付属語の表に数えた文節の数
    suffixes: u64,
}

impl Counter {
    /// 1 つの連続の隣り合う文節の組を数える。前か次が付属語だけの文節の組は数えない
    /// (読点の直後の「と」「は」などは予測の文脈としての価値が低い)。未知語を含む文節の組も数えない
    /// (辞書に無いカタカナ語が「ー」「ちろう」のような 1 文字ずつの断片に分かれている)
    fn add_run(&mut self, run: &[Bunsetsu]) {
        // 付属語の表は文節ごとに数える。除く文節は組と同じ (付属語だけ・数字を含む・未知語を含む)
        for b in run {
            let Some((noun, reading, surface)) = &b.suffix else {
                continue;
            };
            if b.head.is_none() || b.unknown || has_digit(&b.surface) {
                continue;
            }
            self.suffixes += 1;
            let next_id = self.nexts.id(&format!("{reading}\t{surface}"));
            let context = self.contexts.id(noun);
            *self.suffix_table.entry((context, next_id)).or_default() += 1;
        }
        for pair in run.windows(2) {
            let (prev, next) = (&pair[0], &pair[1]);
            let Some(head) = &prev.head else {
                continue;
            };
            if has_digit(&prev.surface)
                || has_digit(&next.surface)
                || next.head.is_none()
                || prev.unknown
                || next.unknown
            {
                continue;
            }
            self.pairs += 1;
            let next_id = self.nexts.id(&format!("{}\t{}", next.reading, next.surface));
            let context = self.contexts.id(&learn::context_key(&prev.surface));
            *self.bunsetsu_table.entry((context, next_id)).or_default() += 1;
            let context = self.contexts.id(head);
            *self.backoff_table.entry((context, next_id)).or_default() += 1;
        }
    }

    /// 回数が min_count 以上の組を、文脈ごとに回数の多い順 (同じ回数は表記・読みの順) に上位
    /// MAX_PER_CONTEXT 件まで残す。文脈は文字列の順に並べる
    fn select(&self, table: &HashMap<(u32, u32), u32>, min_count: u32) -> Vec<ContextRows<'_>> {
        let mut by_context: HashMap<u32, Vec<(&str, &str, u32)>> = HashMap::new();
        for (&(context, next), &count) in table {
            if count < min_count {
                continue;
            }
            let (reading, surface) = self.nexts.get(next).split_once('\t').expect("読み\\t表記");
            by_context.entry(context).or_default().push((reading, surface, count));
        }
        let mut list: Vec<ContextRows<'_>> = by_context
            .into_iter()
            .map(|(context, mut rows)| {
                rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| (a.1, a.0).cmp(&(b.1, b.0))));
                rows.truncate(MAX_PER_CONTEXT);
                (self.contexts.get(context), rows)
            })
            .collect();
        list.sort_by(|a, b| a.0.cmp(b.0));
        list
    }
}

/// 表を 1 行 1 件で書く
fn write_table(out: &mut impl Write, tag: &str, list: &[ContextRows<'_>]) -> std::io::Result<()> {
    for (context, rows) in list {
        for (reading, surface, count) in rows {
            writeln!(out, "{tag}\t{context}\t{reading}\t{surface}\t{count}")?;
        }
    }
    Ok(())
}

/// ダンプを順に展開し、BATCH_LINES 行ずつ逆変換のスレッドへ渡す
fn read_dumps(inputs: &[PathBuf], tx: SyncSender<Vec<String>>) -> Result<(), String> {
    for path in inputs {
        eprintln!("読み込み中: {}", path.display());
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let reader = BufReader::with_capacity(1 << 20, bzip2::read::MultiBzDecoder::new(file));
        let mut batch = Vec::with_capacity(BATCH_LINES);
        for line in reader.lines() {
            batch.push(line.map_err(|e| format!("{}: {e}", path.display()))?);
            if batch.len() == BATCH_LINES {
                if tx.send(std::mem::replace(&mut batch, Vec::with_capacity(BATCH_LINES))).is_err() {
                    return Ok(());
                }
            }
        }
        if !batch.is_empty() && tx.send(batch).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

/// 行の束を受け取って逆変換し、結果を返す。読めない行があっても止めずに続ける
/// (止めると読み込みのスレッドが送り先を失って待ち続けるため)
fn reverse_worker(reverser: &Reverser, rx: &Mutex<Receiver<Vec<String>>>, tx: SyncSender<Result<Batch, String>>) {
    loop {
        let lines = match rx.lock().expect("rx lock").recv() {
            Ok(lines) => lines,
            Err(_) => return,
        };
        let mut batch = Batch::default();
        let mut result = Ok(());
        for line in &lines {
            if let Err(e) = reverser.add_line(line, &mut batch) {
                result = Err(e);
            }
        }
        if tx.send(result.map(|()| batch)).is_err() {
            return;
        }
    }
}

fn main() -> BoxResult<()> {
    let args = parse_args()?;
    let started = Instant::now();

    let extra: Vec<PathBuf> = args.ut.is_file().then(|| args.ut.clone()).into_iter().collect();
    let mut dictionary = Dictionary::load(&args.dict, &extra)?;
    if let Some(parent) = args.dict.parent() {
        dictionary.load_single_kanji(&parent.join("single_kanji/single_kanji.tsv"))?;
    }
    let matrix = ConnectionMatrix::load(&args.dict.join("connection_single_column.txt"))?;
    let functional = FunctionalIds::load(&args.dict.join("id.def"))?;
    let index = SurfaceIndex::build(&dictionary);
    eprintln!(
        "辞書の読み込み: {:.1} 秒 [{}]{}",
        started.elapsed().as_secs_f64(),
        args.dict.display(),
        extra.iter().map(|p| format!(" + [{}]", p.display())).collect::<String>()
    );

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let (line_tx, line_rx) = sync_channel::<Vec<String>>(threads * 2);
    let line_rx = Mutex::new(line_rx);
    let (batch_tx, batch_rx) = sync_channel::<Result<Batch, String>>(threads * 2);
    let mut counter = Counter::default();
    let (mut docs, mut runs, mut failed) = (0u64, 0u64, 0u64);
    let mut error: Option<String> = None;
    let read_result = std::thread::scope(|scope| {
        let inputs = &args.inputs;
        let reader = scope.spawn(move || read_dumps(inputs, line_tx));
        for _ in 0..threads {
            let batch_tx = batch_tx.clone();
            let (line_rx, dictionary, index, matrix, functional) = (&line_rx, &dictionary, &index, &matrix, &functional);
            scope.spawn(move || {
                let reverser = Reverser::new(dictionary, index, matrix, functional);
                reverse_worker(&reverser, line_rx, batch_tx);
            });
        }
        drop(batch_tx);
        let mut reported = 0;
        for batch in batch_rx {
            match batch {
                Ok(batch) => {
                    docs += batch.docs;
                    runs += batch.runs;
                    failed += batch.failed;
                    for run in &batch.bunsetsu {
                        counter.add_run(run);
                    }
                }
                Err(e) => {
                    error.get_or_insert(e);
                }
            }
            if docs >= reported + 50_000 {
                reported = docs;
                eprintln!("  文書 {docs} ({:.0} 秒)", started.elapsed().as_secs_f64());
            }
        }
        reader.join().expect("読み込みのスレッド")
    });
    read_result?;
    if let Some(e) = error {
        return Err(e.into());
    }

    let bunsetsu_list = counter.select(&counter.bunsetsu_table, args.min_count);
    let backoff_list = counter.select(&counter.backoff_table, args.min_count);
    let suffix_list = counter.select(&counter.suffix_table, args.min_count);
    let mut out = BufWriter::new(File::create(&args.out)?);
    write_table(&mut out, "c", &bunsetsu_list)?;
    write_table(&mut out, "b", &backoff_list)?;
    write_table(&mut out, "s", &suffix_list)?;
    out.flush()?;

    let rows = |list: &[ContextRows<'_>]| list.iter().map(|(_, rows)| rows.len()).sum::<usize>();
    println!("入力: {}", args.inputs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    println!("文書数: {docs}");
    println!("連続の数: {runs}");
    println!("逆変換に失敗した連続の数: {failed}");
    println!("数えた組の数: {}", counter.pairs);
    println!("最小回数: {}", args.min_count);
    println!(
        "文節の表 (c): 組の種類 {}・残した文脈 {}・件数 {}",
        counter.bunsetsu_table.len(),
        bunsetsu_list.len(),
        rows(&bunsetsu_list)
    );
    println!(
        "バックオフの表 (b): 組の種類 {}・残した文脈 {}・件数 {}",
        counter.backoff_table.len(),
        backoff_list.len(),
        rows(&backoff_list)
    );
    println!(
        "付属語の表 (s): 数えた文節の数 {}・組の種類 {}・残した文脈 {}・件数 {}",
        counter.suffixes,
        counter.suffix_table.len(),
        suffix_list.len(),
        rows(&suffix_list)
    );
    println!("所要時間: {:.1} 秒 [{}]", started.elapsed().as_secs_f64(), args.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 は名詞、2 は助詞 (付属語。テストでは 月 にも使う)
    fn functional() -> FunctionalIds {
        FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n".as_bytes()).unwrap()
    }

    fn sample() -> (Dictionary, SurfaceIndex) {
        let mut dict = Dictionary::empty();
        let data = "きょう\t1\t1\t3000\t今日\n\
                    は\t2\t2\t500\tは\n\
                    も\t2\t2\t500\tも\n\
                    よい\t1\t1\t3000\t良い\n\
                    てんき\t1\t1\t3000\t天気\n\
                    がつ\t2\t2\t2000\t月\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        let index = SurfaceIndex::build(&dict);
        (dict, index)
    }

    /// 本文を数え、(表の区別, 文脈, 読み, 表記, 回数) の行にする
    fn count_texts(texts: &[&str], min_count: u32) -> (Vec<(String, String, String, String, u32)>, Batch) {
        let (dict, index) = sample();
        let (matrix, functional) = (ConnectionMatrix::empty(), functional());
        let reverser = Reverser::new(&dict, &index, &matrix, &functional);
        let mut batch = Batch::default();
        for text in texts {
            reverser.add_text(text, &mut batch);
        }
        let mut counter = Counter::default();
        for run in &batch.bunsetsu {
            counter.add_run(run);
        }
        (table_rows(&counter, min_count), batch)
    }

    fn table_rows(counter: &Counter, min_count: u32) -> Vec<(String, String, String, String, u32)> {
        let mut rows = Vec::new();
        let tables = [("c", &counter.bunsetsu_table), ("b", &counter.backoff_table), ("s", &counter.suffix_table)];
        for (tag, table) in tables {
            for (context, list) in counter.select(table, min_count) {
                for (reading, surface, count) in list {
                    rows.push((tag.to_string(), context.to_string(), reading.to_string(), surface.to_string(), count));
                }
            }
        }
        rows
    }

    fn row(tag: &str, context: &str, reading: &str, surface: &str, count: u32) -> (String, String, String, String, u32) {
        (tag.to_string(), context.to_string(), reading.to_string(), surface.to_string(), count)
    }

    fn bunsetsu(reading: &str, surface: &str, head: Option<&str>) -> Bunsetsu {
        Bunsetsu {
            reading: reading.to_string(),
            surface: surface.to_string(),
            head: head.map(str::to_string),
            unknown: false,
            suffix: None,
        }
    }

    /// 逆変換した語。付属語は id 2 (助詞)、それ以外は id 1 (名詞)
    fn word(reading: &str, surface: &str, functional: bool, unknown: bool) -> ReversedWord {
        ReversedWord {
            reading: reading.to_string(),
            surface: surface.to_string(),
            functional,
            unknown,
            left_id: if functional { 2 } else { 1 },
        }
    }

    #[test]
    fn 付属語を前の語にまとめ先頭の付属語は単独の文節にする() {
        let mut tail = bunsetsu("きょうはも", "今日はも", Some("今日"));
        tail.suffix = Some(("今日".to_string(), "はも".to_string(), "はも".to_string()));
        assert_eq!(
            group(
                vec![
                    word("は", "は", true, false),
                    word("きょう", "今日", false, false),
                    word("は", "は", true, false),
                    word("も", "も", true, false),
                ],
                &functional()
            ),
            [bunsetsu("は", "は", None), tail]
        );
        // 未知語は付属語としてまとめた語にあっても文節の印になる
        let got = group(
            vec![word("ぬ", "ヌ", false, true), word("きょう", "今日", false, false), word("ぬ", "ヌ", true, true)],
            &functional(),
        );
        assert_eq!(got.iter().map(|b| b.unknown).collect::<Vec<_>>(), [true, true]);
    }

    #[test]
    fn 文節の末尾の名詞系の語と助詞_助動詞の連続を付属語にする() {
        // 1 名詞・2 助詞・3 助動詞・4 名詞,接尾・5 動詞
        let functional =
            FunctionalIds::load_from("1 名詞,一般\n2 助詞,格助詞\n3 助動詞\n4 名詞,接尾,人名\n5 動詞,自立\n".as_bytes())
                .unwrap();
        let w = |reading: &str, surface: &str, left_id: u16| ReversedWord {
            reading: reading.to_string(),
            surface: surface.to_string(),
            functional: matches!(left_id, 2..=4),
            unknown: false,
            left_id,
        };
        let suffixes = |words: Vec<ReversedWord>| -> Vec<Option<(String, String, String)>> {
            group(words, &functional).into_iter().map(|b| b.suffix).collect()
        };
        let some = |n: &str, r: &str, s: &str| Some((n.to_string(), r.to_string(), s.to_string()));
        // 田中さんでした|雨: さん (名詞,接尾) をキーに、でした (助詞・助動詞の連続) を数える
        assert_eq!(
            suffixes(vec![w("たなか", "田中", 1), w("さん", "さん", 4), w("で", "で", 2), w("した", "した", 3), w("あめ", "雨", 1)]),
            [some("さん", "でした", "でした"), None]
        );
        // 名詞の後に付属語が無い・動詞の後の助動詞は数えない
        assert_eq!(suffixes(vec![w("たべ", "食べ", 5), w("た", "た", 3)]), [None]);
        assert_eq!(suffixes(vec![w("あめ", "雨", 1)]), [None]);
    }

    #[test]
    fn 付属語の表に名詞系の語の後ろの付属語の連続を数える() {
        let (rows, _) = count_texts(&["今日は", "今日は良い天気", "今日も"], 1);
        let suffix: Vec<_> = rows.into_iter().filter(|r| r.0 == "s").collect();
        assert_eq!(suffix, [row("s", "今日", "は", "は", 2), row("s", "今日", "も", "も", 1)]);
        // 最小回数で捨てる
        let (rows, _) = count_texts(&["今日は", "今日は良い天気", "今日も"], 2);
        let suffix: Vec<_> = rows.into_iter().filter(|r| r.0 == "s").collect();
        assert_eq!(suffix, [row("s", "今日", "は", "は", 2)]);
    }

    /// 表記の列から語の列を作り、カタカナの断片の印を付けて未知語かどうかを返す
    fn fragments(surfaces: &[&str]) -> Vec<bool> {
        let mut words: Vec<ReversedWord> = surfaces
            .iter()
            .map(|s| word(s, s, false, false))
            .collect();
        mark_fragments(&mut words);
        words.iter().map(|w| w.unknown).collect()
    }

    #[test]
    fn カタカナの連続を1文字の語を含む語に割ったときはその連続の語を未知語にする() {
        assert_eq!(fragments(&["今日", "ベイカ", "ー", "は"]), [false, true, true, false]);
        // 1 語で引けた語と、2 文字以上の語だけに割れた連続は残す
        assert_eq!(fragments(&["タワー", "は"]), [false, false]);
        assert_eq!(fragments(&["コンピュータ", "ウイルス"]), [false, false]);
        // カタカナ 1 文字だけの連続は対象外
        assert_eq!(fragments(&["今日", "ド", "天気"]), [false, false, false]);
        // 連続の一部とかな・漢字にまたがる語も、連続に文字が重なれば対象
        assert_eq!(fragments(&["ア", "イス屋", "の"]), [true, true, false]);
        // 連続ごとに判定する
        assert_eq!(fragments(&["タワー", "の", "ベイカ", "ー"]), [false, false, true, true]);
    }

    #[test]
    fn 表記がーで始まる語を未知語にする() {
        // ひらがなの後の長音はカタカナの連続にならない
        assert_eq!(fragments(&["すご", "ー", "く"]), [false, true, false]);
        assert_eq!(fragments(&["が", "ーる"]), [false, true]);
        // ー を途中・末尾に含む語は対象外
        assert_eq!(fragments(&["タワー", "は"]), [false, false]);
    }

    #[test]
    fn カタカナの断片を含む前と次の文節の組を数えない() {
        // ベイカー は辞書の 1 文字の語 ー を使って ベイカ|ー に割られる (ベイカ は辞書の語)
        let mut dict = Dictionary::empty();
        let data = "きょう\t1\t1\t3000\t今日\n\
                    よい\t1\t1\t3000\t良い\n\
                    てんき\t1\t1\t3000\t天気\n\
                    べいか\t1\t1\t3000\tベイカ\n\
                    ー\t1\t1\t3000\tー\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        let index = SurfaceIndex::build(&dict);
        let (matrix, functional) = (ConnectionMatrix::empty(), functional());
        let reverser = Reverser::new(&dict, &index, &matrix, &functional);
        let mut batch = Batch::default();
        reverser.add_text("今日ベイカー天気良い", &mut batch);
        let run = &batch.bunsetsu[0];
        assert_eq!(run.iter().map(|b| b.surface.as_str()).collect::<Vec<_>>(), ["今日", "ベイカ", "ー", "天気", "良い"]);
        assert_eq!(run.iter().map(|b| b.unknown).collect::<Vec<_>>(), [false, true, true, false, false]);
        let mut counter = Counter::default();
        counter.add_run(run);
        assert_eq!(table_rows(&counter, 1), [row("c", "天気", "よい", "良い", 1), row("b", "天気", "よい", "良い", 1)]);
    }

    #[test]
    fn 未知語を含む前と次の文節の組を数えない() {
        // ヌ は辞書に無いかなの 1 文字 (未知語)。今日|ヌ と ヌ|天気 は数えず、天気|良い だけを数える
        let (rows, batch) = count_texts(&["今日ヌ天気良い"], 1);
        assert_eq!(batch.bunsetsu[0].iter().map(|b| b.surface.as_str()).collect::<Vec<_>>(), ["今日", "ヌ", "天気", "良い"]);
        assert_eq!(rows, [row("c", "天気", "よい", "良い", 1), row("b", "天気", "よい", "良い", 1)]);
    }

    #[test]
    fn 記号と英字と空白で連続を区切る() {
        assert_eq!(
            split_runs("今日は、良い天気ABC今日 は。。3月").collect::<Vec<_>>(),
            ["今日は", "良い天気", "今日", "は", "3月"]
        );
    }

    #[test]
    fn 文節の表とバックオフの表に隣り合う文節を数える() {
        let (rows, batch) = count_texts(&["今日は良い天気", "今日は良い天気"], 1);
        assert_eq!(batch.docs, 2);
        assert_eq!(batch.runs, 2);
        assert_eq!(batch.failed, 0);
        assert_eq!(
            rows,
            [
                row("c", "今日は", "よい", "良い", 2),
                row("c", "良い", "てんき", "天気", 2),
                row("b", "今日", "よい", "良い", 2),
                row("b", "良い", "てんき", "天気", 2),
                row("s", "今日", "は", "は", 2),
            ]
        );
    }

    #[test]
    fn 区切りをまたいだ組を数えない() {
        // 今日は|良い と 今日|天気 は区切りをまたぐ
        let (rows, batch) = count_texts(&["今日は、良い。今日 天気良い"], 1);
        assert_eq!(batch.runs, 4);
        // 付属語の表は 1 文節の連続 (今日は) も数える
        assert_eq!(
            rows,
            [row("c", "天気", "よい", "良い", 1), row("b", "天気", "よい", "良い", 1), row("s", "今日", "は", "は", 1)]
        );
    }

    #[test]
    fn 付属語だけの前と次の文節の組を数えない() {
        // 区切りの直後の は は付属語だけの文節で、は|天気 は数えない
        let (rows, _) = count_texts(&["今日、は天気"], 1);
        assert!(rows.is_empty());
        // 付属語だけの文節は、連続の途中に置いても前・次のどちらとしても数えない
        let mut counter = Counter::default();
        counter.add_run(&[
            bunsetsu("きょう", "今日", Some("今日")),
            bunsetsu("は", "は", None),
            bunsetsu("てんき", "天気", Some("天気")),
        ]);
        assert_eq!(counter.pairs, 0);
        assert!(counter.bunsetsu_table.is_empty());
        assert!(counter.backoff_table.is_empty());
    }

    #[test]
    fn 数字を含む組を数えない() {
        let (rows, _) = count_texts(&["3月良い天気", "良い3月"], 1);
        assert_eq!(rows, [row("c", "良い", "てんき", "天気", 1), row("b", "良い", "てんき", "天気", 1)]);
    }

    #[test]
    fn 逆変換に失敗した連続を数える() {
        // 単漢字辞書にも無い漢字は読めない
        let (rows, batch) = count_texts(&["今日は鬱、良い天気"], 1);
        assert_eq!((batch.runs, batch.failed), (2, 1));
        assert_eq!(rows, [row("c", "良い", "てんき", "天気", 1), row("b", "良い", "てんき", "天気", 1)]);
    }

    #[test]
    fn 最小回数未満の組を捨てる() {
        let (rows, _) = count_texts(&["今日は良い天気", "今日は良い天気", "今日は天気"], 2);
        assert_eq!(
            rows,
            [
                row("c", "今日は", "よい", "良い", 2),
                row("c", "良い", "てんき", "天気", 2),
                row("b", "今日", "よい", "良い", 2),
                row("b", "良い", "てんき", "天気", 2),
                row("s", "今日", "は", "は", 3),
            ]
        );
    }

    #[test]
    fn 文脈ごとに回数の多い順に上位件数まで残し同じ回数は表記の順にする() {
        let mut counter = Counter::default();
        let prev = || bunsetsu("きょう", "今日", Some("今日"));
        // 数字を含む表記は数えないので、番号はカタカナにする (語ア, 語ィ, …)
        let name = |i: usize| format!("語{}", char::from_u32(u32::from('ア') + i as u32).unwrap());
        for i in 0..MAX_PER_CONTEXT + 2 {
            // 最後の 2 つだけ多く数える
            let surface = name(i);
            let times = match i {
                i if i == MAX_PER_CONTEXT + 1 => 3,
                i if i == MAX_PER_CONTEXT => 2,
                _ => 1,
            };
            for _ in 0..times {
                counter.add_run(&[prev(), bunsetsu("ご", &surface, Some(&surface))]);
            }
        }
        let list = counter.select(&counter.bunsetsu_table, 1);
        assert_eq!(list.len(), 1);
        let (context, rows) = &list[0];
        assert_eq!(*context, "今日");
        assert_eq!(rows.len(), MAX_PER_CONTEXT);
        assert_eq!(rows[0], ("ご", &*name(MAX_PER_CONTEXT + 1), 3));
        assert_eq!(rows[1], ("ご", &*name(MAX_PER_CONTEXT), 2));
        assert_eq!(rows[2], ("ご", &*name(0), 1));
        assert_eq!(rows[MAX_PER_CONTEXT - 1], ("ご", &*name(MAX_PER_CONTEXT - 3), 1));
    }

    #[test]
    fn 文節の表の文脈は前の文節の表記の末尾16文字にする() {
        let mut counter = Counter::default();
        let long = "あいうえおかきくけこさしすせそたちつてと";
        counter.add_run(&[bunsetsu(long, long, Some(long)), bunsetsu("てんき", "天気", Some("天気"))]);
        let rows = table_rows(&counter, 1);
        assert_eq!(rows[0], row("c", "おかきくけこさしすせそたちつてと", "てんき", "天気", 1));
        // バックオフの表の文脈は自立語の表記のまま
        assert_eq!(rows[1], row("b", long, "てんき", "天気", 1));
    }

    #[test]
    fn ダンプのtextを持たない行は読み飛ばす() {
        let (dict, index) = sample();
        let (matrix, functional) = (ConnectionMatrix::empty(), functional());
        let reverser = Reverser::new(&dict, &index, &matrix, &functional);
        let mut batch = Batch::default();
        reverser.add_line(r#"{"index":{"_type":"_doc","_id":"1"}}"#, &mut batch).unwrap();
        reverser.add_line(r#"{"title":"今日","text":"今日は良い天気"}"#, &mut batch).unwrap();
        assert!(reverser.add_line("{壊れた行", &mut batch).is_err());
        assert_eq!((batch.docs, batch.runs, batch.bunsetsu.len()), (1, 1, 1));
    }
}
