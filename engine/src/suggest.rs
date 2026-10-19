// SUGGEST: 全体変換と予測を 1 つの点数で並べる (docs/design/unified-suggest.md の E)
//
// 全体変換は CONVNBEST の並び (打鍵による補正を含む) の先頭から、予測は打鍵のファジィ照合の結果から取り、
// 点数 s (小さいほど上) の順に並べる。読み全体の学習表記は先頭に固定し、全体変換の通常の最良経路が
// 保護の対象なら、補正の候補とずれのある予測をその上に置かない。

use crate::config::Config;
use crate::convert::{self, Context, TypoSource};
use crate::dict::Dictionary;
use crate::learn::LearningStore;
use crate::matrix::ConnectionMatrix;
use crate::pos::FunctionalIds;
use crate::predict;
use crate::typo::{self, Keys, Tuning};
use crate::userdict::UserDict;

/// 並びに入れる全体変換の候補の数
const MAX_WHOLES: usize = 3;

/// 全体変換の先頭を入れる位置の上限 (0 始まり。9 番目)
const HEAD_POSITION: usize = 8;

/// 候補の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 全体変換
    Whole,
    /// 全体変換の補正の候補
    Typo,
    /// 予測
    Predict,
    /// ずれを 1 つ以上使った予測
    PredictTypo,
}

impl Kind {
    /// プロトコルでの種類の名前
    pub fn name(self) -> &'static str {
        match self {
            Kind::Whole => "whole",
            Kind::Typo => "typo",
            Kind::Predict => "predict",
            Kind::PredictTypo => "predict_typo",
        }
    }
}

/// 並びの 1 件。全体変換は文節ごとの (読み, 表記)、予測は 1 つの (候補の完全な読み, 表記)
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub kind: Kind,
    pub segments: Vec<(String, String)>,
}

impl Suggestion {
    pub fn surface(&self) -> String {
        self.segments.iter().map(|(_, surface)| surface.as_str()).collect()
    }
}

/// 並べる前の候補
struct Item {
    suggestion: Suggestion,
    score: i64,
    /// 同じ点数のときの順 (全体変換 0、予測 1)
    rank: u8,
}

