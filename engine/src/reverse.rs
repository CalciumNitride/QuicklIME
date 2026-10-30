// 逆変換: 表記から読みを推測する (docs/spec/engine.md「逆変換 (REVERSE)」)
//
// 表記の文字の格子に表記で引いた語を並べ、変換 (convert.rs) と同じ単語コスト・連接コスト・
// 文節ペナルティで Viterbi をかけ、最小コストの語の列 (読み, 表記) を返す。
// 変換のラティスは読みの文字の上に作るので、表記の文字の上に作るこちらは別に持つ。

use std::collections::HashSet;

use crate::convert::{MAX_READING_CHARS, SEGMENT_PENALTY, UNKNOWN_WORD_COST};
use crate::dict::{self, Dictionary, SurfaceIndex};
use crate::learn::LearningStore;
use crate::matrix::ConnectionMatrix;
use crate::pos::{FunctionalIds, DEFAULT_NOUN_ID};
use crate::suggest::to_hiragana;
use crate::userdict::UserDict;

/// 逆変換した読みの最大文字数 (TSF 層の再変換の長さの上限 kMaxReconvertLength と同じ)
pub const MAX_READING_LEN: usize = 127;

/// 単漢字辞書の読みの語の単語コスト。辞書の語より高くし、辞書の語で読めない漢字に使われるようにする
const SINGLE_KANJI_COST: i32 = UNKNOWN_WORD_COST;

/// 逆変換の格子の 1 語
struct Node {
    start: usize,
    end: usize,
    reading: String,
    surface: String,
    left_id: u16,
    right_id: u16,
    word_cost: i32,
    /// 学習にある (読み, 表記) の組
    learned: bool,
    /// どの語にも当たらないかなの 1 文字
    unknown: bool,
    best_cost: i64,
    best_prev: usize,
}

fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヶ' | 'ー')
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// 逆変換の対象にする文字 (docs/spec/conversion.md「漢字を含む選択の再変換」)
pub(crate) fn is_allowed(c: char) -> bool {
    is_kana(c) || dict::is_kanji(c) || is_digit(c) || c == '〇' || c == '〆'
}

/// 逆変換した 1 語
#[derive(Debug, PartialEq)]
pub struct ReversedWord {
    pub reading: String,
    pub surface: String,
    /// 付属語 (左の品詞 ID が付属語) か
    pub functional: bool,
    /// 未知語 (どの語にも当たらず 1 文字の一般名詞として入れたかな) か
    pub unknown: bool,
    /// 左の品詞 ID
    pub left_id: u16,
}

