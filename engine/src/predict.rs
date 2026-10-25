// 予測入力 (PREDICT・SUGGEST の予測) と読みなしの後続語予測 (NEXTWORD) の候補生成
//
// PREDICT は読みの前方一致でユーザ辞書の短縮よみ (記載順) → 文脈の句の学習 (前文脈があるとき、
// 割引の大きい順) → ユーザ辞書の名詞系 (記載順) → 句の学習 (確定履歴、割引の大きい順)
// → 同梱辞書とインポート辞書の名詞系 (両者を混ぜてコスト順。同コストは同梱辞書が先)
// を検索し、この順に合成して返す。表記の重複は先勝ちで除き、表記が入力かなそのままの候補は出しても意味が無いため除外する。
// 完全一致で枠が埋まらないときは、タイプミス補正 (かなを標準の綴りに戻した打鍵のファジィ照合) で補充する。
// SUGGEST の予測 (scored) は短縮よみ・ユーザ辞書の名詞系・句の学習・辞書の語を打鍵のファジィ照合で
// 常に引き、候補に点数を付ける (docs/spec/suggest.md「予測の照合」「点数」)。
// NEXTWORD は文脈の句の学習 → 後続語の統計の文節の表 → バックオフの表の順に前文脈で引く
// (辞書には語の連接の統計が無いため、日本語版ウィキペディアから数えた表を別に持つ。
// docs/spec/suggest.md「文節の連なりの統計」)。
// 候補の最大数・最小読み文字数・補正の有無は設定 (config.rs) に従う。

use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::Path;

use crate::config::Config;
use crate::dict::Dictionary;
use crate::learn::{self, LearningStore};
use crate::typo;
use crate::userdict::UserDict;

/// タイプミス補正 (曖昧一致) を使う最小の読み文字数。
/// 短い読みは1かな違いの別語が多すぎて誤補正だらけになる
const MIN_FUZZY_CHARS: usize = 3;

/// ファジィ照合で打鍵を使い切った位置のうち、その先の語を列挙する位置の数
const MAX_FUZZY_PREFIXES: usize = 8;

/// 読みの前方一致で予測候補 (読み, 表記) を返す。context は前文脈 (直前に確定した文節の表記)。
/// 読みは採用時の LEARN に使うため、入力の接頭辞ではなく候補の完全な読みを返す
pub fn predict(
    kana: &str,
    context: Option<&str>,
    dict: &Dictionary,
    user: &UserDict,
    learning: &LearningStore,
    cfg: &Config,
) -> Vec<(String, String)> {
    let max = cfg.max_predictions;
    if kana.chars().count() < cfg.min_suggest_chars {
        return Vec::new();
    }

    let mut results: Vec<(String, String)> = Vec::new();
    // ユーザ辞書 (短縮よみ → 名詞系) はユーザが明示登録した語なので履歴より先に出す
    for (reading, surface) in user.shortcut_prefix(kana, max) {
        push_unique(&mut results, kana, reading.to_string(), surface.to_string());
    }
    if let Some(context) = context {
        for (reading, surface) in learning.context_prefix(context, kana, max) {
            push_unique(&mut results, kana, reading.to_string(), surface.to_string());
        }
    }
    for (reading, surface) in user.word_prefix(kana, max) {
        push_unique(&mut results, kana, reading.to_string(), surface.to_string());
    }
    for (reading, surface) in learning.phrase_prefix(kana, max) {
        push_unique(&mut results, kana, reading.to_string(), surface.to_string());
    }
    // インポート辞書は数十万語規模になりうるため、履歴より前に置くと予測枠を占拠して
    // 履歴や一般語が出なくなる。同梱辞書と同じ「一般語」としてコスト順に混ぜる。
    // 安定ソートなので同コストは同梱辞書が先。
    // 履歴との重複と入力かな一致の除外で減る分を見込み、辞書は多めに引く
    let mut dict_hits = dict.predict_prefix_scored(kana, max * 2 + 1);
    dict_hits.extend(user.imported_prefix(kana, max * 2 + 1));
    dict_hits.sort_by_key(|(cost, _, _)| *cost);
    for (_, reading, surface) in dict_hits {
        if results.len() >= max {
            break;
        }
        push_unique(&mut results, kana, reading, surface);
    }
    // 完全一致で枠が埋まらないときだけタイプミス補正で補充する
    // (埋まっていれば曖昧検索そのものを実行せず、通常時のコストをゼロにする)。
    // PREDICT には打鍵が届かないので、かなを標準の綴りに戻した打鍵で照合する
    if cfg.typo_correction && results.len() < max && kana.chars().count() >= MIN_FUZZY_CHARS {
        let keys = typo::Keys::standard(kana);
        let matcher = typo::Matcher::new(&keys, typo::TUNING.allowed_edits(keys.len()), &typo::TUNING);
        let mut fuzzy: Vec<(i16, String, String)> = Vec::new();
        for position in dict.fuzzy_prefixes(&matcher, MAX_FUZZY_PREFIXES) {
            if position.edits == 0 {
                continue;
            }
            for (reading, entry) in dict.predict_prefix_entries(&position.prefix, max * 2 + 1) {
                fuzzy.push((entry.cost, reading, entry.surface.clone()));
            }
        }
        fuzzy.sort_by_key(|(cost, _, _)| *cost);
        for (_, reading, surface) in fuzzy {
            if results.len() >= max {
                break;
            }
            push_unique(&mut results, kana, reading, surface);
        }
    }
    results.truncate(max);
    results
}

