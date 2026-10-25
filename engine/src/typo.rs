// タイプミス補正: 1 打の誤りを戻した読みの生成 (docs/spec/engine.md「補正した読み」) と、
// 打鍵の上でずれを測る照合 (docs/spec/suggest.md「打鍵の受け渡し」「ずれのモデル」)
//
// 打鍵 (TSF 層が送るかな 1 字ごとの打鍵) があればその上で、無ければかなを標準のローマ字に戻した
// 近似の上で、1 打の誤り (余分・抜け・隣のキー・入れ替え) を戻してから、TSF 層と同じ規則でかなに直す。
// ローマ字の表は TSF 層の既定のテーブル (tsf/src/romaji.cpp kDefaultTable) と同じ内容。
// ユーザのローマ字テーブルはエンジンから見えないため、run の打鍵が作ったかなの綴りを表に足して使う

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
    /// 戻した誤りのコスト (同じ読みになる誤りが複数あれば安いほう)
    pub cost: i32,
}

struct Tables {
    to_kana: HashMap<&'static str, &'static str>,
    /// テーブルのキーの真の接頭辞 (続きの入力を待つかの判定)
    prefixes: HashSet<&'static str>,
    /// かな → (訓令式, ヘボン式) の綴り
    to_romaji: HashMap<&'static str, (&'static str, &'static str)>,
    /// かな → 表のすべての綴り (照合の標準の綴り)
    spellings: HashMap<&'static str, Vec<&'static str>>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut to_kana = HashMap::new();
        let mut prefixes = HashSet::new();
        let mut to_romaji: HashMap<&'static str, (&'static str, &'static str)> = HashMap::new();
        let mut spellings: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        spellings.insert("ー", vec!["-"]);
        for &(key, kana) in ROMAJI_TABLE {
            to_kana.insert(key, kana);
            spellings.entry(kana).or_default().push(key);
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
        Tables { to_kana, prefixes, to_romaji, spellings }
    })
}

fn is_vowel(c: u8) -> bool {
    matches!(c, b'a' | b'i' | b'u' | b'e' | b'o')
}

/// ローマ字をかなに直す (TSF 層の RomajiComposer::Convert と同じ規則。末尾の n は確定時と同じく
/// ん にする)。かなにならない打鍵が残れば None
pub fn romaji_to_kana(romaji: &str) -> Option<String> {
    convert_romaji(romaji, &CustomTable::default())
}

/// 計測用 (examples/fuzzy_eval.rs): 打鍵 romaji を TSF 層の RomajiComposer と同じ規則でかなにし、
/// SUGGEST の打鍵のフィールド (docs/spec/suggest.md「打鍵の受け渡し」) とともに (かな, フィールド) を返す。
/// 末尾の変換できない打鍵は未完成のローマ字として残す (n も ん にしない)。途中でかなにならない打鍵が
/// あれば None
#[allow(dead_code)]
pub fn compose(romaji: &str) -> Option<(String, String)> {
    let t = tables();
    let mut kana = String::new();
    let mut elements: Vec<String> = Vec::new();
    let mut pending = String::new();
    let mut push = |k: &str, keys: &str, kana: &mut String| {
        for (i, c) in k.chars().enumerate() {
            kana.push(c);
            elements.push(if i == 0 { keys.to_string() } else { String::new() });
        }
    };
    for c in romaji.chars() {
        if c == '-' {
            if pending == "n" {
                push("ん", "n", &mut kana);
                pending.clear();
            } else if !pending.is_empty() {
                return None;
            }
            push("ー", "-", &mut kana);
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
                push(k, &pending, &mut kana);
                pending.clear();
                break;
            }
            if t.prefixes.contains(pending.as_str()) {
                break;
            }
            let bytes = pending.as_bytes();
            if bytes.len() >= 2 && bytes[0] == bytes[1] && !is_vowel(bytes[0]) && bytes[0] != b'n' {
                let first = pending.remove(0);
                push("っ", &first.to_string(), &mut kana);
                continue;
            }
            if bytes[0] == b'n' && bytes.len() >= 2 {
                pending.remove(0);
                push("ん", "n", &mut kana);
                continue;
            }
            return None;
        }
    }
    let field = format!("{}{PENDING_SEPARATOR}{pending}", elements.join(&ELEMENT_SEPARATOR.to_string()));
    Some((kana, field))
}

/// run の打鍵が作ったかなのうち標準の綴りに無いもの (romaji.tsv で変えた綴り)。
/// 表の綴りより先に引く
#[derive(Default)]
struct CustomTable {
    /// 打鍵 → かな
    to_kana: HashMap<String, String>,
    /// 打鍵の真の接頭辞
    prefixes: HashSet<String>,
}

impl CustomTable {
    fn insert(&mut self, keys: &str, kana: &str) {
        for (end, _) in keys.char_indices().skip(1) {
            self.prefixes.insert(keys[..end].to_string());
        }
        self.to_kana.insert(keys.to_string(), kana.to_string());
    }

    fn knows(&self, pending: &str) -> bool {
        self.to_kana.contains_key(pending) || self.prefixes.contains(pending)
    }
}

