// 数字で打った数の別の書き方
//
// 「1234」に対して 千二百三十四・一二三四・1,234・１２３４ などの表記を作る。
// 仕様は docs/spec/conversion.md「数字の書き方の候補」と docs/spec/engine.md「数字の書き方の候補の位置」を参照。

/// 書き方の種類の数 (forms の戻り値の長さ)
pub const FORM_COUNT: usize = 5;

/// 対象にする数の最大桁数 (9999兆9999億9999万9999)
const MAX_DIGITS: usize = 16;

const KANJI_DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

/// 万・億・兆の単位 (4 桁ごと。下の桁から)
const MAN_UNITS: [&str; 4] = ["", "万", "億", "兆"];

/// 数字だけの読みから、書き方ごとの表記を
/// [位取りの漢数字, 並べた漢数字, 桁区切り, 万の単位まじり, 全角/半角] の順で返す。
/// 作らない書き方 (条件に合わない・打ったとおりやほかの書き方と同じ) は None。
/// 対象外の読み (数字以外を含む・桁数が範囲外・半角と全角が混ざる) はすべて None
pub fn forms(reading: &str) -> [Option<String>; FORM_COUNT] {
    let mut result: [Option<String>; FORM_COUNT] = Default::default();
    let Some((digits, fullwidth)) = parse(reading) else {
        return result;
    };
    let leading_zero = digits[0] == 0 && digits.len() > 1;

    let candidates = [
        (!leading_zero).then(|| positional_kanji(&digits)),
        (digits.len() >= 2).then(|| digits.iter().map(|&d| KANJI_DIGITS[d as usize]).collect()),
        (digits.len() >= 4 && !leading_zero).then(|| comma_separated(&digits, fullwidth)),
        (digits.len() >= 5 && !leading_zero).then(|| man_units(&digits, fullwidth)),
        Some(digits_text(&digits, !fullwidth)),
    ];
    for (i, candidate) in candidates.into_iter().enumerate() {
        let Some(candidate) = candidate else { continue };
        if candidate != reading && !result.iter().flatten().any(|s| *s == candidate) {
            result[i] = Some(candidate);
        }
    }
    result
}

/// 読みを桁の値の列と全角かどうかに分ける。対象外なら None
fn parse(reading: &str) -> Option<(Vec<u8>, bool)> {
    let mut digits = Vec::new();
    let mut half = false;
    let mut full = false;
    for c in reading.chars() {
        if c.is_ascii_digit() {
            half = true;
            digits.push(c as u8 - b'0');
        } else if ('０'..='９').contains(&c) {
            full = true;
            digits.push((c as u32 - '０' as u32) as u8);
        } else {
            return None;
        }
    }
    // 半角と全角が混ざると、桁区切りなどに使う幅も全角/半角の書き方の向きも決まらない
    if digits.is_empty() || digits.len() > MAX_DIGITS || (half && full) {
        return None;
    }
    Some((digits, full))
}

/// 桁の列を数字の文字列にする
fn digits_text(digits: &[u8], fullwidth: bool) -> String {
    digits
        .iter()
        .map(|&d| {
            if fullwidth {
                char::from_u32('０' as u32 + u32::from(d)).unwrap()
            } else {
                char::from(b'0' + d)
            }
        })
        .collect()
}

/// 下の桁から 4 桁ずつに分けた各単位の値の列 (下の単位から)
fn man_groups(digits: &[u8]) -> Vec<u32> {
    digits
        .rchunks(4)
        .map(|chunk| chunk.iter().fold(0, |v, &d| v * 10 + u32::from(d)))
        .collect()
}

/// 位取りの漢数字 (千二百三十四、十二万五、一億)
fn positional_kanji(digits: &[u8]) -> String {
    let groups = man_groups(digits);
    if groups.iter().all(|&g| g == 0) {
        return KANJI_DIGITS[0].to_string();
    }
    let mut result = String::new();
    for (unit, &group) in groups.iter().enumerate().rev() {
        if group == 0 {
            continue;
        }
        for (place, mark) in [(1000, "千"), (100, "百"), (10, "十")] {
            let d = group / place % 10;
            if d == 0 {
                continue;
            }
            if d != 1 {
                result.push(KANJI_DIGITS[d as usize]);
            }
            result.push_str(mark);
        }
        if group % 10 != 0 {
            result.push(KANJI_DIGITS[(group % 10) as usize]);
        }
        result.push_str(MAN_UNITS[unit]);
    }
    result
}

/// 3 桁ごとの桁区切り (1,234、１，２３４)
fn comma_separated(digits: &[u8], fullwidth: bool) -> String {
    let separator = if fullwidth { "，" } else { "," };
    let head = digits.len() % 3;
    let mut parts: Vec<String> = Vec::new();
    if head > 0 {
        parts.push(digits_text(&digits[..head], fullwidth));
    }
    for chunk in digits[head..].chunks(3) {
        parts.push(digits_text(chunk, fullwidth));
    }
    parts.join(separator)
}