/// 点数付きの予測の候補 (SUGGEST の並びに入れるもの)
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// 候補の完全な読み (辞書の正しい読み)
    pub reading: String,
    pub surface: String,
    /// 点数 (小さいほど上。docs/spec/suggest.md「点数」)
    pub score: i64,
    /// 照合に使ったずれの数
    pub edits: usize,
}

/// 打鍵のファジィ照合 matcher で予測の候補を集め、点数を付けて返す (SUGGEST の予測)。
/// 並びは短縮よみ → ユーザ辞書の名詞系 → 句の学習 → 同梱辞書とインポート辞書の語 (点数順) で、
/// 呼び出し側が点数で並べ直す。表記が入力かなそのままの候補は除き、同じ表記は点数の小さいほうの
/// 1 件にする。context は前文脈 (直前に確定した文節の表記)、c0 は全体変換の通常の最良経路のコスト。
/// word_score(読み, 表記, left_id, right_id, 単語コスト) は、その語 1 語だけの経路のコスト (学習の
/// 割引を引いたもの) から c0 を引き、PREDICT_BIAS を足した値。句の学習の点数は
/// PHRASE_BASE − 句の学習の割引 − c0 (前文脈があればその文脈の句の学習の割引も引き、前文脈に続いた
/// 文節で句の学習に無いものも候補にする)
/// (docs/spec/suggest.md「点数」)
pub fn scored(
    kana: &str,
    matcher: &typo::Matcher,
    context: Option<&str>,
    dict: &Dictionary,
    user: &UserDict,
    learning: &LearningStore,
    cfg: &Config,
    word_score: &dyn Fn(&str, &str, u16, u16, i16) -> i64,
    c0: i64,
) -> Vec<Prediction> {
    let max = cfg.max_predictions;
    let mut results: Vec<Prediction> = Vec::new();
    // 表記 → results の位置
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut push = |reading: &str, surface: &str, score: i64, edits: usize| {
        if surface == kana {
            return;
        }
        let prediction =
            Prediction { reading: reading.to_string(), surface: surface.to_string(), score, edits };
        match index.get(surface) {
            Some(&i) if score < results[i].score => results[i] = prediction,
            Some(_) => {}
            None => {
                index.insert(surface.to_string(), results.len());
                results.push(prediction);
            }
        }
    };
    // 短縮よみの点数は仮の値 (通常の最良経路のすぐ上)
    for (reading, surface, cost, edits) in user.shortcut_fuzzy(matcher, max) {
        push(reading, surface, -1 + i64::from(cost), edits);
    }
    for (word, cost, edits) in user.word_fuzzy(matcher, max) {
        let score = word_score(&word.reading, &word.surface, word.left_id, word.right_id, word.cost)
            + i64::from(cost);
        push(&word.reading, &word.surface, score, edits);
    }
    let base = learning.phrase_base() - c0;
    let mut phrases: Vec<(i64, &str, &str, usize)> = learning
        .phrase_fuzzy(matcher)
        .into_iter()
        .map(|m| {
            let ctx_bonus = context.map_or(0, |c| learning.ctx_phrase_bonus(c, m.reading, m.surface));
            (base - m.bonus - ctx_bonus + i64::from(m.cost), m.reading, m.surface, m.edits)
        })
        .collect();
    // 前文脈に続いた文節は、句の学習が古くなって消えていても候補にする
    if let Some(c) = context {
        for m in learning.ctx_phrase_fuzzy(c, matcher) {
            if !learning.has_phrase(m.reading, m.surface) {
                phrases.push((base - m.bonus + i64::from(m.cost), m.reading, m.surface, m.edits));
            }
        }
    }
    // 学習は数千件あり短い打鍵では多く当たるので、点数の小さい max 件だけを入れる
    phrases.sort_by_key(|&(score, _, _, _)| score);
    phrases.truncate(max);
    for (score, reading, surface, edits) in phrases {
        push(reading, surface, score, edits);
    }
    // 同じ語は複数の位置から引けるので、点数の安いほうを残す
    let mut hits: Vec<(i64, String, String, usize)> = Vec::new();
    let positions = dict.fuzzy_prefixes(matcher, MAX_FUZZY_PREFIXES);
    for position in &positions {
        for (reading, entry) in dict.predict_prefix_entries(&position.prefix, max * 2 + 1) {
            let score = word_score(&reading, &entry.surface, entry.left_id, entry.right_id, entry.cost)
                + i64::from(position.cost);
            hits.push((score, reading, entry.surface.clone(), position.edits));
        }
    }
    for position in user.imported_fuzzy_prefixes(matcher, MAX_FUZZY_PREFIXES) {
        for (reading, entry) in user.imported_prefix_entries(&position.prefix, max * 2 + 1) {
            let score = word_score(&reading, &entry.surface, entry.left_id, entry.right_id, entry.cost)
                + i64::from(position.cost);
            hits.push((score, reading, entry.surface.clone(), position.edits));
        }
    }
    // 安定ソートなので同じ点数は同梱辞書が先
    hits.sort_by_key(|(score, _, _, _)| *score);
    let mut seen: Vec<(String, String)> = Vec::new();
    for (score, reading, surface, edits) in hits {
        let key = (reading, surface);
        if seen.contains(&key) {
            continue;
        }
        push(&key.0, &key.1, score, edits);
        seen.push(key);
    }
    results
}

