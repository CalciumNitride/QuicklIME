// 予測入力 (PREDICT) の候補生成
//
// 読みの前方一致で履歴 (確定履歴、新しい順) と辞書 (コスト順) を検索し、
// 履歴 → 辞書の順に合成して返す。表記の重複は先勝ち (履歴優先) で除き、
// 表記が入力かなそのままの候補は出しても意味が無いため除外する。

use crate::dict::Dictionary;
use crate::learn::LearningStore;

/// 予測を出す最小の読み文字数 (1文字では候補が広すぎて役に立たない)
const MIN_PREFIX_CHARS: usize = 2;

/// 予測候補の最大数。TSF 層の候補ウィンドウが1ページ (9行) に収まり、
/// かつ「選択なし」表示 (selection = 候補数) がページ計算で溢れない値にする
pub const MAX_PREDICTIONS: usize = 8;

/// 読みの前方一致で予測候補 (読み, 表記) を返す。
/// 読みは採用時の LEARN に使うため、入力の接頭辞ではなく候補の完全な読みを返す
pub fn predict(kana: &str, dict: &Dictionary, learning: &LearningStore) -> Vec<(String, String)> {
    if kana.chars().count() < MIN_PREFIX_CHARS {
        return Vec::new();
    }

    let mut results: Vec<(String, String)> = Vec::new();
    for (reading, surface) in learning.predict_prefix(kana, MAX_PREDICTIONS) {
        push_unique(&mut results, kana, reading.to_string(), surface.to_string());
    }
    // 履歴との重複と入力かな一致の除外で減る分を見込み、辞書は多めに引く
    for (reading, surface) in dict.predict_prefix(kana, MAX_PREDICTIONS * 2 + 1) {
        if results.len() >= MAX_PREDICTIONS {
            break;
        }
        push_unique(&mut results, kana, reading, surface);
    }
    results.truncate(MAX_PREDICTIONS);
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
            predict("きょう", &sample_dict(), &learning),
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
        let results = predict("きょう", &sample_dict(), &learning);
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
            predict("きょう", &dict, &LearningStore::in_memory()),
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
            predict("ap", &sample_dict(), &learning),
            vec![("apple".to_string(), "Apple".to_string())]
        );
    }

    #[test]
    fn 二文字未満は候補を出さない() {
        assert!(predict("き", &sample_dict(), &LearningStore::in_memory()).is_empty());
        assert!(predict("", &sample_dict(), &LearningStore::in_memory()).is_empty());
    }

    #[test]
    fn 候補数は上限で打ち切る() {
        let mut dict = Dictionary::empty();
        for i in 0..20 {
            dict.load_from(format!("きょう{i:02}\t100\t100\t{}\t表記{i:02}\n", 1000 + i).as_bytes())
                .unwrap();
        }
        dict.finalize();
        let results = predict("きょう", &dict, &LearningStore::in_memory());
        assert_eq!(results.len(), MAX_PREDICTIONS);
    }
}