/// かな kana (打鍵は keys) の SUGGEST の並びを返す。ctx は直前に確定した文節
pub fn suggest(
    kana: &str,
    ctx: Option<&Context>,
    keys: &Keys,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    cfg: &Config,
    tuning: &Tuning,
) -> Vec<Suggestion> {
    let source = if cfg.typo_correction { TypoSource::Keys(keys) } else { TypoSource::Off };
    let nbest = convert::convert_nbest_scored(
        kana, ctx, dict, user, matrix, functional, learning, source, tuning,
    );
    let c0 = nbest.best_cost.unwrap_or(0);

    // 書き換えの候補 (表記をひらがなに直すと読みと同じ候補) は、辞書の読み全体の書き換えの項目が
    // 2 語に分かれる変換より安く出て全体変換の枠を埋めるので、1 件だけを最後に置く。最良経路と読み全体の
    // 学習表記は、表記がかなの種類だけ違っても書き換えとして扱わない
    let is_rewrite = |i: usize, surface: &str| {
        nbest.best != Some(i) && !(nbest.learned && i == 0) && to_hiragana(surface) == kana
    };
    let shown: Vec<usize> =
        (0..nbest.candidates.len()).filter(|&i| nbest.candidates[i].surface() != kana).collect();
    let rewrite =
        shown.iter().take(MAX_WHOLES).copied().find(|&i| is_rewrite(i, &nbest.candidates[i].surface()));
    let mut taken: Vec<usize> = shown
        .iter()
        .copied()
        .filter(|&i| !is_rewrite(i, &nbest.candidates[i].surface()))
        .take(MAX_WHOLES - usize::from(rewrite.is_some()))
        .collect();
    taken.extend(rewrite);

    // 全体変換: CONVNBEST の順を崩さないよう、前の候補の s を下回る値は前の候補の s に切り上げる。
    // 最後に置いた書き換えの候補は前の候補の s
    let mut pinned: Option<Item> = None;
    let mut items: Vec<Item> = Vec::new();
    // 全体変換の先頭 (読み全体の学習表記が無ければ通常の最良経路) の表記
    let head_index = if nbest.learned { Some(0) } else { nbest.best };
    let mut head: Option<String> = None;
    let mut prev: Option<i64> = None;
    for &i in &taken {
        let candidate = &nbest.candidates[i];
        let surface = candidate.surface();
        let score = if rewrite == Some(i) {
            prev.unwrap_or(0)
        } else {
            let score = nbest.costs[i].map_or(prev.unwrap_or(0), |cost| cost - c0);
            prev.map_or(score, |p| score.max(p))
        };
        prev = Some(score);
        if head_index == Some(i) {
            head = Some(surface);
        }
        let kind = if candidate.typo { Kind::Typo } else { Kind::Whole };
        let item = Item {
            suggestion: Suggestion { kind, segments: candidate.segments.clone() },
            score,
            rank: 0,
        };
        if nbest.learned && i == 0 {
            pinned = Some(item);
        } else {
            items.push(item);
        }
    }
    let best_protected = nbest.best.is_some_and(|i| nbest.candidates[i].protected);

    if cfg.suggest && kana.chars().count() >= cfg.min_suggest_chars {
        let edits = if cfg.typo_correction { tuning.allowed_edits(keys.len()) } else { 0 };
        let matcher = typo::Matcher::new(keys, edits, tuning);
        let ctx_id = nbest.context_id;
        let word_score = |left_id: u16, right_id: u16, cost: i16| {
            convert::word_path_cost(ctx_id, left_id, right_id, cost, matrix, functional) - c0
                + tuning.predict_bias
        };
        let context = ctx.map(|c| c.surface.as_str());
        for p in predict::scored(kana, &matcher, context, dict, user, learning, cfg, &word_score) {
            let kind = if p.edits > 0 { Kind::PredictTypo } else { Kind::Predict };
            items.push(Item {
                suggestion: Suggestion { kind, segments: vec![(p.reading, p.surface)] },
                score: p.score,
                rank: 1,
            });
        }
    }

    // 安定ソートなので、同じ点数・同じ種類の中は元の順
    items.sort_by_key(|item| (item.score, item.rank));
    let is_whole = |kind: Kind| matches!(kind, Kind::Whole | Kind::Typo);
    let mut list: Vec<Item> = Vec::new();
    for item in pinned.into_iter().chain(items) {
        let surface = item.suggestion.surface();
        match list.iter().position(|s| s.suggestion.surface() == surface) {
            None => list.push(item),
            // 全体変換は文節を持ち、採用範囲の縮めと全体変換の学習が効くので、表記が同じ予測より
            // 下にあっても全体変換を残す。位置は上の予測のもの
            Some(i) if is_whole(item.suggestion.kind) && !is_whole(list[i].suggestion.kind) => {
                list[i].suggestion = item.suggestion;
            }
            Some(_) => {}
        }
    }

    // 最良経路の表記が読みのかなと同じで並びに無いときも、ずれの無い s <= 0 の候補を基準にして効かせる
    if best_protected {
        let is_typo = |item: &Item| matches!(item.suggestion.kind, Kind::Typo | Kind::PredictTypo);
        if let Some(last) = list.iter().rposition(|item| !is_typo(item) && item.score <= 0) {
            let mut moved: Vec<Item> = Vec::new();
            let mut kept: Vec<Item> = Vec::new();
            for item in list.drain(..=last) {
                if is_typo(&item) {
                    moved.push(item);
                } else {
                    kept.push(item);
                }
            }
            kept.extend(moved);
            kept.append(&mut list);
            list = kept;
        }
    }
    let mut list: Vec<Suggestion> = list.into_iter().map(|item| item.suggestion).collect();

    if let Some(h) = head.as_ref().and_then(|head| list.iter().position(|s| s.surface() == *head)) {
        if h > HEAD_POSITION {
            let item = list.remove(h);
            list.insert(HEAD_POSITION, item);
        }
    }
    list.truncate(cfg.max_predictions);
    list
}