/// romaji_to_kana の本体。custom の綴りを表より先に引く
fn convert_romaji(romaji: &str, custom: &CustomTable) -> Option<String> {
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
        let mut extended = pending.clone();
        extended.push(c);
        if c == '-' && !custom.knows(&extended) {
            if !flush(&mut pending, &mut kana) {
                return None;
            }
            kana.push('ー');
            continue;
        }
        if !c.is_ascii_lowercase() && !custom.knows(&extended) {
            return None;
        }
        pending.push(c);
        loop {
            if pending.is_empty() {
                break;
            }
            if let Some(k) = custom.to_kana.get(pending.as_str()) {
                kana.push_str(k);
                pending.clear();
                break;
            }
            if let Some(k) = t.to_kana.get(pending.as_str()) {
                kana.push_str(k);
                pending.clear();
                break;
            }
            if t.prefixes.contains(pending.as_str()) || custom.prefixes.contains(pending.as_str()) {
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

//// 誤りの種類 (打った側から見た誤り)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// QWERTY で隣のキーに置き換わった
    Adjacent,
    /// 余分な 1 打
    Extra,
    /// 1 打の抜け
    Missing,
    /// 隣り合う 2 打の入れ替え
    Swap,
}

/// ずれのコスト・許すずれの数・全体変換の補正の候補のしきい値・予測の点数の補正
/// (docs/spec/suggest.md「ずれのモデル」「点数」、docs/spec/engine.md「全体変換の補正」)。計測 (examples/fuzzy_eval.rs) で値を振るため、
/// 定数を 1 つの値にまとめて渡す
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub edit_adjacent: i32,
    pub edit_extra: i32,
    pub edit_missing: i32,
    pub edit_swap: i32,
    /// 許すずれ 1 つあたりの打鍵の数
    pub keys_per_edit: usize,
    /// 許すずれの数の上限
    pub max_edits: usize,
    /// 全体変換の補正の候補を出す Δ の下限
    pub typo_show: f64,
    /// 全体変換の補正の候補を先頭に置く Δ の下限
    pub typo_top: f64,
    /// 辞書の語・ユーザ辞書の名詞系の語の予測の点数に足す量。打ち終えた読みの経路は、長い語の予測より
    /// 安く出やすいため
    pub predict_bias: i64,
}

pub const TUNING: Tuning = Tuning {
    edit_adjacent: 3000,
    edit_extra: 3500,
    edit_missing: 3500,
    edit_swap: 3000,
    keys_per_edit: 4,
    max_edits: 3,
    typo_show: -50.0,
    typo_top: 0.0,
    predict_bias: 2000,
};

impl Tuning {
    pub fn cost(&self, kind: EditKind) -> i32 {
        match kind {
            EditKind::Adjacent => self.edit_adjacent,
            EditKind::Extra => self.edit_extra,
            EditKind::Missing => self.edit_missing,
            EditKind::Swap => self.edit_swap,
        }
    }

    /// 打鍵の数 keys (未完成のローマ字を含む) から許すずれの数
    pub fn allowed_edits(&self, keys: usize) -> usize {
        if self.keys_per_edit == 0 {
            return 0;
        }
        self.max_edits.min(keys / self.keys_per_edit)
    }
}

/// romaji から 1 打の誤りを戻した打鍵列を、戻した誤りの種類とともに列挙する。
/// min_pos より前の打鍵には誤りを認めない
fn romaji_edits(romaji: &str, min_pos: usize, out: &mut Vec<(String, EditKind)>) {
    let bytes = romaji.as_bytes();
    let n = bytes.len();
    let mut push = |v: Vec<u8>, kind: EditKind| {
        if let Ok(s) = String::from_utf8(v) {
            out.push((s, kind));
        }
    };
    // 余分な 1 打を消す
    for i in min_pos..n {
        let mut v = bytes.to_vec();
        v.remove(i);
        push(v, EditKind::Extra);
    }
    // 抜けた 1 打を足す (よく抜ける字と、隣の字の重ね)
    for i in min_pos..=n {
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
            push(v, EditKind::Missing);
        }
    }
    // 隣のキーに置き換える
    for i in min_pos..n {
        for c in neighbor_keys(bytes[i]) {
            let mut v = bytes.to_vec();
            v[i] = c;
            push(v, EditKind::Adjacent);
        }
    }
    // 隣り合う 2 打を入れ替える
    for i in min_pos..n.saturating_sub(1) {
        if bytes[i] != bytes[i + 1] {
            let mut v = bytes.to_vec();
            v.swap(i, i + 1);
            push(v, EditKind::Swap);
        }
    }
}

/// 補正の対象にするかな (ひらがなと長音記号)
fn is_kana(c: char) -> bool {
    ('ぁ'..='ゖ').contains(&c) || c == 'ー'
}

/// 補正した読みを集める。同じ読みは 1 件にし、コストは安いほうにする
struct CorrectionSet<'a> {
    chars: &'a [char],
    index: HashMap<String, usize>,
    result: Vec<Correction>,
}

impl<'a> CorrectionSet<'a> {
    fn new(chars: &'a [char]) -> Self {
        CorrectionSet { chars, index: HashMap::new(), result: Vec::new() }
    }

