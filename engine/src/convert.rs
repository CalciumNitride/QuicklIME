// 変換候補の生成
//
// フェーズ4-3: Viterbi の最小コスト経路を品詞情報で文節にまとめ、
// 文節ごとの候補リストを返す。
// 全文一括の候補 (candidates) も互換のため残している。

use crate::dict::Dictionary;
use crate::learn::LearningStore;
use crate::matrix::ConnectionMatrix;
use crate::pos::FunctionalIds;

/// 辞書由来の候補の最大数 (文全体・文節共通)。
/// かなの同音異義語は多い (例:「きょう」は約50語) ため、ある程度大きくしておく。
/// TSF 側の候補ウィンドウはページ表示で対応する
const MAX_DICT_CANDIDATES: usize = 24;

/// 辞書引きする読みの最大文字数 (ラティス構築時)
const MAX_READING_CHARS: usize = 16;

/// 未知語 (辞書に無い1文字) ノードの単語コスト。
/// 辞書語の経路が常に優先されるよう十分大きくする
const UNKNOWN_WORD_COST: i32 = 12000;

/// 未知語ノードに与える文脈ID (Mozc 辞書の一般名詞相当。暫定)
const UNKNOWN_WORD_ID: u16 = 1851;

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
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    let Some(path) = viterbi_path(kana, dict, matrix) else {
        return Vec::new();
    };

    // 付属語 (助詞・助動詞・接尾辞) を直前の自立語にまとめて文節を作る
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
        .map(|group| segment_from_group(group, dict, learning))
        .collect()
}

/// 文節境界 (文字数) を指定してかな文字列を変換する (Shift+←→ での文節伸縮用)。
/// lengths の合計が入力の文字数と一致しない場合は空を返す
pub fn convert_segments_fixed(
    kana: &str,
    lengths: &[usize],
    dict: &Dictionary,
    matrix: &ConnectionMatrix,
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
        let group = viterbi_path(&reading, dict, matrix).unwrap_or_else(|| {
            vec![PathWord {
                reading: reading.clone(),
                surface: reading.clone(),
                left_id: UNKNOWN_WORD_ID,
            }]
        });
        segments.push(segment_from_group(&group, dict, learning));
    }
    segments
}