/// カタカナをひらがなにする (convert::to_katakana の逆。対象外の文字はそのまま)
pub(crate) fn to_hiragana(text: &str) -> String {
    text.chars()
        .map(|c| {
            if ('ァ'..='ヶ').contains(&c) {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pos::FunctionalIds;

    fn functional() -> FunctionalIds {
        FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n3 助動詞\n".as_bytes()).unwrap()
    }

    fn dict_from(data: &str) -> Dictionary {
        let mut dict = Dictionary::empty();
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    fn sample_dict() -> Dictionary {
        dict_from(
            "よろしく\t1\t1\t3000\tよろしく\n\
             おねがい\t1\t1\t3000\tお願い\n\
             ねぎ\t1\t1\t4000\tネギ\n\
             します\t3\t3\t1000\tします\n\
             ありがとう\t1\t1\t3000\t有難う\n\
             ありがたい\t1\t1\t3500\t有難い\n\
             きょう\t1\t1\t2000\t今日\n\
             きょうと\t1\t1\t2500\t京都\n\
             は\t2\t2\t500\tは\n",
        )
    }

    /// 打鍵のフィールド (要素を \x1f、未完成のローマ字を \x1e で区切る) で SUGGEST の並びを作る
    fn run(
        kana: &str,
        field: &str,
        dict: &Dictionary,
        user: &UserDict,
        learning: &LearningStore,
        cfg: &Config,
        tuning: &Tuning,
    ) -> Vec<Suggestion> {
        let keys = Keys::parse(kana, field).unwrap();
        suggest(
            kana, None, &keys, dict, user, &ConnectionMatrix::empty(), &functional(), learning,
            cfg, tuning,
        )
    }

    fn surfaces(list: &[Suggestion]) -> Vec<String> {
        list.iter().map(Suggestion::surface).collect()
    }

    #[test]
    fn 未完成のローマ字まで照合した予測がずれ0で出る() {
        let got = run(
            "ありが", "a\x1fri\x1fga\x1et", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &Config::default(), &typo::TUNING,
        );
        let item = got.iter().find(|s| s.surface() == "有難う").expect("有難う が無い");
        assert_eq!(item.kind, Kind::Predict);
        // 予測の読みは辞書の正しい読み
        assert_eq!(item.segments, vec![("ありがとう".to_string(), "有難う".to_string())]);
    }

    #[test]
    fn ずれのある予測は種類が予測の補正になる() {
        // arigtou: ありがとう の a 抜け
        let got = run(
            "ありgとう", "a\x1fri\x1fg\x1fto\x1fu", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &Config::default(), &typo::TUNING,
        );
        let item = got.iter().find(|s| s.surface() == "有難う").expect("有難う が無い");
        assert_eq!(item.kind, Kind::PredictTypo);
    }

    #[test]
    fn 読み全体の学習表記は先頭に固定する() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょう", "京");
        let mut tuning = typo::TUNING;
        // 予測をどれも全体変換より上にしても、学習表記は先頭
        tuning.predict_bias = -100_000;
        let got = run(
            "きょう", "kyo\x1f\x1fu", &sample_dict(), &UserDict::empty(), &learning,
            &Config::default(), &tuning,
        );
        assert_eq!(got[0].surface(), "京");
        assert_eq!(got[0].kind, Kind::Whole);
        assert!(got[1..].iter().any(|s| s.kind == Kind::Predict), "{:?}", surfaces(&got));
    }

    #[test]
    fn 同じ点数なら全体変換が予測より先() {
        // 全体変換の キョウ は経路のコストを持たないので前の 今日 と同じ s 0。予測の 京都 は
        // 1 語の経路のコスト 3500 − C0 3000 で、PREDICT_BIAS -500 なら s 0
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -500;
        let got = run(
            "きょう", "kyo\x1f\x1fu", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &Config::default(), &tuning,
        );
        let listed = surfaces(&got);
        let katakana = listed.iter().position(|s| s == "キョウ").unwrap();
        let kyoto = listed.iter().position(|s| s == "京都").unwrap();
        assert!(katakana < kyoto, "{listed:?}");
        // 予測を 1 安くすれば上に来る
        tuning.predict_bias = -501;
        let got = run(
            "きょう", "kyo\x1f\x1fu", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &Config::default(), &tuning,
        );
        let listed = surfaces(&got);
        let katakana = listed.iter().position(|s| s == "キョウ").unwrap();
        let kyoto = listed.iter().position(|s| s == "京都").unwrap();
        assert!(kyoto < katakana, "{listed:?}");
    }

    #[test]
    fn 表記が同じ全体変換と予測は全体変換が上の位置に残る() {
        // 予測の 今日 (s = PREDICT_BIAS) が全体変換の 今日 (s 0) より上でも、全体変換を予測の位置に残す
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -1;
        let got = run(
            "きょう", "kyo\x1f\x1fu", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &Config::default(), &tuning,
        );
        assert_eq!((got[0].surface().as_str(), got[0].kind), ("今日", Kind::Whole), "{:?}", surfaces(&got));
        assert_eq!(got.iter().filter(|s| s.surface() == "今日").count(), 1);
        // 補正の候補も、ずれのある予測より下でも文節つきの typo のまま、予測の位置に残る
        // 読み全体の語は高くして、補正の経路は文節に分かれる語を通るようにする
        let mut dict_data = String::from("よろしくおねがいします\t1\t1\t9000\tよろしくお願いします\n");
        dict_data.push_str(
            "よろしく\t1\t1\t3000\tよろしく\nおねがい\t1\t1\t3000\tお願い\nねぎ\t1\t1\t4000\tネギ\n\
             します\t3\t3\t1000\tします\n",
        );
        let dict = dict_from(&dict_data);
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -100_000;
        let got = run(
            "よろしくおねぎあします", "yo\x1fro\x1fsi\x1fku\x1fo\x1fne\x1fgi\x1fa\x1fsi\x1fma\x1fsu",
            &dict, &UserDict::empty(), &LearningStore::in_memory(), &Config::default(), &tuning,
        );
        assert_eq!(got[0].surface(), "よろしくお願いします", "{:?}", surfaces(&got));
        assert_eq!(got[0].kind, Kind::Typo);
        assert!(got[0].segments.len() > 1, "{:?}", got[0].segments);
        assert_eq!(got.iter().filter(|s| s.surface() == "よろしくお願いします").count(), 1);
    }

    #[test]
    fn かなと同じ表記の保護された最良経路でも補正の候補とずれのある予測を上に置かない() {
        // 学習でかなのままの表記を選んだ読み (最良経路は保護の対象で、表記が読みのかなと同じなので並びに無い)
        let dict = dict_from(
            "ありがとう\t1\t1\t3000\t有難う\nありがたい\t1\t1\t3000\t有難い\n",
        );
        let mut learning = LearningStore::in_memory();
        learning.record("ありがとう", "ありがとう");
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -100_000;
        let got = run(
            "ありがとう", "a\x1fri\x1fga\x1fto\x1fu", &dict, &UserDict::empty(), &learning,
            &Config::default(), &tuning,
        );
        let listed = surfaces(&got);
        assert!(!listed.contains(&"ありがとう".to_string()), "{listed:?}");
        let typo = listed.iter().position(|s| s == "有難い").expect("ずれのある予測が無い");
        assert_eq!(got[typo].kind, Kind::PredictTypo);
        let whole = listed.iter().position(|s| s == "有難う").unwrap();
        assert!(whole < typo, "{listed:?}");
        assert!(!matches!(got[0].kind, Kind::Typo | Kind::PredictTypo), "{listed:?}");
    }

    #[test]
    fn 保護された最良経路より上に補正の候補とずれのある予測が来ない() {
        // ユーザ辞書の語 (保護の対象) を含む最良経路
        let mut user = UserDict::empty();
        user.load_from("ありがと\t蟻が十\t名詞\n".as_bytes(), &functional());
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -100_000;
        // ありgとう を ありが + と + う の経路にせず、ずれのある予測を安くする
        let got = run(
            "ありがと", "a\x1fri\x1fga\x1fto", &sample_dict(), &user,
            &LearningStore::in_memory(), &Config::default(), &tuning,
        );
        let surfaces = surfaces(&got);
        let best = surfaces.iter().position(|s| s == "蟻が十").expect("最良経路が無い");
        for (i, s) in got.iter().enumerate() {
            if matches!(s.kind, Kind::Typo | Kind::PredictTypo) {
                assert!(i > best, "{surfaces:?}");
            }
        }
        // ずれの無い予測は上に来てよい
        assert!(got[..best].iter().all(|s| s.kind == Kind::Predict), "{surfaces:?}");
    }

    #[test]
    fn 全体変換の先頭は9番目より後ろにならない() {
        let mut data = String::from("きょう\t1\t1\t2000\t今日\n");
        for i in 0..20 {
            data.push_str(&format!("きょう{}\t1\t1\t100\t予測{i:02}\n", "あいうえおかきくけこさしすせそたちつてと".chars().nth(i).unwrap()));
        }
        let dict = dict_from(&data);
        let mut tuning = typo::TUNING;
        tuning.predict_bias = -100_000;
        let mut cfg = Config::default();
        cfg.max_predictions = 18;
        let got = run(
            "きょう", "kyo\x1f\x1fu", &dict, &UserDict::empty(), &LearningStore::in_memory(), &cfg,
            &tuning,
        );
        assert_eq!(got.len(), 18);
        assert_eq!(got[HEAD_POSITION].surface(), "今日", "{:?}", surfaces(&got));
    }

    #[test]
    fn 補正が無効なら予測は完全な前方一致と同じになる() {
        let mut cfg = Config::default();
        cfg.typo_correction = false;
        let dict = sample_dict();
        // ありがとう の o を i と打った (補正があれば 有難う が出る)
        let got = run(
            "ありがちう", "a\x1fri\x1fga\x1fti\x1fu", &dict, &UserDict::empty(),
            &LearningStore::in_memory(), &cfg, &typo::TUNING,
        );
        assert!(got.iter().all(|s| !matches!(s.kind, Kind::Typo | Kind::PredictTypo)));
        assert!(!surfaces(&got).contains(&"有難う".to_string()));
        // 打ち間違いの無い打鍵では、予測の集合が前方一致 (PREDICT) と同じ
        let got = run(
            "ありが", "a\x1fri\x1fga", &dict, &UserDict::empty(), &LearningStore::in_memory(), &cfg,
            &typo::TUNING,
        );
        let mut predicted: Vec<String> = got
            .iter()
            .filter(|s| s.kind == Kind::Predict)
            .map(Suggestion::surface)
            .collect();
        let mut expected: Vec<String> = predict::predict(
            "ありが", None, &dict, &UserDict::empty(), &LearningStore::in_memory(), &cfg,
        )
        .into_iter()
        .map(|(_, s)| s)
        .collect();
        predicted.sort();
        expected.sort();
        assert_eq!(predicted, expected);
    }

    fn oyasumi_dict(with_second: bool) -> Dictionary {
        // UT 辞書と同じく、読み全体の書き換えの項目が 2 語の変換 (お休み / なさい) より安い
        let mut data = String::from(
            "おやすみなさい\t1\t1\t3500\tおやすみなさい\n\
             おやすみなさい\t1\t1\t4000\tオヤスミなさい\n\
             おやすみなさい\t1\t1\t4100\tおやすみなサイ\n\
             おやすみなさい\t1\t1\t4200\tオヤスミナサイ\n\
             おやすみ\t1\t1\t3300\tお休み\n\
             なさい\t3\t3\t1000\tなさい\n",
        );
        if with_second {
            // N-best のコストの幅の外の、読み全体の辞書の語 (CONVNBEST の辞書の完全一致の候補として後ろに入る)
            data.push_str("おやすみなさい\t1\t1\t9000\t御休みなさい\n");
        }
        dict_from(&data)
    }

    const OYASUMI_KEYS: &str = "o\x1fya\x1fsu\x1fmi\x1fna\x1fsa\x1fi";

    #[test]
    fn 書き換えの候補は1件だけを全体変換の最後に置く() {
        let mut cfg = Config::default();
        cfg.suggest = false;
        let got = run(
            "おやすみなさい", OYASUMI_KEYS, &oyasumi_dict(true), &UserDict::empty(),
            &LearningStore::in_memory(), &cfg, &typo::TUNING,
        );
        // CONVNBEST では 5・6 番目の 2 語の変換が、書き換えより先に入る
        assert_eq!(surfaces(&got), vec!["お休みなさい", "御休みなさい", "オヤスミなさい"]);
        // 読み全体の学習表記は書き換えの候補として扱わない
        let mut learning = LearningStore::in_memory();
        learning.record("おやすみなさい", "オヤスミナサイ");
        let got = run(
            "おやすみなさい", OYASUMI_KEYS, &oyasumi_dict(true), &UserDict::empty(), &learning, &cfg,
            &typo::TUNING,
        );
        assert_eq!(got[0].surface(), "オヤスミナサイ", "{:?}", surfaces(&got));
        assert_eq!(got.len(), MAX_WHOLES, "{:?}", surfaces(&got));
        assert_eq!(got[MAX_WHOLES - 1].surface(), "オヤスミなさい", "{:?}", surfaces(&got));
    }

    #[test]
    fn 書き換えでない候補が足りなければ全体変換の枠を空ける() {
        let mut cfg = Config::default();
        cfg.suggest = false;
        let got = run(
            "おやすみなさい", OYASUMI_KEYS, &oyasumi_dict(false), &UserDict::empty(),
            &LearningStore::in_memory(), &cfg, &typo::TUNING,
        );
        assert_eq!(surfaces(&got), vec!["お休みなさい", "オヤスミなさい"]);
    }

    #[test]
    fn 最良経路がカタカナの読みはカタカナが先頭のまま() {
        let dict = dict_from("てれび\t1\t1\t3000\tテレビ\nてれび\t1\t1\t5000\t手れび\n");
        let mut cfg = Config::default();
        cfg.suggest = false;
        let got = run(
            "てれび", "te\x1fre\x1fbi", &dict, &UserDict::empty(), &LearningStore::in_memory(), &cfg,
            &typo::TUNING,
        );
        assert_eq!(got[0].surface(), "テレビ", "{:?}", surfaces(&got));
    }

    #[test]
    fn サジェストが無効なら全体変換だけになる() {
        let mut cfg = Config::default();
        cfg.suggest = false;
        let got = run(
            "ありが", "a\x1fri\x1fga\x1et", &sample_dict(), &UserDict::empty(),
            &LearningStore::in_memory(), &cfg, &typo::TUNING,
        );
        assert!(!got.is_empty());
        assert!(got.iter().all(|s| matches!(s.kind, Kind::Whole | Kind::Typo)));
        assert!(got.len() <= MAX_WHOLES);
    }

    #[test]
    fn 全体変換の補正の候補はconvnbestに打鍵を付けたものと同じ() {
        let dict = sample_dict();
        let kana = "よろしくおねぎあします";
        let field = "yo\x1fro\x1fsi\x1fku\x1fo\x1fne\x1fgi\x1fa\x1fsi\x1fma\x1fsu";
        let keys = Keys::parse(kana, field).unwrap();
        let nbest = convert::convert_nbest_scored(
            kana, None, &dict, &UserDict::empty(), &ConnectionMatrix::empty(), &functional(),
            &LearningStore::in_memory(), TypoSource::Keys(&keys), &typo::TUNING,
        );
        let typo_candidate = nbest.candidates.iter().find(|c| c.typo).expect("補正の候補が無い");
        let got = run(
            kana, field, &dict, &UserDict::empty(), &LearningStore::in_memory(), &Config::default(),
            &typo::TUNING,
        );
        let item = got.iter().find(|s| s.kind == Kind::Typo).expect("SUGGEST に補正の候補が無い");
        assert_eq!(item.segments, typo_candidate.segments);
        assert_eq!(item.surface(), "よろしくお願いします");
    }
}