    /// かなの連続 chars[run_start..run_end] を fixed に置き換えた読みを足す。元と同じ読み・
    /// 先頭のかなが変わる読みは足さない
    fn push(&mut self, run_start: usize, run_end: usize, fixed: &[char], cost: i32) {
        let chars = self.chars;
        let n = chars.len();
        if fixed == &chars[run_start..run_end] {
            return;
        }
        let mut full: Vec<char> = Vec::with_capacity(n + 2);
        full.extend_from_slice(&chars[..run_start]);
        full.extend_from_slice(fixed);
        full.extend_from_slice(&chars[run_end..]);
        if full.first() != chars.first() {
            return;
        }
        let reading: String = full.iter().collect();
        if let Some(&i) = self.index.get(&reading) {
            self.result[i].cost = self.result[i].cost.min(cost);
            return;
        }
        let prefix = chars.iter().zip(&full).take_while(|(a, b)| a == b).count();
        let suffix = chars[prefix..]
            .iter()
            .rev()
            .zip(full[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        self.index.insert(reading.clone(), self.result.len());
        self.result.push(Correction {
            reading,
            start: prefix,
            typed_end: n - suffix,
            corrected_end: full.len() - suffix,
            cost,
        });
    }

    fn finish(mut self) -> Vec<Correction> {
        // 生成の順に依らない結果にする (同じコストの語の選び方を毎回同じにするため)
        self.result.sort_by(|a, b| a.reading.cmp(&b.reading));
        self.result
    }
}

/// 打ったかな kana の 1 打の誤りを戻した読みを返す (補正後の読みで重複を除く)。打鍵が無いときの
/// 近似で、かなを標準のローマ字 (訓令式・ヘボン式) に戻して作る。先頭のかなは変えず、
/// MIN_TYPO_CHARS 文字未満の読みでは作らない。かな以外を含む読みでは、かなの連続の中だけで作る。
/// コストは戻した誤りの種類の tuning のコスト
pub fn corrections(kana: &str, tuning: &Tuning) -> Vec<Correction> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n < MIN_TYPO_CHARS {
        return Vec::new();
    }
    let mut set = CorrectionSet::new(&chars);
    let mut run_start = 0;
    while run_start < n {
        if !is_kana(chars[run_start]) {
            run_start += 1;
            continue;
        }
        let run_end = (run_start..n).find(|&i| !is_kana(chars[i])).unwrap_or(n);
        let run = &chars[run_start..run_end];
        if let Some(tokens) = kana_tokens(run) {
            let mut edits: Vec<(String, EditKind)> = Vec::new();
            let mut variants: Vec<String> = Vec::new();
            for hepburn in [false, true] {
                for short_n in [false, true] {
                    let romaji = kana_to_romaji(&tokens, hepburn, short_n);
                    // 戻した綴りがかなに戻らないなら、その綴りからは作らない
                    if !variants.contains(&romaji)
                        && romaji_to_kana(&romaji).is_some_and(|k| k.chars().eq(run.iter().copied()))
                    {
                        romaji_edits(&romaji, 0, &mut edits);
                        variants.push(romaji);
                    }
                }
            }
            for (edit, kind) in edits {
                let Some(fixed) = romaji_to_kana(&edit) else {
                    continue;
                };
                let fixed: Vec<char> = fixed.chars().collect();
                set.push(run_start, run_end, &fixed, tuning.cost(kind));
            }
        }
        run_start = run_end;
    }
    set.finish()
}

/// 打鍵 keys の上で 1 打の誤りを戻した読みを返す。かなへの直し方は、標準の表に run の打鍵の綴りを
/// 足したもの。ほかの規則は corrections と同じで、先頭のかなの打鍵には誤りを認めない。
/// 未完成のローマ字は読みに含まれないので、そこには誤りを探さない
pub fn key_corrections(kana: &str, keys: &Keys, tuning: &Tuning) -> Vec<Correction> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n < MIN_TYPO_CHARS || chars != keys.kana {
        return Vec::new();
    }
    let mut set = CorrectionSet::new(&chars);
    let clusters = keys.clusters();
    let all_kana = |&(s, e): &(usize, usize)| chars[s..e].iter().all(|&c| is_kana(c));
    let mut c = 0;
    while c < clusters.len() {
        if !all_kana(&clusters[c]) {
            c += 1;
            continue;
        }
        let mut e = c;
        while e < clusters.len() && all_kana(&clusters[e]) {
            e += 1;
        }
        let (run_start, run_end) = (clusters[c].0, clusters[e - 1].1);
        c = e;
        let romaji: String = keys.keys[keys.starts[run_start]..keys.starts[run_end]].iter().collect();
        let run = &chars[run_start..run_end];
        // 打鍵から run のかなに戻らないとき (表で決まらない綴りが混ざったとき) は作らない
        if !convert_romaji(&romaji, &keys.custom_table)
            .is_some_and(|k| k.chars().eq(run.iter().copied()))
        {
            continue;
        }
        let min_pos = if run_start == 0 { keys.first_len() } else { 0 };
        let mut edits: Vec<(String, EditKind)> = Vec::new();
        romaji_edits(&romaji, min_pos, &mut edits);
        for (edit, kind) in edits {
            let Some(fixed) = convert_romaji(&edit, &keys.custom_table) else {
                continue;
            };
            let fixed: Vec<char> = fixed.chars().collect();
            set.push(run_start, run_end, &fixed, tuning.cost(kind));
        }
    }
    set.finish()
}

/// 打鍵のフィールドの、かな 1 字ごとの要素の区切り (ASCII Unit Separator) と、
/// 要素と未完成のローマ字の区切り (ASCII Record Separator)
const ELEMENT_SEPARATOR: char = '\x1f';
const PENDING_SEPARATOR: char = '\x1e';

/// かなの打鍵 (docs/spec/suggest.md「打鍵の受け渡し」)。打鍵の無いかなは標準の綴りで補ってある
pub struct Keys {
    /// 送られたかな
    kana: Vec<char>,
    /// かな 1 字ごとの要素を連結し、未完成のローマ字を後ろに足した打鍵
    keys: Vec<char>,
    /// かな i 字目の要素の始まり (keys の位置)。末尾 (kana.len() 番目) は未完成のローマ字の始まり
    starts: Vec<usize>,
    /// run の打鍵が作ったかなのかたまりのうち、標準の綴りに無いもの: かな → 打鍵
    custom: HashMap<String, Vec<String>>,
    /// custom の打鍵 → かな
    custom_table: CustomTable,
}

