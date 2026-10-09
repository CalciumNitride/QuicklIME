// 英単語辞書と ASCIISTART の照合 (docs/design/modeless-detection.md)
//
// モードレス入力で英字への切替が成立したときに、TSF 層から送られた「かなのかたまりごとの
// 打鍵列」のどこから英単語が始まるかを、英単語辞書との照合で決める。
// 一般語 (data/english-words.txt。SCOWL から生成) と固有名詞・技術用語
// (data/english-names.txt) を読み込む。ユーザ辞書の英字の語は UserDict が持つ。

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// 英単語辞書のファイル名 (辞書ディレクトリ直下)
pub const WORD_FILES: [&str; 2] = ["english-words.txt", "english-names.txt"];

/// 開始位置 0 以外の境目で、英字区間にする打鍵列の最小の長さ。1〜2文字はほぼ必ず何かの
/// 英単語の頭になり、日本語の末尾に英字が1〜2文字だけ付く結果になるため
const MIN_TAIL_CHARS: usize = 3;

/// ASCIISTART の照合方法
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchMode {
    /// 前方一致 (完全一致を含む)。打鍵中の判定用 (英単語をまだ打っている途中)
    Prefix,
    /// 完全一致。run 終了時の判定用 (打ち終わった語)
    Exact,
}

impl MatchMode {
    /// 要求の照合方法フィールド (prefix / exact) を解釈する
    pub fn parse(field: &str) -> Option<Self> {
        match field {
            "prefix" => Some(MatchMode::Prefix),
            "exact" => Some(MatchMode::Exact),
            _ => None,
        }
    }
}

/// 小文字にした語の整列済み配列
pub struct EnglishDict {
    words: Vec<String>,
}

impl EnglishDict {
    pub fn empty() -> Self {
        EnglishDict { words: Vec::new() }
    }

    /// dir 直下の WORD_FILES を読み込む。無いファイルは飛ばし、読めた語数を返す
    pub fn load_dir(dir: &Path) -> (Self, usize) {
        let readers: Vec<BufReader<File>> = WORD_FILES
            .iter()
            .filter_map(|name| File::open(dir.join(name)).ok().map(BufReader::new))
            .collect();
        let dict = Self::load_from(readers);
        let count = dict.words.len();
        (dict, count)
    }

    /// 1行1語。# で始まる行と空行は飛ばし、英字だけの語を小文字にして持つ
    pub fn load_from(readers: impl IntoIterator<Item = impl BufRead>) -> Self {
        let mut words = Vec::new();
        for reader in readers {
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                let word = line.trim();
                if word.starts_with('#') || !is_ascii_word(word) {
                    continue;
                }
                words.push(word.to_ascii_lowercase());
            }
        }
        words.sort();
        words.dedup();
        EnglishDict { words }
    }

    /// text (小文字) が mode の方法で語に一致するか
    pub fn matches(&self, text: &str, mode: MatchMode) -> bool {
        sorted_matches(&self.words, text, mode)
    }
}

/// 空でなく ASCII 英字だけからなる語か
pub fn is_ascii_word(word: &str) -> bool {
    !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphabetic())
}

/// 整列済みの words に、text が mode の方法で一致する語があるか (二分探索)
pub fn sorted_matches(words: &[String], text: &str, mode: MatchMode) -> bool {
    let index = words.partition_point(|w| w.as_str() < text);
    words.get(index).is_some_and(|w| match mode {
        MatchMode::Prefix => w.starts_with(text),
        MatchMode::Exact => w == text,
    })
}

