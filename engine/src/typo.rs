// 変換へのタイプミス補正の、1 打の誤りを戻した読みの生成 (docs/design/typo-conversion.md)
//
// エンジンには打鍵列が届かないので、かなを標準のローマ字に戻し、ローマ字の上で 1 打の誤り
// (余分・抜け・隣のキー・入れ替え) を戻してから、TSF 層と同じ規則でかなに直す。
// ローマ字の表は TSF 層の既定のテーブル (tsf/src/romaji.cpp kDefaultTable) と同じ内容。
// ユーザのローマ字テーブルはエンジンから見えないため、標準の綴りでの近似になる

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// 補正を作る最小の読み文字数。短い読みは 1 打違いの別の語が多く、誤補正の元になる
pub const MIN_TYPO_CHARS: usize = 3;

/// ローマ字 → かなの表 (tsf/src/romaji.cpp の kDefaultTable と同じ並び)。
/// かな → ローマ字の綴りもこの並びで選ぶ (同じかなの綴りは先に載っているものを訓令式とする)
const ROMAJI_TABLE: &[(&str, &str)] = &[
    ("a", "あ"), ("i", "い"), ("u", "う"), ("e", "え"), ("o", "お"),
    ("ka", "か"), ("ki", "き"), ("ku", "く"), ("ke", "け"), ("ko", "こ"),
    ("kya", "きゃ"), ("kyi", "きぃ"), ("kyu", "きゅ"), ("kye", "きぇ"), ("kyo", "きょ"),
    ("qa", "くぁ"), ("qi", "くぃ"), ("qu", "く"), ("qe", "くぇ"), ("qo", "くぉ"),
    ("ga", "が"), ("gi", "ぎ"), ("gu", "ぐ"), ("ge", "げ"), ("go", "ご"),
    ("gya", "ぎゃ"), ("gyi", "ぎぃ"), ("gyu", "ぎゅ"), ("gye", "ぎぇ"), ("gyo", "ぎょ"),
    ("sa", "さ"), ("si", "し"), ("su", "す"), ("se", "せ"), ("so", "そ"),
    ("sya", "しゃ"), ("syi", "しぃ"), ("syu", "しゅ"), ("sye", "しぇ"), ("syo", "しょ"),
    ("sha", "しゃ"), ("shi", "し"), ("shu", "しゅ"), ("she", "しぇ"), ("sho", "しょ"),
    ("za", "ざ"), ("zi", "じ"), ("zu", "ず"), ("ze", "ぜ"), ("zo", "ぞ"),
    ("zya", "じゃ"), ("zyi", "じぃ"), ("zyu", "じゅ"), ("zye", "じぇ"), ("zyo", "じょ"),
    ("ja", "じゃ"), ("ji", "じ"), ("ju", "じゅ"), ("je", "じぇ"), ("jo", "じょ"),
    ("jya", "じゃ"), ("jyi", "じぃ"), ("jyu", "じゅ"), ("jye", "じぇ"), ("jyo", "じょ"),
    ("ta", "た"), ("ti", "ち"), ("tu", "つ"), ("te", "て"), ("to", "と"),
    ("tya", "ちゃ"), ("tyi", "ちぃ"), ("tyu", "ちゅ"), ("tye", "ちぇ"), ("tyo", "ちょ"),
    ("cha", "ちゃ"), ("chi", "ち"), ("chu", "ちゅ"), ("che", "ちぇ"), ("cho", "ちょ"),
    ("tsa", "つぁ"), ("tsi", "つぃ"), ("tsu", "つ"), ("tse", "つぇ"), ("tso", "つぉ"),
    ("tha", "てゃ"), ("thi", "てぃ"), ("thu", "てゅ"), ("the", "てぇ"), ("tho", "てょ"),
    ("twu", "とぅ"),
    ("da", "だ"), ("di", "ぢ"), ("du", "づ"), ("de", "で"), ("do", "ど"),
    ("dya", "ぢゃ"), ("dyi", "ぢぃ"), ("dyu", "ぢゅ"), ("dye", "ぢぇ"), ("dyo", "ぢょ"),
    ("dha", "でゃ"), ("dhi", "でぃ"), ("dhu", "でゅ"), ("dhe", "でぇ"), ("dho", "でょ"),
    ("dwu", "どぅ"),
    ("na", "な"), ("ni", "に"), ("nu", "ぬ"), ("ne", "ね"), ("no", "の"),
    ("nya", "にゃ"), ("nyi", "にぃ"), ("nyu", "にゅ"), ("nye", "にぇ"), ("nyo", "にょ"),
    ("nn", "ん"),
    ("ha", "は"), ("hi", "ひ"), ("hu", "ふ"), ("he", "へ"), ("ho", "ほ"),
    ("hya", "ひゃ"), ("hyi", "ひぃ"), ("hyu", "ひゅ"), ("hye", "ひぇ"), ("hyo", "ひょ"),
    ("fa", "ふぁ"), ("fi", "ふぃ"), ("fu", "ふ"), ("fe", "ふぇ"), ("fo", "ふぉ"),
    ("ba", "ば"), ("bi", "び"), ("bu", "ぶ"), ("be", "べ"), ("bo", "ぼ"),
    ("bya", "びゃ"), ("byi", "びぃ"), ("byu", "びゅ"), ("bye", "びぇ"), ("byo", "びょ"),
    ("pa", "ぱ"), ("pi", "ぴ"), ("pu", "ぷ"), ("pe", "ぺ"), ("po", "ぽ"),
    ("pya", "ぴゃ"), ("pyi", "ぴぃ"), ("pyu", "ぴゅ"), ("pye", "ぴぇ"), ("pyo", "ぴょ"),
    ("ma", "ま"), ("mi", "み"), ("mu", "む"), ("me", "め"), ("mo", "も"),
    ("mya", "みゃ"), ("myi", "みぃ"), ("myu", "みゅ"), ("mye", "みぇ"), ("myo", "みょ"),
    ("ya", "や"), ("yu", "ゆ"), ("yo", "よ"), ("ye", "いぇ"),
    ("ra", "ら"), ("ri", "り"), ("ru", "る"), ("re", "れ"), ("ro", "ろ"),
    ("rya", "りゃ"), ("ryi", "りぃ"), ("ryu", "りゅ"), ("rye", "りぇ"), ("ryo", "りょ"),
    ("wa", "わ"), ("wi", "うぃ"), ("wu", "う"), ("we", "うぇ"), ("wo", "を"),
    ("wha", "うぁ"), ("whi", "うぃ"), ("whu", "う"), ("whe", "うぇ"), ("who", "うぉ"),
    ("va", "ゔぁ"), ("vi", "ゔぃ"), ("vu", "ゔ"), ("ve", "ゔぇ"), ("vo", "ゔぉ"),
    ("xa", "ぁ"), ("xi", "ぃ"), ("xu", "ぅ"), ("xe", "ぇ"), ("xo", "ぉ"),
    ("la", "ぁ"), ("li", "ぃ"), ("lu", "ぅ"), ("le", "ぇ"), ("lo", "ぉ"),
    ("xya", "ゃ"), ("xyu", "ゅ"), ("xyo", "ょ"),
    ("lya", "ゃ"), ("lyu", "ゅ"), ("lyo", "ょ"),
    ("xtu", "っ"), ("ltu", "っ"), ("ltsu", "っ"),
    ("xn", "ん"),
];

