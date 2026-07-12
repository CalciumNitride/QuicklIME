// 変換候補の生成
//
// フェーズ4-1: 読み全体の辞書完全一致 (コスト昇順・最大5件) + カタカナ + ひらがな。
// フェーズ4-2で Viterbi による文単位の変換に拡張する。

use crate::dict::Dictionary;

/// 辞書由来の候補の最大数
const MAX_DICT_CANDIDATES: usize = 5;

/// かな文字列に対する変換候補リストを返す
pub fn candidates(kana: &str, dict: &Dictionary) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    // 辞書の完全一致候補をコスト順に (同じ表記は除く)
    let mut entries: Vec<_> = dict.lookup(kana).iter().collect();
    entries.sort_by_key(|e| e.cost);
    for entry in entries {
        if result.len() >= MAX_DICT_CANDIDATES {
            break;
        }
        if !result.contains(&entry.surface) {
            result.push(entry.surface.clone());
        }
    }

    // カタカナ・ひらがなは常に候補に含める
    for extra in [to_katakana(kana), kana.to_string()] {
        if !result.contains(&extra) {
            result.push(extra);
        }
    }
    result
}

/// ひらがなをカタカナへ変換する (対象外の文字はそのまま)
fn to_katakana(kana: &str) -> String {
    kana.chars()
        .map(|c| {
            // ひらがな (ぁ U+3041 〜 ゖ U+3096) はカタカナと 0x60 差で並んでいる
            if ('ぁ'..='ゖ').contains(&c) {
                char::from_u32(c as u32 + 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dict() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "にほんご\t1851\t1851\t7869\tニホンゴ\n\
                    にほんご\t1851\t1851\t3793\t日本語\n\
                    にほんご\t1920\t1920\t12879\t日本語\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict
    }

    #[test]
    fn 辞書候補がコスト順に先頭へ来る() {
        // 日本語(3793) → ニホンゴ(7869) → 日本語(12879 は重複除去) → にほんご
        assert_eq!(
            candidates("にほんご", &sample_dict()),
            vec!["日本語", "ニホンゴ", "にほんご"]
        );
    }

    #[test]
    fn 辞書に無ければカタカナとひらがなのみ() {
        assert_eq!(candidates("かな", &Dictionary::empty()), vec!["カナ", "かな"]);
    }

    #[test]
    fn ひらがなをカタカナに変換する() {
        assert_eq!(to_katakana("きょう"), "キョウ");
        assert_eq!(to_katakana("あーa1。"), "アーa1。");
    }
}
