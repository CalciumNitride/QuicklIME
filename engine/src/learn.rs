// 学習 (確定履歴) の記録と永続化 (docs/design/learning-score.md)
//
// 学習は 3 種類で、どれも「使った度合い」(d, 最後に数えたときの確定の通し番号 t) を持つ。
// - 語の学習 (読み, 表記): 確定した文節を辞書の語に分けたときの自立語。Viterbi の単語コストを下げる
// - 句の学習 (読み, 表記, 種類): 確定した文節 (seg) と、2 文節以上の確定の読み全体 (whole)。
//   予測の確定履歴と読み全体の学習表記
// - 文脈の句の学習 (文脈, 読み, 表記): 直前の文節の表記 (末尾 16 文字) に続いた文節。前文脈付きの予測と
//   後続語の予測
// 古さは確定の回数で測り、今の度合いは d × 2^(−(N − t) / HALF_LIFE)。割引は種類ごとの
// min(上限, 重み × log2(1 + 今の度合い))。
// 永続化は learning_stats.tsv の追記ログ (同じ件は後の行が勝つ)。learning.tsv と learning_context.tsv は
// learning_stats.tsv が無いときに一度だけ取り込み、以後は書かない (古いエンジンに戻したときのために残す)。
//
// 境界学習: 人が文節伸縮で直した分割の文節読みを記憶し、変換時の文節境界を
// その位置へ寄せるのに使う。永続化は learning_boundary.tsv (1行1読み)

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::typo;

/// 文脈キー (直前文節の表記) の最大文字数。
/// 長文の貼り付けなど異常に長い文脈は末尾だけをキーにする
const MAX_CONTEXT_KEY_CHARS: usize = 16;

/// 境界学習に記録する文節読みの最小文字数。
/// 「は」のような1文字の付属語まで境界にすると、どこでも切れる方向に効いてしまう
pub const MIN_BOUNDARY_CHARS: usize = 2;

/// learning_stats.tsv を書き直すときに捨てる今の度合いの下限
const MIN_KEEP: f64 = 0.05;

/// 度合いと割引の定数 (docs/design/learning-score.md の B)。評価 (examples/learning_eval.rs) で
/// 値を振れるよう、ストアごとに持つ
#[derive(Clone, Copy, Debug)]
pub struct Weights {
    /// 度合いが半分になる確定 (文節) の回数
    pub half_life: f64,
    pub word_weight: f64,
    pub word_max: f64,
    /// 文脈の句の学習の重みと上限
    pub context_weight: f64,
    pub context_max: f64,
    pub phrase_weight: f64,
    pub phrase_max: f64,
    /// 予測の句の学習の点数の基準 (割引を引く前の値)
    pub phrase_base: i64,
}

pub const WEIGHTS: Weights = Weights {
    half_life: 2000.0,
    word_weight: 1000.0,
    word_max: 1500.0,
    context_weight: 2500.0,
    context_max: 8000.0,
    phrase_weight: 1500.0,
    phrase_max: 6000.0,
    phrase_base: 6000,
};

/// 学習の 1 件の使った度合い
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    /// 最後に数えたときの度合い
    pub d: f64,
    /// 最後に数えたときの確定の通し番号
    pub t: u64,
}

/// 文節 (読み, 表記) を辞書の語に分けた結果。分けられれば自立語の (読み, 表記) を経路の順に返す
/// (付属語は含めない)。分けられなければ None
pub type Splitter<'a> = dyn Fn(&str, &str) -> Option<Vec<(String, String)>> + 'a;

/// 句の学習の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhraseKind {
    /// 確定した文節
    Segment,
    /// 2 文節以上の確定の読み全体
    Whole,
}

impl PhraseKind {
    fn name(self) -> &'static str {
        match self {
            PhraseKind::Segment => "seg",
            PhraseKind::Whole => "whole",
        }
    }

    fn parse(name: &str) -> Option<PhraseKind> {
        match name {
            "seg" => Some(PhraseKind::Segment),
            "whole" => Some(PhraseKind::Whole),
            _ => None,
        }
    }
}

/// 句の学習の 1 件
struct Phrase {
    stat: Stat,
    kind: PhraseKind,
    /// 辞書の語に分けられなかった句
    unsplit: bool,
}

impl Phrase {
    /// 名詞の 1 語としてラティスに載せる句か。読み全体は読み全体の学習表記が受け持ち、数字を含む表記は
    /// 1 語が 1-best になると数字の書き方の候補が出なくなるため載せない
    fn in_lattice(&self, surface: &str) -> bool {
        self.unsplit
            && self.kind == PhraseKind::Segment
            && !surface.chars().any(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
    }
}

pub struct LearningStore {
    /// 読み → 表記 → 語の学習
    words: HashMap<String, HashMap<String, Stat>>,
    /// 読み → 表記 → 句の学習
    phrases: HashMap<String, HashMap<String, Phrase>>,
    /// 文脈キー → 読み → 表記 → 文脈の句の学習。前文脈から引くため、この順に持つ
    ctx_phrases: HashMap<String, HashMap<String, HashMap<String, Stat>>>,
    /// ラティスに載せる句の読みの最大文字数 (0 なら無し)。ラティスで部分文字列を引く上限に使う
    unsplit_max: usize,
    /// 確定の通し番号 (確定した文節を数えるたびに 1 増える)
    n: u64,
    weights: Weights,
    /// learning_stats.tsv。None ならメモリ上のみ (保存先不明・テスト時)
    stats_path: Option<PathBuf>,
    /// 人が文節伸縮で直した分割の文節読み
    boundary: HashSet<String>,
    /// 境界学習の追記先ファイル (learning_boundary.tsv)
    boundary_path: Option<PathBuf>,
    /// boundary に入っている読みの最大文字数。変換時の部分文字列探索の上限に使う
    boundary_max: usize,
}

impl LearningStore {
    /// 既定の保存先から読み込む。learning_stats.tsv が無ければ learning.tsv と learning_context.tsv を
    /// 取り込んで作る。split は文節を辞書の語に分ける処理 (convert::split_words)
    pub fn load_default(split: &Splitter) -> Self {
        let Some(path) = default_path() else {
            eprintln!("学習ファイルの保存先を特定できません。学習はこのセッション限りになります");
            return LearningStore::in_memory();
        };
        LearningStore::open(&path, split)
    }

    /// learning.tsv のパス path と同じディレクトリの学習ファイルを使うストアを作る
    pub fn open(path: &Path, split: &Splitter) -> Self {
        let mut store = LearningStore::in_memory();
        store.stats_path = Some(sibling_path(path, "_stats"));
        store.load_files(path, split);
        let boundary_path = sibling_path(path, "_boundary");
        if let Ok(file) = File::open(&boundary_path) {
            store.load_boundary_from(BufReader::new(file));
            eprintln!(
                "境界学習データを読み込みました: {} 件 [{}]",
                store.boundary.len(),
                boundary_path.display()
            );
        }
        store.boundary_path = Some(boundary_path);
        store
    }