/// 抜けた 1 打として足す字。ローマ字の 1 打の抜けに多い字 (母音・n・y・h・s・t・k・r)
const INSERT_CHARS: &[u8] = b"aiueonyhstkr";

/// QWERTY の英字の段 (上から)。段ごとの左端のずれ (キー幅単位) と並び
const KEY_ROWS: [(f32, &[u8]); 3] = [(0.0, b"qwertyuiop"), (0.25, b"asdfghjkl"), (0.75, b"zxcvbnm")];

/// 1 打の誤りを戻した読み
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    /// 補正後の読み全体
    pub reading: String,
    /// 変わった所の始まり (文字位置。打ったかな・補正後の読みで共通)
    pub start: usize,
    /// 打ったかなでの変わった所の終わり (文字位置)
    pub typed_end: usize,
    /// 補正後の読みでの変わった所の終わり (文字位置)
    pub corrected_end: usize,
}

struct Tables {
    to_kana: HashMap<&'static str, &'static str>,
    /// テーブルのキーの真の接頭辞 (続きの入力を待つかの判定)
    prefixes: HashSet<&'static str>,
    /// かな → (訓令式, ヘボン式) の綴り
    to_romaji: HashMap<&'static str, (&'static str, &'static str)>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut to_kana = HashMap::new();
        let mut prefixes = HashSet::new();
        let mut to_romaji: HashMap<&'static str, (&'static str, &'static str)> = HashMap::new();
        for &(key, kana) in ROMAJI_TABLE {
            to_kana.insert(key, kana);
            for end in 1..key.len() {
                prefixes.insert(&key[..end]);
            }
            let hepburn = (key.starts_with("sh")
                || key.starts_with("ch")
                || key.starts_with("ts")
                || key.starts_with('f')
                || (key.starts_with('j') && !key.starts_with("jy")))
                .then_some(key);
            match to_romaji.get_mut(kana) {
                None => {
                    to_romaji.insert(kana, (key, hepburn.unwrap_or(key)));
                }
                Some(entry) => {
                    if let Some(h) = hepburn {
                        if entry.1 == entry.0 {
                            entry.1 = h;
                        }
                    }
                }
            }
        }
        Tables { to_kana, prefixes, to_romaji }
    })
}

