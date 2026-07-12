// 変換候補の生成
//
// フェーズ3時点では「カタカナ / ひらがな」の2候補を返すだけの暫定実装。
// フェーズ4でかな漢字変換 (辞書 + Viterbi) に置き換える。

/// かな文字列に対する変換候補リストを返す
pub fn candidates(kana: &str) -> Vec<String> {
    vec![to_katakana(kana), kana.to_string()]
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

    #[test]
    fn ひらがなをカタカナに変換する() {
        assert_eq!(to_katakana("にほんご"), "ニホンゴ");
        assert_eq!(to_katakana("きょう"), "キョウ");
    }

    #[test]
    fn 対象外の文字はそのまま() {
        assert_eq!(to_katakana("あーa1。"), "アーa1。");
    }

    #[test]
    fn 候補はカタカナとひらがなの順() {
        assert_eq!(candidates("かな"), vec!["カナ", "かな"]);
    }
}