/// 万の単位まじり (12万5、1億5000)
fn man_units(digits: &[u8], fullwidth: bool) -> String {
    let mut result = String::new();
    for (unit, &group) in man_groups(digits).iter().enumerate().rev() {
        if group == 0 {
            continue;
        }
        let values: Vec<u8> = group.to_string().bytes().map(|b| b - b'0').collect();
        result.push_str(&digits_text(&values, fullwidth));
        result.push_str(MAN_UNITS[unit]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn got(reading: &str) -> Vec<Option<String>> {
        forms(reading).to_vec()
    }

    fn some(values: [Option<&str>; FORM_COUNT]) -> Vec<Option<String>> {
        values.iter().map(|v| v.map(str::to_string)).collect()
    }

    #[test]
    fn 四桁の数はすべての書き方が出る_万の単位まじりは除く() {
        assert_eq!(
            got("1234"),
            some([Some("千二百三十四"), Some("一二三四"), Some("1,234"), None, Some("１２３４")])
        );
    }

    #[test]
    fn 値が0の位は書かない() {
        assert_eq!(
            got("120005"),
            some([
                Some("十二万五"),
                Some("一二〇〇〇五"),
                Some("120,005"),
                Some("12万5"),
                Some("１２０００５"),
            ])
        );
    }

    #[test]
    fn 一桁の数は位取りの漢数字と全角だけ() {
        assert_eq!(got("0"), some([Some("〇"), None, None, None, Some("０")]));
        assert_eq!(got("7"), some([Some("七"), None, None, None, Some("７")]));
    }

    #[test]
    fn 十百千の前の一は書かない() {
        assert_eq!(forms("10")[0].as_deref(), Some("十"));
        assert_eq!(forms("10")[1].as_deref(), Some("一〇"));
        assert_eq!(forms("15")[0].as_deref(), Some("十五"));
        assert_eq!(forms("100")[0].as_deref(), Some("百"));
        assert_eq!(forms("1000")[0].as_deref(), Some("千"));
        assert_eq!(forms("1000")[2].as_deref(), Some("1,000"));
        assert_eq!(forms("1005")[0].as_deref(), Some("千五"));
    }

    #[test]
    fn 万億兆の前の一は書く() {
        assert_eq!(
            got("10000"),
            some([
                Some("一万"),
                Some("一〇〇〇〇"),
                Some("10,000"),
                Some("1万"),
                Some("１００００"),
            ])
        );
        assert_eq!(forms("10005")[0].as_deref(), Some("一万五"));
        assert_eq!(forms("100000000")[0].as_deref(), Some("一億"));
    }

    #[test]
    fn 値が0の単位は万の単位まじりに書かない() {
        assert_eq!(forms("100005000")[0].as_deref(), Some("一億五千"));
        assert_eq!(forms("100005000")[3].as_deref(), Some("1億5000"));
        assert_eq!(forms("1200000000")[3].as_deref(), Some("12億"));
    }

    #[test]
    fn 先頭が0の数は並べた漢数字と全角だけ() {
        assert_eq!(got("007"), some([None, Some("〇〇七"), None, None, Some("００７")]));
        assert_eq!(forms("01234")[2], None);
        assert_eq!(forms("01234")[3], None);
    }

    #[test]
    fn 全角の数は全角で区切り半角を書き方に足す() {
        assert_eq!(
            got("１２３４"),
            some([Some("千二百三十四"), Some("一二三四"), Some("１，２３４"), None, Some("1234")])
        );
        assert_eq!(forms("１２０００５")[3].as_deref(), Some("１２万５"));
    }

    #[test]
    fn 十六桁まで対象になる() {
        let max = "9999999999999999";
        let f = forms(max);
        assert_eq!(
            f[0].as_deref(),
            Some("九千九百九十九兆九千九百九十九億九千九百九十九万九千九百九十九")
        );
        assert_eq!(f[2].as_deref(), Some("9,999,999,999,999,999"));
        assert_eq!(f[3].as_deref(), Some("9999兆9999億9999万9999"));
    }

    #[test]
    fn 十七桁以上と数字以外を含む読みは対象外() {
        assert!(forms("12345678901234567").iter().all(Option::is_none));
        assert!(forms("0。5").iter().all(Option::is_none));
        assert!(forms("1,000").iter().all(Option::is_none));
        assert!(forms("12じ").iter().all(Option::is_none));
        assert!(forms("").iter().all(Option::is_none));
        assert!(forms("1２").iter().all(Option::is_none));
    }
}