fn is_vowel(c: u8) -> bool {
    matches!(c, b'a' | b'i' | b'u' | b'e' | b'o')
}

/// ローマ字をかなに直す (TSF 層の RomajiComposer::Convert と同じ規則。末尾の n は確定時と同じく
/// ん にする)。かなにならない打鍵が残れば None
pub fn romaji_to_kana(romaji: &str) -> Option<String> {
    let t = tables();
    let mut kana = String::new();
    let mut pending = String::new();
    let flush = |pending: &mut String, kana: &mut String| -> bool {
        match pending.as_str() {
            "" => true,
            "n" => {
                kana.push('ん');
                pending.clear();
                true
            }
            _ => false,
        }
    };
    for c in romaji.chars() {
        if c == '-' {
            if !flush(&mut pending, &mut kana) {
                return None;
            }
            kana.push('ー');
            continue;
        }
        if !c.is_ascii_lowercase() {
            return None;
        }
        pending.push(c);
        loop {
            if pending.is_empty() {
                break;
            }
            if let Some(k) = t.to_kana.get(pending.as_str()) {
                kana.push_str(k);
                pending.clear();
                break;
            }
            if t.prefixes.contains(pending.as_str()) {
                break;
            }
            let bytes = pending.as_bytes();
            if bytes.len() >= 2 && bytes[0] == bytes[1] && !is_vowel(bytes[0]) && bytes[0] != b'n' {
                kana.push('っ');
                pending.remove(0);
                continue;
            }
            if bytes[0] == b'n' && bytes.len() >= 2 {
                kana.push('ん');
                pending.remove(0);
                continue;
            }
            return None;
        }
    }
    flush(&mut pending, &mut kana).then_some(kana)
}

/// かなの連続を、かなごとの綴りの列にする。ローマ字にできない文字があれば None
fn kana_tokens(chars: &[char]) -> Option<Vec<(&'static str, &'static str)>> {
    let t = tables();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut buf = String::new();
    while i < chars.len() {
        if chars[i] == 'ー' {
            tokens.push(("-", "-"));
            i += 1;
            continue;
        }
        // 拗音などの 2 文字のかなを先に見る (きゃ を き + ゃ に割らない)
        let mut found = None;
        for len in [2, 1] {
            if i + len > chars.len() {
                continue;
            }
            buf.clear();
            buf.extend(&chars[i..i + len]);
            if let Some(&spellings) = t.to_romaji.get(buf.as_str()) {
                found = Some((spellings, len));
                break;
            }
        }
        let (spellings, len) = found?;
        tokens.push(spellings);
        i += len;
    }
    Some(tokens)
}