    /// テスト用: メモリ上のみのストア
    pub fn in_memory() -> Self {
        LearningStore {
            words: HashMap::new(),
            phrases: HashMap::new(),
            ctx_phrases: HashMap::new(),
            unsplit_max: 0,
            n: 0,
            weights: WEIGHTS,
            stats_path: None,
            boundary: HashSet::new(),
            boundary_path: None,
            boundary_max: 0,
        }
    }

    /// 評価用: 定数を差し替える
    #[allow(dead_code)]
    pub fn set_weights(&mut self, weights: Weights) {
        self.weights = weights;
    }

    /// 学習ファイル (learning.tsv のパス) の隣の learning_stats.tsv を読む。無ければ古いファイルを取り込む
    fn load_files(&mut self, learning_path: &Path, split: &Splitter) {
        let stats_path = self.stats_path.clone().expect("stats_path");
        if let Ok(file) = File::open(&stats_path) {
            let lines = self.load_stats_from(BufReader::new(file));
            self.refresh_phrases(split);
            eprintln!("学習データを読み込みました: {} 件 [{}]", self.entry_count(), stats_path.display());
            if lines > 2 * self.entry_count() {
                self.rewrite();
            }
            return;
        }
        let ctx_path = sibling_path(learning_path, "_context");
        let plain = File::open(learning_path).ok();
        let context = File::open(&ctx_path).ok();
        if plain.is_none() && context.is_none() {
            return;
        }
        if let Some(file) = plain {
            self.import_plain(BufReader::new(file), split);
        }
        if let Some(file) = context {
            self.import_context(BufReader::new(file));
        }
        eprintln!(
            "古い学習データを取り込みました: {} 件 [{}・{}]",
            self.entry_count(),
            learning_path.display(),
            ctx_path.display()
        );
        self.rewrite();
    }

    /// learning_stats.tsv の内容を読む (同じ件は後の行が勝つ)。戻り値は読んだ有効な行の数
    pub fn load_stats_from(&mut self, reader: impl BufRead) -> usize {
        let mut lines = 0;
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            let f: Vec<&str> = line.split('\t').collect();
            let parse = |d: &str, t: &str| -> Option<Stat> {
                Some(Stat { d: d.parse().ok()?, t: t.parse().ok()? })
            };
            let filled = |fields: &[&str]| fields.iter().all(|f| !f.is_empty());
            let ok = match f.as_slice() {
                ["n", n] => n.parse().map(|n| self.n = n).is_ok(),
                ["w", reading, surface, d, t] if filled(&[reading, surface]) => {
                    parse(d, t).map(|stat| self.set_word(reading, surface, stat)).is_some()
                }
                ["p", reading, surface, d, t, kind] if filled(&[reading, surface]) => {
                    match (parse(d, t), PhraseKind::parse(kind)) {
                        (Some(stat), Some(kind)) => {
                            let phrase = Phrase { stat, kind, unsplit: false };
                            self.phrases
                                .entry(reading.to_string())
                                .or_default()
                                .insert(surface.to_string(), phrase);
                            true
                        }
                        _ => false,
                    }
                }
                ["q", context, reading, surface, d, t] if filled(&[context, reading, surface]) => {
                    parse(d, t).map(|stat| self.set_ctx_phrase(context, reading, surface, stat)).is_some()
                }
                _ => false,
            };
            lines += usize::from(ok);
        }
        lines
    }

    /// 読み込んだ句を辞書の語に分け直し、分けられない句を求める (learning_stats.tsv は分けた結果を持たない)
    pub fn refresh_phrases(&mut self, split: &Splitter) {
        for (reading, surfaces) in &mut self.phrases {
            for (surface, phrase) in surfaces.iter_mut() {
                phrase.unsplit = split(reading, surface).is_none();
            }
        }
        self.rebuild_index();
    }