/// 表記 text を語の列 (読み, 表記) にする。対象外の文字がある・格子が繋がらない・読みが
/// MAX_READING_LEN 文字を超えるときは None。index は dict の逆引きの索引
pub fn reverse(
    text: &str,
    dict: &Dictionary,
    index: &SurfaceIndex,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Option<Vec<(String, String)>> {
    let words = reverse_words(text, dict, index, user, matrix, functional, learning)?;
    Some(words.into_iter().map(|w| (w.reading, w.surface)).collect())
}

/// reverse と同じ逆変換で、語が付属語かどうか・未知語かどうかも返す
pub fn reverse_words(
    text: &str,
    dict: &Dictionary,
    index: &SurfaceIndex,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Option<Vec<ReversedWord>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 || !chars.iter().all(|&c| is_allowed(c)) {
        return None;
    }
    // カタカナをひらがなにした文字の列。かなの部分を読みの辞書で引くのに使う (1 文字ずつ対応する)
    let hira: Vec<char> = to_hiragana(text).chars().collect();

    let mut nodes: Vec<Node> = Vec::new();
    // 変換の build_lattice と同じく、自立語には語の学習の割引を当てる
    let mut push = |start: usize,
                    len: usize,
                    reading: &str,
                    surface: &str,
                    left_id: u16,
                    right_id: u16,
                    cost: i32,
                    unknown: bool| {
        let functional_word = functional.is_functional(left_id);
        let bonus = if functional_word { 0 } else { learning.word_bonus(reading, surface) };
        nodes.push(Node {
            start,
            end: start + len,
            reading: reading.to_string(),
            surface: surface.to_string(),
            left_id,
            right_id,
            word_cost: (i64::from(cost) - bonus) as i32,
            learned: bonus > 0 || learning.has_phrase(reading, surface),
            unknown,
            best_cost: i64::MAX,
            best_prev: 0,
        });
    };

    for start in 0..n {
        let rest = &chars[start..];
        // 表記に漢字・々 を含む語。数字を含む語 (読み みっか の 3日 など) は使わず、数字はいつも
        // 数字の並びの語にして、読みに数字をそのまま入れる
        let has_digit = |surface: &str| surface.chars().any(is_digit);
        for m in index
            .common_prefix_search(dict, rest, MAX_READING_CHARS)
            .into_iter()
            .chain(user.imported_surface_prefix(rest, MAX_READING_CHARS))
        {
            for e in m.entries.into_iter().filter(|e| !has_digit(&e.surface)) {
                push(start, m.len, &m.reading, &e.surface, e.left_id, e.right_id, i32::from(e.cost), false);
            }
        }
        for (len, w) in user.surface_prefix_words(rest) {
            if !has_digit(&w.surface) {
                push(start, len, &w.reading, &w.surface, w.left_id, w.right_id, i32::from(w.cost), false);
            }
        }

        if is_kana(chars[start]) {
            // かなだけの表記の語: 読みは表記をひらがなにしたものに決まるので、読みの辞書を引いて表記で絞る
            let run = rest.iter().take_while(|&&c| is_kana(c)).count();
            let kana = &hira[start..start + run];
            let surface_of = |len: usize| -> String { rest[..len].iter().collect() };
            for (len, entries) in dict
                .common_prefix_search(kana, MAX_READING_CHARS)
                .into_iter()
                .chain(user.imported_common_prefix(kana, MAX_READING_CHARS))
            {
                let surface = surface_of(len);
                let reading: String = kana[..len].iter().collect();
                for e in entries.iter().filter(|e| e.surface == surface) {
                    push(start, len, &reading, &e.surface, e.left_id, e.right_id, i32::from(e.cost), false);
                }
            }
            for (len, w) in user.common_prefix_words(kana) {
                if w.surface == surface_of(len) {
                    push(start, len, &w.reading, &w.surface, w.left_id, w.right_id, i32::from(w.cost), false);
                }
            }
            // どの語にも当たらないときの 1 文字。ヶ は ゖ でなく、地名などの語の中の読み け にする
            let reading = if chars[start] == 'ヶ' { 'け' } else { hira[start] };
            push(
                start, 1, &reading.to_string(), &chars[start].to_string(),
                DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, UNKNOWN_WORD_COST, true,
            );
        }

        if is_digit(chars[start]) && (start == 0 || !is_digit(chars[start - 1])) {
            let len = rest.iter().take_while(|&&c| is_digit(c)).count();
            let (left_id, right_id, cost) = digit_word(&rest[..len], dict);
            let digits: String = rest[..len].iter().collect();
            push(start, len, &digits, &digits, left_id, right_id, cost, false);
        }

        if dict::is_kanji(chars[start]) {
            let surface = chars[start].to_string();
            for reading in dict.lookup_single_kanji_readings(chars[start]) {
                push(start, 1, reading, &surface, DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, SINGLE_KANJI_COST, false);
            }
        }
    }

    // 学習にある (読み, 表記) の組があれば、同じ位置の同じ表記の別の読みの語を外す
    let learned: HashSet<(usize, usize, &str)> =
        nodes.iter().filter(|n| n.learned).map(|n| (n.start, n.end, n.surface.as_str())).collect();
    let keep: Vec<bool> = nodes
        .iter()
        .map(|n| n.learned || !learned.contains(&(n.start, n.end, n.surface.as_str())))
        .collect();
    drop(learned);
    let mut kept = keep.into_iter();
    nodes.retain(|_| kept.next().unwrap_or(false));

    // 辞書では数字の後で使わない助数詞の読み (日 → ひ など) の方が安いことがあるため外す
    nodes.retain(|node| {
        !(node.start > 0
            && is_digit(chars[node.start - 1])
            && !node.learned
            && functional.is_counter(node.left_id)
            && is_counter_misread(&node.surface, &node.reading))
    });

    best_path(nodes, n, matrix, functional)
}

/// 数字の直後の助数詞の表記の見出しと、その見出しの数字の後の読み
/// (docs/spec/engine.md「数字の直後の助数詞の読み」)
const COUNTER_READINGS: &[(&str, &[&str])] = &[
    ("日", &["にち"]),
    ("人", &["にん"]),
    ("時", &["じ"]),
    ("次", &["じ"]),
    ("字", &["じ"]),
    ("年", &["ねん"]),
    ("歳", &["さい"]),
    ("名", &["めい"]),
    ("問", &["もん"]),
    ("巻", &["かん"]),
    ("戦", &["せん"]),
    ("銭", &["せん"]),
    ("話", &["わ"]),
    ("種", &["しゅ"]),
    ("色", &["しょく"]),
    ("食", &["しょく"]),
    ("軒", &["けん"]),
    ("店", &["てん"]),
    ("戸", &["こ"]),
    ("個", &["こ"]),
    ("床", &["しょう"]),
    ("勝", &["しょう"]),
    ("章", &["しょう"]),
    ("球", &["きゅう"]),
    ("泊", &["はく"]),
    ("畳", &["じょう"]),
    ("元", &["げん"]),
    ("円", &["えん"]),
    ("合", &["ごう"]),
    ("世", &["せい"]),
    ("体", &["たい"]),
    ("位", &["い"]),
    ("通", &["つう"]),
    ("連", &["れん"]),
    ("重", &["じゅう"]),
    ("品", &["ひん"]),
    ("棟", &["とう"]),
    ("行", &["ぎょう", "こう"]),
    ("分", &["ふん"]),
    ("か月", &["かげつ"]),
    ("カ月", &["かげつ"]),
    ("ヶ月", &["かげつ"]),
    ("ヵ月", &["かげつ"]),
    ("ケ月", &["かげつ"]),
];

/// 表記 surface が COUNTER_READINGS の見出し (最長のもの) で始まり、読み reading がその見出しの
/// 読みのどれでも始まらないか。連濁・半濁音化 (ふん → ぷん・ぶん) を受けるため濁点・半濁点を外して比べる
fn is_counter_misread(surface: &str, reading: &str) -> bool {
    let Some((_, readings)) = COUNTER_READINGS
        .iter()
        .filter(|(head, _)| surface.starts_with(head))
        .max_by_key(|(head, _)| head.chars().count())
    else {
        return false;
    };
    let reading = strip_voicing(reading);
    !readings.iter().any(|r| reading.starts_with(&strip_voicing(r)))
}

/// ひらがなの濁点・半濁点を外す
fn strip_voicing(s: &str) -> String {
    const VOICED: &str = "がぎぐげござじずぜぞだぢづでどばびぶべぼぱぴぷぺぽゔ";
    const PLAIN: &str = "かきくけこさしすせそたちつてとはひふへほはひふへほう";
    s.chars()
        .map(|c| VOICED.chars().position(|v| v == c).and_then(|i| PLAIN.chars().nth(i)).unwrap_or(c))
        .collect()
}

/// 数字の並び digits の語の (left_id, right_id, 単語コスト)。変換の merge_digit_runs が数字の語を
/// まとめたときと同じく、1 桁ずつの語 (半角は辞書の数字の語、辞書に無い全角は未知語) を連ねた
/// 左端の left_id・右端の right_id と単語コストの和にする
fn digit_word(digits: &[char], dict: &Dictionary) -> (u16, u16, i32) {
    let words: Vec<(u16, u16, i32)> = digits
        .iter()
        .map(|&c| {
            let digit = c.to_string();
            dict.lookup(&digit)
                .iter()
                .filter(|e| e.surface == digit)
                .min_by_key(|e| e.cost)
                .map_or((DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, UNKNOWN_WORD_COST), |e| {
                    (e.left_id, e.right_id, i32::from(e.cost))
                })
        })
        .collect();
    let first = words.first().expect("数字の並びは 1 文字以上");
    let last = words.last().expect("数字の並びは 1 文字以上");
    (first.0, last.1, words.iter().map(|w| w.2).sum())
}

/// 格子 nodes (表記の文字数は n) の最小コストの経路の語を返す。経路が無い・読みが
/// MAX_READING_LEN 文字を超えるときは None
fn best_path(
    mut nodes: Vec<Node>,
    n: usize,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<Vec<ReversedWord>> {
    // 安定ソートで、同じ位置の語の並び (同じコストの経路の選び方) を引いた順に保つ
    nodes.sort_by_key(|node| node.start);
    // ending_at[p] = 位置 p で終わる語。usize::MAX は BOS
    let mut ending_at: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    ending_at[0].push(usize::MAX);
    for (i, node) in nodes.iter().enumerate() {
        ending_at[node.end].push(i);
    }
    for i in 0..nodes.len() {
        // 変換と同じく自立語ごとに文節ペナルティを足す。前文脈が無いので、先頭の付属語にも足す
        let penalty = if functional.is_functional(nodes[i].left_id) && nodes[i].start > 0 {
            0
        } else {
            i64::from(SEGMENT_PENALTY)
        };
        let mut best = (i64::MAX, 0);
        for &p in &ending_at[nodes[i].start] {
            let (from, right_id) = if p == usize::MAX { (0, 0) } else { (nodes[p].best_cost, nodes[p].right_id) };
            if from == i64::MAX {
                continue;
            }
            let cost = from
                + i64::from(matrix.get(right_id, nodes[i].left_id))
                + i64::from(nodes[i].word_cost)
                + penalty;
            if cost < best.0 {
                best = (cost, p);
            }
        }
        (nodes[i].best_cost, nodes[i].best_prev) = best;
    }
    let mut cursor = ending_at[n]
        .iter()
        .filter(|&&i| i != usize::MAX && nodes[i].best_cost != i64::MAX)
        .min_by_key(|&&i| nodes[i].best_cost + i64::from(matrix.get(nodes[i].right_id, 0)))
        .copied()?;
    let mut words = Vec::new();
    loop {
        words.push(ReversedWord {
            reading: std::mem::take(&mut nodes[cursor].reading),
            surface: std::mem::take(&mut nodes[cursor].surface),
            functional: functional.is_functional(nodes[cursor].left_id),
            unknown: nodes[cursor].unknown,
            left_id: nodes[cursor].left_id,
        });
        match nodes[cursor].best_prev {
            usize::MAX => break,
            prev => cursor = prev,
        }
    }
    words.reverse();
    let length: usize = words.iter().map(|w| w.reading.chars().count()).sum();
    (length <= MAX_READING_LEN).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 は名詞、2 は助詞 (付属語)、3 は接尾語 (付属語)
    fn functional() -> FunctionalIds {
        FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n3 接尾,一般\n".as_bytes()).unwrap()
    }

    fn sample() -> (Dictionary, SurfaceIndex) {
        let mut dict = Dictionary::empty();
        let data = "きょう\t1\t1\t3000\t今日\n\
                    こんにち\t1\t1\t5000\t今日\n\
                    いま\t1\t1\t3000\t今\n\
                    ひ\t1\t1\t4000\t日\n\
                    は\t2\t2\t500\tは\n\
                    よい\t1\t1\t3000\t良い\n\
                    いい\t1\t1\t3500\t良い\n\
                    てんき\t1\t1\t3000\t天気\n\
                    てん\t1\t1\t4000\t天\n\
                    とうきょう\t1\t1\t3000\t東京\n\
                    きょうと\t1\t1\t3000\t京都\n\
                    と\t3\t3\t3000\t都\n\
                    がつ\t3\t3\t2000\t月\n\
                    つき\t1\t1\t3000\t月\n\
                    3\t1\t1\t2000\t3\n\
                    3がつ\t1\t1\t1000\t3月\n\
                    みっか\t1\t1\t1000\t3日\n\
                    にち\t3\t3\t2000\t日\n\
                    かたかな\t1\t1\t3000\tカタカナ\n\
                    ひとびと\t1\t1\t3000\t人々\n\
                    しめきり\t1\t1\t3000\t〆切\n\
                    しめ\t1\t1\t3000\t〆\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict.load_single_kanji_from("あられ\t霰\n".as_bytes()).unwrap();
        let index = SurfaceIndex::build(&dict);
        (dict, index)
    }

    fn run_with(text: &str, user: &UserDict, learning: &LearningStore) -> Option<Vec<(String, String)>> {
        let (dict, index) = sample();
        reverse(text, &dict, &index, user, &ConnectionMatrix::empty(), &functional(), learning)
    }

    fn run(text: &str) -> Option<Vec<(String, String)>> {
        run_with(text, &UserDict::empty(), &LearningStore::in_memory())
    }

    /// 助数詞の検査用。4 は助数詞 (付属語)。実辞書と同じく、数字の後で使わない読みの方を安くする
    fn counter_functional() -> FunctionalIds {
        FunctionalIds::load_from("1 名詞,一般\n4 名詞,接尾,助数詞\n".as_bytes()).unwrap()
    }

    fn counter_sample() -> (Dictionary, SurfaceIndex) {
        let mut dict = Dictionary::empty();
        let data = "2\t1\t1\t2000\t2\n\
                    3\t1\t1\t2000\t3\n\
                    みっか\t1\t1\t1000\t3日\n\
                    ひ\t4\t4\t1734\t日\n\
                    にち\t4\t4\t1826\t日\n\
                    にほん\t1\t1\t3000\t日本\n\
                    ひとのり\t4\t4\t3000\t人乗り\n\
                    にんのり\t4\t4\t3500\t人乗り\n\
                    ときごろ\t4\t4\t3000\t時ごろ\n\
                    じごろ\t4\t4\t3500\t時ごろ\n\
                    かつきたらず\t4\t4\t3000\tか月足らず\n\
                    かげつたらず\t4\t4\t3500\tか月足らず\n\
                    ぶ\t4\t4\t1000\t分\n\
                    ぶん\t4\t4\t1500\t分\n\
                    ぷん\t4\t4\t2000\t分\n\
                    ふん\t4\t4\t2500\t分\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        let index = SurfaceIndex::build(&dict);
        (dict, index)
    }

    fn run_counter(text: &str, learning: &LearningStore) -> Option<Vec<(String, String)>> {
        let (dict, index) = counter_sample();
        let user = UserDict::empty();
        reverse(text, &dict, &index, &user, &ConnectionMatrix::empty(), &counter_functional(), learning)
    }

    fn words(list: &[(&str, &str)]) -> Option<Vec<(String, String)>> {
        Some(list.iter().map(|&(r, s)| (r.to_string(), s.to_string())).collect())
    }

    #[test]
    fn 漢字かな混じりの表記を語の読みに分ける() {
        assert_eq!(
            run("今日は良い天気"),
            words(&[("きょう", "今日"), ("は", "は"), ("よい", "良い"), ("てんき", "天気")])
        );
    }

    #[test]
    fn 付属語かどうかも返す版は語の列が同じで助詞と助数詞が付属語になる() {        let (dict, index) = sample();
        let (user, learning) = (UserDict::empty(), LearningStore::in_memory());
        let run_words = |text: &str| {
            reverse_words(text, &dict, &index, &user, &ConnectionMatrix::empty(), &functional(), &learning)
        };
        for text in ["今日は良い天気", "東京都", "3月", "カタカナ", "霰は"] {
            let pairs = run_words(text).map(|w| w.into_iter().map(|w| (w.reading, w.surface)).collect());
            assert_eq!(pairs, run(text), "{text}");
        }
        let flags = |text: &str| run_words(text).unwrap().iter().map(|w| w.functional).collect::<Vec<_>>();
        assert_eq!(flags("今日は良い天気"), [false, true, false, false]);
        assert_eq!(run_words("今日はabc"), None);
        let (dict, index) = counter_sample();
        let got = reverse_words("3日", &dict, &index, &user, &ConnectionMatrix::empty(), &counter_functional(), &learning)
            .unwrap();
        assert_eq!(got.iter().map(|w| w.functional).collect::<Vec<_>>(), [false, true]);
    }

    #[test]
    fn どの語にも当たらないかなの1文字だけを未知語にする() {
        let (dict, index) = sample();
        let (user, learning) = (UserDict::empty(), LearningStore::in_memory());
        let unknown = |text: &str| {
            reverse_words(text, &dict, &index, &user, &ConnectionMatrix::empty(), &functional(), &learning)
                .unwrap()
                .iter()
                .map(|w| w.unknown)
                .collect::<Vec<_>>()
        };
        assert_eq!(unknown("ヌぬは"), [true, true, false]);
        assert_eq!(unknown("今日はカタカナ"), [false, false, false]);
        // 数字の並びと単漢字辞書の読みは未知語にしない
        assert_eq!(unknown("3月"), [false, false]);
        assert_eq!(unknown("霰"), [false]);
    }

    #[test]
    fn 語の分け方によらず読みをつなげる() {
        let got = run("東京都").unwrap();
        assert_eq!(got.iter().map(|(r, _)| r.as_str()).collect::<String>(), "とうきょうと");
        assert_eq!(got.iter().map(|(_, s)| s.as_str()).collect::<String>(), "東京都");
    }

    #[test]
    fn 数字は半角も全角もそのまま読みに入れる() {
        assert_eq!(run("3月"), words(&[("3", "3"), ("がつ", "月")]));
        assert_eq!(run("３月"), words(&[("３", "３"), ("がつ", "月")]));
        // 数字の並びは 1 語
        assert_eq!(run("33月"), words(&[("33", "33"), ("がつ", "月")]));
        // 数字を含む辞書の語 (3がつ の 3月、みっか の 3日) は使わない
        assert_eq!(run("3日"), words(&[("3", "3"), ("にち", "日")]));
    }

    #[test]
    fn かなは読みの辞書で引きカタカナはひらがなの読みにする() {
        assert_eq!(run("カタカナ"), words(&[("かたかな", "カタカナ")]));
        // 辞書に無いかなは 1 文字ずつ
        assert_eq!(run("ヌぬ"), words(&[("ぬ", "ヌ"), ("ぬ", "ぬ")]));
        assert_eq!(run("ヶ"), words(&[("け", "ヶ")]));
    }

    #[test]
    fn 踊り字を含む語を引ける() {
        assert_eq!(run("人々"), words(&[("ひとびと", "人々")]));
    }

    #[test]
    fn 〇と〆は漢字を含む語の中なら読める() {
        assert_eq!(run("〆切"), words(&[("しめきり", "〆切")]));
        // 漢字を含まない表記の語 (しめ/〆) は逆引きの索引に無いので、単独では読めない
        assert_eq!(run("〆"), None);
        assert_eq!(run("〇月"), None);
    }

    #[test]
    fn 対象外の文字があれば失敗する() {
        assert_eq!(run("今日はabc"), None);
        assert_eq!(run("今日 は"), None);
        assert_eq!(run(""), None);
    }

    #[test]
    fn 学習にある読みを同じ表記の別の読みより優先する() {
        let mut learning = LearningStore::in_memory();
        let split = [("こんにち".to_string(), "今日".to_string())];
        learning.learn_segment("こんにち", "今日", "", Some(&split));
        assert_eq!(run_with("今日", &UserDict::empty(), &learning), words(&[("こんにち", "今日")]));
        assert_eq!(
            run_with("今日は", &UserDict::empty(), &learning),
            words(&[("こんにち", "今日"), ("は", "は")])
        );
    }

    #[test]
    fn 数字の直後の助数詞は数字の後の読みにする() {
        let learning = LearningStore::in_memory();
        assert_eq!(run_counter("3日", &learning), words(&[("3", "3"), ("にち", "日")]));
        assert_eq!(run_counter("3人乗り", &learning), words(&[("3", "3"), ("にんのり", "人乗り")]));
        assert_eq!(run_counter("3時ごろ", &learning), words(&[("3", "3"), ("じごろ", "時ごろ")]));
        assert_eq!(
            run_counter("2か月足らず", &learning),
            words(&[("2", "2"), ("かげつたらず", "か月足らず")])
        );
        // 数字の後でなければ外さない
        assert_eq!(run_counter("日", &learning), words(&[("ひ", "日")]));
    }

    #[test]
    fn 助数詞の読みは濁点と半濁点を外して比べる() {
        let learning = LearningStore::in_memory();
        // ぶ だけが外れ、ぶん・ぷん・ふん は残る (残るうち最も安い ぶん になる)
        assert_eq!(run_counter("3分", &learning), words(&[("3", "3"), ("ぶん", "分")]));
        assert!(is_counter_misread("分", "ぶ"));
        assert!(!is_counter_misread("分", "ふん"));
        assert!(!is_counter_misread("分", "ぷん"));
        assert!(!is_counter_misread("分", "ぶん"));
        assert!(!is_counter_misread("行", "ぎょう"));
        assert!(!is_counter_misread("行", "こう"));
        assert!(is_counter_misread("行", "いき"));
        // 表に無い見出しは外さない
        assert!(!is_counter_misread("月", "つき"));
    }

    #[test]
    fn 助数詞でない語は数字の直後でも外さない() {
        let learning = LearningStore::in_memory();
        assert_eq!(run_counter("3日本", &learning), words(&[("3", "3"), ("にほん", "日本")]));
    }

    #[test]
    fn 学習にある助数詞の読みは数字の直後でも外さない() {
        let mut learning = LearningStore::in_memory();
        let split = [("ひ".to_string(), "日".to_string())];
        learning.learn_segment("ひ", "日", "", Some(&split));
        assert_eq!(run_counter("3日", &learning), words(&[("3", "3"), ("ひ", "日")]));
    }

    #[test]
    fn 辞書の語に無い漢字は単漢字辞書の読みにし読めなければ失敗する() {
        assert_eq!(run("霰"), words(&[("あられ", "霰")]));
        assert_eq!(run("霰は"), words(&[("あられ", "霰"), ("は", "は")]));
        assert_eq!(run("鬱"), None);
    }

    #[test]
    fn ユーザ辞書とインポート辞書の語を引ける() {
        let mut user = UserDict::empty();
        user.load_from("かんべ\t神戸\t姓\nぬぬ\tヌヌ\t名詞\n".as_bytes(), &functional());
        user.load_imported_from(["あらしやま\t嵐山\t地名\n".as_bytes()], &functional());
        let learning = LearningStore::in_memory();
        assert_eq!(run_with("神戸", &user, &learning), words(&[("かんべ", "神戸")]));
        assert_eq!(run_with("嵐山", &user, &learning), words(&[("あらしやま", "嵐山")]));
        assert_eq!(run_with("ヌヌ", &user, &learning), words(&[("ぬぬ", "ヌヌ")]));
    }

    #[test]
    fn 読みが上限を超えると失敗する() {
        let at_limit = "ア".repeat(MAX_READING_LEN);
        assert_eq!(run(&at_limit).map(|w| w.len()), Some(MAX_READING_LEN));
        assert_eq!(run(&"ア".repeat(MAX_READING_LEN + 1)), None);
    }
}
