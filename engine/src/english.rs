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

/// 一致とみなす打鍵列の最小の長さ (完全一致と、完全一致でない前方一致)。短い打鍵列はほぼ必ず
/// 何かの英単語の頭になり、日本語の末尾に英字が少しだけ付く結果になるため
const MIN_EXACT_CHARS: usize = 3;
const MIN_PREFIX_CHARS: usize = 4;

/// ASCIISTART の照合方法
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchMode {
    /// 前方一致 (完全一致を含む)。打鍵中の判定用 (英単語をまだ打っている途中)
    Prefix,
    /// 完全一致。run 終了時の判定用 (打ち終わった語)
    Exact,
    /// 長さの制限なしの完全一致。直前の確定が英字のときの判定用 (英文中の短い語 is・an を拾う)
    ExactAny,
}

impl MatchMode {
    /// 要求の照合方法フィールド (prefix / exact / exact-any) を解釈する
    pub fn parse(field: &str) -> Option<Self> {
        match field {
            "prefix" => Some(MatchMode::Prefix),
            "exact" => Some(MatchMode::Exact),
            "exact-any" => Some(MatchMode::ExactAny),
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
        MatchMode::Exact | MatchMode::ExactAny => w == text,
    })
}

/// ASCIISTART の照合: 先頭の要素から順に、その要素から後ろの打鍵列 (小文字にしたもの) が
/// 英単語に一致するかを調べ、最初に一致した要素の位置を返す。どこでも一致しなければ None。
/// 最も前の境目を採るのは、後ろの短い打鍵列 (th など) ほど偶然に一致しやすいため。
/// 完全一致は MIN_EXACT_CHARS 文字以上、完全一致でない前方一致 (mode が Prefix のとき) は
/// MIN_PREFIX_CHARS 文字以上のときだけ一致とする (開始位置 0 も同じ)。ExactAny は長さの制限なしの
/// 完全一致。
/// matches(text, mode) は text が mode の方法で英単語に一致するか
pub fn ascii_start(
    elements: &[&str],
    mode: MatchMode,
    matches: impl Fn(&str, MatchMode) -> bool,
) -> Option<usize> {
    let lowered: Vec<String> = elements.iter().map(|e| e.to_ascii_lowercase()).collect();
    for start in 0..lowered.len() {
        let tail: String = lowered[start..].concat();
        let length = tail.chars().count();
        if length == 0 || (mode != MatchMode::ExactAny && length < MIN_EXACT_CHARS) {
            break;
        }
        if matches(&tail, MatchMode::Exact)
            || (mode == MatchMode::Prefix
                && length >= MIN_PREFIX_CHARS
                && matches(&tail, MatchMode::Prefix))
        {
            return Some(start);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(text: &str) -> EnglishDict {
        EnglishDict::load_from([text.as_bytes()])
    }

    fn start(d: &EnglishDict, mode: MatchMode, elements: &[&str]) -> Option<usize> {
        ascii_start(elements, mode, |text, m| d.matches(text, m))
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
        let d = dict("account\napple\ngithub\nthe\nworld\n");
        let prefix = |elements: &[&str]| start(&d, MatchMode::Prefix, elements);
        // きょ|う|は|ぎ + th → gith (github)
        assert_eq!(prefix(&["kyo", "u", "ha", "gi", "th"]), Some(3));
        // こ|れ|は|あ|っ + pl → appl (haappl・aappl は一致しない)
        assert_eq!(prefix(&["ko", "re", "ha", "a", "p", "pl"]), Some(3));
        // きょ|う|は + the (3文字の完全一致)
        assert_eq!(prefix(&["kyo", "u", "ha", "the"]), Some(3));
        // を + rl → worl (world) は先頭から一致するので run 全体
        assert_eq!(prefix(&["wo", "rl"]), Some(0));
        // きょ|う|は|あ|っ + co → acco (account)
        assert_eq!(prefix(&["kyo", "u", "ha", "a", "c", "co"]), Some(3));
        // どこにも一致しなければ一致なし
        assert_eq!(prefix(&["ko", "re", "ha", "xyzw"]), None);
        // 大文字は小文字にして照合する
        assert_eq!(prefix(&["ko", "re", "ha", "A", "p", "PL"]), Some(3));
    }

    #[test]
    fn 完全一致は打ち終わった語の境目だけを返す() {
        let d = dict("st\nt\nwant\n");
        let exact = |elements: &[&str]| start(&d, MatchMode::Exact, elements);
        // きょ|う|は|わ|ん + t → want
        assert_eq!(exact(&["kyo", "u", "ha", "wa", "n", "t"]), Some(3));
        // 前方一致なら一致する打ち途中の語 (wan → want) は完全一致では一致しない
        assert_eq!(exact(&["kyo", "u", "ha", "wa", "n"]), None);
        // nt・t、st は短すぎるので、辞書にあっても一致にしない
        assert_eq!(exact(&["wa", "ta", "si", "ha", "n", "t"]), None);
        assert_eq!(exact(&["ka", "i", "sya", "s", "t"]), None);
    }

    #[test]
    fn 長さの制限は完全一致3文字以上_前方一致4文字以上で開始位置0にも当てはめる() {
        let d = dict("ab\nabc\nabcde\n");
        // 完全一致: 3文字は一致、2文字は一致しない (開始位置 0 でも)
        assert_eq!(start(&d, MatchMode::Exact, &["ka", "abc"]), Some(1));
        assert_eq!(start(&d, MatchMode::Exact, &["ab"]), None);
        // 前方一致の要求でも、3文字の打鍵列は完全一致なら一致する
        assert_eq!(start(&d, MatchMode::Prefix, &["a", "bc"]), Some(0));
        // 完全一致でない前方一致は4文字から
        let d = dict("abcde\n");
        assert_eq!(start(&d, MatchMode::Prefix, &["a", "bc"]), None);
        assert_eq!(start(&d, MatchMode::Prefix, &["a", "bcd"]), Some(0));
        assert_eq!(start(&d, MatchMode::Prefix, &["ka", "a", "bcd"]), Some(1));
    }

    #[test]
    fn exact_anyは長さの制限なしの完全一致() {
        let d = dict("a\nan\nis\npen\n");
        assert_eq!(start(&d, MatchMode::ExactAny, &["i", "s"]), Some(0));
        assert_eq!(start(&d, MatchMode::ExactAny, &["a"]), Some(0));
        assert_eq!(start(&d, MatchMode::ExactAny, &["pe", "n"]), Some(0));
        // 完全一致だけなので、語の頭 (pe → pen) は一致しない
        assert_eq!(start(&d, MatchMode::ExactAny, &["pe"]), None);
        // exact では同じ短い語が一致しない
        assert_eq!(start(&d, MatchMode::Exact, &["i", "s"]), None);
        assert_eq!(MatchMode::parse("exact-any"), Some(MatchMode::ExactAny));
    }
}