    /// 古い learning.tsv (読み\t表記) を記録順に、各行を 1 回の文脈なしの文節の確定として取り込む
    pub fn import_plain(&mut self, reader: impl BufRead, split: &Splitter) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some((reading, surface)) = line.split_once('\t') {
                if !reading.is_empty() && !surface.is_empty() && !surface.contains('\t') {
                    self.learn_segment(reading, surface, "", split(reading, surface).as_deref());
                }
            }
        }
    }

    /// 古い learning_context.tsv (文脈\t読み\t表記) を記録順に取り込む。各行は文脈の句の学習だけを数える
    /// (同じ確定の語の学習と句の学習は learning.tsv の行で数えるため)
    pub fn import_context(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            let mut parts = line.splitn(3, '\t');
            if let (Some(context), Some(reading), Some(surface)) =
                (parts.next(), parts.next(), parts.next())
            {
                if !context.is_empty() && !reading.is_empty() && !surface.is_empty() {
                    self.count_ctx_phrase(context, reading, surface);
                }
            }
        }
    }

    /// 確定した文節を数える (docs/design/learning-score.md の A の 2〜4)。words は文節を辞書の語に分けた
    /// 自立語 (分けられなければ None)、context は直前の文節の表記 (無ければ空)
    pub fn learn_segment(
        &mut self,
        reading: &str,
        surface: &str,
        context: &str,
        words: Option<&[(String, String)]>,
    ) {
        if reading.is_empty() || surface.is_empty() {
            return;
        }
        self.n += 1;
        for (r, s) in words.unwrap_or_default() {
            if counts_as_word(r, s) {
                self.count_word(r, s);
            }
        }
        if !context.is_empty() {
            self.count_ctx_phrase(context, reading, surface);
        }
        self.learn_phrase(reading, surface, PhraseKind::Segment, words.is_some());
    }

    /// 句の学習だけを数える (文節、2 文節以上の確定の読み全体、LEARNWORD)。split は句を辞書の語に分けられたか。
    /// 文節として数えたことのある句は、読み全体として数えても文節のままにする
    pub fn learn_phrase(&mut self, reading: &str, surface: &str, kind: PhraseKind, split: bool) {
        if reading.is_empty() || surface.is_empty() {
            return;
        }
        let w = self.weights;
        let n = self.n;
        let phrase = self
            .phrases
            .entry(reading.to_string())
            .or_default()
            .entry(surface.to_string())
            .or_insert(Phrase { stat: Stat { d: 0.0, t: n }, kind, unsplit: false });
        phrase.stat = Stat { d: now(&phrase.stat, n, &w) + 1.0, t: n };
        if kind == PhraseKind::Segment {
            phrase.kind = PhraseKind::Segment;
        }
        phrase.unsplit = !split;
        let (stat, kind) = (phrase.stat, phrase.kind);
        if phrase.in_lattice(surface) {
            self.unsplit_max = self.unsplit_max.max(reading.chars().count());
        }
        self.append(&format!("p\t{reading}\t{surface}\t{}\t{}\t{}", stat.d, stat.t, kind.name()));
    }

    fn count_word(&mut self, reading: &str, surface: &str) {
        let (w, n) = (self.weights, self.n);
        let stat = self
            .words
            .entry(reading.to_string())
            .or_default()
            .entry(surface.to_string())
            .or_insert(Stat { d: 0.0, t: n });
        *stat = Stat { d: now(stat, n, &w) + 1.0, t: n };
        let stat = *stat;
        self.append(&format!("w\t{reading}\t{surface}\t{}\t{}", stat.d, stat.t));
    }

    fn count_ctx_phrase(&mut self, context: &str, reading: &str, surface: &str) {
        let key = context_key(context);
        let (w, n) = (self.weights, self.n);
        let stat = self
            .ctx_phrases
            .entry(key.clone())
            .or_default()
            .entry(reading.to_string())
            .or_default()
            .entry(surface.to_string())
            .or_insert(Stat { d: 0.0, t: n });
        *stat = Stat { d: now(stat, n, &w) + 1.0, t: n };
        let stat = *stat;
        self.append(&format!("q\t{key}\t{reading}\t{surface}\t{}\t{}", stat.d, stat.t));
    }

    fn set_word(&mut self, reading: &str, surface: &str, stat: Stat) {
        self.words.entry(reading.to_string()).or_default().insert(surface.to_string(), stat);
    }

    fn set_ctx_phrase(&mut self, context: &str, reading: &str, surface: &str, stat: Stat) {
        self.ctx_phrases
            .entry(context_key(context))
            .or_default()
            .entry(reading.to_string())
            .or_default()
            .insert(surface.to_string(), stat);
    }

    /// 数えた件の行と通し番号の行を learning_stats.tsv へ追記する
    fn append(&self, line: &str) {
        let Some(path) = &self.stats_path else {
            return;
        };
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| write!(file, "{line}\nn\t{}\n", self.n));
        if let Err(e) = result {
            eprintln!("学習ファイルへの書き込みに失敗しました ({e})");
        }
    }

    /// 今の度合いが MIN_KEEP 未満の件を捨て、learning_stats.tsv をメモリの内容で書き直す
    fn rewrite(&mut self) {
        let (w, n) = (self.weights, self.n);
        let keep = |stat: &Stat| now(stat, n, &w) >= MIN_KEEP;
        for surfaces in self.words.values_mut() {
            surfaces.retain(|_, stat| keep(stat));
        }
        self.words.retain(|_, surfaces| !surfaces.is_empty());
        for surfaces in self.phrases.values_mut() {
            surfaces.retain(|_, phrase| keep(&phrase.stat));
        }
        self.phrases.retain(|_, surfaces| !surfaces.is_empty());
        for readings in self.ctx_phrases.values_mut() {
            for surfaces in readings.values_mut() {
                surfaces.retain(|_, stat| keep(stat));
            }
            readings.retain(|_, surfaces| !surfaces.is_empty());
        }
        self.ctx_phrases.retain(|_, readings| !readings.is_empty());
        self.rebuild_index();

        let Some(path) = &self.stats_path else {
            return;
        };
        // 並びを毎回同じにする (書き直しの前後で差分を見やすくする)
        let mut lines: Vec<(u64, String)> = Vec::new();
        for (reading, surfaces) in &self.words {
            for (surface, s) in surfaces {
                lines.push((s.t, format!("w\t{reading}\t{surface}\t{}\t{}\n", s.d, s.t)));
            }
        }
        for (reading, surfaces) in &self.phrases {
            for (surface, p) in surfaces {
                let (s, kind) = (p.stat, p.kind.name());
                lines.push((s.t, format!("p\t{reading}\t{surface}\t{}\t{}\t{kind}\n", s.d, s.t)));
            }
        }
        for (context, readings) in &self.ctx_phrases {
            for (reading, surfaces) in readings {
                for (surface, s) in surfaces {
                    lines.push((s.t, format!("q\t{context}\t{reading}\t{surface}\t{}\t{}\n", s.d, s.t)));
                }
            }
        }
        lines.sort();
        let mut content = format!("n\t{}\n", self.n);
        for (_, line) in lines {
            content.push_str(&line);
        }
        if let Err(e) = rewrite_file(path, &content) {
            eprintln!("学習ファイルの書き直しに失敗しました ({e})");
        }
    }

    /// phrases から unsplit_max を作り直す
    fn rebuild_index(&mut self) {
        self.unsplit_max = self
            .phrases
            .iter()
            .filter(|(_, surfaces)| surfaces.iter().any(|(s, p)| p.in_lattice(s)))
            .map(|(reading, _)| reading.chars().count())
            .max()
            .unwrap_or(0);
    }

    /// 3 種類の学習の件数の合計
    pub fn entry_count(&self) -> usize {
        let words: usize = self.words.values().map(HashMap::len).sum();
        let phrases: usize = self.phrases.values().map(HashMap::len).sum();
        let ctx_phrases: usize =
            self.ctx_phrases.values().flat_map(HashMap::values).map(HashMap::len).sum();
        words + phrases + ctx_phrases
    }

    /// 確定の通し番号
    #[allow(dead_code)]
    pub fn serial(&self) -> u64 {
        self.n
    }

    /// 語の学習の今の度合い (テスト・評価用)
    #[allow(dead_code)]
    pub fn word_degree(&self, reading: &str, surface: &str) -> Option<f64> {
        self.words.get(reading)?.get(surface).map(|s| now(s, self.n, &self.weights))
    }

    /// 語の学習の記録 (テスト用)
    #[allow(dead_code)]
    pub fn word_stat(&self, reading: &str, surface: &str) -> Option<Stat> {
        self.words.get(reading)?.get(surface).copied()
    }

    /// 句の学習の記録 (テスト用)
    #[allow(dead_code)]
    pub fn phrase_stat(&self, reading: &str, surface: &str) -> Option<(Stat, PhraseKind)> {
        self.phrases.get(reading)?.get(surface).map(|p| (p.stat, p.kind))
    }

    /// 文脈の句の学習の記録 (テスト用)
    #[allow(dead_code)]
    pub fn ctx_phrase_stat(&self, context: &str, reading: &str, surface: &str) -> Option<Stat> {
        self.ctx_phrases.get(&context_key(context))?.get(reading)?.get(surface).copied()
    }

    /// 予測の句の学習の点数の基準 (PHRASE_BASE)
    pub fn phrase_base(&self) -> i64 {
        self.weights.phrase_base
    }

    /// 語の学習の割引
    pub fn word_bonus(&self, reading: &str, surface: &str) -> i64 {
        let w = &self.weights;
        self.words
            .get(reading)
            .and_then(|s| s.get(surface))
            .map_or(0, |s| bonus(now(s, self.n, w), w.word_weight, w.word_max))
    }

    /// 直前の文節の表記 context に続く文節 (読み, 表記) の文脈の句の学習の割引
    pub fn ctx_phrase_bonus(&self, context: &str, reading: &str, surface: &str) -> i64 {
        self.ctx_phrase_stat(context, reading, surface).map_or(0, |s| self.ctx_value(&s).0)
    }

    /// 文脈の句の (割引, 今の度合い, 番号)
    fn ctx_value(&self, s: &Stat) -> (i64, f64, u64) {
        let w = &self.weights;
        let d = now(s, self.n, w);
        (bonus(d, w.context_weight, w.context_max), d, s.t)
    }

    /// 句の学習があるか
    pub fn has_phrase(&self, reading: &str, surface: &str) -> bool {
        self.phrases.get(reading).is_some_and(|s| s.contains_key(surface))
    }

    /// 句の学習の割引 (テスト・評価用)
    #[allow(dead_code)]
    pub fn phrase_bonus(&self, reading: &str, surface: &str) -> i64 {
        self.phrases.get(reading).and_then(|s| s.get(surface)).map_or(0, |p| self.phrase_value(p).0)
    }

    /// 句の (割引, 今の度合い, 番号)。割引の大きい順に並べるときの比較に使う
    fn phrase_value(&self, p: &Phrase) -> (i64, f64, u64) {
        let w = &self.weights;
        let d = now(&p.stat, self.n, w);
        (bonus(d, w.phrase_weight, w.phrase_max), d, p.stat.t)
    }

    /// 読みの句の学習の表記を割引の大きい順に返す (同じ割引は今の度合い、番号の大きい順)
    pub fn phrase_surfaces(&self, reading: &str) -> Vec<&str> {
        let Some(surfaces) = self.phrases.get(reading) else {
            return Vec::new();
        };
        let mut list: Vec<(&str, (i64, f64, u64))> =
            surfaces.iter().map(|(s, p)| (s.as_str(), self.phrase_value(p))).collect();
        list.sort_by(|a, b| compare_value(&b.1, &a.1).then_with(|| a.0.cmp(b.0)));
        list.into_iter().map(|(s, _)| s).collect()
    }

    /// 読み全体の学習表記: 読みの句の学習のうち割引が最も大きい表記
    pub fn best_phrase(&self, reading: &str) -> Option<&str> {
        self.phrase_surfaces(reading).into_iter().next()
    }

    /// ラティスに載せる句 (文節で、辞書の語に分けられず、表記に数字を含まない) のうち読みが reading の
    /// ものの (表記, 割引)
    pub fn unsplit_phrases(&self, reading: &str) -> Vec<(&str, i64)> {
        let Some(surfaces) = self.phrases.get(reading) else {
            return Vec::new();
        };
        let mut list: Vec<(&str, i64)> = surfaces
            .iter()
            .filter(|(s, p)| p.in_lattice(s))
            .map(|(s, p)| (s.as_str(), self.phrase_value(p).0))
            .collect();
        list.sort();
        list
    }

    /// ラティスに載せる句の読みの最大文字数 (0 なら無し)
    pub fn max_unsplit_chars(&self) -> usize {
        self.unsplit_max
    }

    /// 読みが prefix で始まる句の学習を割引の大きい順に返す (PREDICT の確定履歴)
    pub fn phrase_prefix(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        let mut matches: Vec<(&str, &str, (i64, f64, u64))> = Vec::new();
        for (reading, surfaces) in &self.phrases {
            if !reading.starts_with(prefix) {
                continue;
            }
            for (surface, p) in surfaces {
                matches.push((reading, surface, self.phrase_value(p)));
            }
        }
        sort_matches(&mut matches);
        matches.truncate(limit);
        matches.into_iter().map(|(r, s, _)| (r, s)).collect()
    }

    /// 句の学習を打鍵のファジィ照合 matcher で引き、(読み, 表記, 割引, ずれのコスト, ずれの数) を返す
    /// (SUGGEST の予測。並びは呼び出し側が点数で決める)
    pub fn phrase_fuzzy(&self, matcher: &typo::Matcher) -> Vec<PhraseMatch<'_>> {
        let mut result = Vec::new();
        for (reading, surfaces) in &self.phrases {
            let Some((cost, edits)) = matcher.match_reading(reading) else {
                continue;
            };
            for (surface, p) in surfaces {
                result.push(PhraseMatch { reading, surface, bonus: self.phrase_value(p).0, cost, edits });
            }
        }
        // HashMap の走査順によらず同じ並びにする
        result.sort_by(|a, b| (a.reading, a.surface).cmp(&(b.reading, b.surface)));
        result
    }

    /// 前文脈 context の文脈の句の学習を打鍵のファジィ照合 matcher で引き、(読み, 表記, 文脈の句の学習の割引,
    /// ずれのコスト, ずれの数) を返す (SUGGEST の予測)
    pub fn ctx_phrase_fuzzy(&self, context: &str, matcher: &typo::Matcher) -> Vec<PhraseMatch<'_>> {
        let Some(readings) = self.ctx_phrases.get(&context_key(context)) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for (reading, surfaces) in readings {
            let Some((cost, edits)) = matcher.match_reading(reading) else {
                continue;
            };
            for (surface, s) in surfaces {
                result.push(PhraseMatch { reading, surface, bonus: self.ctx_value(s).0, cost, edits });
            }
        }
        result.sort_by(|a, b| (a.reading, a.surface).cmp(&(b.reading, b.surface)));
        result
    }

    /// 前文脈 context の後に数えた文脈の句の学習のうち、読みが prefix で始まるものを割引の大きい順に返す
    /// (prefix が空なら全件)。表記の重複は先勝ちで除く (PREDICT の前文脈・NEXTWORD)
    pub fn context_prefix(&self, context: &str, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        let Some(readings) = self.ctx_phrases.get(&context_key(context)) else {
            return Vec::new();
        };
        let mut matches: Vec<(&str, &str, (i64, f64, u64))> = Vec::new();
        for (reading, surfaces) in readings {
            if !reading.starts_with(prefix) {
                continue;
            }
            for (surface, s) in surfaces {
                matches.push((reading, surface, self.ctx_value(s)));
            }
        }
        sort_matches(&mut matches);
        let mut results: Vec<(&str, &str)> = Vec::new();
        for (reading, surface, _) in matches {
            if results.len() >= limit {
                break;
            }
            if !results.iter().any(|(_, s)| *s == surface) {
                results.push((reading, surface));
            }
        }
        results
    }

    /// (読み, 表記) の学習を消す (docs/design/learning-score.md の F)。words は (読み, 表記) を辞書の語に
    /// 分けた自立語。句の学習の (読み, 表記)、文脈の句の学習の (読み, 表記) (文脈によらず)、語の学習のうち
    /// 分けた自立語を消し、消したら learning_stats.tsv を書き直す。戻り値は消した件数
    pub fn forget(&mut self, reading: &str, surface: &str, words: Option<&[(String, String)]>) -> usize {
        let mut count = 0;
        if let Some(surfaces) = self.phrases.get_mut(reading) {
            count += usize::from(surfaces.remove(surface).is_some());
            if surfaces.is_empty() {
                self.phrases.remove(reading);
            }
        }
        for readings in self.ctx_phrases.values_mut() {
            if let Some(surfaces) = readings.get_mut(reading) {
                count += usize::from(surfaces.remove(surface).is_some());
                if surfaces.is_empty() {
                    readings.remove(reading);
                }
            }
        }
        self.ctx_phrases.retain(|_, readings| !readings.is_empty());
        for (r, s) in words.unwrap_or_default() {
            if let Some(surfaces) = self.words.get_mut(r) {
                count += usize::from(surfaces.remove(s).is_some());
                if surfaces.is_empty() {
                    self.words.remove(r);
                }
            }
        }
        if count > 0 {
            self.rewrite();
        }
        count
    }

    /// 境界学習の追記ログ (1行1読み) を読み込む
    pub fn load_boundary_from(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            self.insert_boundary(line.trim_end());
        }
    }

    /// 人が文節伸縮で直した分割の文節読みを境界として記録し、ファイルへ追記する
    pub fn record_boundary(&mut self, reading: &str) {
        if !self.insert_boundary(reading) {
            return; // 既知の境界。追記ログが際限なく伸びないよう書き込みを省く
        }
        if let Some(path) = &self.boundary_path {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{reading}"));
            if let Err(e) = result {
                eprintln!("境界学習ファイルへの書き込みに失敗しました ({e})");
            }
        }
    }

    /// 境界の集合へ追加する。新規に追加できたら true
    fn insert_boundary(&mut self, reading: &str) -> bool {
        let len = reading.chars().count();
        if len < MIN_BOUNDARY_CHARS || !self.boundary.insert(reading.to_string()) {
            return false;
        }
        self.boundary_max = self.boundary_max.max(len);
        true
    }

    /// 読みが学習済みの文節境界かどうか
    pub fn is_boundary(&self, reading: &str) -> bool {
        self.boundary.contains(reading)
    }

    /// 学習済み境界の読みの最大文字数 (0 なら境界学習なし)
    pub fn max_boundary_chars(&self) -> usize {
        self.boundary_max
    }
}