/// 後続語の統計の表 (next_word.tsv。docs/spec/suggest.md「表」)。
/// 行は文脈ごとに連続し回数の多い順に並んでいるので、ファイルの本文をそのまま持ち、文脈から行の範囲を引く
/// (行ごとに文字列を持つと、表の数倍のメモリを使うため)
pub struct NextWordTable {
    text: String,
    /// 文節の表 (c): 前の文節の表記 (末尾 16 文字) → text の行の範囲
    bunsetsu: HashMap<Box<str>, Range<usize>>,
    /// バックオフの表 (b): 前の文節の最後の自立語の表記 → text の行の範囲
    backoff: HashMap<Box<str>, Range<usize>>,
    rows: usize,
}

impl NextWordTable {
    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::from_text(std::fs::read_to_string(path)?))
    }

    /// 表の本文から作る。形式の合わない行は読み飛ばす。同じ文脈が離れた位置に再び現れたときは
    /// 最初のまとまりだけを使う
    pub fn from_text(text: String) -> Self {
        let mut bunsetsu: HashMap<Box<str>, Range<usize>> = HashMap::new();
        let mut backoff: HashMap<Box<str>, Range<usize>> = HashMap::new();
        let mut rows = 0;
        // 読んでいるまとまりの (表の区別, 文脈, 範囲)
        let mut current: Option<(&str, &str, Range<usize>)> = None;
        let mut close = |block: Option<(&str, &str, Range<usize>)>| {
            if let Some((tag, context, range)) = block {
                let map = if tag == "c" { &mut bunsetsu } else { &mut backoff };
                map.entry(context.into()).or_insert(range);
            }
        };
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let range = start..start + line.len();
            start = range.end;
            let Some((tag, context, _, _)) = parse_next_word_line(line) else {
                continue;
            };
            rows += 1;
            match &mut current {
                Some((t, c, r)) if *t == tag && *c == context => r.end = range.end,
                _ => close(current.replace((tag, context, range))),
            }
        }
        close(current);
        NextWordTable { text, bunsetsu, backoff, rows }
    }

    /// 読み込んだ件数
    pub fn len(&self) -> usize {
        self.rows
    }

    /// 文脈 context の次の文節 (読み, 表記) を回数の多い順に返す
    fn lookup<'a>(&'a self, map: &HashMap<Box<str>, Range<usize>>, context: &str) -> Vec<(&'a str, &'a str)> {
        let Some(range) = map.get(context) else {
            return Vec::new();
        };
        self.text[range.clone()]
            .split_inclusive('\n')
            .filter_map(parse_next_word_line)
            .map(|(_, _, reading, surface)| (reading, surface))
            .collect()
    }
}

/// next_word.tsv の 1 行 (表の区別\t文脈\t読み\t表記\t回数) を (表の区別, 文脈, 読み, 表記) にする
fn parse_next_word_line(line: &str) -> Option<(&str, &str, &str, &str)> {
    let mut fields = line.trim_end_matches(['\r', '\n']).split('\t');
    let tag = fields.next().filter(|t| matches!(*t, "c" | "b"))?;
    let context = fields.next().filter(|s| !s.is_empty())?;
    let reading = fields.next().filter(|s| !s.is_empty())?;
    let surface = fields.next().filter(|s| !s.is_empty())?;
    fields.next()?.parse::<u32>().ok()?;
    Some((tag, context, reading, surface))
}