impl Keys {
    /// TSF 層が送る打鍵のフィールドを解釈する。要素の数がかなの文字数と合わなければ Err
    pub fn parse(kana: &str, field: &str) -> Result<Keys, String> {
        let chars: Vec<char> = kana.chars().collect();
        let (elements, pending) = field.split_once(PENDING_SEPARATOR).unwrap_or((field, ""));
        let elements: Vec<&str> = elements.split(ELEMENT_SEPARATOR).collect();
        if elements.len() != chars.len() {
            return Err(format!(
                "打鍵の要素の数 ({}) がかなの文字数 ({}) と合いません",
                elements.len(),
                chars.len()
            ));
        }
        let typed: Vec<Option<&str>> =
            elements.iter().map(|e| (!e.is_empty() && e.is_ascii()).then_some(*e)).collect();
        // ASCII でない要素 (文書から読んだかな) は打鍵なし。空の要素は前のかたまりの続き
        let mut owner: Vec<Option<usize>> = vec![None; chars.len()];
        for i in 0..chars.len() {
            owner[i] = if typed[i].is_some() {
                Some(i)
            } else if elements[i].is_empty() && i > 0 {
                owner[i - 1]
            } else {
                None
            };
        }
        let pending: Vec<char> =
            if pending.is_ascii() { pending.chars().collect() } else { Vec::new() };
        Ok(Keys::build(chars, &typed, &owner, pending))
    }

    /// 打鍵の無いかな kana を標準の綴りで補った打鍵
    pub fn standard(kana: &str) -> Keys {
        let chars: Vec<char> = kana.chars().collect();
        let n = chars.len();
        Keys::build(chars, &vec![None; n], &vec![None; n], Vec::new())
    }

    /// owner[i] は i 字目のかなを作った打鍵の要素の位置 (打鍵が無ければ None)
    fn build(kana: Vec<char>, typed: &[Option<&str>], owner: &[Option<usize>], pending: Vec<char>) -> Keys {
        let n = kana.len();
        let mut elements: Vec<String> = vec![String::new(); n];
        let mut custom: HashMap<String, Vec<String>> = HashMap::new();
        let mut custom_table = CustomTable::default();
        let mut i = 0;
        while i < n {
            if let Some(start) = owner[i] {
                let end = (i + 1..n).find(|&j| owner[j] != Some(start)).unwrap_or(n);
                let keys = typed[start].unwrap_or_default();
                elements[start] = keys.to_string();
                let token: String = kana[start..end].iter().collect();
                if !is_standard_spelling(&token, keys) {
                    let list = custom.entry(token.clone()).or_default();
                    if !list.iter().any(|k| k == keys) {
                        list.push(keys.to_string());
                    }
                    custom_table.insert(keys, &token);
                }
                i = end;
                continue;
            }
            let end = (i..n).find(|&j| owner[j].is_some()).unwrap_or(n);
            // っ で終わる並びは、後ろに続く打鍵の 1 打目を重ねた子音で補う
            let next = match owner.get(end).copied().flatten() {
                Some(o) => typed[o].and_then(|k| k.chars().next()),
                None => pending.first().copied(),
            };
            for (offset, spelling) in fill(&kana[i..end], next).into_iter().enumerate() {
                elements[i + offset] = spelling;
            }
            i = end;
        }
        let mut keys: Vec<char> = Vec::new();
        let mut starts: Vec<usize> = Vec::with_capacity(n + 1);
        for element in &elements {
            starts.push(keys.len());
            keys.extend(element.chars());
        }
        starts.push(keys.len());
        keys.extend(pending);
        Keys { kana, keys, starts, custom, custom_table }
    }

    /// 打鍵の数 (未完成のローマ字を含む)
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// 先頭のかなのかたまりの打鍵の数 (ここまでの打鍵には誤りを認めない)
    fn first_len(&self) -> usize {
        self.clusters().first().map_or(0, |&(_, end)| self.starts[end])
    }

    /// かなのかたまり (要素が空でないかなと、それに続く要素が空のかな) の [始まり, 終わり) の並び
    fn clusters(&self) -> Vec<(usize, usize)> {
        let mut result: Vec<(usize, usize)> = Vec::new();
        for i in 0..self.kana.len() {
            if i == 0 || self.starts[i + 1] > self.starts[i] {
                result.push((i, i + 1));
            } else if let Some(last) = result.last_mut() {
                last.1 = i + 1;
            }
        }
        result
    }
}

/// keys が token の標準の綴りか。っ の子音 1 打と ん の n は、次のかなで決まる標準の綴りの一部なので、
/// run の綴りとして足さない (足すと次のかなによらず一致してしまう)
fn is_standard_spelling(token: &str, keys: &str) -> bool {
    if tables().spellings.get(token).is_some_and(|list| list.contains(&keys)) {
        return true;
    }
    let mut it = keys.chars();
    let single = it.next().filter(|_| it.next().is_none());
    match token {
        "っ" => single.is_some_and(|c| c.is_ascii_lowercase() && !is_vowel(c as u8) && c != 'n'),
        "ん" => keys == "n",
        _ => token == keys,
    }
}