/// phrase_fuzzy・ctx_phrase_fuzzy の 1 件
pub struct PhraseMatch<'a> {
    pub reading: &'a str,
    pub surface: &'a str,
    pub bonus: i64,
    pub cost: i32,
    pub edits: usize,
}

/// 語の学習に数える語か。読みが 1 文字の語・表記が読みと同じひらがなの語・表記に数字を含む語に割引が付くと、
/// ほかの文で 1 文字のひらがなの語に細かく割れたり (「て|い|…」)、数字に置き換わったり (いち → 1) するため
/// 数えない
fn counts_as_word(reading: &str, surface: &str) -> bool {
    let hiragana = surface == reading && surface.chars().all(|c| ('ぁ'..='ゖ').contains(&c) || c == 'ー');
    let digit = surface.chars().any(|c| c.is_ascii_digit() || ('０'..='９').contains(&c));
    reading.chars().count() > 1 && !hiragana && !digit
}

/// 今の度合い
fn now(stat: &Stat, n: u64, w: &Weights) -> f64 {
    let age = n.saturating_sub(stat.t) as f64;
    stat.d * (-age / w.half_life).exp2()
}

/// 今の度合い d の割引
fn bonus(d: f64, weight: f64, max: f64) -> i64 {
    (weight * (1.0 + d).log2()).min(max).round() as i64
}