/// ASCIISTART の照合: 先頭の要素から順に、その要素から後ろの打鍵列 (小文字にしたもの) が
/// 英単語に一致するか (matches) を調べ、最初に一致した要素の位置を返す。一致しなければ 0。
/// 最も前の境目を採るのは、後ろの短い打鍵列 (th など) ほど偶然に一致しやすいため。
/// 開始位置 0 以外は、後ろの打鍵列が MIN_TAIL_CHARS 文字未満なら候補にしない
pub fn ascii_start(elements: &[&str], matches: impl Fn(&str) -> bool) -> usize {
    let lowered: Vec<String> = elements.iter().map(|e| e.to_ascii_lowercase()).collect();
    for start in 0..lowered.len() {
        let tail: String = lowered[start..].concat();
        if start > 0 && tail.chars().count() < MIN_TAIL_CHARS {
            break;
        }
        if !tail.is_empty() && matches(&tail) {
            return start;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(text: &str) -> EnglishDict {
        EnglishDict::load_from([text.as_bytes()])
    }

    fn start(d: &EnglishDict, mode: MatchMode, elements: &[&str]) -> usize {
        ascii_start(elements, |text| d.matches(text, mode))
    }

    #[test]
    fn コメントと英字以外の行を除いて小文字で持つ() {
        let d = dict("# コメント\nApple\n\nworld\ncafé\nit's\nzoom\n");
        assert_eq!(d.words, vec!["apple", "world", "zoom"]);
    }

    #[test]
    fn 前方一致は完全一致を含み完全一致は語そのものだけ() {
        let d = dict("apple\nworld\n");
        assert!(d.matches("appl", MatchMode::Prefix));
        assert!(d.matches("apple", MatchMode::Prefix));
        assert!(!d.matches("apples", MatchMode::Prefix));
        assert!(!d.matches("x", MatchMode::Prefix));
        assert!(d.matches("apple", MatchMode::Exact));
        assert!(!d.matches("appl", MatchMode::Exact));
        assert!(!d.matches("apples", MatchMode::Exact));
    }

    #[test]
    fn 前方一致で最も前の一致した境目を返す() {
        let d = dict("apple\ngithub\nthe\nworld\n");
        let prefix = |elements: &[&str]| start(&d, MatchMode::Prefix, elements);
        // きょ|う|は|ぎ + th → gith (github)
        assert_eq!(prefix(&["kyo", "u", "ha", "gi", "th"]), 3);
        // こ|れ|は|あ|っ + pl → appl (haappl・aappl は一致しない)
        assert_eq!(prefix(&["ko", "re", "ha", "a", "p", "pl"]), 3);
        // きょ|う|は + the
        assert_eq!(prefix(&["kyo", "u", "ha", "the"]), 3);
        // を + rl → worl (world) は先頭から一致するので run 全体
        assert_eq!(prefix(&["wo", "rl"]), 0);
        // どこにも一致しなければ 0
        assert_eq!(prefix(&["ko", "re", "ha", "xyzw"]), 0);
        // 大文字は小文字にして照合する
        assert_eq!(prefix(&["ko", "re", "ha", "A", "p", "PL"]), 3);
    }

    #[test]
    fn 完全一致は打ち終わった語の境目だけを返す() {
        let d = dict("hant\nst\nt\nwant\n");
        let exact = |elements: &[&str]| start(&d, MatchMode::Exact, elements);
        // きょ|う|は|わ|ん + t → want
        assert_eq!(exact(&["kyo", "u", "ha", "wa", "n", "t"]), 3);
        // 前方一致なら境目になる語の途中 (wan) は完全一致では候補にならない
        assert_eq!(exact(&["kyo", "u", "ha", "wa", "n"]), 0);
    }

    #[test]
    fn 二文字以下の英字区間は開始位置0以外では候補にしない() {
        // st・t は辞書にあっても、日本語の末尾に1〜2文字だけ付く境目にはしない
        let d = dict("st\nt\n");
        assert_eq!(start(&d, MatchMode::Exact, &["ka", "i", "sya", "s", "t"]), 0);
        assert_eq!(start(&d, MatchMode::Prefix, &["ka", "i", "sya", "s", "t"]), 0);
        // 3文字ちょうどなら候補になる
        let d = dict("abc\n");
        assert_eq!(start(&d, MatchMode::Exact, &["ka", "abc"]), 1);
    }
}