/// 前文脈 context (直前に確定した文節の表記) の後に続く文節 (読み, 表記) を返す (読みなしの後続語予測)。
/// 文脈の句の学習 (割引の大きい順) → 統計の文節の表を context で引いた件 → バックオフの表を head
/// (前文脈の最後の自立語の表記) で引いた件の順に並べ、表記の重複は先勝ちで除く。table が無い
/// (読み込み前・ファイルなし) ときは学習だけ、head が無いときはバックオフの表を引かない
pub fn next_words(
    context: &str,
    head: Option<&str>,
    learning: &LearningStore,
    table: Option<&NextWordTable>,
    cfg: &Config,
) -> Vec<(String, String)> {
    let max = cfg.max_predictions;
    let mut results: Vec<(String, String)> = Vec::new();
    for (reading, surface) in learning.context_prefix(context, "", max) {
        push_unique(&mut results, "", reading.to_string(), surface.to_string());
    }
    if let Some(table) = table {
        let mut stats = table.lookup(&table.bunsetsu, &learn::context_key(context));
        if let Some(head) = head {
            stats.extend(table.lookup(&table.backoff, head));
        }
        for (reading, surface) in stats {
            if results.len() >= max {
                break;
            }
            push_unique(&mut results, "", reading.to_string(), surface.to_string());
        }
    }
    results.truncate(max);
    results
}