/// 打鍵の無いかなの並び chars を標準の綴り (訓令式、ん は nn) で補う。戻り値はかな 1 字ごとの要素で、
/// 2 文字のかなは 1 字目に綴りを付けて 2 字目を空にする (TSF 層の要素と同じ形)。next は後ろに続く
/// 打鍵の 1 打目 (っ の子音)。表に無い文字はその文字自身を打鍵とする
fn fill(chars: &[char], next: Option<char>) -> Vec<String> {
    let t = tables();
    let mut tokens: Vec<(usize, Option<&'static str>)> = Vec::new();
    let mut i = 0;
    let mut buf = String::new();
    while i < chars.len() {
        let mut found = None;
        for len in [2, 1] {
            if i + len > chars.len() {
                continue;
            }
            buf.clear();
            buf.extend(&chars[i..i + len]);
            if buf == "ー" {
                found = Some((len, "-"));
                break;
            }
            if let Some(&(kunrei, _)) = t.to_romaji.get(buf.as_str()) {
                found = Some((len, kunrei));
                break;
            }
        }
        let (len, spelling) = found.map_or((1, None), |(len, s)| (len, Some(s)));
        tokens.push((i, spelling));
        i += len;
    }
    let mut result: Vec<String> = vec![String::new(); chars.len()];
    let mut following = next;
    for &(start, spelling) in tokens.iter().rev() {
        let part: String = match spelling {
            Some("xtu") => match following {
                Some(c) if c.is_ascii_lowercase() && !is_vowel(c as u8) && c != 'n' => c.to_string(),
                _ => "xtu".to_string(),
            },
            Some(s) => s.to_string(),
            None => chars[start].to_string(),
        };
        following = part.chars().next();
        result[start] = part;
    }
    result
}

//// 照合の DP の到達できない値
const INF: i32 = i32::MAX;

/// 照合の DP の 1 行 (読みのある位置まで綴りを進めたときの状態)。
/// v[j * (k + 1) + e] は打鍵の j 打目までを、ずれ e 個で照合した最小のずれのコスト。
/// v の後半 (同じ並び) は入れ替えの途中 (j 打目を後回しにして j + 1 打目を照合した) 状態。
/// [lo, hi] の外の j の値は常に INF にしておき、行の操作をこの範囲だけで済ませる
#[derive(Clone)]
pub struct Row {
    v: Vec<i32>,
    /// 1 つの j あたりの値の数 (k + 1)
    stride: usize,
    /// 値のありうる j の範囲 [lo, hi]。lo > hi なら空
    lo: usize,
    hi: usize,
}

impl Row {
    fn empty(size: usize, stride: usize) -> Row {
        Row { v: vec![INF; size * 2], stride, lo: usize::MAX, hi: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.lo > self.hi
    }

    /// [lo, hi] の値の位置 (前半と後半)
    fn band(&self) -> [std::ops::Range<usize>; 2] {
        let half = self.v.len() / 2;
        let range = self.lo * self.stride..(self.hi + 1) * self.stride;
        [range.clone(), range.start + half..range.end + half]
    }

    fn clear(&mut self) {
        if !self.is_empty() {
            for range in self.band() {
                self.v[range].fill(INF);
            }
        }
        self.lo = usize::MAX;
        self.hi = 0;
    }

    fn copy_from(&mut self, other: &Row) {
        self.clear();
        if !other.is_empty() {
            for range in other.band() {
                self.v[range.clone()].copy_from_slice(&other.v[range]);
            }
        }
        self.lo = other.lo;
        self.hi = other.hi;
    }

    fn touch(&mut self, j: usize) {
        self.lo = self.lo.min(j);
        self.hi = self.hi.max(j);
    }

    fn merge(&mut self, other: &Row) {
        if other.is_empty() {
            return;
        }
        for range in other.band() {
            for (a, b) in self.v[range.clone()].iter_mut().zip(&other.v[range]) {
                *a = (*a).min(*b);
            }
        }
        self.lo = self.lo.min(other.lo);
        self.hi = self.hi.max(other.hi);
    }
}

/// 打鍵を使い切った位置での (ずれのコスト, ずれの数)。使い切れなければ None
pub type Consumed = Option<(i32, usize)>;

fn better(a: Consumed, b: Consumed) -> Consumed {
    match (a, b) {
        (Some(x), Some(y)) => Some(if y.0 < x.0 { y } else { x }),
        (x, None) => x,
        (None, y) => y,
    }
}

/// step の作業用の行
struct Scratch {
    cur: Row,
    next: Row,
}

/// 打鍵と読みの照合 (docs/spec/suggest.md「ずれのモデル」)。読みのかなの綴りは表の綴りに run の打鍵の
/// 綴りを足した集合で、打鍵の末尾は読みの途中 (かなの綴りの途中) で終わってよい
pub struct Matcher<'a> {
    keys: &'a Keys,
    /// 許すずれの数
    k: usize,
    /// 先頭のかなのかたまりの打鍵の数
    l0: usize,
    /// 隣のキー・余分・抜け・入れ替えのコスト
    costs: [i32; 4],
    /// 綴りを引くかなのかたまりの最大の文字数
    max_token: usize,
    adjacent: Box<[[bool; 26]; 26]>,
    /// これより高いずれのコストの状態は捨てる (呼び出し側が、もう要らないコストを知ったときに下げる)
    bound: std::cell::Cell<i32>,
    scratch: std::cell::RefCell<Scratch>,
    /// match_reading で照合が尽きた読みの先頭 2 字
    dead_heads: std::cell::RefCell<HashSet<(char, char)>>,
}

impl<'a> Matcher<'a> {
    /// edits は許すずれの数 (タイプミス補正が無効なら 0)
    pub fn new(keys: &'a Keys, edits: usize, tuning: &Tuning) -> Matcher<'a> {
        let mut adjacent = Box::new([[false; 26]; 26]);
        for a in b'a'..=b'z' {
            for b in neighbor_keys(a) {
                adjacent[usize::from(a - b'a')][usize::from(b - b'a')] = true;
            }
        }
        let max_token = keys.custom.keys().map(|t| t.chars().count()).max().unwrap_or(0).max(2);
        let size = (keys.keys.len() + 1) * (edits + 1);
        Matcher {
            keys,
            k: edits,
            l0: keys.first_len(),
            costs: [tuning.edit_adjacent, tuning.edit_extra, tuning.edit_missing, tuning.edit_swap],
            max_token,
            adjacent,
            bound: std::cell::Cell::new(INF - 1),
            scratch: std::cell::RefCell::new(Scratch {
                cur: Row::empty(size, edits + 1),
                next: Row::empty(size, edits + 1),
            }),
            dead_heads: std::cell::RefCell::new(HashSet::new()),
        }
    }

    /// これより高いずれのコストの状態を捨てる
    pub fn set_bound(&self, bound: i32) {
        self.bound.set(bound.min(INF - 1));
    }

    fn size(&self) -> usize {
        (self.keys.keys.len() + 1) * (self.k + 1)
    }

    fn at(&self, j: usize, e: usize) -> usize {
        j * (self.k + 1) + e
    }

    fn is_adjacent(&self, a: char, b: char) -> bool {
        a.is_ascii_lowercase()
            && b.is_ascii_lowercase()
            && self.adjacent[a as usize - 'a' as usize][b as usize - 'a' as usize]
    }

    /// 読みの始まり (0 字目) の行
    pub fn start(&self) -> Row {
        let mut row = Row::empty(self.size(), self.k + 1);
        row.v[0] = 0;
        row.touch(0);
        self.extra_closure(&mut row);
        row
    }

    /// 余分な打鍵を読み飛ばす遷移を row の中で閉じる
    fn extra_closure(&self, row: &mut Row) {
        let t = self.keys.keys.len();
        if self.k == 0 || row.is_empty() {
            return;
        }
        let bound = self.bound.get();
        let mut j = row.lo.max(self.l0);
        while j < t && j <= row.hi {
            for e in 0..self.k {
                let c = row.v[self.at(j, e)];
                if c == INF {
                    continue;
                }
                let next = self.at(j + 1, e + 1);
                let v = c + self.costs[1];
                if v <= bound && v < row.v[next] {
                    row.v[next] = v;
                    row.touch(j + 1);
                }
            }
            j += 1;
        }
    }

    /// 綴りの 1 字 a を cur から進めて next に書く
    fn advance(&self, cur: &Row, a: char, next: &mut Row) {
        next.clear();
        if cur.is_empty() {
            return;
        }
        let keys = &self.keys.keys;
        let t = keys.len();
        let k = self.k;
        let size = self.size();
        let bound = self.bound.get();
        let relax = |row: &mut Row, j: usize, e: usize, v: i32, swap: bool| {
            if v > bound {
                return;
            }
            let i = j * (k + 1) + e + if swap { size } else { 0 };
            if v < row.v[i] {
                row.v[i] = v;
                row.touch(j);
            }
        };
        for j in cur.lo..=cur.hi.min(t) {
            for e in 0..=k {
                let c = cur.v[self.at(j, e)];
                if c != INF {
                    if j < t {
                        if keys[j] == a {
                            relax(next, j + 1, e, c, false);
                        } else if j >= self.l0 && e < k && self.is_adjacent(keys[j], a) {
                            relax(next, j + 1, e + 1, c + self.costs[0], false);
                        }
                        if j + 1 < t && j >= self.l0 && e < k && keys[j + 1] == a && keys[j] != a {
                            relax(next, j, e + 1, c + self.costs[3], true);
                        }
                    }
                    if j >= self.l0 && e < k {
                        relax(next, j, e + 1, c + self.costs[2], false);
                    }
                }
                let w = cur.v[size + self.at(j, e)];
                if w != INF && j + 1 < t && keys[j] == a {
                    relax(next, j + 2, e, w, false);
                }
            }
        }
        self.extra_closure(next);
    }

    /// 打鍵を使い切った状態の最小のずれ
    fn consumed(&self, row: &Row) -> Consumed {
        let t = self.keys.keys.len();
        if row.is_empty() || row.hi < t {
            return None;
        }
        (0..=self.k)
            .filter_map(|e| {
                let c = row.v[self.at(t, e)];
                (c != INF).then_some((c, e))
            })
            .min()
    }

    /// 打鍵を使い切った状態を消す。そこから先は抜けとしてしか進めず、使い切った位置の前方一致に
    /// 含まれる候補を高いコストで数え直すだけになるため
    fn drop_consumed(&self, row: &mut Row) {
        let t = self.keys.keys.len();
        if row.is_empty() || row.hi < t {
            return;
        }
        for e in 0..=self.k {
            let i = self.at(t, e);
            row.v[i] = INF;
        }
        let size = self.size();
        let live = |j: usize| (0..=self.k).any(|e| row.v[self.at(j, e)] != INF || row.v[size + self.at(j, e)] != INF);
        match (row.lo..=row.hi).rev().find(|&j| live(j)) {
            Some(hi) => row.hi = hi,
            None => {
                row.lo = usize::MAX;
                row.hi = 0;
            }
        }
    }

    /// 綴り (first があればその字を前に付けたもの) を start の行から進めて out に合わせる。綴りの途中で
    /// 打鍵を使い切った状態を mid に集める
    fn run(&self, start: &Row, first: Option<char>, spelling: &str, out: &mut Row, mid: &mut Consumed, s: &mut Scratch) {
        if start.is_empty() {
            return;
        }
        s.cur.copy_from(start);
        let mut chars = first.into_iter().chain(spelling.chars()).peekable();
        while let Some(a) = chars.next() {
            self.advance(&s.cur, a, &mut s.next);
            std::mem::swap(&mut s.cur, &mut s.next);
            if s.cur.is_empty() {
                return;
            }
            if chars.peek().is_some() {
                *mid = better(*mid, self.consumed(&s.cur));
                self.drop_consumed(&mut s.cur);
            }
        }
        out.merge(&s.cur);
    }

    /// かなのかたまり token の綴り (表の綴りと run の打鍵の綴り) ごとに f を呼ぶ。どちらにも無い
    /// 1 文字はその文字自身を綴りとする。綴りが無ければ false
    fn for_each_spelling(&self, token: &str, mut f: impl FnMut(&str)) -> bool {
        let mut any = false;
        if let Some(list) = tables().spellings.get(token) {
            for s in list {
                f(s);
            }
            any = true;
        }
        if let Some(list) = self.keys.custom.get(token) {
            for s in list {
                f(s);
            }
            any = true;
        }
        if !any && token.chars().nth(1).is_none() {
            f(token);
            any = true;
        }
        any
    }

    /// 読み path の最後の字までの行を返す。rows[d] は読みの d 字目までの行 (rows.len() == path.len())。
    /// 2 つ目はこの字までで打鍵を使い切った最小のずれ (この字のかなの綴りの途中で使い切ったものを含む)。
    /// 使い切った状態は返す行から消してある
    pub fn step(&self, path: &[char], rows: &[Row]) -> (Row, Consumed) {
        let d = path.len();
        debug_assert_eq!(rows.len(), d);
        let mut out = Row::empty(self.size(), self.k + 1);
        let mut mid: Consumed = None;
        let mut scratch = self.scratch.borrow_mut();
        let s = &mut *scratch;
        let mut buf = [0u8; 64];
        for len in 1..=self.max_token.min(d) {
            let mut used = 0;
            for &c in &path[d - len..d] {
                used += c.encode_utf8(&mut buf[used..]).len();
            }
            let token = std::str::from_utf8(&buf[..used]).expect("文字を並べたもの");
            // っ は次のかなの子音の重ね、ん の n 1 打は次のかなが子音 (n・y 以外) のときだけ
            let prev = if d > len { Some(path[d - len - 1]) } else { None };
            let combined = matches!(prev, Some('っ' | 'ん')) && !rows[d - len - 1].is_empty();
            let from = &rows[d - len];
            if from.is_empty() && !combined {
                continue;
            }
            self.for_each_spelling(token, |spelling| {
                self.run(from, None, spelling, &mut out, &mut mid, s);
                if !combined {
                    return;
                }
                let Some(first) = spelling.chars().next() else { return };
                let consonant = first.is_ascii_lowercase() && !is_vowel(first as u8) && first != 'n';
                let before = &rows[d - len - 1];
                if prev == Some('っ') && consonant {
                    self.run(before, Some(first), spelling, &mut out, &mut mid, s);
                } else if prev == Some('ん') && consonant && first != 'y' {
                    self.run(before, Some('n'), spelling, &mut out, &mut mid, s);
                }
            });
        }
        let consumed = better(mid, self.consumed(&out));
        self.drop_consumed(&mut out);
        (out, consumed)
    }

    /// 直近の行から先の綴りがまだ進みうるか (かなのかたまりは最長で max_token + 1 字前の行から進む)
    pub fn alive(&self, rows: &[Row]) -> bool {
        rows[rows.len().saturating_sub(self.max_token + 1)..].iter().any(|r| !r.is_empty())
    }

    /// 読み path の最後の字で終わるかなのかたまりに、空でない行から始まるものがあるか (step を
    /// 計算する値があるか)。直前の行が空のとき、2 文字以上のかなのかたまりの 2 字目になれない字を
    /// 先に除いて、fst の子をすべて計算しないようにする
    pub fn reachable(&self, path: &[char], rows: &[Row]) -> bool {
        let d = path.len();
        if !rows[d - 1].is_empty() {
            return true;
        }
        // 1 文字のかなの前の っ・ん (直前の行は っ・ん の 1 字だけの綴りで空になりうる)
        if d >= 2 && matches!(path[d - 2], 'っ' | 'ん') && !rows[d - 2].is_empty() {
            return true;
        }
        let t = tables();
        let mut buf = [0u8; 64];
        for len in 2..=self.max_token.min(d) {
            let mut used = 0;
            for &c in &path[d - len..d] {
                used += c.encode_utf8(&mut buf[used..]).len();
            }
            let token = std::str::from_utf8(&buf[..used]).expect("文字を並べたもの");
            // run の打鍵の綴りのかなは 3 文字以上のこともあるので、その途中も先へ進める
            let known = t.spellings.contains_key(token)
                || self.keys.custom.keys().any(|c| c.starts_with(token));
            if !known {
                continue;
            }
            if !rows[d - len].is_empty() {
                return true;
            }
            if d > len && matches!(path[d - len - 1], 'っ' | 'ん') && !rows[d - len - 1].is_empty() {
                return true;
            }
        }
        false
    }

    /// 読み reading の先頭で打鍵を使い切る最小のずれ (前方一致の照合)。使い切れなければ None
    pub fn match_reading(&self, reading: &str) -> Consumed {
        let path: Vec<char> = reading.chars().collect();
        // 学習とユーザ辞書は読みを 1 件ずつ照合するので、先頭 2 字で照合が尽きた読みを覚えて、同じ先頭の
        // 読みを計算しない (かなのかたまりが 2 文字までなら、先頭 2 字の行で先が決まる)
        let head = (path.len() >= 2 && self.max_token == 2).then(|| (path[0], path[1]));
        if head.is_some_and(|h| self.dead_heads.borrow().contains(&h)) {
            return None;
        }
        let mut rows: Vec<Row> = vec![self.start()];
        let mut best: Consumed = None;
        for d in 1..=path.len() {
            if !self.reachable(&path[..d], &rows) {
                break;
            }
            let (row, consumed) = self.step(&path[..d], &rows);
            best = better(best, consumed);
            rows.push(row);
            if !self.alive(&rows) {
                break;
            }
        }
        if let Some(h) = head {
            if best.is_none() && (rows.len() <= 2 || rows[1..3].iter().all(Row::is_empty)) {
                self.dead_heads.borrow_mut().insert(h);
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(kana: &str) -> Vec<String> {
        corrections(kana, &TUNING).into_iter().map(|c| c.reading).collect()
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
        let all = corrections("おねぎあします", &TUNING);
        let c = all.iter().find(|c| c.reading == "おねがいします").unwrap();
        assert_eq!((c.start, c.typed_end, c.corrected_end), (2, 4, 4));
        // 余分なかなを消す補正は、補正後の読みでの範囲が空になる
        let all = corrections("ありがあとう", &TUNING);
        let c = all.iter().find(|c| c.reading == "ありがとう").unwrap();
        assert_eq!((c.start, c.typed_end, c.corrected_end), (3, 4, 3));
    }

    #[test]
    fn 三文字未満の読みでは作らない() {
        assert!(corrections("あり", &TUNING).is_empty());
        assert!(!corrections("ありが", &TUNING).is_empty());
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

    /// kana を打鍵のフィールド field で照合し、reading の前方一致のずれを返す
    fn matched(kana: &str, field: &str, reading: &str) -> Consumed {
        let keys = Keys::parse(kana, field).unwrap();
        let matcher = Matcher::new(&keys, TUNING.allowed_edits(keys.len()), &TUNING);
        matcher.match_reading(reading)
    }

    #[test]
    fn 打鍵の要素の数がかなの文字数と合わなければエラー() {
        assert!(Keys::parse("ありが", "a\x1fri").is_err());
        assert!(Keys::parse("ありが", "a\x1fri\x1fga\x1fto").is_err());
        assert!(Keys::parse("ありが", "a\x1fri\x1fga\x1et").is_ok());
        // 未完成のローマ字の区切りが無くてもよい
        assert!(Keys::parse("ありが", "a\x1fri\x1fga").is_ok());
    }

    #[test]
    fn ascii_でない要素は打鍵なしとして標準の綴りで補う() {
        // 文書から読んだかなは打鍵がかなそのもの
        let keys = Keys::parse("ありがとう", "あ\x1fり\x1fが\x1fと\x1fう").unwrap();
        assert_eq!(keys.keys.iter().collect::<String>(), "arigatou");
        let keys = Keys::parse("しゃっきん", "し\x1fゃ\x1fっ\x1fき\x1fん").unwrap();
        assert_eq!(keys.keys.iter().collect::<String>(), "syakkinn");
        // っ で終わる打鍵なしの並びは、後ろの打鍵の子音を重ねる
        let keys = Keys::parse("きって", "き\x1fっ\x1fte").unwrap();
        assert_eq!(keys.keys.iter().collect::<String>(), "kitte");
        assert_eq!(matched("ありがとう", "あ\x1fり\x1fが\x1fと\x1fう", "ありがとう"), Some((0, 0)));
    }

    #[test]
    fn 標準の綴りに無い打鍵で作ったかなはずれ0で一致する() {
        // romaji.tsv で lr を れ に割り当てた
        assert_eq!(matched("かれー", "ka\x1flr\x1f-", "かれー"), Some((0, 0)));
        // 標準の綴りなら 1 打の誤りになる打鍵
        assert!(matched("かれー", "ka\x1fre\x1f-", "かれー").is_some_and(|(_, e)| e == 0));
        // 打鍵から作る補正も独自の綴りでかなに直す (標準の表だけでは lr がかなにならず、補正を作れない)
        let keys = Keys::parse("かれーらいす", "ka\x1flr\x1f-\x1fra\x1fi\x1fsu").unwrap();
        let all = key_corrections("かれーらいす", &keys, &TUNING);
        assert!(all.iter().any(|c| c.reading == "かれーらいず"), "{all:?}");
    }

    #[test]
    fn 許すずれの数は打鍵の数から決める() {
        let edits: Vec<usize> = [0, 3, 4, 7, 8, 11, 12, 30].iter().map(|&n| TUNING.allowed_edits(n)).collect();
        assert_eq!(edits, vec![0, 0, 1, 1, 2, 2, 3, 3]);
        // 打鍵 3 (未完成のローマ字を含む) は完全一致だけ
        assert_eq!(matched("あい", "a\x1fi\x1eg", "ありが"), None);
        assert_eq!(matched("あい", "a\x1fi\x1eg", "あいが"), Some((0, 0)));
        // 打鍵 4 でずれ 1 つ (r の抜け)
        assert_eq!(matched("あいが", "a\x1fi\x1fga", "ありが"), Some((TUNING.edit_missing, 1)));
        // 打鍵 4 でずれ 2 つは照合しない
        assert_eq!(matched("あいご", "a\x1fi\x1fgo", "ありが"), None);
        // 打鍵 12 以上でずれ 3 つまで (onegaisimasu の抜け・余分・隣のキー)
        let got = matched("おねがしいまうす", "o\x1fne\x1fga\x1fsi\x1fi\x1fma\x1fu\x1fsu", "おねがいします");
        assert!(got.is_some_and(|(_, e)| e <= 3), "{got:?}");
    }

    #[test]
    fn 先頭のかなの打鍵の誤りは照合しない() {
        // nu の u を隣の i に戻せば にこう だが、先頭のかなの打鍵なので直さない
        assert_eq!(matched("ぬこう", "nu\x1fko\x1fu", "にこう"), None);
        // 2 字目からは直す (u を隣の i に戻す)
        assert_eq!(matched("にこう", "ni\x1fko\x1fu", "にこい"), Some((TUNING.edit_adjacent, 1)));
    }

    #[test]
    fn 予測の照合は末尾のかなの綴りの途中で終わってよい() {
        assert_eq!(matched("ありが", "a\x1fri\x1fga\x1et", "ありがとう"), Some((0, 0)));
        // っ と ん は次のかなの綴りで決まる
        assert_eq!(matched("きって", "ki\x1ft\x1fte", "きって"), Some((0, 0)));
        assert_eq!(matched("かんじ", "ka\x1fn\x1fji", "かんじ"), Some((0, 0)));
        assert_ne!(matched("かんじ", "ka\x1fn\x1fji", "かにじ"), Some((0, 0)));
    }

    #[test]
    fn 隣り合うかなをまたぐ入れ替えを照合する() {
        // onegiasimasu (gi|a = ga|i の入れ替え)
        let field = "o\x1fne\x1fgi\x1fa\x1fsi\x1fma\x1fsu";
        assert_eq!(matched("おねぎあします", field, "おねがいします"), Some((TUNING.edit_swap, 1)));
        let keys = Keys::parse("おねぎあします", field).unwrap();
        let all = key_corrections("おねぎあします", &keys, &TUNING);
        let c = all.iter().find(|c| c.reading == "おねがいします").unwrap();
        assert_eq!(c.cost, TUNING.edit_swap);
        assert_eq!((c.start, c.typed_end, c.corrected_end), (2, 4, 4));
    }
}
