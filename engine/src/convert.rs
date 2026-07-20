// 変換候補の生成
//
// フェーズ4-3: Viterbi の最小コスト経路を品詞情報で文節にまとめ、
// 文節ごとの候補リストを返す。
// 全文一括の候補 (candidates) も互換のため残している。

use crate::dict::Dictionary;
use crate::learn::LearningStore;
use crate::matrix::ConnectionMatrix;
use crate::pos::{FunctionalIds, DEFAULT_NOUN_ID};
use crate::userdict::UserDict;

/// 辞書由来の候補の最大数 (文全体・文節共通)。
/// かなの同音異義語は多い (例:「きょう」は約50語) ため、ある程度大きくしておく。
/// TSF 側の候補ウィンドウはページ表示で対応する
const MAX_DICT_CANDIDATES: usize = 24;

/// 辞書引きする読みの最大文字数 (ラティス構築時)
const MAX_READING_CHARS: usize = 16;

/// 未知語 (辞書に無い1文字) ノードの単語コスト。
/// 辞書語の経路が常に優先されるよう十分大きくする
const UNKNOWN_WORD_COST: i32 = 12000;

/// 文節境界ペナルティ。自立語 (付属語でない語) ごとに Viterbi のコストへ加算する。
/// 辞書にはコスト0の短いかな語が多く、素のコストでは「き+き+無+れ」のような
/// 細切れ経路が複合語 (聞き慣れ) より安くなるため、文節数が少ない経路を優先させる。
/// 値は実辞書の回帰コーパスで調整した (「ききなれない」は自立語1語差で約800必要)
const SEGMENT_PENALTY: i32 = 1000;

/// 変換結果の1文節
pub struct Segment {
    /// この文節の読み (ひらがな)
    pub reading: String,
    /// 候補リスト (先頭が最良)
    pub candidates: Vec<String>,
}

/// Viterbi 経路上の1単語
struct PathWord {
    reading: String,
    surface: String,
    left_id: u16,
}

/// かな文字列を文節列へ変換する
pub fn convert_segments(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    let Some(path) = viterbi_path(kana, dict, user, matrix, functional) else {
        return Vec::new();
    };
    // 辞書の数字は1桁単位のため、連続する数字を1語にまとめてから文節を作る
    let path = merge_digit_runs(path);

    // 付属語 (助詞・助動詞・接尾語) を直前の自立語にまとめて文節を作る
    let mut groups: Vec<Vec<PathWord>> = Vec::new();
    for word in path {
        if !groups.is_empty() && functional.is_functional(word.left_id) {
            groups.last_mut().unwrap().push(word);
        } else {
            groups.push(vec![word]);
        }
    }

    groups
        .iter()
        .map(|group| segment_from_group(group, dict, user, learning))
        .collect()
}