/// 単語列 (1文節分) から候補リスト付きの Segment を作る。
/// 候補: 経路上の表記 → 先頭語を入れ替えた表記 → 読み全体の辞書候補
///       → カタカナ → ひらがな。学習済みの表記があれば先頭へ移動する
fn segment_from_group(group: &[PathWord], dict: &Dictionary, learning: &LearningStore) -> Segment {
    let reading: String = group.iter().map(|w| w.reading.as_str()).collect();
    let best: String = group.iter().map(|w| w.surface.as_str()).collect();
    let mut result = vec![best];

    // 短縮よみ (ユーザ辞書) は経路表記の直後に置く (記載順)。
    // 末尾の学習表記の先頭移動が最優先なのは変わらない
    for shortcut in dict.lookup_shortcuts(&reading) {
        if !result.iter().any(|s| s == shortcut) {
            result.push(shortcut.to_string());
        }
    }

    // 先頭の自立語を入れ替えた候補 (例: 今日+は -> 京は, 教は...)。
    // 読みと同じ表記 (ひらがなのまま) は末尾で必ず追加するのでここでは除く
    let rest: String = group[1..].iter().map(|w| w.surface.as_str()).collect();
    let mut firsts: Vec<_> = dict.lookup(&group[0].reading).iter().collect();
    firsts.sort_by_key(|e| e.cost);
    for entry in firsts {
        if result.len() >= MAX_DICT_CANDIDATES {
            break;
        }
        let candidate = entry.surface.clone() + &rest;
        if candidate != reading && !result.contains(&candidate) {
            result.push(candidate);
        }
    }

    // 読み全体の完全一致候補 (単語をまたぐ表記など)
    let mut entries: Vec<_> = dict.lookup(&reading).iter().collect();
    entries.sort_by_key(|e| e.cost);
    for entry in entries {
        if result.len() >= MAX_DICT_CANDIDATES {
            break;
        }
        if entry.surface != reading && !result.contains(&entry.surface) {
            result.push(entry.surface.clone());
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
pub fn candidates(kana: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    // 文としての最小コスト変換 (入力と同じ = 変換できなかった場合は加えない)
    if let Some(sentence) = convert_sentence(kana, dict, matrix) {
        if sentence != kana {
            result.push(sentence);
        }
    }

    // 短縮よみ (ユーザ辞書) は文変換候補の直後に置く (記載順)
    for shortcut in dict.lookup_shortcuts(kana) {
        if !result.iter().any(|s| s == shortcut) {
            result.push(shortcut.to_string());
        }
    }

    // 読み全体の完全一致候補をコスト順に (同じ表記は除く)
    let mut entries: Vec<_> = dict.lookup(kana).iter().collect();
    entries.sort_by_key(|e| e.cost);
    for entry in entries {
        if result.len() >= MAX_DICT_CANDIDATES + 1 {
            break;
        }
        if !result.contains(&entry.surface) {
            result.push(entry.surface.clone());
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
pub fn convert_sentence(kana: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> Option<String> {
    let path = viterbi_path(kana, dict, matrix)?;
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
fn viterbi_path(kana: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> Option<Vec<PathWord>> {
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
        // 未知語ノード (1文字をそのまま出力)。どんな入力でも経路が成立する保険
        let ch = chars[start].to_string();
        nodes.push(Node {
            start,
            reading: ch.clone(),
            left_id: UNKNOWN_WORD_ID,
            right_id: UNKNOWN_WORD_ID,
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
        let mut best_cost = i64::MAX;
        let mut best_prev = 0;
        for &p in &ending_at[nodes[i].start] {
            if nodes[p].best_cost == i64::MAX {
                continue; // 到達不能な経路
            }
            let cost = nodes[p].best_cost
                + i64::from(matrix.get(nodes[p].right_id, nodes[i].left_id))
                + i64::from(nodes[i].word_cost);
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
        // id 2 = 助詞, id 3 = 助動詞
        let data = "1 名詞,一般\n2 助詞,係助詞\n3 助動詞,特殊・デス\n";
        FunctionalIds::load_from(data.as_bytes()).unwrap()
    }

    #[test]
    fn 文を最小コストで変換する() {
        // 今日(2000) + は(500) + 晴れ(3000) + です(1000) が最小経路になる
        let result = convert_sentence("きょうははれです", &sample_dict(), &ConnectionMatrix::empty());
        assert_eq!(result.unwrap(), "今日は晴れです");
    }

    #[test]
    fn 付属語が前の文節にまとまる() {
        let segments = convert_segments(
            "きょうははれです",
            &sample_dict(),
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

    #[test]
    fn 短縮よみが文節候補の2番目に入る() {
        let mut dict = sample_dict();
        dict.load_shortcuts_from("きょう\tmail@example.com\t短縮よみ\n".as_bytes()).unwrap();
        let segments = convert_segments(
            "きょう",
            &dict,
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
        let mut dict = sample_dict();
        dict.load_shortcuts_from("きょう\tmail@example.com\t短縮よみ\n".as_bytes()).unwrap();
        let mut learning = LearningStore::in_memory();
        learning.record("きょう", "京");
        let segments = convert_segments(
            "きょう",
            &dict,
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
        let mut dict = sample_dict();
        dict.load_shortcuts_from("にほんご\tNIHONGO\t短縮よみ\n".as_bytes()).unwrap();
        let got = candidates("にほんご", &dict, &ConnectionMatrix::empty());
        assert_eq!(got, vec!["日本語", "NIHONGO", "ニホンゴ", "にほんご"]);
    }

    #[test]
    fn 品詞表が無ければ単語ごとに文節になる() {
        let segments = convert_segments(
            "きょうは",
            &sample_dict(),
            &ConnectionMatrix::empty(),
            &FunctionalIds::empty(),
            &LearningStore::in_memory(),
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょう", "は"]);
    }

    #[test]
    fn 辞書に無い文字は未知語としてそのまま通す() {
        let result = convert_sentence("きょうはx", &sample_dict(), &ConnectionMatrix::empty());
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
        let result = convert_sentence("あ", &dict, &matrix);
        assert_eq!(result.unwrap(), "阿");
    }

    #[test]
    fn 候補は文変換_完全一致_カタカナ_ひらがなの順() {
        let got = candidates("にほんご", &sample_dict(), &ConnectionMatrix::empty());
        assert_eq!(got, vec!["日本語", "ニホンゴ", "にほんご"]);
    }

    #[test]
    fn 空文字列は文変換しない() {
        assert!(convert_sentence("", &Dictionary::empty(), &ConnectionMatrix::empty()).is_none());
        assert!(convert_segments(
            "",
            &Dictionary::empty(),
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
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        // 経路は 今日+は。先頭語を 京 に入れ替えた「京は」も候補に入る
        assert!(segments[0].candidates.contains(&"京は".to_string()));
    }

    #[test]
    fn 文節境界を固定して変換できる() {
        let dict = sample_dict();
        let learning = LearningStore::in_memory();

        // 4,4 なら通常の文節分割と同じ
        let segments = convert_segments_fixed(
            "きょうははれです", &[4, 4], &dict, &ConnectionMatrix::empty(), &learning);
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょうは", "はれです"]);
        assert_eq!(segments[0].candidates[0], "今日は");

        // 3,5 なら「きょう / ははれです」で各範囲内を再変換する
        let segments = convert_segments_fixed(
            "きょうははれです", &[3, 5], &dict, &ConnectionMatrix::empty(), &learning);
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
            &ConnectionMatrix::empty(),
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
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &learning,
        );
        // 学習した「京は」(辞書候補に無い表記) が先頭に挿入される
        assert_eq!(segments[0].candidates[0], "京は");
        assert_eq!(segments[0].candidates[1], "今日は");
    }

    #[test]
    fn 変換できない入力はカタカナとひらがなのみ() {
        // 空辞書では未知語経路が入力そのままを返すため、候補には加えない
        let got = candidates("かな", &Dictionary::empty(), &ConnectionMatrix::empty());
        assert_eq!(got, vec!["カナ", "かな"]);
    }
}
