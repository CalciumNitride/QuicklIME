// 変換候補の生成
//
// フェーズ4-2: ラティス + Viterbi による文単位のかな漢字変換。
// 候補の並び: [文の変換結果] → [読み全体の辞書完全一致 (コスト順)] → カタカナ → ひらがな

use crate::dict::Dictionary;
use crate::matrix::ConnectionMatrix;

/// 辞書由来の完全一致候補の最大数
const MAX_DICT_CANDIDATES: usize = 5;

/// 辞書引きする読みの最大文字数 (ラティス構築時)
const MAX_READING_CHARS: usize = 16;

/// 未知語 (辞書に無い1文字) ノードの単語コスト。
/// 辞書語の経路が常に優先されるよう十分大きくする
const UNKNOWN_WORD_COST: i32 = 12000;

/// 未知語ノードに与える文脈ID (Mozc 辞書の一般名詞相当。暫定)
const UNKNOWN_WORD_ID: u16 = 1851;

/// かな文字列に対する変換候補リストを返す
pub fn candidates(kana: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    // 文としての最小コスト変換 (入力と同じ = 変換できなかった場合は加えない)
    if let Some(sentence) = convert_sentence(kana, dict, matrix) {
        if sentence != kana {
            result.push(sentence);
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

    // カタカナ・ひらがなは常に候補に含める
    for extra in [to_katakana(kana), kana.to_string()] {
        if !result.contains(&extra) {
            result.push(extra);
        }
    }
    result
}

/// Viterbi 用のラティスノード
struct Node {
    /// 読みの開始位置 (文字単位)
    start: usize,
    left_id: u16,
    right_id: u16,
    word_cost: i32,
    surface: String,
    /// BOS からこのノードまでの最小コスト
    best_cost: i64,
    /// 最小コスト経路での直前ノード (nodes 内の index)
    best_prev: usize,
}

/// ラティスを構築して最小コスト経路の表記を返す
pub fn convert_sentence(kana: &str, dict: &Dictionary, matrix: &ConnectionMatrix) -> Option<String> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n == 0 {
        return None;
    }

    // nodes[0] は BOS (文頭)。文脈IDは 0 (BOS/EOS)
    let mut nodes: Vec<Node> = vec![Node {
        start: 0,
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
        for end in (start + 1)..=n.min(start + MAX_READING_CHARS) {
            let reading: String = chars[start..end].iter().collect();
            for entry in dict.lookup(&reading) {
                nodes.push(Node {
                    start,
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
        nodes.push(Node {
            start,
            left_id: UNKNOWN_WORD_ID,
            right_id: UNKNOWN_WORD_ID,
            word_cost: UNKNOWN_WORD_COST,
            surface: chars[start].to_string(),
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

    // 経路を逆順にたどって表記を連結する
    let mut surfaces: Vec<&str> = Vec::new();
    let mut cursor = best_end?;
    while cursor != 0 {
        surfaces.push(&nodes[cursor].surface);
        cursor = nodes[cursor].best_prev;
    }
    surfaces.reverse();
    Some(surfaces.concat())
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
        dict
    }

    #[test]
    fn 文を最小コストで変換する() {
        // 今日(2000) + は(500) + 晴れ(3000) + です(1000) が最小経路になる
        let result = convert_sentence("きょうははれです", &sample_dict(), &ConnectionMatrix::empty());
        assert_eq!(result.unwrap(), "今日は晴れです");
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
    }

    #[test]
    fn 変換できない入力はカタカナとひらがなのみ() {
        // 空辞書では未知語経路が入力そのままを返すため、候補には加えない
        let got = candidates("かな", &Dictionary::empty(), &ConnectionMatrix::empty());
        assert_eq!(got, vec!["カナ", "かな"]);
    }
}
