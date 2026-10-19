// 予測入力 (PREDICT・SUGGEST の予測) と読みなしの後続語予測 (NEXTWORD) の候補生成
//
// PREDICT は読みの前方一致でユーザ辞書の短縮よみ (記載順) → 前文脈付きの学習 (前文脈があるとき、
// 新しい順) → ユーザ辞書の名詞系 (記載順) → 履歴 (確定履歴、新しい順)
// → 同梱辞書とインポート辞書の名詞系 (両者を混ぜてコスト順。同コストは同梱辞書が先)
// を検索し、この順に合成して返す。表記の重複は先勝ちで除き、表記が入力かなそのままの候補は出しても意味が無いため除外する。
// 完全一致で枠が埋まらないときは、タイプミス補正 (かなを標準の綴りに戻した打鍵のファジィ照合) で補充する。
// SUGGEST の予測 (scored) は同じ対象を打鍵のファジィ照合で常に引き、候補に点数を付ける
// (docs/design/unified-suggest.md の C・E)。
// NEXTWORD は前文脈付きの学習だけを前文脈で引く (辞書には語の連接の統計が無いため)。
// 候補の最大数・最小読み文字数・補正の有無は設定 (config.rs) に従う。

use crate::config::Config;
use crate::dict::Dictionary;
use crate::learn::LearningStore;
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
        for (reading, surface) in learning.predict_context(context, kana, max) {
            push_unique(&mut results, kana, reading.to_string(), surface.to_string());
        }
    }
    for (reading, surface) in user.word_prefix(kana, max) {
        push_unique(&mut results, kana, reading.to_string(), surface.to_string());
    }
    for (reading, surface) in learning.predict_prefix(kana, max) {
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
    /// 点数 (小さいほど上。docs/design/unified-suggest.md の E)
    pub score: i64,
    /// 照合に使ったずれの数
    pub edits: usize,
}

/// 打鍵のファジィ照合 matcher で予測の候補を集め、点数を付けて返す (SUGGEST の予測)。
/// 並びは短縮よみ → 前文脈付きの学習 → ユーザ辞書の名詞系 → 確定履歴 → 同梱辞書とインポート辞書の語
/// (点数順) で、呼び出し側が点数で並べ直す。表記が入力かなそのままの候補は除く。
/// word_score(left_id, right_id, 単語コスト) は、その語 1 語だけの経路のコストから全体変換の
/// 通常の最良経路のコストを引き、PREDICT_BIAS を足した値
pub fn scored(
    kana: &str,
    matcher: &typo::Matcher,
    context: Option<&str>,
    dict: &Dictionary,
    user: &UserDict,
    learning: &LearningStore,
    cfg: &Config,
    word_score: &dyn Fn(u16, u16, i16) -> i64,
) -> Vec<Prediction> {
    let max = cfg.max_predictions;
    let mut results: Vec<Prediction> = Vec::new();
    let mut push = |reading: &str, surface: &str, score: i64, edits: usize| {
        if surface != kana {
            results.push(Prediction {
                reading: reading.to_string(),
                surface: surface.to_string(),
                score,
                edits,
            });
        }
    };
    // 学習と短縮よみの点数は仮の値 (学習の点数付けは別の設計で決める)
    for (reading, surface, cost, edits) in user.shortcut_fuzzy(matcher, max) {
        push(reading, surface, -1 + i64::from(cost), edits);
    }
    if let Some(context) = context {
        for (i, (reading, surface, cost, edits)) in
            learning.fuzzy_context(context, matcher, max).into_iter().enumerate()
        {
            push(reading, surface, 100 * i as i64 + i64::from(cost), edits);
        }
    }
    for (word, cost, edits) in user.word_fuzzy(matcher, max) {
        let score = word_score(word.left_id, word.right_id, word.cost) + i64::from(cost);
        push(&word.reading, &word.surface, score, edits);
    }
    for (i, (reading, surface, cost, edits)) in
        learning.fuzzy_prefix(matcher, max).into_iter().enumerate()
    {
        push(reading, surface, 500 + 100 * i as i64 + i64::from(cost), edits);
    }
    // 同じ語は複数の位置から引けるので、点数の安いほうを残す
    let mut hits: Vec<(i64, String, String, usize)> = Vec::new();
    let positions = dict.fuzzy_prefixes(matcher, MAX_FUZZY_PREFIXES);
    for position in &positions {
        for (reading, entry) in dict.predict_prefix_entries(&position.prefix, max * 2 + 1) {
            let score = word_score(entry.left_id, entry.right_id, entry.cost) + i64::from(position.cost);
            hits.push((score, reading, entry.surface.clone(), position.edits));
        }
    }
    for position in user.imported_fuzzy_prefixes(matcher, MAX_FUZZY_PREFIXES) {
        for (reading, entry) in user.imported_prefix_entries(&position.prefix, max * 2 + 1) {
            let score = word_score(entry.left_id, entry.right_id, entry.cost) + i64::from(position.cost);
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

/// 前文脈 context (直前に確定した文節の表記) の後に確定したことのある語 (読み, 表記) を、
/// 記録が新しい順に返す (読みなしの後続語予測)
pub fn next_words(context: &str, learning: &LearningStore, cfg: &Config) -> Vec<(String, String)> {
    learning
        .predict_context(context, "", cfg.max_predictions)
        .into_iter()
        .map(|(reading, surface)| (reading.to_string(), surface.to_string()))
        .collect()
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
        learning.record("きょうしつ", "教室");
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
        learning.record("きょうと", "京都");
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
        learning.record("apple", "Apple");
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
        learning.record("めーるべん", "メール便");
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
        learning.record("かんべい", "寛平");
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
        learning.record("かんべつ", "鑑別");
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
        learning.record("めーる", "mail@example.com");
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
    fn 前文脈の学習が短縮よみの直後で確定履歴より前に出る() {
        let user = user_from("かんべ\tmail@example.com\t短縮よみ\nかんべ\t神戸\t姓\n");
        let mut learning = LearningStore::in_memory();
        learning.record("かんべい", "寛平");
        learning.record_ctx("今日は", "かんべつ", "鑑別");
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
    fn 前文脈が無い予測は文脈付きの学習を使わない() {
        let mut learning = LearningStore::in_memory();
        learning.record_ctx("今日は", "きょうしつ", "教室");
        // record_ctx は文脈なしの学習を記録しないので、辞書の候補だけになる
        assert_eq!(
            predict("きょう", None, &sample_dict(), &no_user(), &learning, &cfg()),
            vec![
                ("きょうと".to_string(), "京都".to_string()),
                ("きょう".to_string(), "今日".to_string()),
                ("きょうかい".to_string(), "教会".to_string()),
            ]
        );
    }

    #[test]
    fn 後続語予測は前文脈の学習を新しい順に返す() {
        let mut learning = LearningStore::in_memory();
        learning.record_ctx("今日は", "いいてんき", "いい天気");
        learning.record_ctx("今日は", "はれ", "晴れ");
        learning.record("あめ", "雨");
        assert_eq!(
            next_words("今日は", &learning, &cfg()),
            vec![
                ("はれ".to_string(), "晴れ".to_string()),
                ("いいてんき".to_string(), "いい天気".to_string()),
            ]
        );
        let mut one = cfg();
        one.max_predictions = 1;
        assert_eq!(next_words("今日は", &learning, &one).len(), 1);
        assert!(next_words("明日は", &learning, &cfg()).is_empty());
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