/// かなの連続を標準のローマ字に戻す。hepburn ならヘボン式の綴り、short_n なら ん を
/// 打てるところで n 1 打にする。っ は次の子音の重ね (重ねられなければ xtu)
fn kana_to_romaji(tokens: &[(&'static str, &'static str)], hepburn: bool, short_n: bool) -> String {
    // 後ろから組み立てる。っ の重ねる子音は、後ろに続く打鍵列の先頭 (っっ なら後ろの っ の子音) で決まる
    let mut parts: Vec<&str> = Vec::with_capacity(tokens.len());
    let mut next: Option<u8> = None;
    for &(kunrei, hepburn_spelling) in tokens.iter().rev() {
        let spelling = if hepburn { hepburn_spelling } else { kunrei };
        let part: &str = match spelling {
            "xtu" => match next {
                Some(c) if c.is_ascii_lowercase() && !is_vowel(c) && c != b'n' => {
                    &DOUBLED[usize::from(c - b'a')..usize::from(c - b'a') + 1]
                }
                _ => "xtu",
            },
            "nn" if short_n => match next {
                Some(c) if c.is_ascii_lowercase() && !is_vowel(c) && c != b'n' && c != b'y' => "n",
                None => "n",
                _ => "nn",
            },
            s => s,
        };
        next = Some(part.as_bytes()[0]);
        parts.push(part);
    }
    parts.iter().rev().copied().collect()
}

/// っ で重ねる子音の 1 文字 (英小文字の並び)
const DOUBLED: &str = "abcdefghijklmnopqrstuvwxyz";

/// 計測用 (examples/typo_eval.rs): かなだけの読みを標準のローマ字 (ん は nn) にする。
/// hepburn ならヘボン式の綴り。ローマ字にできない文字があれば None
#[allow(dead_code)]
pub fn to_romaji(kana: &str, hepburn: bool) -> Option<String> {
    let chars: Vec<char> = kana.chars().collect();
    Some(kana_to_romaji(&kana_tokens(&chars)?, hepburn, false))
}

/// QWERTY で c の隣のキー (同じ段の左右と、上下の段の隣接するもの)
pub fn neighbor_keys(c: u8) -> Vec<u8> {
    let Some((row, col)) = KEY_ROWS
        .iter()
        .enumerate()
        .find_map(|(r, (_, keys))| keys.iter().position(|&k| k == c).map(|p| (r, p)))
    else {
        return Vec::new();
    };
    let x = KEY_ROWS[row].0 + col as f32;
    let mut result = Vec::new();
    for (r, (offset, keys)) in KEY_ROWS.iter().enumerate() {
        if r.abs_diff(row) > 1 {
            continue;
        }
        for (p, &k) in keys.iter().enumerate() {
            let dx = (offset + p as f32 - x).abs();
            let adjacent = if r == row { dx == 1.0 } else { dx < 1.0 };
            if adjacent {
                result.push(k);
            }
        }
    }
    result
}

/// romaji から 1 打の誤りを戻した打鍵列を列挙する
fn romaji_edits(romaji: &str, out: &mut HashSet<String>) {
    let bytes = romaji.as_bytes();
    let n = bytes.len();
    let mut push = |v: Vec<u8>| {
        if let Ok(s) = String::from_utf8(v) {
            out.insert(s);
        }
    };
    // 余分な 1 打を消す
    for i in 0..n {
        let mut v = bytes.to_vec();
        v.remove(i);
        push(v);
    }
    // 抜けた 1 打を足す (よく抜ける字と、隣の字の重ね)
    for i in 0..=n {
        let mut chars: Vec<u8> = INSERT_CHARS.to_vec();
        if i > 0 {
            chars.push(bytes[i - 1]);
        }
        if i < n {
            chars.push(bytes[i]);
        }
        for c in chars {
            let mut v = bytes.to_vec();
            v.insert(i, c);
            push(v);
        }
    }
    // 隣のキーに置き換える
    for i in 0..n {
        for c in neighbor_keys(bytes[i]) {
            let mut v = bytes.to_vec();
            v[i] = c;
            push(v);
        }
    }
    // 隣り合う 2 打を入れ替える
    for i in 0..n.saturating_sub(1) {
        if bytes[i] != bytes[i + 1] {
            let mut v = bytes.to_vec();
            v.swap(i, i + 1);
            push(v);
        }
    }
}

