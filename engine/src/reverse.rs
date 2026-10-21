// 逆変換: 表記から読みを推測する (docs/design/selection-reconversion.md の B)
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
    best_cost: i64,
    best_prev: usize,
}

fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヶ' | 'ー')
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// 逆変換の対象にする文字 (docs/design/selection-reconversion.md の A)
fn is_allowed(c: char) -> bool {
    is_kana(c) || dict::is_kanji(c) || is_digit(c) || c == '〇' || c == '〆'
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
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 || !chars.iter().all(|&c| is_allowed(c)) {
        return None;
    }
    // カタカナをひらがなにした文字の列。かなの部分を読みの辞書で引くのに使う (1 文字ずつ対応する)
    let hira: Vec<char> = to_hiragana(text).chars().collect();

    let mut nodes: Vec<Node> = Vec::new();
    // 変換の build_lattice と同じく、自立語には語の学習の割引を当てる
    let mut push = |start: usize, len: usize, reading: &str, surface: &str, left_id: u16, right_id: u16, cost: i32| {
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
                push(start, m.len, &m.reading, &e.surface, e.left_id, e.right_id, i32::from(e.cost));
            }
        }
        for (len, w) in user.surface_prefix_words(rest) {
            if !has_digit(&w.surface) {
                push(start, len, &w.reading, &w.surface, w.left_id, w.right_id, i32::from(w.cost));
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
                    push(start, len, &reading, &e.surface, e.left_id, e.right_id, i32::from(e.cost));
                }
            }
            for (len, w) in user.common_prefix_words(kana) {
                if w.surface == surface_of(len) {
                    push(start, len, &w.reading, &w.surface, w.left_id, w.right_id, i32::from(w.cost));
                }
            }
            // どの語にも当たらないときの 1 文字。ヶ は ゖ でなく、地名などの語の中の読み け にする
            let reading = if chars[start] == 'ヶ' { 'け' } else { hira[start] };
            push(
                start, 1, &reading.to_string(), &chars[start].to_string(),
                DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, UNKNOWN_WORD_COST,
            );
        }

        if is_digit(chars[start]) && (start == 0 || !is_digit(chars[start - 1])) {
            let len = rest.iter().take_while(|&&c| is_digit(c)).count();
            let (left_id, right_id, cost) = digit_word(&rest[..len], dict);
            let digits: String = rest[..len].iter().collect();
            push(start, len, &digits, &digits, left_id, right_id, cost);
        }

        if dict::is_kanji(chars[start]) {
            let surface = chars[start].to_string();
            for reading in dict.lookup_single_kanji_readings(chars[start]) {
                push(start, 1, reading, &surface, DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, SINGLE_KANJI_COST);
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

    best_path(nodes, n, matrix, functional)
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

/// 格子 nodes (表記の文字数は n) の最小コストの経路の (読み, 表記) を返す。経路が無い・読みが
/// MAX_READING_LEN 文字を超えるときは None
fn best_path(
    mut nodes: Vec<Node>,
    n: usize,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<Vec<(String, String)>> {
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
        words.push((std::mem::take(&mut nodes[cursor].reading), std::mem::take(&mut nodes[cursor].surface)));
        match nodes[cursor].best_prev {
            usize::MAX => break,
            prev => cursor = prev,
        }
    }
    words.reverse();
    let length: usize = words.iter().map(|(reading, _)| reading.chars().count()).sum();
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