/// (割引, 今の度合い, 番号) の比較
fn compare_value(a: &(i64, f64, u64), b: &(i64, f64, u64)) -> std::cmp::Ordering {
    a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)).then_with(|| a.2.cmp(&b.2))
}

/// (読み, 表記, 値) を値の大きい順に並べる (同じ値は読み・表記の順)
fn sort_matches(matches: &mut [(&str, &str, (i64, f64, u64))]) {
    matches.sort_by(|a, b| compare_value(&b.2, &a.2).then_with(|| (a.0, a.1).cmp(&(b.0, b.1))));
}

/// ファイルの内容を content で置き換える。書き込みの途中で失敗しても元のファイルが
/// 壊れないよう、同じディレクトリの一時ファイルに書いてから rename する
pub fn rewrite_file(path: &Path, content: &str) -> std::io::Result<()> {
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let result = fs::write(&tmp, content).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// 文脈キーの正規化: 末尾 MAX_CONTEXT_KEY_CHARS 文字に切り詰める
fn context_key(context: &str) -> String {
    let chars: Vec<char> = context.chars().collect();
    chars[chars.len().saturating_sub(MAX_CONTEXT_KEY_CHARS)..].iter().collect()
}

/// 既定の学習ファイルパス。優先順: QUICKLIME_LEARN_FILE > %APPDATA%\QuicklIME\learning.tsv
fn default_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("QUICKLIME_LEARN_FILE") {
        return Some(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(appdata).join("QuicklIME");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("learning.tsv"))
}

/// 自動学習した複合語の保存先 (learning.tsv → learning_word.tsv)。
/// ユーザ辞書ではなく学習データなので、学習ファイルと同じディレクトリに置く
pub fn learned_word_path() -> Option<PathBuf> {
    Some(sibling_path(&default_path()?, "_word"))
}

/// 学習ファイルパスから派生ファイルのパスを導出する
/// (learning.tsv + "_context" → learning_context.tsv)。QUICKLIME_LEARN_FILE で
/// 学習ファイルを差し替えたときも同じディレクトリに対で作られる
fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("learning");
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => path.with_file_name(format!("{stem}{suffix}.{ext}")),
        None => path.with_file_name(format!("{stem}{suffix}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Table = &'static [(&'static str, &'static str, &'static [(&'static str, &'static str)])];

    /// テスト用の分け方: 表を引き、無ければ分けられない
    fn splitter(table: Table) -> impl Fn(&str, &str) -> Option<Vec<(String, String)>> {
        move |reading: &str, surface: &str| {
            table.iter().find(|(r, s, _)| *r == reading && *s == surface).map(|(_, _, words)| {
                words.iter().map(|(r, s)| (r.to_string(), s.to_string())).collect()
            })
        }
    }

    const TABLE: Table = &[
        ("きょうは", "今日は", &[("きょう", "今日")]),
        ("きょうは", "京は", &[("きょう", "京")]),
        ("きる", "切る", &[("きる", "切る")]),
        ("きる", "着る", &[("きる", "着る")]),
        ("ふくを", "服を", &[("ふく", "服")]),
        ("いいてんき", "いい天気", &[("いい", "いい"), ("てんき", "天気")]),
        ("はれ", "晴れ", &[("はれ", "晴れ")]),
        ("きょうしつ", "教室", &[("きょうしつ", "教室")]),
        ("きょうと", "京都", &[("きょうと", "京都")]),
    ];

    /// LEARN の文節 1 つと同じに数える
    fn learn(store: &mut LearningStore, reading: &str, surface: &str, context: &str) {
        let words = splitter(TABLE)(reading, surface);
        store.learn_segment(reading, surface, context, words.as_deref());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quicklime-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 数えるたびに度合いが1増え半減期の確定で半分になる() {
        let mut store = LearningStore::in_memory();
        store.set_weights(Weights { half_life: 10.0, ..WEIGHTS });
        learn(&mut store, "きる", "切る", "");
        assert_eq!(store.word_degree("きる", "切る"), Some(1.0));
        assert_eq!(store.serial(), 1);
        learn(&mut store, "きる", "切る", "");
        // 前回から 1 確定ぶん古くなった度合いに 1 を足す
        let expected = (-1.0f64 / 10.0).exp2() + 1.0;
        assert!((store.word_degree("きる", "切る").unwrap() - expected).abs() < 1e-9);
        // ほかの文節を HALF_LIFE 回確定すると半分になる
        for _ in 0..10 {
            learn(&mut store, "はれ", "晴れ", "");
        }
        assert!((store.word_degree("きる", "切る").unwrap() - expected / 2.0).abs() < 1e-9);
        assert_eq!(store.serial(), 12);
        // 通し番号は文節ごとに 1 増える (語の数や分けられるかによらない)。読み全体は増やさない
        learn(&mut store, "いいてんき", "いい天気", "今日は");
        learn(&mut store, "まいぐらふぃ", "マイグラフィ", "");
        assert_eq!(store.serial(), 14);
        store.learn_phrase("きょうははれ", "今日は晴れ", PhraseKind::Whole, false);
        assert_eq!(store.serial(), 14);
    }

    #[test]
    fn 割引は度合いの対数で上限で止まる() {
        let mut store = LearningStore::in_memory();
        assert_eq!(store.word_bonus("きる", "切る"), 0);
        learn(&mut store, "きる", "切る", "");
        // 重み × log2(1 + 1)
        assert_eq!(store.word_bonus("きる", "切る"), 1000);
        assert_eq!(store.phrase_bonus("きる", "切る"), 1500);
        learn(&mut store, "きる", "切る", "");
        // 語の学習は 1000 × log2(3) ≈ 1585 が上限 1500 で止まる
        assert_eq!(store.word_bonus("きる", "切る"), 1500);
        for _ in 0..100 {
            learn(&mut store, "きる", "切る", "");
        }
        assert_eq!(store.word_bonus("きる", "切る"), WEIGHTS.word_max as i64);
        assert_eq!(store.phrase_bonus("きる", "切る"), WEIGHTS.phrase_max as i64);
    }

    #[test]
    fn 文節は語_句_文脈の句の学習を数える() {
        let mut store = LearningStore::in_memory();
        learn(&mut store, "きょうは", "今日は", "晴れ");
        assert!(store.word_stat("きょう", "今日").is_some());
        assert_eq!(store.phrase_stat("きょうは", "今日は").map(|p| p.1), Some(PhraseKind::Segment));
        assert!(store.ctx_phrase_stat("晴れ", "きょうは", "今日は").is_some());
        assert_eq!(store.ctx_phrase_bonus("晴れ", "きょうは", "今日は"), 2500);
        assert_eq!(store.ctx_phrase_bonus("明日は", "きょうは", "今日は"), 0);
        // 文脈が空なら文脈の句の学習は数えない
        learn(&mut store, "はれ", "晴れ", "");
        assert_eq!(store.entry_count(), 3 + 2);
    }

    #[test]
    fn 一文字の読み_読みと同じひらがな_数字を含む語は語の学習に数えない() {
        let mut store = LearningStore::in_memory();
        let words = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter().map(|(r, s)| (r.to_string(), s.to_string())).collect()
        };
        // いい (ひらがなのまま) は数えず、天気 は数える
        learn(&mut store, "いいてんき", "いい天気", "");
        assert!(store.word_stat("いい", "いい").is_none());
        assert!(store.word_stat("てんき", "天気").is_some());
        // 1 文字の読みの語 (て・手)、数字を含む語 (1・１日)、ひらがなのままの語 (きる・ずー) は数えない
        store.learn_segment("ての", "手の", "", Some(&words(&[("て", "手")])));
        store.learn_segment("いちにち", "1日", "", Some(&words(&[("いち", "1"), ("にち", "日")])));
        store.learn_segment("いちにち", "１日", "", Some(&words(&[("いちにち", "１日")])));
        store.learn_segment("きるずー", "きるずー", "", Some(&words(&[("きる", "きる"), ("ずー", "ずー")])));
        assert!(store.word_stat("て", "手").is_none());
        assert!(store.word_stat("いち", "1").is_none());
        assert!(store.word_stat("にち", "日").is_some());
        assert!(store.word_stat("いちにち", "１日").is_none());
        assert!(store.word_stat("きる", "きる").is_none());
        assert!(store.word_stat("ずー", "ずー").is_none());
        // カタカナは数える
        store.learn_segment("きる", "キル", "", Some(&words(&[("きる", "キル")])));
        assert!(store.word_stat("きる", "キル").is_some());
        // 句の学習はどれも数える
        assert!(store.phrase_stat("ての", "手の").is_some());
        assert!(store.phrase_stat("きるずー", "きるずー").is_some());
    }

    #[test]
    fn 分けられない文節は句の学習だけを数えラティスに載せる句になる() {
        let mut store = LearningStore::in_memory();
        learn(&mut store, "まいぐらふぃ", "マイグラフィ", "今日は");
        // 句の学習と文脈の句の学習
        assert_eq!(store.entry_count(), 2);
        assert_eq!(store.unsplit_phrases("まいぐらふぃ"), vec![("マイグラフィ", 1500)]);
        assert_eq!(store.max_unsplit_chars(), 6);
        learn(&mut store, "きょうは", "今日は", "");
        assert!(store.unsplit_phrases("きょうは").is_empty());
    }

    #[test]
    fn 読み全体と数字を含む表記の句はラティスに載せない() {
        let mut store = LearningStore::in_memory();
        store.learn_phrase("まいぐらふぃは", "マイグラフィは", PhraseKind::Whole, false);
        learn(&mut store, "1234えん", "1,234円", "");
        learn(&mut store, "１にち", "１日", "");
        assert!(store.unsplit_phrases("まいぐらふぃは").is_empty());
        assert!(store.unsplit_phrases("1234えん").is_empty());
        assert!(store.unsplit_phrases("１にち").is_empty());
        assert_eq!(store.max_unsplit_chars(), 0);
        // 読み全体の学習表記には使う
        assert_eq!(store.best_phrase("まいぐらふぃは"), Some("マイグラフィは"));
        // 文節として数えると文節になり、ラティスに載る
        learn(&mut store, "まいぐらふぃは", "マイグラフィは", "");
        assert_eq!(store.phrase_stat("まいぐらふぃは", "マイグラフィは").map(|p| p.1), Some(PhraseKind::Segment));
        assert_eq!(store.unsplit_phrases("まいぐらふぃは").len(), 1);
        // 文節として数えた句は、読み全体として数えても文節のまま
        store.learn_phrase("まいぐらふぃは", "マイグラフィは", PhraseKind::Whole, false);
        assert_eq!(store.phrase_stat("まいぐらふぃは", "マイグラフィは").map(|p| p.1), Some(PhraseKind::Segment));
    }

    #[test]
    fn 一回だけ別の表記を選んでもよく使う表記が先頭のまま() {
        let mut store = LearningStore::in_memory();
        for _ in 0..3 {
            learn(&mut store, "きる", "切る", "");
        }
        learn(&mut store, "きる", "着る", "");
        assert_eq!(store.best_phrase("きる"), Some("切る"));
        assert_eq!(store.phrase_surfaces("きる"), vec!["切る", "着る"]);
        assert!(store.word_bonus("きる", "切る") > store.word_bonus("きる", "着る"));
        assert_eq!(store.best_phrase("はれ"), None);
    }

    #[test]
    fn 前方一致の句は割引の大きい順に返す() {
        let mut store = LearningStore::in_memory();
        learn(&mut store, "きょうと", "京都", "");
        learn(&mut store, "きょうしつ", "教室", "");
        learn(&mut store, "きょうしつ", "教室", "");
        learn(&mut store, "はれ", "晴れ", "");
        assert_eq!(store.phrase_prefix("きょう", 8), vec![("きょうしつ", "教室"), ("きょうと", "京都")]);
        assert_eq!(store.phrase_prefix("きょう", 1), vec![("きょうしつ", "教室")]);
        assert!(store.phrase_prefix("あめ", 8).is_empty());
    }

    #[test]
    fn 文脈で引くと割引の大きい順に返り長い文脈は末尾で引く() {
        let mut store = LearningStore::in_memory();
        learn(&mut store, "はれ", "晴れ", "今日は");
        learn(&mut store, "いいてんき", "いい天気", "今日は");
        learn(&mut store, "いいてんき", "いい天気", "今日は");
        learn(&mut store, "きる", "着る", "明日は");
        // 文脈の句の学習の文節を返す
        assert_eq!(
            store.context_prefix("今日は", "", 8),
            vec![("いいてんき", "いい天気"), ("はれ", "晴れ")]
        );
        assert_eq!(store.context_prefix("今日は", "は", 8), vec![("はれ", "晴れ")]);
        assert_eq!(store.context_prefix("今日は", "", 1), vec![("いいてんき", "いい天気")]);
        assert!(store.context_prefix("昨日は", "", 8).is_empty());
        // 末尾 16 文字が同じ文脈は同じキー
        let long = "あ".repeat(30) + "服を";
        learn(&mut store, "きる", "着る", &long);
        let other = "い".repeat(30) + &"あ".repeat(14) + "服を";
        assert_eq!(store.context_prefix(&other, "", 8), vec![("きる", "着る")]);
        assert!(store.ctx_phrase_bonus(&other, "きる", "着る") > 0);
    }

    #[test]
    fn 打鍵のファジィ照合で句の学習を引く() {
        let mut store = LearningStore::in_memory();
        learn(&mut store, "きょうと", "京都", "");
        learn(&mut store, "きょうしつ", "教室", "");
        learn(&mut store, "はれ", "晴れ", "");
        // kyouti: kyouto の o を隣の i と打った
        let keys = typo::Keys::parse("きょうち", "kyo\x1f\x1fu\x1fti").unwrap();
        let matcher = typo::Matcher::new(&keys, 1, &typo::TUNING);
        let got = store.phrase_fuzzy(&matcher);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].reading, got[0].surface, got[0].bonus), ("きょうと", "京都", store.phrase_bonus("きょうと", "京都")));
        assert_eq!((got[0].cost, got[0].edits), (typo::TUNING.edit_adjacent, 1));
        // 文脈の句の学習も前文脈で引く
        learn(&mut store, "きょうと", "京都", "明日は");
        let got = store.ctx_phrase_fuzzy("明日は", &matcher);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].reading, got[0].surface, got[0].bonus), ("きょうと", "京都", 2500));
        assert!(store.ctx_phrase_fuzzy("今日は", &matcher).is_empty());
    }

    #[test]
    fn 再起動しても度合いと番号が同じになる() {
        let dir = temp_dir("learn-reload");
        let path = dir.join("learning.tsv");
        let split = splitter(TABLE);
        let mut store = LearningStore::open(&path, &split);
        learn(&mut store, "きる", "切る", "");
        learn(&mut store, "きる", "切る", "");
        learn(&mut store, "きる", "着る", "服を");
        learn(&mut store, "まいぐらふぃ", "マイグラフィ", "");
        assert!(dir.join("learning_stats.tsv").exists());
        let reloaded = LearningStore::open(&path, &split);
        assert_eq!(reloaded.serial(), store.serial());
        assert_eq!(reloaded.word_stat("きる", "切る"), store.word_stat("きる", "切る"));
        assert_eq!(reloaded.word_stat("きる", "着る"), store.word_stat("きる", "着る"));
        assert_eq!(reloaded.phrase_stat("きる", "切る"), store.phrase_stat("きる", "切る"));
        assert_eq!(
            reloaded.ctx_phrase_stat("服を", "きる", "着る"),
            store.ctx_phrase_stat("服を", "きる", "着る")
        );
        assert_eq!(reloaded.entry_count(), store.entry_count());
        // 分けられない句は読み直しで分け直して求める
        assert_eq!(reloaded.unsplit_phrases("まいぐらふぃ"), vec![("マイグラフィ", 1500)]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 行が件数の2倍を超えると起動時に書き直し古い件を捨てる() {
        let dir = temp_dir("learn-compact");
        let path = dir.join("learning.tsv");
        let stats = dir.join("learning_stats.tsv");
        // 2 件に 7 行。古い件 (今の度合い 1 × 2^-12.5 < MIN_KEEP) は捨てる。知らない種類の行 (c) は読み飛ばす
        fs::write(
            &stats,
            "w\tきる\t切る\t1\t1\nn\t1\nw\tきる\t切る\t2\t25000\nn\t25000\nw\tはれ\t晴れ\t1\t1\n\
             c\t服を\tきる\t着る\t1\t1\nn\t25001\n",
        )
        .unwrap();
        let store = LearningStore::open(&path, &splitter(TABLE));
        assert_eq!(store.serial(), 25001);
        assert_eq!(store.word_stat("きる", "切る"), Some(Stat { d: 2.0, t: 25000 }));
        assert_eq!(store.word_stat("はれ", "晴れ"), None);
        assert_eq!(fs::read_to_string(&stats).unwrap(), "n\t25001\nw\tきる\t切る\t2\t25000\n");
        assert!(!dir.join("learning_stats.tsv.tmp").exists());
        // 2 倍以下なら書き直さない
        let content = "n\t25001\nw\tきる\t切る\t2\t25000\nw\tはれ\t晴れ\t1\t1\n";
        fs::write(&stats, content).unwrap();
        let store = LearningStore::open(&path, &splitter(TABLE));
        assert!(store.word_stat("はれ", "晴れ").is_some());
        assert_eq!(fs::read_to_string(&stats).unwrap(), content);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 学習の統計が無ければ古いファイルから作り古いファイルは変えない() {
        let dir = temp_dir("learn-import");
        let path = dir.join("learning.tsv");
        let ctx_path = dir.join("learning_context.tsv");
        let plain = "きる\t切る\nきょうは\t今日は\n壊れた行\nきる\t切る\n";
        let context = "服を\tきる\t着る\n文脈\t読みだけ\n";
        fs::write(&path, plain).unwrap();
        fs::write(&ctx_path, context).unwrap();
        let store = LearningStore::open(&path, &splitter(TABLE));
        // learning.tsv の各行を 1 回の文節の確定として数える
        assert_eq!(store.serial(), 3);
        let degree = store.word_degree("きる", "切る").unwrap();
        assert!((degree - ((-2.0f64 / 2000.0).exp2() + 1.0)).abs() < 1e-9);
        assert_eq!(store.phrase_stat("きょうは", "今日は").map(|p| p.1), Some(PhraseKind::Segment));
        // learning_context.tsv の行は文脈の句の学習だけ (語・句の学習は数えない)
        assert!(store.ctx_phrase_stat("服を", "きる", "着る").is_some());
        assert!(store.word_stat("きる", "着る").is_none());
        assert!(store.phrase_stat("きる", "着る").is_none());
        assert!(dir.join("learning_stats.tsv").exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), plain);
        assert_eq!(fs::read_to_string(&ctx_path).unwrap(), context);
        // 2 回目の起動は learning_stats.tsv を読み、古いファイルは取り込み直さない
        fs::write(&path, "はれ\t晴れ\n").unwrap();
        let reloaded = LearningStore::open(&path, &splitter(TABLE));
        assert_eq!(reloaded.serial(), 3);
        assert!(reloaded.phrase_stat("はれ", "晴れ").is_none());
        assert_eq!(reloaded.entry_count(), store.entry_count());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 学習の統計も古いファイルも無ければ何も作らない() {
        let dir = temp_dir("learn-empty");
        let store = LearningStore::open(&dir.join("learning.tsv"), &splitter(TABLE));
        assert_eq!(store.entry_count(), 0);
        assert!(!dir.join("learning_stats.tsv").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn forgetで句_文脈の句_語の学習を消して書き直す() {
        let dir = temp_dir("learn-forget");
        let path = dir.join("learning.tsv");
        let split = splitter(TABLE);
        let mut store = LearningStore::open(&path, &split);
        learn(&mut store, "きょうは", "京は", "晴れ");
        learn(&mut store, "きょうは", "京は", "明日");
        learn(&mut store, "きょうは", "今日は", "");
        learn(&mut store, "はれ", "晴れ", "今日は");
        let words = split("きょうは", "京は");
        // 表記が違えば消えない
        assert_eq!(store.forget("きょうは", "京都", None), 0);
        // 句 1 + 文脈の句 2 + 語 1
        assert_eq!(store.forget("きょうは", "京は", words.as_deref()), 4);
        assert!(store.phrase_stat("きょうは", "京は").is_none());
        assert!(store.ctx_phrase_stat("明日", "きょうは", "京は").is_none());
        assert!(store.word_stat("きょう", "京").is_none());
        assert!(store.phrase_stat("きょうは", "今日は").is_some());
        assert_eq!(store.forget("きょうは", "京は", words.as_deref()), 0);
        // 書き直したファイルを読み直すと同じ状態
        let content = fs::read_to_string(dir.join("learning_stats.tsv")).unwrap();
        assert!(!content.contains("京"), "{content}");
        let reloaded = LearningStore::open(&path, &split);
        assert_eq!(reloaded.entry_count(), store.entry_count());
        assert_eq!(reloaded.serial(), store.serial());
        assert_eq!(reloaded.context_prefix("今日は", "", 8), vec![("はれ", "晴れ")]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 派生学習ファイルのパスを導出する() {
        assert_eq!(
            sibling_path(Path::new("C:\\dir\\learning.tsv"), "_context"),
            PathBuf::from("C:\\dir\\learning_context.tsv")
        );
        assert_eq!(
            sibling_path(Path::new("C:\\dir\\learning.tsv"), "_stats"),
            PathBuf::from("C:\\dir\\learning_stats.tsv")
        );
        assert_eq!(
            sibling_path(Path::new("C:\\dir\\learning.tsv"), "_boundary"),
            PathBuf::from("C:\\dir\\learning_boundary.tsv")
        );
        assert_eq!(
            sibling_path(Path::new("learn"), "_context"),
            PathBuf::from("learn_context")
        );
    }

    #[test]
    fn 記録した文節境界が引ける() {
        let mut store = LearningStore::in_memory();
        store.record_boundary("きょうは");
        store.record_boundary("いいてんき");
        assert!(store.is_boundary("きょうは"));
        assert!(store.is_boundary("いいてんき"));
        assert!(!store.is_boundary("きょうはい"));
        assert_eq!(store.max_boundary_chars(), 5);
    }

    #[test]
    fn 短すぎる読みは境界にしない() {
        let mut store = LearningStore::in_memory();
        store.record_boundary("は");
        store.record_boundary("");
        assert!(!store.is_boundary("は"));
        assert_eq!(store.max_boundary_chars(), 0);
    }

    #[test]
    fn 境界の追記ログを読み込む() {
        let mut store = LearningStore::in_memory();
        store.load_boundary_from("きょうは\nいいてんき\nは\n\n".as_bytes());
        assert!(store.is_boundary("きょうは"));
        assert!(store.is_boundary("いいてんき"));
        assert!(!store.is_boundary("は"));
        assert_eq!(store.max_boundary_chars(), 5);
    }
}