/// 表記が入力かなそのままの候補と、既出の表記 (履歴優先の先勝ち) を除いて追加する
fn push_unique(results: &mut Vec<(String, String)>, kana: &str, reading: String, surface: String) {
    if surface != kana && !results.iter().any(|(_, s)| *s == surface) {
        results.push((reading, surface));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pos::FunctionalIds;

    /// 既定設定の省略用
    fn cfg() -> Config {
        Config::default()
    }

    /// ユーザ辞書なし (空) の省略用
    fn no_user() -> UserDict {
        UserDict::empty()
    }

    /// 1 語の文節 (読み, 表記) を文脈なしで確定したときと同じに数える
    fn record(learning: &mut LearningStore, reading: &str, surface: &str) {
        record_ctx(learning, "", reading, surface);
    }

    /// 1 語の文節 (読み, 表記) を直前の文節の表記 context の後に確定したときと同じに数える
    fn record_ctx(learning: &mut LearningStore, context: &str, reading: &str, surface: &str) {
        let words = [(reading.to_string(), surface.to_string())];
        learning.learn_segment(reading, surface, context, Some(&words));
    }

    /// TSV 文字列からユーザ辞書を作る
    fn user_from(data: &str) -> UserDict {
        let mut user = UserDict::empty();
        user.load_from(data.as_bytes(), &FunctionalIds::empty());
        user
    }

    fn sample_dict() -> Dictionary {
        let mut dict = Dictionary::empty();
        dict.load_from(
            "きょう\t100\t100\t3000\t今日\n\
             きょうと\t100\t100\t2000\t京都\n\
             きょうかい\t100\t100\t4000\t教会\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 履歴が辞書より先に出る() {
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "きょうしつ", "教室");
        assert_eq!(
            predict("きょう", None, &sample_dict(), &no_user(), &learning, &cfg()),
            vec![
                ("きょうしつ".to_string(), "教室".to_string()),
                ("きょうと".to_string(), "京都".to_string()),
                ("きょう".to_string(), "今日".to_string()),
                ("きょうかい".to_string(), "教会".to_string()),
            ]
        );
    }

    #[test]
    fn 履歴と同じ表記の辞書候補は重複しない() {
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "きょうと", "京都");
        let results = predict("きょう", None, &sample_dict(), &no_user(), &learning, &cfg());
        assert_eq!(results.iter().filter(|(_, s)| s == "京都").count(), 1);
        assert_eq!(results[0], ("きょうと".to_string(), "京都".to_string()));
    }

    #[test]
    fn 入力かなと同じ表記は除外される() {
        let mut dict = Dictionary::empty();
        dict.load_from("きょう\t100\t100\t1000\tきょう\nきょう\t100\t100\t2000\t今日\n".as_bytes())
            .unwrap();
        dict.finalize();
        assert_eq!(
            predict("きょう", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg()),
            vec![("きょう".to_string(), "今日".to_string())]
        );
    }

    #[test]
    fn 英字読みの履歴も前方一致で出る() {
        // 英字モードの入力 (英単語) も、確定履歴があればサジェストの対象になる。
        // 辞書の読みはかなのみなので、英字入力では履歴だけがヒットする
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "apple", "Apple");
        assert_eq!(
            predict("ap", None, &sample_dict(), &no_user(), &learning, &cfg()),
            vec![("apple".to_string(), "Apple".to_string())]
        );
    }

    #[test]
    fn 二文字未満は候補を出さない() {
        assert!(predict("き", None, &sample_dict(), &no_user(), &LearningStore::in_memory(), &cfg()).is_empty());
        assert!(predict("", None, &sample_dict(), &no_user(), &LearningStore::in_memory(), &cfg()).is_empty());
    }

    #[test]
    fn 短縮よみが履歴より先に出る() {
        let mut dict = Dictionary::empty();
        dict.load_from("めーるあどれす\t100\t100\t3000\tメールアドレス\n".as_bytes()).unwrap();
        dict.finalize();
        let user = user_from("めーる\tmail@example.com\t短縮よみ\n");
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "めーるべん", "メール便");
        assert_eq!(
            predict("めーる", None, &dict, &user, &learning, &cfg()),
            vec![
                ("めーる".to_string(), "mail@example.com".to_string()),
                ("めーるべん".to_string(), "メール便".to_string()),
                ("めーるあどれす".to_string(), "メールアドレス".to_string()),
            ]
        );
    }

    #[test]
    fn ユーザ登録の名詞が短縮よみの後で履歴より先に出る() {
        let user = user_from("かんべ\tmail@example.com\t短縮よみ\nかんべ\t神戸\t姓\n");
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "かんべい", "寛平");
        assert_eq!(
            predict("かんべ", None, &sample_dict(), &user, &learning, &cfg()),
            vec![
                ("かんべ".to_string(), "mail@example.com".to_string()),
                ("かんべ".to_string(), "神戸".to_string()),
                ("かんべい".to_string(), "寛平".to_string()),
            ]
        );
    }

    #[test]
    fn インポート辞書の名詞は手動登録と履歴の後に出る() {
        let mut user = user_from("かんべ\t神戸\t姓\n");
        user.load_imported_from(
            ["かんべ\t神戸\t地名\nかんべい\t官兵衛\t人名\n".as_bytes()],
            &FunctionalIds::empty(),
        );
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "かんべつ", "鑑別");
        // 手動登録と同じ表記のインポート語 (神戸) は重複して出ない
        assert_eq!(
            predict("かんべ", None, &sample_dict(), &user, &learning, &cfg()),
            vec![
                ("かんべ".to_string(), "神戸".to_string()),
                ("かんべつ".to_string(), "鑑別".to_string()),
                ("かんべい".to_string(), "官兵衛".to_string()),
            ]
        );
    }

    #[test]
    fn インポート辞書の名詞は同梱辞書とコスト順に混ざる() {
        // インポート語のコストは 5000。同コストの同梱辞書の語 (恐竜) が先
        let mut dict = Dictionary::empty();
        dict.load_from(
            "きょうと\t100\t100\t2000\t京都\n\
             きょうりゅう\t100\t100\t5000\t恐竜\n\
             きょうかい\t100\t100\t6000\t教会\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let mut user = UserDict::empty();
        user.load_imported_from(["きょうしつ\t教室\t名詞\n".as_bytes()], &FunctionalIds::empty());
        assert_eq!(
            predict("きょう", None, &dict, &user, &LearningStore::in_memory(), &cfg()),
            vec![
                ("きょうと".to_string(), "京都".to_string()),
                ("きょうりゅう".to_string(), "恐竜".to_string()),
                ("きょうしつ".to_string(), "教室".to_string()),
                ("きょうかい".to_string(), "教会".to_string()),
            ]
        );
    }

    #[test]
    fn 短縮よみと同じ表記の履歴は重複しない() {
        let user = user_from("めーる\tmail@example.com\t短縮よみ\n");
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "めーる", "mail@example.com");
        assert_eq!(
            predict("めーる", None, &Dictionary::empty(), &user, &learning, &cfg()),
            vec![("めーる".to_string(), "mail@example.com".to_string())]
        );
    }

    #[test]
    fn タイプミスでも補正候補が出る() {
        // 「にほんご」を「にひんご」と打ち間違えても候補が出る
        let mut dict = Dictionary::empty();
        dict.load_from("にほんご\t100\t100\t3000\t日本語\n".as_bytes()).unwrap();
        dict.finalize();
        assert_eq!(
            predict("にひんご", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg()),
            vec![("にほんご".to_string(), "日本語".to_string())]
        );
    }

    #[test]
    fn 三文字未満はタイプミス補正しない() {
        let mut dict = Dictionary::empty();
        dict.load_from("にほ\t100\t100\t3000\t二歩\n".as_bytes()).unwrap();
        dict.finalize();
        // 「にお」は「にほ」の1かな違いだが、2文字なので補正は発動しない
        assert!(predict("にお", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg()).is_empty());
    }

    #[test]
    fn 完全一致が埋まっていればタイプミス補正しない() {
        let mut dict = Dictionary::empty();
        for i in 0..cfg().max_predictions {
            dict.load_from(
                format!("きょうの{i}\t100\t100\t{}\t表記{i}\n", 1000 + i).as_bytes(),
            )
            .unwrap();
        }
        // きょうと は「きょうの」の1かな違い (低コスト) だが、完全一致の上限件数で枠が埋まる
        dict.load_from("きょうと\t100\t100\t100\t京都\n".as_bytes()).unwrap();
        dict.finalize();
        let results = predict("きょうの", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg());
        assert_eq!(results.len(), cfg().max_predictions);
        assert!(results.iter().all(|(_, s)| s != "京都"));
    }

    #[test]
    fn 補正候補は完全一致の後に出る() {
        let mut dict = Dictionary::empty();
        dict.load_from(
            "きょうの\t100\t100\t5000\t今日の\nきょうも\t100\t100\t100\t今日も\n".as_bytes(),
        )
        .unwrap();
        dict.finalize();
        // 補正の「今日も」(mo の m を隣の n と打った) の方が低コストでも、完全一致の「今日の」が先
        assert_eq!(
            predict("きょうの", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg()),
            vec![
                ("きょうの".to_string(), "今日の".to_string()),
                ("きょうも".to_string(), "今日も".to_string()),
            ]
        );
    }

    #[test]
    fn 文脈の句の学習が短縮よみの直後で確定履歴より前に出る() {
        let user = user_from("かんべ\tmail@example.com\t短縮よみ\nかんべ\t神戸\t姓\n");
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "かんべい", "寛平");
        record_ctx(&mut learning, "今日は", "かんべつ", "鑑別");
        assert_eq!(
            predict("かんべ", Some("今日は"), &sample_dict(), &user, &learning, &cfg()),
            vec![
                ("かんべ".to_string(), "mail@example.com".to_string()),
                ("かんべつ".to_string(), "鑑別".to_string()),
                ("かんべ".to_string(), "神戸".to_string()),
                ("かんべい".to_string(), "寛平".to_string()),
            ]
        );
        // 前文脈が違えば文脈付きの学習は出ない
        assert_eq!(
            predict("かんべ", Some("明日は"), &sample_dict(), &user, &learning, &cfg()),
            predict("かんべ", None, &sample_dict(), &user, &learning, &cfg())
        );
    }

    #[test]
    fn 前文脈の句の学習は前文脈が一致するときだけ前に出る() {
        // 京都 は文脈なしで 2 回、教会 は文脈「今日は」で 1 回確定した
        let mut learning = LearningStore::in_memory();
        record(&mut learning, "きょうと", "京都");
        record(&mut learning, "きょうと", "京都");
        record_ctx(&mut learning, "今日は", "きょうかい", "教会");
        let got = predict("きょう", None, &sample_dict(), &no_user(), &learning, &cfg());
        assert_eq!(got[0], ("きょうと".to_string(), "京都".to_string()));
        let got = predict("きょう", Some("今日は"), &sample_dict(), &no_user(), &learning, &cfg());
        assert_eq!(got[0], ("きょうかい".to_string(), "教会".to_string()));
        let got = predict("きょう", Some("明日は"), &sample_dict(), &no_user(), &learning, &cfg());
        assert_eq!(got[0], ("きょうと".to_string(), "京都".to_string()));
    }

    #[test]
    fn 予測の前文脈の句の学習は句の学習に無い文節も候補にする() {
        // 古い learning_context.tsv の取り込みは文脈の句の学習だけを数える (句の学習は無い)
        let mut learning = LearningStore::in_memory();
        learning.import_context("今日は\tきょうかいで\t教会で\n".as_bytes());
        assert!(!learning.has_phrase("きょうかいで", "教会で"));
        let keys = typo::Keys::parse("きょう", "kyo\x1f\x1fu").unwrap();
        let matcher = typo::Matcher::new(&keys, 0, &typo::TUNING);
        let word_score = |_: &str, _: &str, _: u16, _: u16, cost: i16| i64::from(cost);
        let run = |context: Option<&str>| {
            scored("きょう", &matcher, context, &sample_dict(), &no_user(), &learning, &cfg(), &word_score, 0)
        };
        // PHRASE_BASE − 句の学習の割引 0 − 文脈の句の学習の割引 2500 − c0 0
        let expected = crate::learn::WEIGHTS.phrase_base - 2500;
        assert!(run(Some("今日は")).iter().any(|p| p.surface == "教会で" && p.score == expected));
        assert!(run(None).iter().all(|p| p.surface != "教会で"));
        assert!(run(Some("明日は")).iter().all(|p| p.surface != "教会で"));
    }

    #[test]
    fn 後続語予測は文脈の句の学習を割引の大きい順に返す() {
        let mut learning = LearningStore::in_memory();
        record_ctx(&mut learning, "今日は", "いいてんき", "いい天気");
        record_ctx(&mut learning, "今日は", "はれ", "晴れ");
        record(&mut learning, "あめ", "雨");
        // 同じ回数なら今の度合いの大きい (新しい) ほうが先
        assert_eq!(
            next_words("今日は", None, &learning, None, &cfg()),
            vec![
                ("はれ".to_string(), "晴れ".to_string()),
                ("いいてんき".to_string(), "いい天気".to_string()),
            ]
        );
        // 多く数えたほうが先 (最後に確定した順ではない)
        record_ctx(&mut learning, "今日は", "いいてんき", "いい天気");
        record_ctx(&mut learning, "今日は", "はれ", "晴れ");
        record_ctx(&mut learning, "今日は", "いいてんき", "いい天気");
        assert_eq!(next_words("今日は", None, &learning, None, &cfg())[0].1, "いい天気");
        let mut one = cfg();
        one.max_predictions = 1;
        assert_eq!(next_words("今日は", None, &learning, None, &one).len(), 1);
        assert!(next_words("明日は", None, &learning, None, &cfg()).is_empty());
    }

    fn sample_table() -> NextWordTable {
        NextWordTable::from_text(
            "c\t今日は\tいいてんき\tいい天気\t30\n\
             c\t今日は\tはれ\t晴れ\t20\n\
             c\t明日は\tあめ\t雨\t40\n\
             b\t今日\tくもり\t曇り\t50\n\
             b\t今日\tいいてんき\tいい天気\t45\n\
             b\t今日\tさむい\t寒い\t15\n"
                .to_string(),
        )
    }

    fn surfaces(list: &[(String, String)]) -> Vec<&str> {
        list.iter().map(|(_, s)| s.as_str()).collect()
    }

    #[test]
    fn 後続語の統計の表を文脈ごとに読む() {
        let table = sample_table();
        assert_eq!(table.len(), 6);
        assert_eq!(table.lookup(&table.bunsetsu, "今日は"), [("いいてんき", "いい天気"), ("はれ", "晴れ")]);
        assert_eq!(table.lookup(&table.bunsetsu, "明日は"), [("あめ", "雨")]);
        assert_eq!(table.lookup(&table.backoff, "今日").len(), 3);
        assert!(table.lookup(&table.backoff, "今日は").is_empty());
        // 形式の合わない行は読み飛ばし、CRLF の行も読める
        let table = NextWordTable::from_text("x\t今日\ta\tA\t1\nc\t今日\tb\n\nc\t今日\tはれ\t晴れ\t3\r\n".to_string());
        assert_eq!(table.len(), 1);
        assert_eq!(table.lookup(&table.bunsetsu, "今日"), [("はれ", "晴れ")]);
    }

    #[test]
    fn 後続語予測は学習_文節の表_バックオフの表の順に並び表記の重複を除く() {
        let mut learning = LearningStore::in_memory();
        record_ctx(&mut learning, "今日は", "はれ", "晴れ");
        let table = sample_table();
        let got = next_words("今日は", Some("今日"), &learning, Some(&table), &cfg());
        assert_eq!(surfaces(&got), ["晴れ", "いい天気", "曇り", "寒い"]);
        // max_predictions で切れる
        let mut three = cfg();
        three.max_predictions = 3;
        let got = next_words("今日は", Some("今日"), &learning, Some(&table), &three);
        assert_eq!(surfaces(&got), ["晴れ", "いい天気", "曇り"]);
    }

    #[test]
    fn 後続語予測の文節の表は前文脈の末尾16文字で引く() {
        let table = NextWordTable::from_text("c\tおかきくけこさしすせそたちつてと\tはれ\t晴れ\t3\n".to_string());
        let learning = LearningStore::in_memory();
        let got = next_words("あいうえおかきくけこさしすせそたちつてと", None, &learning, Some(&table), &cfg());
        assert_eq!(surfaces(&got), ["晴れ"]);
    }

    #[test]
    fn 後続語予測は表が無ければ学習だけで前文脈の自立語が無ければバックオフの表を引かない() {
        let mut learning = LearningStore::in_memory();
        record_ctx(&mut learning, "今日も", "はれ", "晴れ");
        let table = sample_table();
        assert_eq!(
            next_words("今日も", Some("今日"), &learning, None, &cfg()),
            next_words("今日も", None, &learning, None, &cfg())
        );
        assert_eq!(surfaces(&next_words("今日も", None, &learning, Some(&table), &cfg())), ["晴れ"]);
        assert_eq!(
            surfaces(&next_words("今日も", Some("今日"), &learning, Some(&table), &cfg())),
            ["晴れ", "曇り", "いい天気", "寒い"]
        );
    }

    /// scored の (読み, 表記, 点数)。辞書の語の点数は 単語コスト − 語の学習の割引 − c0 にする
    fn scored_of(kana: &str, field: &str, context: Option<&str>, learning: &LearningStore, c0: i64) -> Vec<(String, String, i64)> {
        let keys = typo::Keys::parse(kana, field).unwrap();
        let matcher = typo::Matcher::new(&keys, 0, &typo::TUNING);
        let word_score = |reading: &str, surface: &str, _: u16, _: u16, cost: i16| {
            i64::from(cost) - learning.word_bonus(reading, surface) - c0
        };
        let mut got: Vec<(String, String, i64)> =
            scored(kana, &matcher, context, &sample_dict(), &no_user(), learning, &cfg(), &word_score, c0)
                .into_iter()
                .map(|p| (p.reading, p.surface, p.score))
                .collect();
        got.sort_by_key(|(_, _, score)| *score);
        got
    }

    #[test]
    fn 予測の句の学習の点数は基準から割引とc0を引いたもの() {
        let mut learning = LearningStore::in_memory();
        // 教室 は辞書に無い (句の学習だけの予測)
        let words = [("きょうしつ".to_string(), "教室".to_string())];
        learning.learn_segment("きょうしつ", "教室", "", Some(&words));
        let got = scored_of("きょう", "kyo\x1f\x1fu", None, &learning, 1000);
        let phrase = crate::learn::WEIGHTS.phrase_base - 1500 - 1000;
        assert!(got.contains(&("きょうしつ".to_string(), "教室".to_string(), phrase)), "{got:?}");
        // 前文脈があれば、その文脈の句の学習の割引も引く
        learning.learn_segment("きょうしつ", "教室", "今日は", Some(&words));
        let bonus = learning.phrase_bonus("きょうしつ", "教室");
        let context = learning.ctx_phrase_bonus("今日は", "きょうしつ", "教室");
        let got = scored_of("きょう", "kyo\x1f\x1fu", Some("今日は"), &learning, 1000);
        let expected = crate::learn::WEIGHTS.phrase_base - bonus - 1000 - context;
        assert!(got.contains(&("きょうしつ".to_string(), "教室".to_string(), expected)), "{got:?}");
    }

    #[test]
    fn 同じ表記の辞書の語と句の学習は点数の小さいほうの1件になる() {
        let mut learning = LearningStore::in_memory();
        let words = [("きょうと".to_string(), "京都".to_string())];
        learning.learn_segment("きょうと", "京都", "", Some(&words));
        // 辞書の 京都: 2000 − 1000 − 0 = 1000、句の学習: 6000 − 1500 − 0 = 4500
        let got = scored_of("きょう", "kyo\x1f\x1fu", None, &learning, 0);
        let kyoto: Vec<_> = got.iter().filter(|(_, s, _)| s == "京都").collect();
        assert_eq!(kyoto, vec![&("きょうと".to_string(), "京都".to_string(), 1000)]);
        // 辞書に無い表記の句の学習はそのまま残る
        let mut learning = LearningStore::in_memory();
        learning.learn_segment("きょうと", "京都府", "", None);
        let got = scored_of("きょう", "kyo\x1f\x1fu", None, &learning, 0);
        assert!(got.contains(&("きょうと".to_string(), "京都府".to_string(), 4500)), "{got:?}");
        assert!(got.contains(&("きょうと".to_string(), "京都".to_string(), 2000)), "{got:?}");
    }

    #[test]
    fn 候補数は上限で打ち切る() {
        let mut dict = Dictionary::empty();
        for i in 0..20 {
            dict.load_from(format!("きょう{i:02}\t100\t100\t{}\t表記{i:02}\n", 1000 + i).as_bytes())
                .unwrap();
        }
        dict.finalize();
        let results = predict("きょう", None, &dict, &no_user(), &LearningStore::in_memory(), &cfg());
        assert_eq!(results.len(), cfg().max_predictions);
    }
}