/// 補正の対象にするかな (ひらがなと長音記号)
fn is_kana(c: char) -> bool {
    ('ぁ'..='ゖ').contains(&c) || c == 'ー'
}

/// 打ったかな kana の 1 打の誤りを戻した読みを返す (補正後の読みで重複を除く)。
/// 先頭のかなは変えず、MIN_TYPO_CHARS 文字未満の読みでは作らない。
/// かな以外を含む読みでは、かなの連続の中だけで作る
pub fn corrections(kana: &str) -> Vec<Correction> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n < MIN_TYPO_CHARS {
        return Vec::new();
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    let mut run_start = 0;
    while run_start < n {
        if !is_kana(chars[run_start]) {
            run_start += 1;
            continue;
        }
        let run_end = (run_start..n).find(|&i| !is_kana(chars[i])).unwrap_or(n);
        let run = &chars[run_start..run_end];
        if let Some(tokens) = kana_tokens(run) {
            let mut edits: HashSet<String> = HashSet::new();
            let mut variants: Vec<String> = Vec::new();
            for hepburn in [false, true] {
                for short_n in [false, true] {
                    let romaji = kana_to_romaji(&tokens, hepburn, short_n);
                    // 戻した綴りがかなに戻らないなら、その綴りからは作らない
                    if !variants.contains(&romaji)
                        && romaji_to_kana(&romaji).is_some_and(|k| k.chars().eq(run.iter().copied()))
                    {
                        romaji_edits(&romaji, &mut edits);
                        variants.push(romaji);
                    }
                }
            }
            for edit in edits {
                let Some(fixed) = romaji_to_kana(&edit) else {
                    continue;
                };
                let fixed: Vec<char> = fixed.chars().collect();
                if fixed.as_slice() == run {
                    continue;
                }
                let mut full: Vec<char> = Vec::with_capacity(n + 2);
                full.extend_from_slice(&chars[..run_start]);
                full.extend_from_slice(&fixed);
                full.extend_from_slice(&chars[run_end..]);
                if full.first() != chars.first() {
                    continue;
                }
                let reading: String = full.iter().collect();
                if !seen.insert(reading.clone()) {
                    continue;
                }
                let prefix = chars.iter().zip(&full).take_while(|(a, b)| a == b).count();
                let suffix = chars[prefix..]
                    .iter()
                    .rev()
                    .zip(full[prefix..].iter().rev())
                    .take_while(|(a, b)| a == b)
                    .count();
                result.push(Correction {
                    reading,
                    start: prefix,
                    typed_end: n - suffix,
                    corrected_end: full.len() - suffix,
                });
            }
        }
        run_start = run_end;
    }
    // HashSet の順に依らない結果にする (同じコストの語の選び方を毎回同じにするため)
    result.sort_by(|a, b| a.reading.cmp(&b.reading));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(kana: &str) -> Vec<String> {
        corrections(kana).into_iter().map(|c| c.reading).collect()
    }

    #[test]
    fn ローマ字をtsf層と同じ規則でかなに直す() {
        assert_eq!(romaji_to_kana("onegaisimasu").as_deref(), Some("おねがいします"));
        assert_eq!(romaji_to_kana("kitte").as_deref(), Some("きって"));
        assert_eq!(romaji_to_kana("kanji").as_deref(), Some("かんじ"));
        assert_eq!(romaji_to_kana("konnnichiha").as_deref(), Some("こんにちは"));
        assert_eq!(romaji_to_kana("hon").as_deref(), Some("ほん"));
        assert_eq!(romaji_to_kana("ra-men").as_deref(), Some("らーめん"));
        assert_eq!(romaji_to_kana("matcha"), None);
        assert_eq!(romaji_to_kana("arigatpou"), None);
    }

    #[test]
    fn かなを訓令式とヘボン式のローマ字に戻す() {
        let chars: Vec<char> = "しゃっちょうさんふじ".chars().collect();
        let tokens = kana_tokens(&chars).unwrap();
        assert_eq!(kana_to_romaji(&tokens, false, false), "syattyousannhuzi");
        assert_eq!(kana_to_romaji(&tokens, true, false), "shacchousannfuji");
        assert_eq!(kana_to_romaji(&tokens, true, true), "shacchousanfuji");
        // ん の後が母音・や行・な行なら n 1 打にしない
        let chars: Vec<char> = "きんようびんな".chars().collect();
        let tokens = kana_tokens(&chars).unwrap();
        assert_eq!(kana_to_romaji(&tokens, false, true), "kinnyoubinnna");
        // っ が続けば後ろの っ と同じ子音を重ね、重ねられなければ xtu
        let chars: Vec<char> = "いっったっ".chars().collect();
        let tokens = kana_tokens(&chars).unwrap();
        assert_eq!(kana_to_romaji(&tokens, false, false), "itttaxtu");
        assert!(readings("いっった").contains(&"いった".to_string()));
    }

    #[test]
    fn 隣のキーは同じ段の左右と上下の段の隣接するキー() {
        let mut keys = neighbor_keys(b's');
        keys.sort();
        assert_eq!(keys, b"adewxz".to_vec());
        let mut keys = neighbor_keys(b'a');
        keys.sort();
        assert_eq!(keys, b"qswz".to_vec());
    }

    #[test]
    fn 入れ替えた2打を戻す() {
        // onegaisimasu の ai を ia と打った
        assert!(readings("おねぎあします").contains(&"おねがいします".to_string()));
        assert!(readings("よろしくおねぎあします").contains(&"よろしくおねがいします".to_string()));
    }

    #[test]
    fn ありがとうの各誤りから元の読みを作る() {
        // 余分な 1 打 (arigaatou・arrigatou)、抜けた 1 打 (arigatu・arigaou)、
        // 隣のキー (arigayou・arigatiu)、入れ替え (arigaotu) をかなにしたもの
        for typed in ["ありがあとう", "あっりがとう", "ありがつ", "ありがおう", "ありがよう", "ありがちう", "ありがおつ"] {
            assert!(
                readings(typed).contains(&"ありがとう".to_string()),
                "{typed}: {:?}",
                readings(typed)
            );
        }
    }

    #[test]
    fn 先頭のかなは変えない() {
        assert!(readings("あいがとう").contains(&"ありがとう".to_string()));
        assert!(readings("ありがとう").iter().all(|r| r.starts_with('あ')));
        // sarigatou の余分な s を消すと ありがとう になるが、先頭のかなが変わるので作らない
        assert!(!readings("さりがとう").contains(&"ありがとう".to_string()));
    }

    #[test]
    fn 変わった所の範囲を返す() {
        let all = corrections("おねぎあします");
        let c = all.iter().find(|c| c.reading == "おねがいします").unwrap();
        assert_eq!((c.start, c.typed_end, c.corrected_end), (2, 4, 4));
        // 余分なかなを消す補正は、補正後の読みでの範囲が空になる
        let all = corrections("ありがあとう");
        let c = all.iter().find(|c| c.reading == "ありがとう").unwrap();
        assert_eq!((c.start, c.typed_end, c.corrected_end), (3, 4, 3));
    }

    #[test]
    fn 三文字未満の読みでは作らない() {
        assert!(corrections("あり").is_empty());
        assert!(!corrections("ありが").is_empty());
    }

    #[test]
    fn かな以外を含む読みはかなの連続の中だけで作る() {
        let all = readings("おねぎあabcします");
        assert!(all.contains(&"おねがいabcします".to_string()));
        assert!(all.iter().all(|r| r.contains("abc")));
    }

    #[test]
    fn 元の読みと同じものとかなにならないものは作らない() {
        let all = readings("ありがとう");
        assert!(!all.contains(&"ありがとう".to_string()));
        assert!(all.iter().all(|r| r.chars().all(is_kana)));
    }
}