/// 数字 (半角・全角) かどうか
fn is_digit_char(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// 経路上で隣接する数字語 (読みがすべて数字) を1語に結合する。
/// 辞書の数字エントリは1桁単位のため、そのままでは「12」が桁ごとの文節に割れる。
/// 全角数字は辞書に無く未知語1文字ノードになるが、読みベースの判定で同様にまとまる
fn merge_digit_runs(path: Vec<PathWord>) -> Vec<PathWord> {
    let mut result: Vec<PathWord> = Vec::new();
    for word in path {
        let is_digits = word.reading.chars().all(is_digit_char);
        match result.last_mut() {
            Some(last) if is_digits && last.reading.chars().all(is_digit_char) => {
                last.reading.push_str(&word.reading);
                last.surface.push_str(&word.surface);
            }
            _ => result.push(word),
        }
    }
    result
}

/// 文節境界 (文字数) を指定してかな文字列を変換する (Shift+←→ での文節伸縮用)。
/// lengths の合計が入力の文字数と一致しない場合は空を返す
pub fn convert_segments_fixed(
    kana: &str,
    lengths: &[usize],
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    let chars: Vec<char> = kana.chars().collect();
    if lengths.is_empty() || lengths.iter().sum::<usize>() != chars.len()
        || lengths.contains(&0)
    {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut begin = 0;
    for &length in lengths {
        let reading: String = chars[begin..begin + length].iter().collect();
        begin += length;

        // 文節の範囲内だけで最小コスト経路を求める
        let group = viterbi_path(&reading, dict, user, matrix, functional).unwrap_or_else(|| {
            vec![PathWord {
                reading: reading.clone(),
                surface: reading.clone(),
                left_id: DEFAULT_NOUN_ID,
            }]
        });
        segments.push(segment_from_group(&group, dict, user, learning));
    }
    segments
}

/// 読みに完全一致する候補を (コスト, 表記) で集める。
/// システム辞書とユーザ登録の名詞系単語をまとめてコスト昇順に並べる
fn exact_candidates<'a>(
    reading: &str,
    dict: &'a Dictionary,
    user: &'a UserDict,
) -> Vec<(i16, &'a str)> {
    let mut hits: Vec<(i16, &str)> = dict
        .lookup(reading)
        .iter()
        .map(|e| (e.cost, e.surface.as_str()))
        .chain(user.lookup_words(reading).into_iter().map(|w| (w.cost, w.surface.as_str())))
        .collect();
    hits.sort_by_key(|(cost, _)| *cost);
    hits
}

/// 単語列 (1文節分) から候補リスト付きの Segment を作る。
/// 候補: 経路上の表記 → 読み全体の辞書候補 → 先頭語を入れ替えた表記
///       → カタカナ → ひらがな。学習済みの表記があれば先頭へ移動する。
/// 読み全体の完全一致 (「した」→ 下) はユーザが求める同音異義語そのものなので、
/// 先頭語入れ替え (し+た → 死た) より先に積む。逆順だと先頭語が1文字の読みの
/// とき入れ替え候補だけで MAX_DICT_CANDIDATES を使い切り、完全一致が脱落する
fn segment_from_group(
    group: &[PathWord],
    dict: &Dictionary,
    user: &UserDict,
    learning: &LearningStore,
) -> Segment {
    let reading: String = group.iter().map(|w| w.reading.as_str()).collect();
    let best: String = group.iter().map(|w| w.surface.as_str()).collect();
    let mut result = vec![best];

    // 短縮よみ (ユーザ辞書) は経路表記の直後に置く (記載順)。
    // 末尾の学習表記の先頭移動が最優先なのは変わらない
    for shortcut in user.lookup_shortcuts(&reading) {
        if !result.iter().any(|s| s == shortcut) {
            result.push(shortcut.to_string());
        }
    }

    // 読み全体の完全一致候補 (「した」→ 下 など)
    for (_, surface) in exact_candidates(&reading, dict, user) {
        if result.len() >= MAX_DICT_CANDIDATES {
            break;
        }
        if surface != reading && !result.iter().any(|s| s == surface) {
            result.push(surface.to_string());
        }
    }

    // 先頭の自立語を入れ替えた候補 (例: 今日+は -> 京は, 教は...)。
    // 読みと同じ表記 (ひらがなのまま) は末尾で必ず追加するのでここでは除く
    let rest: String = group[1..].iter().map(|w| w.surface.as_str()).collect();
    for (_, surface) in exact_candidates(&group[0].reading, dict, user) {
        if result.len() >= MAX_DICT_CANDIDATES {
            break;
        }
        let candidate = surface.to_string() + &rest;
        if candidate != reading && !result.contains(&candidate) {
            result.push(candidate);
        }
    }

    // 数字で始まる文節 (「10じ」など) は、数字部分の読み全体が辞書に無いため
    // 上の完全一致・先頭語入れ替えが働かない。代わりに数字に続く部分 (助数詞など) を
    // 入れ替えた候補を積む (「10次」しか出ず「10時」が選べなくなるのを防ぐ)
    if group.len() >= 2 && group[0].reading.chars().all(is_digit_char) {
        let tail_reading: String = group[1..].iter().map(|w| w.reading.as_str()).collect();
        for (_, surface) in exact_candidates(&tail_reading, dict, user) {
            if result.len() >= MAX_DICT_CANDIDATES {
                break;
            }
            let candidate = group[0].surface.clone() + surface;
            if candidate != reading && !result.contains(&candidate) {
                result.push(candidate);
            }
        }
    }

    // 記号候補 (「やじるし」→「→」など)。通常語より後ろに置きたいので
    // 辞書候補の末尾に追記し、MAX_DICT_CANDIDATES の枠には数えない
    // (数えると記号が多い読みで通常語が押し出されるため)
    for symbol in dict.lookup_symbols(&reading) {
        if !result.contains(symbol) {
            result.push(symbol.clone());
        }
    }

    for extra in [to_katakana(&reading), reading.clone()] {
        if !result.contains(&extra) {
            result.push(extra);
        }
    }

    // 学習済みの表記を先頭へ (候補に無ければ追加)
    if let Some(learned) = learning.get(&reading) {
        result.retain(|s| s != learned);
        result.insert(0, learned.to_string());
    }
    Segment { reading, candidates: result }
}

/// かな文字列に対する全文一括の変換候補リストを返す (クエリツール・互換用)
pub fn candidates(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    // 文としての最小コスト変換 (入力と同じ = 変換できなかった場合は加えない)
    if let Some(sentence) = convert_sentence(kana, dict, user, matrix, functional) {
        if sentence != kana {
            result.push(sentence);
        }
    }

    // 短縮よみ (ユーザ辞書) は文変換候補の直後に置く (記載順)
    for shortcut in user.lookup_shortcuts(kana) {
        if !result.iter().any(|s| s == shortcut) {
            result.push(shortcut.to_string());
        }
    }

    // 読み全体の完全一致候補をコスト順に (同じ表記は除く)
    for (_, surface) in exact_candidates(kana, dict, user) {
        if result.len() >= MAX_DICT_CANDIDATES + 1 {
            break;
        }
        if !result.iter().any(|s| s == surface) {
            result.push(surface.to_string());
        }
    }

    // 記号候補 (文節候補と同様、通常語の後ろに追記する)
    for symbol in dict.lookup_symbols(kana) {
        if !result.contains(symbol) {
            result.push(symbol.clone());
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

/// ラティスを構築して最小コスト経路の表記を返す
pub fn convert_sentence(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<String> {
    let path = viterbi_path(kana, dict, user, matrix, functional)?;
    Some(path.into_iter().map(|w| w.surface).collect())
}

/// Viterbi 用のラティスノード
struct Node {
    /// 読みの開始位置 (文字単位)
    start: usize,
    reading: String,
    left_id: u16,
    right_id: u16,
    word_cost: i32,
    surface: String,
    /// BOS からこのノードまでの最小コスト
    best_cost: i64,
    /// 最小コスト経路での直前ノード (nodes 内の index)
    best_prev: usize,
}

/// ラティスを構築して最小コスト経路の単語列を返す
fn viterbi_path(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<Vec<PathWord>> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n == 0 {
        return None;
    }

    // nodes[0] は BOS (文頭)。文脈IDは 0 (BOS/EOS)
    let mut nodes: Vec<Node> = vec![Node {
        start: 0,
        reading: String::new(),
        left_id: 0,
        right_id: 0,
        word_cost: 0,
        surface: String::new(),
        best_cost: 0,
        best_prev: 0,
    }];

    // ending_at[p] = 位置 p で終わるノードの index 一覧 (BOS は位置 0 で終わる扱い)
    let mut ending_at: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    ending_at[0].push(0);

    // 辞書語ノードと未知語ノードを生成する
    for start in 0..n {
        // start から始まる登録語を1回のトライ走査でまとめて引く
        // (一致文字数の短い順に返るため、ノード生成順は旧来の end 昇順と同じ)
        for (len, entries) in dict.common_prefix_search(&chars[start..], MAX_READING_CHARS) {
            let end = start + len;
            let reading: String = chars[start..end].iter().collect();
            for entry in entries {
                nodes.push(Node {
                    start,
                    reading: reading.clone(),
                    left_id: entry.left_id,
                    right_id: entry.right_id,
                    word_cost: i32::from(entry.cost),
                    surface: entry.surface.clone(),
                    best_cost: i64::MAX,
                    best_prev: 0,
                });
                ending_at[end].push(nodes.len() - 1);
            }
        }
        // ユーザ登録の名詞系単語も通常の辞書語と同様にノードにする
        for (len, word) in user.common_prefix_words(&chars[start..]) {
            let end = start + len;
            nodes.push(Node {
                start,
                reading: word.reading.clone(),
                left_id: word.left_id,
                right_id: word.right_id,
                word_cost: i32::from(word.cost),
                surface: word.surface.clone(),
                best_cost: i64::MAX,
                best_prev: 0,
            });
            ending_at[end].push(nodes.len() - 1);
        }
        // 未知語ノード (1文字をそのまま出力)。どんな入力でも経路が成立する保険
        let ch = chars[start].to_string();
        nodes.push(Node {
            start,
            reading: ch.clone(),
            left_id: DEFAULT_NOUN_ID,
            right_id: DEFAULT_NOUN_ID,
            word_cost: UNKNOWN_WORD_COST,
            surface: ch,
            best_cost: i64::MAX,
            best_prev: 0,
        });
        ending_at[start + 1].push(nodes.len() - 1);
    }

    // Viterbi: ノードを開始位置順に処理し、直前ノード群から最小コストを選ぶ。
    // (nodes は生成順が開始位置昇順になっている。BOS を除いて回す)
    let order: Vec<usize> = {
        let mut idx: Vec<usize> = (1..nodes.len()).collect();
        idx.sort_by_key(|&i| nodes[i].start);
        idx
    };
    for i in order {
        // 自立語の開始 = 文節の開始とみなしてペナルティを加算する
        // (付属語は前の文節に吸収されるため対象外)
        let penalty = if functional.is_functional(nodes[i].left_id) {
            0
        } else {
            i64::from(SEGMENT_PENALTY)
        };
        let mut best_cost = i64::MAX;
        let mut best_prev = 0;
        for &p in &ending_at[nodes[i].start] {
            if nodes[p].best_cost == i64::MAX {
                continue; // 到達不能な経路
            }
            let cost = nodes[p].best_cost
                + i64::from(matrix.get(nodes[p].right_id, nodes[i].left_id))
                + i64::from(nodes[i].word_cost)
                + penalty;
            if cost < best_cost {
                best_cost = cost;
                best_prev = p;
            }
        }
        nodes[i].best_cost = best_cost;
        nodes[i].best_prev = best_prev;
    }

    // EOS: 位置 n で終わるノードから文末への接続コストを含めて最良を選ぶ
    let mut best_end: Option<usize> = None;
    let mut best_end_cost = i64::MAX;
    for &i in &ending_at[n] {
        if nodes[i].best_cost == i64::MAX {
            continue;
        }
        let cost = nodes[i].best_cost + i64::from(matrix.get(nodes[i].right_id, 0));
        if cost < best_end_cost {
            best_end_cost = cost;
            best_end = Some(i);
        }
    }

    // 経路を逆順にたどって単語列を作る
    let mut indices: Vec<usize> = Vec::new();
    let mut cursor = best_end?;
    while cursor != 0 {
        indices.push(cursor);
        cursor = nodes[cursor].best_prev;
    }
    indices.reverse();
    Some(
        indices
            .into_iter()
            .map(|i| PathWord {
                reading: std::mem::take(&mut nodes[i].reading),
                surface: std::mem::take(&mut nodes[i].surface),
                left_id: nodes[i].left_id,
            })
            .collect(),
    )
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
        let data = "きょう\t1\t1\t2000\t今日\n\
                    きょう\t1\t1\t4000\t京\n\
                    は\t2\t2\t500\tは\n\
                    はれ\t1\t1\t3000\t晴れ\n\
                    です\t3\t3\t1000\tです\n\
                    にほんご\t1\t1\t3793\t日本語\n\
                    にほんご\t1\t1\t7869\tニホンゴ\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    fn sample_functional() -> FunctionalIds {
        // id 2 = 助詞, id 3 = 助動詞, id 4 = 数詞, id 5 = 助数詞
        let data = "1 名詞,一般\n\
                    2 助詞,係助詞\n\
                    3 助動詞,特殊・デス\n\
                    4 名詞,数,アラビア数字\n\
                    5 名詞,接尾,助数詞\n";
        FunctionalIds::load_from(data.as_bytes()).unwrap()
    }

    /// ユーザ辞書なし (空) の省略用
    fn no_user() -> UserDict {
        UserDict::empty()
    }

    #[test]
    fn 文を最小コストで変換する() {
        // 今日(2000) + は(500) + 晴れ(3000) + です(1000) が最小経路になる
        let result = convert_sentence(
            "きょうははれです",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
        );
        assert_eq!(result.unwrap(), "今日は晴れです");
    }

    #[test]
    fn 付属語が前の文節にまとまる() {
        let segments = convert_segments(
            "きょうははれです",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょうは", "はれです"]);
        assert_eq!(segments[0].candidates[0], "今日は");
        assert_eq!(segments[1].candidates[0], "晴れです");
    }

    #[test]
    fn 文節候補にカタカナとひらがなを含む() {
        let segments = convert_segments(
            "きょうは",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        assert_eq!(c[0], "今日は");
        assert!(c.contains(&"キョウハ".to_string()));
        assert!(c.contains(&"きょうは".to_string()));
    }

    #[test]
    fn 記号が文節候補に入りカタカナより前に来る() {
        let mut dict = sample_dict();
        dict.load_symbols_from(
            "記号\t↑\tきょう やじるし\t上矢印 (テスト用の読み)\n".as_bytes(),
        )
        .unwrap();
        let segments = convert_segments(
            "きょう",
            &dict,
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        let symbol = c.iter().position(|s| s == "↑").unwrap();
        let katakana = c.iter().position(|s| s == "キョウ").unwrap();
        assert!(c.iter().position(|s| s == "今日").unwrap() < symbol);
        assert!(symbol < katakana);
    }

    /// 短縮よみだけを持つユーザ辞書を作る
    fn user_with_shortcut(reading: &str, surface: &str) -> UserDict {
        let mut user = UserDict::empty();
        user.load_from(
            format!("{reading}\t{surface}\t短縮よみ\n").as_bytes(),
            &FunctionalIds::empty(),
        );
        user
    }

    #[test]
    fn 短縮よみが文節候補の2番目に入る() {
        let segments = convert_segments(
            "きょう",
            &sample_dict(),
            &user_with_shortcut("きょう", "mail@example.com"),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        assert_eq!(c[0], "今日");
        assert_eq!(c[1], "mail@example.com");
    }

    #[test]
    fn 学習表記は短縮よみより前に出る() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょう", "京");
        let segments = convert_segments(
            "きょう",
            &sample_dict(),
            &user_with_shortcut("きょう", "mail@example.com"),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &learning,
        );
        let c = &segments[0].candidates;
        assert_eq!(c[0], "京");
        assert_eq!(c[1], "今日");
        assert_eq!(c[2], "mail@example.com");
    }

    #[test]
    fn 短縮よみが全文候補にも入る() {
        let got = candidates(
            "にほんご",
            &sample_dict(),
            &user_with_shortcut("にほんご", "NIHONGO"),
            &ConnectionMatrix::empty(),
            &sample_functional(),
        );
        assert_eq!(got, vec!["日本語", "NIHONGO", "ニホンゴ", "にほんご"]);
    }

    #[test]
    fn ユーザ登録の名詞が文中で変換される() {
        // 「かんべ」は辞書に無いが、ユーザ辞書の姓として登録されている
        let mut user = UserDict::empty();
        user.load_from("かんべ\t神戸\t姓\n".as_bytes(), &FunctionalIds::empty());
        let result = convert_sentence(
            "かんべです", &sample_dict(), &user, &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(result.unwrap(), "神戸です");
    }

    #[test]
    fn ユーザ登録の名詞が文節候補にも入る() {
        // 「きょう」に同読みのユーザ語を登録すると、辞書候補とコスト順で混ざる
        // (ユーザ語のコスト3000は 今日(2000) と 京(4000) の間)
        let mut user = UserDict::empty();
        user.load_from("きょう\t匡\t名\n".as_bytes(), &FunctionalIds::empty());
        let segments = convert_segments(
            "きょう",
            &sample_dict(),
            &user,
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let c = &segments[0].candidates;
        let user_word = c.iter().position(|s| s == "匡").unwrap();
        assert!(c.iter().position(|s| s == "今日").unwrap() < user_word);
        assert!(user_word < c.iter().position(|s| s == "京").unwrap());
    }

    #[test]
    fn 品詞表が無ければ単語ごとに文節になる() {
        let segments = convert_segments(
            "きょうは",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &FunctionalIds::empty(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょう", "は"]);
    }

    #[test]
    fn 辞書に無い文字は未知語としてそのまま通す() {
        let result = convert_sentence(
            "きょうはx", &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional());
        assert_eq!(result.unwrap(), "今日はx");
    }

    #[test]
    fn 連接コストが単語選択に影響する() {
        // 読み「あ」に同コストの2候補。BOS(右ID=0) からの連接コストで「阿」が勝つ
        let mut dict = Dictionary::empty();
        dict.load_from("あ\t1\t1\t100\t亜\nあ\t2\t2\t100\t阿\n".as_bytes()).unwrap();
        dict.finalize();
        // 3x3 行列: get(0,1)=1000 (亜への接続が高い), get(0,2)=0
        let matrix = ConnectionMatrix::parse(
            "3\n0\n1000\n0\n0\n0\n0\n0\n0\n0\n",
        )
        .unwrap();
        // 品詞表は空 (ペナルティは両候補に等しく載り、連接コストだけで決まる)
        let result = convert_sentence("あ", &dict, &no_user(), &matrix, &FunctionalIds::empty());
        assert_eq!(result.unwrap(), "阿");
    }

    #[test]
    fn 候補は文変換_完全一致_カタカナ_ひらがなの順() {
        let got = candidates(
            "にほんご", &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional());
        assert_eq!(got, vec!["日本語", "ニホンゴ", "にほんご"]);
    }

    #[test]
    fn 空文字列は文変換しない() {
        assert!(convert_sentence(
            "", &Dictionary::empty(), &no_user(), &ConnectionMatrix::empty(),
            &FunctionalIds::empty()).is_none());
        assert!(convert_segments(
            "",
            &Dictionary::empty(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &FunctionalIds::empty(),
            &LearningStore::in_memory()
        )
        .is_empty());
    }

    #[test]
    fn 先頭語を入れ替えた文節候補が出る() {
        let segments = convert_segments(
            "きょうは",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        // 経路は 今日+は。先頭語を 京 に入れ替えた「京は」も候補に入る
        assert!(segments[0].candidates.contains(&"京は".to_string()));
    }

    /// 「した」→「し+た」のように、1文字の自立語 + 付属語に分解される読みの辞書。
    /// 「し」の同音異義語を MAX_DICT_CANDIDATES 以上入れて、入れ替え候補が
    /// 枠を使い切る状況を作る (実際の Mozc 辞書で起きる状況の縮小版)
    fn dict_with_many_first_word_homophones() -> Dictionary {
        let mut dict = Dictionary::empty();
        let mut data = String::from(
            "し\t1\t1\t0\tし\n\
             た\t2\t2\t0\tた\n\
             した\t1\t1\t100\t下\n",
        );
        for i in 0..MAX_DICT_CANDIDATES {
            // 音読み「し」の漢字の代役としてダミー表記を積む
            data.push_str(&format!("し\t1\t1\t{}\t死{}\n", 200 + i, i));
        }
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 読み全体の完全一致が先頭語入れ替えより前に出る() {
        // 経路は し+た (cost 0+0)。読み全体の完全一致「下」が
        // 入れ替え候補 (死0た...) より前に来る
        let segments = convert_segments(
            "した",
            &dict_with_many_first_word_homophones(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        let whole = c.iter().position(|s| s == "下").unwrap();
        let swapped = c.iter().position(|s| s == "死0た").unwrap();
        assert!(whole < swapped);
    }

    #[test]
    fn 先頭語の同音異義語が多くても完全一致が候補から漏れない() {
        // 「し」のエントリが MAX_DICT_CANDIDATES 以上あっても「下」が候補に残る
        let segments = convert_segments(
            "した",
            &dict_with_many_first_word_homophones(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert!(segments[0].candidates.contains(&"下".to_string()));
    }

    #[test]
    fn 文節境界を固定して変換できる() {
        let dict = sample_dict();
        let learning = LearningStore::in_memory();

        // 4,4 なら通常の文節分割と同じ
        let segments = convert_segments_fixed(
            "きょうははれです", &[4, 4], &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょうは", "はれです"]);
        assert_eq!(segments[0].candidates[0], "今日は");

        // 3,5 なら「きょう / ははれです」で各範囲内を再変換する
        let segments = convert_segments_fixed(
            "きょうははれです", &[3, 5], &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょう", "ははれです"]);
        assert_eq!(segments[0].candidates[0], "今日");
        assert_eq!(segments[1].candidates[0], "は晴れです");
    }

    #[test]
    fn 文節長の合計が合わなければ空を返す() {
        let empty = convert_segments_fixed(
            "きょうは",
            &[3, 3],
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn 学習済みの表記が文節候補の先頭に来る() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょうは", "京は");
        let segments = convert_segments(
            "きょうは",
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &learning,
        );
        // 学習した「京は」(辞書候補に無い表記) が先頭に挿入される
        assert_eq!(segments[0].candidates[0], "京は");
        assert_eq!(segments[0].candidates[1], "今日は");
    }

    #[test]
    fn 文節境界ペナルティで複合語が細切れに勝つ() {
        // 素のコストでは き+き+無+れ (0+0+0+0) が 聞き慣れ (2000) より安いが、
        // 自立語4語 (ペナルティ2800) vs 1語 (700) の差で複合語が選ばれる
        let mut dict = Dictionary::empty();
        dict.load_from(
            "き\t1\t1\t0\tき\n\
             な\t1\t1\t0\t無\n\
             れ\t1\t1\t0\tれ\n\
             ききなれ\t1\t1\t2000\t聞き慣れ\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let result = convert_sentence(
            "ききなれ", &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(result.unwrap(), "聞き慣れ");
    }

    /// 実辞書と同様に数字が1桁単位でしか入っていない辞書 (id 4 = 数詞, 5 = 助数詞)
    fn digit_dict() -> Dictionary {
        let mut dict = Dictionary::empty();
        dict.load_from(
            "1\t4\t4\t1900\t1\n\
             2\t4\t4\t1900\t2\n\
             じ\t5\t5\t18\t時\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 連続する数字が助数詞ごと1文節にまとまる() {
        let segments = convert_segments(
            "12じ",
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["12じ"]);
        assert_eq!(segments[0].candidates[0], "12時");
    }

    #[test]
    fn 数字文節では助数詞の入れ替え候補が出る() {
        // 経路上の助数詞が「次」でも、読み「じ」の別候補「時」で入れ替えた 12時 が選べる
        let mut dict = Dictionary::empty();
        dict.load_from(
            "1\t4\t4\t1900\t1\n\
             2\t4\t4\t1900\t2\n\
             じ\t5\t5\t10\t次\n\
             じ\t5\t5\t18\t時\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let segments = convert_segments(
            "12じ",
            &dict,
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let c = &segments[0].candidates;
        assert_eq!(c[0], "12次");
        assert!(c.contains(&"12時".to_string()));
    }

    #[test]
    fn 全角数字も未知語のまま1文節にまとまる() {
        // 全角数字は辞書に無く未知語1文字ノードになるが、読みベースの判定で結合される
        let segments = convert_segments(
            "１２じ",
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["１２じ"]);
        assert_eq!(segments[0].candidates[0], "１２時");
    }

    #[test]
    fn 文節境界を固定すれば数字も分かれる() {
        // Shift+← などでユーザが数字を手動分割した場合はそのまま尊重する
        let segments = convert_segments_fixed(
            "12",
            &[1, 1],
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["1", "2"]);
    }

    #[test]
    fn 変換できない入力はカタカナとひらがなのみ() {
        // 空辞書では未知語経路が入力そのままを返すため、候補には加えない
        let got = candidates(
            "かな", &Dictionary::empty(), &no_user(), &ConnectionMatrix::empty(),
            &FunctionalIds::empty());
        assert_eq!(got, vec!["カナ", "かな"]);
    }
}
