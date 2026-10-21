// 変換候補の生成
//
// フェーズ4-3: Viterbi の最小コスト経路を品詞情報で文節にまとめ、
// 文節ごとの候補リストを返す。
// 入力全体を1単位とした N-best (convert_nbest) は docs/design/nbest.md を参照。
// 全文一括の候補 (candidates) も互換のため残している。

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

use crate::dict::Dictionary;
use crate::learn::{LearningStore, MIN_BOUNDARY_CHARS};
use crate::matrix::ConnectionMatrix;
use crate::number;
use crate::pos::{FunctionalIds, DEFAULT_NOUN_ID};
use crate::typo;
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

/// 読みの中の ASCII 英字の連続 (英単語) を1語にしたノードの単語コスト。
/// 英字混じりの読みで英単語が日本語の部分の変換を邪魔しないよう、一般的な名詞程度にする
const ASCII_WORD_COST: i32 = 5000;

/// 英字の連続を正しい表記 (github → GitHub) にしたノードの単語コストを ASCII_WORD_COST から
/// 下げる量。補正した候補を上位に、打ったとおりの候補をすぐ下に出す
const ASCII_FORM_BONUS: i32 = 100;

/// 文節境界ペナルティ。自立語 (付属語でない語) ごとに Viterbi のコストへ加算する。
/// 辞書にはコスト0の短いかな語が多く、素のコストでは「き+き+無+れ」のような
/// 細切れ経路が複合語 (聞き慣れ) より安くなるため、文節数が少ない経路を優先させる。
/// 値は実辞書の回帰コーパスで調整した (「ききなれない」は自立語1語差で約800必要)
const SEGMENT_PENALTY: i32 = 1000;

/// 学習済み文節境界による文節ペナルティの増減。
/// 境界の開始位置では減らして切りやすく、境界の内部では増やして切りにくくする。
/// SEGMENT_PENALTY より小さくして、辞書・連接コストを覆すほど強くしない
/// (学習した境界と紛らわしい別の文が、常に学習側へ引きずられるのを避ける)
const BOUNDARY_BONUS: i32 = 500;

/// 入力全体の N-best の上限件数 (重複を除いた経路の数)
const MAX_NBEST: usize = 10;

/// N-best に入れる経路のコストの上限 (1-best のコストからの差)。
/// 未知語まじりなど、質の悪い経路を候補に出さないため
const NBEST_COST_MARGIN: i64 = 5000;

/// N-best の探索で取り出す部分経路の数の上限。長い入力で打鍵を止めないため
const MAX_NBEST_EXPANSIONS: usize = 3000;

/// 1-best と違う文節が同じ候補 (同じ文節の言い換え) の上限件数。
/// 末尾の1文節の同音異義語だけで N-best の枠が埋まるのを防ぐ
const MAX_NBEST_PER_CLASS: usize = 4;

/// 文節 UI (CONVSEG・CONVCTX。docs/design/typo-segment-ui.md) の補正の語の単語コストに足す量。
/// TYPO_SHOW・TYPO_TOP とともに、打ち間違いの無い文で補正の候補が先頭にも 2 番目にも出ないよう
/// examples/typo_eval.rs の計測で決めた (docs/design/typo-conversion.md の「計測の結果」)。補正の経路は
/// どれも補正の語を 1 つだけ通るので、この値は補正の経路どうしの順位を変えず、Δ だけを
/// TYPO_COST / 文字数 下げる (短い読みほど補正の候補が出にくい)。入力全体の候補 (CONVNBEST) の補正は
/// 誤りの種類ごとのコスト (typo::Tuning) を使う
const TYPO_COST: i32 = 4000;

/// 文節 UI の補正の Δ (通常の最良経路と補正の最良経路のコストの差を、打ったかなの文字数で
/// 割った値) の、補正の文節を出す下限
const TYPO_SHOW: f64 = -50.0;

/// 文節 UI の補正の表記を先頭に置く Δ の下限
const TYPO_TOP: f64 = 0.0;

/// 補正の語を、打ったかなの区間ごとに入れる上限の語数。補正した読みは数が多く、短い語
/// (1 文字の助詞など) が同じ区間に積み重なってラティスが膨らむため
const MAX_TYPO_WORDS: usize = 6;

/// 変換結果の1文節
pub struct Segment {
    /// この文節の読み (ひらがな)
    pub reading: String,
    /// 候補リスト (先頭が最良)
    pub candidates: Vec<String>,
    /// 候補に足した補正の表記 (docs/design/typo-segment-ui.md)。補正の文節でなければ None
    pub typo: Option<String>,
}

/// Viterbi 経路上の1単語
struct PathWord {
    reading: String,
    surface: String,
    left_id: u16,
    right_id: u16,
}

/// 入力全体の候補 (CONVNBEST の1件)。文節ごとの (読み, 表記) を持つ
pub struct SentenceCandidate {
    pub segments: Vec<(String, String)>,
    /// LLM の並べ替えで位置を動かさない候補 (読み全体の学習表記、学習で表記が変わった 1-best、
    /// ユーザ辞書の語を含む候補、英字を含む候補、数字の書き方の候補、補正の候補。
    /// docs/design/llm-rerank.md)
    pub protected: bool,
    /// 1 打の誤りを戻した読みの語 (補正の語) を通る経路の補正の候補 (docs/design/typo-conversion.md、
    /// docs/design/unified-suggest.md の D)
    pub typo: bool,
}

impl SentenceCandidate {
    /// 文節の表記の連結
    pub fn surface(&self) -> String {
        self.segments.iter().map(|(_, surface)| surface.as_str()).collect()
    }
}

/// N-best 経路の1文節
struct PathSegment {
    /// 読みの開始位置 (文字単位)
    start: usize,
    reading: String,
    surface: String,
}

/// 前文脈 (直前に確定した文節の読みと表記)。
/// ビタビの文頭文脈IDの復元に使い、確定直後の変換で連接コストが働くようにする
pub struct Context {
    pub reading: String,
    pub surface: String,
}

/// 前文脈から、ビタビの文頭に使う文脈ID (直前語の right_id) を復元する。
/// 復元できなければ 0 (BOS = 前文脈なしの従来動作)
pub fn resolve_context_id(
    ctx: &Context,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> u16 {
    if ctx.reading.is_empty() || ctx.surface.is_empty() {
        return 0;
    }
    // 前文脈を変換し直し、経路の表記が確定表記と一致すれば末尾語の right_id を使う
    if let Some(path) = viterbi_path(&ctx.reading, dict, user, matrix, functional, 0, None) {
        let joined: String = path.iter().map(|w| w.surface.as_str()).collect();
        if joined == ctx.surface {
            return path.last().map_or(0, |w| w.right_id);
        }
    }
    // 経路と違う表記が確定されていた場合: 読みの末尾を後方最長一致で辞書引きし、
    // 確定表記の末尾とも一致する最小コストのエントリの right_id を使う
    // (文節末尾は助詞・助動詞・活用語尾であることが多く、これでほぼ拾える)
    let chars: Vec<char> = ctx.reading.chars().collect();
    for len in (1..=chars.len().min(MAX_READING_CHARS)).rev() {
        let tail: String = chars[chars.len() - len..].iter().collect();
        let mut best: Option<(i16, u16)> = None;
        let hits = dict
            .lookup(&tail)
            .iter()
            .map(|e| (e.cost, e.surface.as_str(), e.right_id))
            .chain(
                user.lookup_words(&tail)
                    .into_iter()
                    .map(|w| (w.cost, w.surface.as_str(), w.right_id)),
            )
            .chain(
                user.imported_words(&tail)
                    .into_iter()
                    .map(|e| (e.cost, e.surface.as_str(), e.right_id)),
            );
        for (cost, surface, right_id) in hits {
            if ctx.surface.ends_with(surface) && best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, right_id));
            }
        }
        if let Some((_, right_id)) = best {
            return right_id;
        }
    }
    0
}

/// かな文字列を文節列へ変換する。ctx は直前に確定した文節 (無ければ None)。
/// typo なら、補正の最良経路 (docs/design/typo-segment-ui.md) の Δ が TYPO_SHOW 以上のときに
/// その経路で文節を区切り、補正の語を含む文節の候補に補正の表記を 1 件足す
pub fn convert_segments(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    typo: bool,
) -> Vec<Segment> {
    let ctx_id = ctx.map_or(0, |c| resolve_context_id(c, dict, user, matrix, functional));
    let typo_words = typo.then(|| TypoWords::segment(kana));
    let learned = Learned::full(learning);
    let Some(lattice) = build_lattice(
        kana, dict, user, matrix, functional, ctx_id, Some(learned), typo_words.as_ref(),
    ) else {
        return Vec::new();
    };
    let Some(indices) = best_path(&lattice, matrix) else {
        return Vec::new();
    };
    let corrected = if typo { typo_route(&lattice, matrix, kana) } else { None };
    // 補正の語の打ったかなの区間。補正の語は経路に 1 つなので、それを含む文節も 1 つ
    let Some((delta, typo_indices, (typo_start, typo_end))) = corrected.and_then(|(d, route)| {
        let node = route.iter().map(|&i| &lattice.nodes[i]).find(|n| n.typo)?;
        let span = (node.start, node.start + node.reading.chars().count());
        Some((d, route, span))
    }) else {
        let words = lattice.path_words(&indices);
        return segments_from_path(words, dict, user, functional, learning);
    };
    let (_, protected) =
        protected_best(kana, &lattice, &indices, ctx_id, learned, dict, user, matrix, functional, true);
    let top = delta >= TYPO_TOP && learning.best_phrase(kana).is_none() && !protected;

    let mut segments: Vec<Segment> = Vec::new();
    let mut start = 0;
    for group in &group_path(lattice.path_words(&typo_indices), functional) {
        let end = start + group.iter().map(|w| w.reading.chars().count()).sum::<usize>();
        let segment = if start <= typo_start && typo_end <= end {
            // 区間の通常の変換は文節長を固定した変換 (convert_segments_fixed) と同じく作る
            let left_id = segments.last().map_or(ctx_id, |prev: &Segment| {
                let prev_ctx = Context {
                    reading: prev.reading.clone(),
                    surface: prev.candidates[0].clone(),
                };
                resolve_context_id(&prev_ctx, dict, user, matrix, functional)
            });
            typo_segment(group, top, left_id, dict, user, matrix, functional, learning)
        } else {
            segment_from_group(group, dict, user, learning)
        };
        segments.push(segment);
        start = end;
    }
    segments
}

/// 補正の文節 (補正の経路で補正の語を含む文節 group) の Segment を作る。候補は、区間の読みだけの
/// 経路 (文頭の文脈 ID は left_id) の通常の候補に、補正の表記 (group の表記) を 1 件足したもの。
/// 補正の表記は、top で通常の候補の先頭が学習で変わっていなければ先頭、それ以外は 2 番目に置き、
/// 通常の候補に既にあれば足さない
fn typo_segment(
    group: &[PathWord],
    top: bool,
    left_id: u16,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Segment {
    let reading: String = group.iter().map(|w| w.reading.as_str()).collect();
    let corrected: String = group.iter().map(|w| w.surface.as_str()).collect();
    let learned = Learned::fixed(learning);
    let words = viterbi_path(&reading, dict, user, matrix, functional, left_id, Some(learned))
        .unwrap_or_else(|| {
            vec![PathWord {
                reading: reading.clone(),
                surface: reading.clone(),
                left_id: DEFAULT_NOUN_ID,
                right_id: DEFAULT_NOUN_ID,
            }]
        });
    let raw: String = words.iter().map(|w| w.surface.as_str()).collect();
    let mut segment = segment_from_group(&words, dict, user, learning);
    if !segment.candidates.contains(&corrected) {
        let position = if top && segment.candidates[0] == raw { 0 } else { 1 };
        segment.candidates.insert(position, corrected.clone());
        segment.typo = Some(corrected);
    }
    segment
}

/// 1-best の経路 (学習の割引を当てたラティス lattice のノード index 列 indices) を文節にまとめ、
/// 補正の候補に先頭を譲らない印 (ユーザ辞書の語・英字を含む、check_learned なら学習で表記が変わった)
/// とともに返す。学習で表記が変わったかは、学習の割引なしのラティスを作り直して最良経路の表記と比べる
fn protected_best(
    kana: &str,
    lattice: &Lattice,
    indices: &[usize],
    ctx_id: u16,
    learned: Learned,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    check_learned: bool,
) -> (Vec<Segment>, bool) {
    let words = lattice.path_words(indices);
    let has_user = words.iter().any(|w| is_user_word(&w.reading, &w.surface, user));
    let segments = segments_from_path(words, dict, user, functional, learned.store);
    let surface: String = segments.iter().map(|s| s.candidates[0].as_str()).collect();
    let changed = check_learned && {
        let plain = Learned { discount: false, ..learned };
        build_lattice(kana, dict, user, matrix, functional, ctx_id, Some(plain), None)
            .and_then(|plain| best_path(&plain, matrix).map(|i| plain.path_words(&i)))
            // 数字列の結合で区切り記号の表記が変わるため、結合後の表記どうしで比べる
            .is_some_and(|words| {
                merge_digit_runs(words).iter().map(|w| w.surface.as_str()).collect::<String>() != surface
            })
    };
    let protected = changed || has_user || surface.chars().any(|c| c.is_ascii_alphabetic());
    (segments, protected)
}

/// 経路の単語列を文節にまとめる (連続する数字を1語にし、付属語を前の自立語に付ける)
fn group_path(path: Vec<PathWord>, functional: &FunctionalIds) -> Vec<Vec<PathWord>> {
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
}

/// 1-best 経路から候補リスト付きの文節列を作る
fn segments_from_path(
    path: Vec<PathWord>,
    dict: &Dictionary,
    user: &UserDict,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    group_path(path, functional)
        .iter()
        .map(|group| segment_from_group(group, dict, user, learning))
        .collect()
}

/// 数字 (半角・全角) かどうか
fn is_digit_char(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// 数字列の区切りに使える記号かどうか。
/// TSF 層は記号キーを全角かなにして未確定文字列へ入れるため (. → 。, , → 、,
/// / → ・, - → ー)、記号本来の形だけでなくそれらのかな形も区切りとして受ける
fn is_number_separator(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | '/' | ':' | '-'
            | '．' | '，' | '／' | '：' | '－'
            | '。' | '、' | '・' | 'ー'
    )
}

/// 区切り記号の表記を記号本来の形へ直す (fullwidth なら全角形)。
/// 数字に挟まれた「。」は句点ではなく小数点なので「0.12」と出す。
/// 数字が続かない「。」は句点のままにしたいので、結合できたときだけ適用する
fn normalized_separator(c: char, fullwidth: bool) -> char {
    let (half, full) = match c {
        '.' | '．' | '。' => ('.', '．'),
        ',' | '，' | '、' => (',', '，'),
        '/' | '／' | '・' => ('/', '／'),
        ':' | '：' => (':', '：'),
        '-' | '－' | 'ー' => ('-', '－'),
        other => return other,
    };
    if fullwidth { full } else { half }
}

/// 数字列 (数字と区切り記号だけからなり、末尾が数字) かどうか。
/// 結合済みの「0。1」にさらに桁や区切りを継げるかの判定に使う
fn is_number_run(reading: &str) -> bool {
    reading.chars().next_back().is_some_and(is_digit_char)
        && reading.chars().all(|c| is_digit_char(c) || is_number_separator(c))
}

/// 読みが区切り記号1文字だけの語かどうか
fn is_separator_word(reading: &str) -> bool {
    let mut chars = reading.chars();
    chars.next().is_some_and(is_number_separator) && chars.next().is_none()
}

/// word を last の末尾へ連結する (連接IDは結合後の右端のものになる)
fn append_word(last: &mut PathWord, word: PathWord) {
    last.reading.push_str(&word.reading);
    last.surface.push_str(&word.surface);
    last.right_id = word.right_id;
}

/// 経路上で隣接する数字語 (読みがすべて数字) を1語に結合する。
/// 辞書の数字エントリは1桁単位のため、そのままでは「12」が桁ごとの文節に割れる。
/// 数字に挟まれた区切り記号 (0.12, 1/2, 1,000, 12:30, 2026-09-08) も同じ語に取り込む。
/// 読みは未確定文字列の文字位置と対応するので変えず、表記だけ記号本来の形へ直す。
/// 全角数字は辞書に無く未知語1文字ノードになるが、読みベースの判定で同様にまとまる
fn merge_digit_runs(path: Vec<PathWord>) -> Vec<PathWord> {
    let mut result: Vec<PathWord> = Vec::new();
    // 数字列の直後に来た区切り記号。後ろに数字が続いたときだけ数字列へ取り込む
    // (「1、みかん」の読点は区切りではないので、そのまま1語として切り出す)
    let mut pending: Option<PathWord> = None;
    for word in path {
        if is_number_run(&word.reading) {
            if let Some(mut separator) = pending.take() {
                let last = result.last_mut().unwrap();
                let fullwidth = !last.surface.ends_with(|c: char| c.is_ascii_digit());
                separator.surface = separator
                    .surface
                    .chars()
                    .map(|c| normalized_separator(c, fullwidth))
                    .collect();
                append_word(last, separator);
                append_word(last, word);
                continue;
            }
            match result.last_mut() {
                Some(last) if is_number_run(&last.reading) => append_word(last, word),
                _ => result.push(word),
            }
            continue;
        }
        if let Some(separator) = pending.take() {
            result.push(separator);
        }
        if is_separator_word(&word.reading)
            && result.last().is_some_and(|last| is_number_run(&last.reading))
        {
            pending = Some(word);
        } else {
            result.push(word);
        }
    }
    if let Some(separator) = pending {
        result.push(separator);
    }
    result
}

/// 文節境界 (文字数) を指定してかな文字列を変換する (Shift+←→ での文節伸縮用)。
/// lengths の合計が入力の文字数と一致しない場合は空を返す。
/// ctx は直前に確定した文節。文節ごとに独立ビタビのため、2文節目以降は
/// 直前文節の先頭候補を文脈として連鎖させる
pub fn convert_segments_fixed(
    kana: &str,
    lengths: &[usize],
    ctx: Option<&Context>,
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

    let mut ctx_id = ctx.map_or(0, |c| resolve_context_id(c, dict, user, matrix, functional));
    let mut segments = Vec::new();
    let mut begin = 0;
    for &length in lengths {
        let reading: String = chars[begin..begin + length].iter().collect();
        begin += length;

        // 文節の範囲内だけで最小コスト経路を求める
        let learned = Learned::fixed(learning);
        let group = viterbi_path(&reading, dict, user, matrix, functional, ctx_id, Some(learned))
            .unwrap_or_else(|| {
                vec![PathWord {
                    reading: reading.clone(),
                    surface: reading.clone(),
                    left_id: DEFAULT_NOUN_ID,
                    right_id: DEFAULT_NOUN_ID,
                }]
            });
        let segment = segment_from_group(&group, dict, user, learning);
        let next_ctx = Context {
            reading: segment.reading.clone(),
            surface: segment.candidates[0].clone(),
        };
        ctx_id = resolve_context_id(&next_ctx, dict, user, matrix, functional);
        segments.push(segment);
    }
    segments
}

/// 読みに完全一致する候補を (コスト, 表記) で集める。
/// システム辞書・ユーザ登録の名詞系単語・インポート辞書をまとめてコスト昇順に並べる
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
        .chain(user.imported_words(reading).into_iter().map(|e| (e.cost, e.surface.as_str())))
        .collect();
    hits.sort_by_key(|(cost, _)| *cost);
    hits
}

/// 単語列 (1文節分) から候補リスト付きの Segment を作る。
/// 候補: 経路上の表記 → 文節の読みの句の学習の表記 (割引の大きい順) → 短縮よみ → 読み全体の辞書候補
///       → 先頭語を入れ替えた表記 → カタカナ → ひらがな。
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

    for learned in learning.phrase_surfaces(&reading) {
        if !result.iter().any(|s| s == learned) {
            result.push(learned.to_string());
        }
    }

    // 短縮よみ (ユーザ辞書) は経路表記と学習の表記の直後に置く (記載順)
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
    if group.len() >= 2 && is_number_run(&group[0].reading) {
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

    // 数字で打った数の別の書き方 (千二百三十四円・1,234円 など)。書き方は5通りまでで
    // 助数詞の入れ替え候補に押し出されないよう、MAX_DICT_CANDIDATES の枠には数えない
    for form in number::forms(&group[0].reading).into_iter().flatten() {
        let candidate = form + &rest;
        if !result.contains(&candidate) {
            result.push(candidate);
        }
    }

    // 単漢字候補 (「あられ」→ 霰 など)。記号と同じ理由で MAX_DICT_CANDIDATES の枠に数えない
    // (「ひょう」のように単漢字が数十字ある読みで通常語が押し出されるため)
    for kanji in dict.lookup_single_kanji(&reading) {
        if !result.contains(kanji) {
            result.push(kanji.clone());
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
    Segment { reading, candidates: result, typo: None }
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

    // 単漢字・記号候補 (文節候補と同様、通常語の後ろに追記する)
    for kanji in dict.lookup_single_kanji(kana) {
        if !result.contains(kanji) {
            result.push(kanji.clone());
        }
    }
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
    let path = viterbi_path(kana, dict, user, matrix, functional, 0, None)?;
    Some(path.into_iter().map(|w| w.surface).collect())
}

/// かな文字列に対する入力全体の候補 (CONVNBEST) を返す。ctx は直前に確定した文節。
/// 並び: 読み全体の学習表記 → 学習の割引を当てた 1-best → N-best の残り (コスト順)
///       → 1-best の数を書き方ごとにそろえた候補 → 短縮よみ・読み全体の辞書完全一致
///       → 単漢字・記号・カタカナ・ひらがな。
/// typo なら、かなを標準のローマ字に戻した近似で補正の候補 (docs/design/typo-conversion.md) を
/// 1-best の前か直後に 1 件足す。表記が同じ候補は先勝ちで除く
#[allow(dead_code)] // エンジン本体は convert_nbest_scored を使う。テストと計測用の example から使う
pub fn convert_nbest(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    typo: bool,
) -> Vec<SentenceCandidate> {
    let source = if typo { TypoSource::Kana } else { TypoSource::Off };
    convert_nbest_scored(kana, ctx, dict, user, matrix, functional, learning, source, &typo::TUNING)
        .candidates
}

/// 入力全体の候補 (convert_nbest_scored の結果)
pub struct NBest {
    pub candidates: Vec<SentenceCandidate>,
    /// 候補ごとの経路のコスト。経路のコストを持たない候補 (読み全体の学習表記・先頭に置いた補正の
    /// 候補・数字の書き方・辞書の完全一致など) は None
    pub costs: Vec<Option<i64>>,
    /// 通常の最良経路 (学習の割引を当てた 1-best) の候補の位置
    pub best: Option<usize>,
    /// 通常の最良経路のコスト
    pub best_cost: Option<i64>,
    /// 先頭が読み全体の学習表記か
    pub learned: bool,
    /// 文頭の文脈 ID (前文脈から復元したもの。無ければ 0)
    pub context_id: u16,
}

/// convert_nbest と同じ並びを、候補ごとの経路のコストなどとともに返す (SUGGEST 用)。
/// 補正の候補の元は source、補正のコスト・状態の数・Δ のしきい値は tuning に従う
pub fn convert_nbest_scored(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    source: TypoSource,
    tuning: &typo::Tuning,
) -> NBest {
    nbest_with(kana, ctx, dict, user, matrix, functional, learning, source, tuning, false)
}

/// LLM の並べ替え (RERANK) に渡す convert_nbest_scored。補正の候補が無くても、学習で表記が変わった
/// 1-best を保護する (並べ替えは非同期なので、割引なしの Viterbi をもう 1 回解いてよい)
pub fn convert_nbest_rerank(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    source: TypoSource,
    tuning: &typo::Tuning,
) -> NBest {
    nbest_with(kana, ctx, dict, user, matrix, functional, learning, source, tuning, true)
}

/// convert_nbest_scored の本体。protect_learned なら補正の候補の有無によらず、学習で表記が変わった 1-best を
/// 保護する
fn nbest_with(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
    source: TypoSource,
    tuning: &typo::Tuning,
    protect_learned: bool,
) -> NBest {
    let ctx_id = ctx.map_or(0, |c| resolve_context_id(c, dict, user, matrix, functional));
    let typo_words = TypoWords::sentence(kana, source, tuning);
    let mut nbest = NBest {
        candidates: Vec::new(),
        costs: Vec::new(),
        best: None,
        best_cost: None,
        learned: false,
        context_id: ctx_id,
    };
    let learned = Learned::full(learning);
    let Some(lattice) = build_lattice(
        kana, dict, user, matrix, functional, ctx_id, Some(learned), typo_words.as_ref(),
    ) else {
        return nbest;
    };

    let mut surfaces: Vec<String> = Vec::new();
    let result = &mut nbest.candidates;
    let costs = &mut nbest.costs;
    // protected は学習・ユーザ辞書語の印。英字を含むかは表記から決まるのでここで足す。
    // 戻り値は候補の位置 (表記が既にあればその候補の位置) と、足したか
    let mut add = |segments: Vec<(String, String)>,
                   protected: bool,
                   typo: bool,
                   cost: Option<i64>|
     -> (usize, bool) {
        let mut candidate = SentenceCandidate { segments, protected, typo };
        let surface = candidate.surface();
        if let Some(i) = surfaces.iter().position(|s| *s == surface) {
            return (i, false);
        }
        candidate.protected |= surface.chars().any(|c| c.is_ascii_alphabetic());
        surfaces.push(surface);
        result.push(candidate);
        costs.push(cost);
        (result.len() - 1, true)
    };
    let whole = |surface: &str| vec![(kana.to_string(), surface.to_string())];

    let learned_whole = learning.best_phrase(kana);
    if let Some(learned) = learned_whole {
        add(whole(learned), true, false, None);
        nbest.learned = true;
    }
    // Δ = 通常の最良経路と補正の最良経路のコストの差を、打ったかなの文字数で割った値
    let chars = kana.chars().count() as f64;
    let corrected = if typo_words.is_some() { typo_best(&lattice, matrix, functional) } else { None }
        .map(|(gain, segments, cost)| (gain as f64 / chars, segments, cost))
        .filter(|(delta, _, _)| *delta >= tuning.typo_show);
    // 1-best の文節と、その元の単語列 (文節ごと)。数字の書き方の候補に使う
    let mut best_segments: Option<(Vec<(String, String)>, Vec<Vec<PathWord>>)> = None;
    if let Some(indices) = best_path(&lattice, matrix) {
        let best_cost = best_end(&lattice, matrix, 0).map(|(cost, _)| cost);
        nbest.best_cost = best_cost;
        // 学習・ユーザ辞書の語・英字で保護する 1-best は、補正の候補で 2 番目に下げない
        let (segments, protected) = protected_best(
            kana, &lattice, &indices, ctx_id, learned, dict, user, matrix, functional,
            protect_learned || corrected.is_some(),
        );
        let segments: Vec<(String, String)> =
            segments.into_iter().map(|s| (s.reading, s.candidates[0].clone())).collect();
        let top = learned_whole.is_none() && !protected;
        let best = match corrected {
            Some((delta, typo_segments, _)) if top && delta >= tuning.typo_top => {
                add(typo_segments, true, true, None);
                add(segments.clone(), protected, false, best_cost).0
            }
            Some((_, typo_segments, typo_cost)) => {
                let best = add(segments.clone(), protected, false, best_cost).0;
                add(typo_segments, true, true, Some(typo_cost));
                best
            }
            None => add(segments.clone(), protected, false, best_cost).0,
        };
        nbest.best = Some(best);
        best_segments = Some((segments, group_path(lattice.path_words(&indices), functional)));
    }
    for (cost, path, words) in nbest_paths(&lattice, matrix, functional) {
        let has_user = words.iter().any(|w| is_user_word(&w.reading, &w.surface, user));
        add(path.into_iter().map(|s| (s.reading, s.surface)).collect(), has_user, false, Some(cost));
    }

    // 数字の読みに対する漢数字などは LLM の尤度が高く出やすく、打ったとおりの数より
    // 上へ並べ替えられないよう保護する
    if let Some((segments, groups)) = &best_segments {
        let forms: Vec<[Option<String>; number::FORM_COUNT]> =
            groups.iter().map(|group| number::forms(&group[0].reading)).collect();
        for kind in 0..number::FORM_COUNT {
            if forms.iter().all(|f| f[kind].is_none()) {
                continue;
            }
            let replaced = segments
                .iter()
                .zip(groups)
                .zip(&forms)
                .map(|(((reading, surface), group), f)| {
                    let Some(form) = &f[kind] else {
                        return (reading.clone(), surface.clone());
                    };
                    // 文節の表記が学習で数字ごと変わっていれば、続く部分は経路の表記を使う
                    let tail = surface.strip_prefix(group[0].surface.as_str()).map_or_else(
                        || group[1..].iter().map(|w| w.surface.as_str()).collect(),
                        str::to_string,
                    );
                    (reading.clone(), form.clone() + &tail)
                })
                .collect();
            add(replaced, true, false, None);
        }
    }

    // 縦の候補リストで、読みそのものの別表記を選べるようにする (CONVERT の並びと同じ)
    let mut dict_count = 0;
    let shortcuts = user.lookup_shortcuts(kana).into_iter().map(|surface| (surface, true));
    let exact = exact_candidates(kana, dict, user)
        .into_iter()
        .map(|(_, surface)| (surface, is_user_word(kana, surface, user)));
    for (surface, from_user) in shortcuts.chain(exact) {
        if dict_count >= MAX_DICT_CANDIDATES {
            break;
        }
        if add(whole(surface), from_user, false, None).1 {
            dict_count += 1;
        }
    }
    for kanji in dict.lookup_single_kanji(kana) {
        add(whole(kanji), false, false, None);
    }
    for symbol in dict.lookup_symbols(kana) {
        add(whole(symbol), false, false, None);
    }
    add(whole(&to_katakana(kana)), false, false, None);
    add(whole(kana), false, false, None);
    nbest
}

// (読み, 表記) がユーザ辞書 (手動登録・学習した複合語・インポート辞書) の名詞系の語か
fn is_user_word(reading: &str, surface: &str, user: &UserDict) -> bool {
    user.lookup_words(reading).iter().any(|w| w.surface == surface)
        || user.imported_words(reading).iter().any(|e| e.surface == surface)
}

/// 後ろ向き探索の部分経路 (node から EOS まで)
struct Partial {
    node: usize,
    /// 経路上で node の次のノード (partials の index)。node が文末の語なら None
    next: Option<usize>,
    /// node より後ろ (node 自身は含まない) のコスト。EOS への接続を含む
    suffix: i64,
}

/// 前向きの最小コストを見積もりにして文末から A* 探索し、取り出した経路を最大 MAX_NBEST 件、
/// (総コスト, 文節列, 単語列) で返す。表記の連結が同じ経路は1件にまとめ、1-best と違う文節が同じ
/// 分類の中は並べ直して MAX_NBEST_PER_CLASS 件だけ残す (分類どうしの並びはコスト順)
fn nbest_paths(
    lattice: &Lattice,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Vec<(i64, Vec<PathSegment>, Vec<PathSegment>)> {
    let nodes = &lattice.nodes;
    let n = lattice.ending_at.len() - 1;

    // 前向きの最小コストは「BOS からそのノードまで」の正確な値なので、見積もり
    // (前向きの最小コスト + 後ろの確定コスト) の小さい順に取り出すと経路の総コスト順になる
    let mut partials: Vec<Partial> = Vec::new();
    let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
    for &i in &lattice.ending_at[n] {
        if nodes[i].best_cost == i64::MAX {
            continue;
        }
        let suffix = lattice.eos_cost(i, matrix);
        partials.push(Partial { node: i, next: None, suffix });
        heap.push(Reverse((nodes[i].best_cost + suffix, partials.len() - 1)));
    }

    // 完成した経路 (総コスト, 文節列, 単語列)。表記の重複は除いてコスト順
    let mut found: Vec<(i64, Vec<PathSegment>, Vec<PathSegment>)> = Vec::new();
    let mut surfaces: HashSet<String> = HashSet::new();
    let mut expansions = 0;
    while let Some(Reverse((cost, index))) = heap.pop() {
        expansions += 1;
        if expansions > MAX_NBEST_EXPANSIONS {
            break;
        }
        let node = partials[index].node;
        if node != 0 {
            let suffix =
                partials[index].suffix + i64::from(nodes[node].word_cost) + nodes[node].penalty;
            for &p in &lattice.ending_at[nodes[node].start] {
                if nodes[p].best_cost == i64::MAX {
                    continue;
                }
                let suffix = suffix + i64::from(matrix.get(nodes[p].right_id, nodes[node].left_id));
                partials.push(Partial { node: p, next: Some(index), suffix });
                heap.push(Reverse((nodes[p].best_cost + suffix, partials.len() - 1)));
            }
            continue;
        }

        // BOS まで届いた = 経路が1本完成した
        if found.first().is_some_and(|(best, _, _)| cost > best + NBEST_COST_MARGIN) {
            break;
        }
        let mut indices: Vec<usize> = Vec::new();
        let mut cursor = partials[index].next;
        while let Some(c) = cursor {
            indices.push(partials[c].node);
            cursor = partials[c].next;
        }
        let (segments, words) = path_segments(lattice.path_words(&indices), functional);
        let surface: String = segments.iter().map(|s| s.surface.as_str()).collect();
        // 同じ表記の語が品詞違いで複数あるため、区切りや品詞だけが違う経路が多数出る
        if surfaces.insert(surface) {
            found.push((cost, segments, words));
        }
    }
    if found.is_empty() {
        return Vec::new();
    }

    // 1-best と違う文節の (開始位置, 読み) の並びで分類し、分類ごとに
    // (1-best に無い単語の数, コスト) の順へ並べ直す。区切りを変えた候補のうち、
    // 1-best の語を流用したもの (今日|歯医者に) を、語を総入れ替えしたもの (共|歯医者に) より先に出す
    let same = |a: &PathSegment, b: &PathSegment| {
        a.start == b.start && a.reading == b.reading && a.surface == b.surface
    };
    let (best_segments, best_words) = (&found[0].1, &found[0].2);
    let mut class_of: Vec<usize> = vec![0; found.len()];
    // 分類ごとの (1-best に無い単語の数, コスト, found の index)
    let mut classes: Vec<Vec<(usize, i64, usize)>> = Vec::new();
    let mut class_index: HashMap<Vec<(usize, String)>, usize> = HashMap::new();
    for (i, (cost, segments, words)) in found.iter().enumerate().skip(1) {
        let key: Vec<(usize, String)> = segments
            .iter()
            .filter(|s| !best_segments.iter().any(|b| same(b, s)))
            .map(|s| (s.start, s.reading.clone()))
            .collect();
        let novel = words.iter().filter(|w| !best_words.iter().any(|b| same(b, w))).count();
        let next = classes.len();
        let c = *class_index.entry(key).or_insert(next);
        if c == classes.len() {
            classes.push(Vec::new());
        }
        classes[c].push((novel, *cost, i));
        class_of[i] = c;
    }
    for members in &mut classes {
        // 安定ソートなので、同じ (語の数, コスト) は取り出した順のまま
        members.sort_by_key(|&(novel, cost, _)| (novel, cost));
    }

    // 分類どうしの並びはコスト順のまま: 分類 C が i 回目に現れる位置へ、並べ直した C の i 件目を置く
    let mut order: Vec<usize> = vec![0];
    let mut used: Vec<usize> = vec![0; classes.len()];
    for &c in class_of.iter().skip(1) {
        let k = used[c];
        used[c] += 1;
        if k < MAX_NBEST_PER_CLASS {
            order.push(classes[c][k].2);
        }
    }
    order.truncate(MAX_NBEST);
    let mut slots: Vec<Option<(i64, Vec<PathSegment>, Vec<PathSegment>)>> =
        found.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// 経路の単語列を文節に分け、各文節と各単語 (数字をまとめた後の語) に開始位置を付ける
/// (convert_segments と同じ区切り)。戻り値は (文節列, 単語列)
fn path_segments(
    path: Vec<PathWord>,
    functional: &FunctionalIds,
) -> (Vec<PathSegment>, Vec<PathSegment>) {
    let mut segments = Vec::new();
    let mut words = Vec::new();
    let mut start = 0;
    for group in group_path(path, functional) {
        let segment_start = start;
        let mut reading = String::new();
        let mut surface = String::new();
        for word in group {
            reading.push_str(&word.reading);
            surface.push_str(&word.surface);
            let length = word.reading.chars().count();
            words.push(PathSegment { start, reading: word.reading, surface: word.surface });
            start += length;
        }
        segments.push(PathSegment { start: segment_start, reading, surface });
    }
    (segments, words)
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
    /// 文節境界ペナルティ (付属語は 0)。前向きの計算で決まり、N-best の後ろ向き探索でも使う
    penalty: i64,
    /// BOS からこのノードまでの最小コスト
    best_cost: i64,
    /// 最小コスト経路での直前ノード (nodes 内の index)
    best_prev: usize,
    /// 補正の語 (読みは打ったかなの区間、表記は補正した読みの辞書の語)。
    /// 補正の語を通っていない状態 (best_cost) では到達できない
    typo: bool,
    /// typo_costs[s - 1] は補正の語を s 個通った状態での、BOS からこのノードまでの最小コスト
    /// (s は 1 から補正の状態の数まで。補正の語を足さないラティスでは空)
    typo_costs: Vec<i64>,
    /// typo_costs の経路での直前ノード。このノードが補正の語なら、直前ノードは補正の語を
    /// 1 つ少なく通った状態 (s = 1 なら best_prev の側) にある
    typo_prevs: Vec<usize>,
}

impl Node {
    /// まだ前向きの計算をしていないノード
    fn new(start: usize, reading: String, left_id: u16, right_id: u16, word_cost: i32, surface: String) -> Node {
        Node {
            start,
            reading,
            left_id,
            right_id,
            word_cost,
            surface,
            penalty: 0,
            best_cost: i64::MAX,
            best_prev: 0,
            typo: false,
            typo_costs: Vec::new(),
            typo_prevs: Vec::new(),
        }
    }
}

/// ラティスに当てる学習 (docs/design/learning-score.md の C)
#[derive(Clone, Copy)]
struct Learned<'a> {
    store: &'a LearningStore,
    /// 学習した文節境界で文節ペナルティを増減する
    boundary: bool,
    /// 語・句の学習の割引を当てる
    discount: bool,
}

impl<'a> Learned<'a> {
    /// 文節の区切りも決める変換 (境界学習と割引)
    fn full(store: &'a LearningStore) -> Learned<'a> {
        Learned { store, boundary: true, discount: true }
    }

    /// 文節長を固定した変換 (割引だけ。指定された区切りを境界学習で動かさない)
    fn fixed(store: &'a LearningStore) -> Learned<'a> {
        Learned { store, boundary: false, discount: true }
    }
}

/// 前向きの Viterbi まで済ませたラティス
struct Lattice {
    /// nodes[0] は BOS
    nodes: Vec<Node>,
    /// ending_at[p] = 位置 p で終わるノードの index 一覧 (BOS は位置 0 で終わる扱い)
    ending_at: Vec<Vec<usize>>,
    /// 補正の状態の数 (通る補正の語の数の上限)。補正の語を足さなければ 0
    typo_states: usize,
}

/// ラティスに足す補正の語の元
struct TypoWords {
    /// 1 打の誤りを戻した読み
    corrections: Vec<typo::Correction>,
    /// 補正の状態の数 (通る補正の語の数の上限)
    states: usize,
    /// 補正の語の単語コストに足す量を誤りの種類によらず固定するとき (文節 UI) の値
    fixed_cost: Option<i32>,
}

/// 入力全体の候補の補正の元
#[derive(Clone, Copy)]
pub enum TypoSource<'a> {
    /// 補正しない
    Off,
    /// 打鍵が無いので、かなを標準のローマ字に戻した近似で補正する
    Kana,
    /// TSF 層が送った打鍵で補正する
    Keys(&'a typo::Keys),
}

impl TypoWords {
    /// 文節 UI (CONVSEG・CONVCTX) の補正。かなを戻した近似で、補正の語を 1 つだけ通し、
    /// 単語コストに TYPO_COST を足す (docs/design/typo-segment-ui.md)
    fn segment(kana: &str) -> TypoWords {
        TypoWords {
            corrections: typo::corrections(kana, &typo::TUNING),
            states: 1,
            fixed_cost: Some(TYPO_COST),
        }
    }

    /// 入力全体の候補の補正 (docs/design/unified-suggest.md の D)。補正の語のコストは誤りの種類の
    /// コストで、状態の数は打鍵の数から決めた許すずれの数
    fn sentence(kana: &str, source: TypoSource, tuning: &typo::Tuning) -> Option<TypoWords> {
        let (corrections, keys) = match source {
            TypoSource::Off => return None,
            TypoSource::Kana => (typo::corrections(kana, tuning), typo::Keys::standard(kana).len()),
            TypoSource::Keys(keys) => (typo::key_corrections(kana, keys, tuning), keys.len()),
        };
        let states = tuning.allowed_edits(keys);
        (states > 0).then_some(TypoWords { corrections, states, fixed_cost: None })
    }
}

impl Lattice {
    /// 文末 (位置 n) で終わるノード i から EOS への接続コスト
    fn eos_cost(&self, i: usize, matrix: &ConnectionMatrix) -> i64 {
        i64::from(matrix.get(self.nodes[i].right_id, 0))
    }

    /// 経路 (BOS を含まないノード index の列) を単語列にする
    fn path_words(&self, indices: &[usize]) -> Vec<PathWord> {
        indices
            .iter()
            .map(|&i| PathWord {
                reading: self.nodes[i].reading.clone(),
                surface: self.nodes[i].surface.clone(),
                left_id: self.nodes[i].left_id,
                right_id: self.nodes[i].right_id,
            })
            .collect()
    }
}

/// 学習済みの文節境界から、位置ごとの文節ペナルティ調整値を求める。
/// 戻り値[p] は「位置 p から始まる自立語」のペナルティへの加算値で、
/// 学習済み境界の開始位置では負 (切りやすい)、その内部では正 (切りにくい) になる。
/// 境界学習が無ければ空を返し、呼び出し側では調整なしとして扱われる
///
/// 開始位置のボーナスだけでは「きょうは|いいてんき」を学習しても
/// 「きょうはいい|てんき」のような跨いだ区切りを防げないため、内部にも罰則を置く
fn boundary_adjust(kana: &str, chars: &[char], learning: &LearningStore) -> Vec<i64> {
    let max = learning.max_boundary_chars();
    let n = chars.len();
    if max == 0 || n == 0 {
        return Vec::new();
    }
    // 文字位置 → バイト位置。部分文字列を String に組み直さずに引くため
    // (候補バーでは毎打鍵で呼ばれる)
    let mut offsets: Vec<usize> = Vec::with_capacity(n + 1);
    let mut byte = 0;
    for c in chars {
        offsets.push(byte);
        byte += c.len_utf8();
    }
    offsets.push(byte);

    let mut is_start = vec![false; n + 1];
    let mut inside = vec![false; n + 1];
    for start in 0..n {
        for len in MIN_BOUNDARY_CHARS..=max.min(n - start) {
            if learning.is_boundary(&kana[offsets[start]..offsets[start + len]]) {
                is_start[start] = true;
                is_start[start + len] = true;
                inside[start + 1..start + len].fill(true);
            }
        }
    }
    // 境界が重なったときは開始位置を優先する (内部の罰則で相殺しない)
    (0..=n)
        .map(|p| match (is_start[p], inside[p]) {
            (true, _) => -i64::from(BOUNDARY_BONUS),
            (false, true) => i64::from(BOUNDARY_BONUS),
            _ => 0,
        })
        .collect()
}

/// ラティスを構築して最小コスト経路の単語列を返す。
/// left_context_id は文頭の左文脈 (直前に確定した語の right_id)。0 = 前文脈なし (BOS)。
/// learned を渡すと学習を当てる (前文脈の復元では学習で経路を動かしたくないため None を渡す)
fn viterbi_path(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    left_context_id: u16,
    learned: Option<Learned>,
) -> Option<Vec<PathWord>> {
    let mut lattice =
        build_lattice(kana, dict, user, matrix, functional, left_context_id, learned, None)?;
    let indices = best_path(&lattice, matrix)?;
    Some(
        indices
            .into_iter()
            .map(|i| PathWord {
                reading: std::mem::take(&mut lattice.nodes[i].reading),
                surface: std::mem::take(&mut lattice.nodes[i].surface),
                left_id: lattice.nodes[i].left_id,
                right_id: lattice.nodes[i].right_id,
            })
            .collect(),
    )
}

/// ラティスを構築し、前向きの Viterbi で各ノードの最小コストを求める。
/// typo 以外の引数は viterbi_path と同じ。typo があれば補正の語のノードを足し、補正の語を
/// 1 つから補正の状態の数まで通った状態の最小コストも求める。入力が空なら None
fn build_lattice(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    left_context_id: u16,
    learned: Option<Learned>,
    typo: Option<&TypoWords>,
) -> Option<Lattice> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n == 0 {
        return None;
    }
    let adjust = learned
        .filter(|l| l.boundary)
        .map_or_else(Vec::new, |l| boundary_adjust(kana, &chars, l.store));
    let discount = learned.filter(|l| l.discount);
    // 自立語の語の学習の割引を引いた単語コスト
    let word_cost = |reading: &str, surface: &str, left_id: u16, cost: i16| -> i32 {
        let bonus = discount
            .filter(|_| !functional.is_functional(left_id))
            .map_or(0, |l| l.store.word_bonus(reading, surface));
        (i64::from(cost) - bonus) as i32
    };

    // nodes[0] は BOS (文頭)。right_id に前文脈の文脈IDを入れると、
    // 先頭語への連接コストが「直前確定語 → 先頭語」の値になる
    let mut bos = Node::new(0, String::new(), 0, left_context_id, 0, String::new());
    bos.best_cost = 0;
    let mut nodes: Vec<Node> = vec![bos];

    // ending_at[p] = 位置 p で終わるノードの index 一覧 (BOS は位置 0 で終わる扱い)
    let mut ending_at: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    ending_at[0].push(0);

    // inside_ascii[p] = 位置 p が英字の連続の途中 (前後の文字がともに英字)。
    // ここで始まる・終わるノードを作らず、英単語が途中で割れないようにする
    let is_ascii_alpha = |c: char| c.is_ascii_alphabetic();
    let inside_ascii: Vec<bool> = (0..=n)
        .map(|p| p > 0 && p < n && is_ascii_alpha(chars[p - 1]) && is_ascii_alpha(chars[p]))
        .collect();

    // 辞書の語に分けられなかった句の学習 (読み → 名詞の 1 語)。部分文字列をバイト位置で切り出す
    let unsplit_max = discount.map_or(0, |l| l.store.max_unsplit_chars());
    let offsets: Vec<usize> = if unsplit_max > 0 {
        let mut offsets: Vec<usize> = kana.char_indices().map(|(i, _)| i).collect();
        offsets.push(kana.len());
        offsets
    } else {
        Vec::new()
    };
    let noun_id = if unsplit_max > 0 {
        functional.find_id("名詞,一般").unwrap_or(DEFAULT_NOUN_ID)
    } else {
        DEFAULT_NOUN_ID
    };

    // 辞書語ノードと未知語ノードを生成する
    for start in 0..n {
        if inside_ascii[start] {
            continue;
        }
        // start から始まる登録語を1回のトライ走査でまとめて引く
        // (一致文字数の短い順に返るため、ノード生成順は旧来の end 昇順と同じ)
        for (len, entries) in dict.common_prefix_search(&chars[start..], MAX_READING_CHARS) {
            let end = start + len;
            if inside_ascii[end] {
                continue;
            }
            let reading: String = chars[start..end].iter().collect();
            for entry in entries {
                let cost = word_cost(&reading, &entry.surface, entry.left_id, entry.cost);
                nodes.push(Node::new(
                    start, reading.clone(), entry.left_id, entry.right_id, cost, entry.surface.clone(),
                ));
                ending_at[end].push(nodes.len() - 1);
            }
        }
        // ユーザ登録の名詞系単語とインポート辞書も通常の辞書語と同様にノードにする
        for (len, word) in user.common_prefix_words(&chars[start..]) {
            let end = start + len;
            if inside_ascii[end] {
                continue;
            }
            let cost = word_cost(&word.reading, &word.surface, word.left_id, word.cost);
            nodes.push(Node::new(
                start, word.reading.clone(), word.left_id, word.right_id, cost, word.surface.clone(),
            ));
            ending_at[end].push(nodes.len() - 1);
        }
        for (len, entries) in user.imported_common_prefix(&chars[start..], MAX_READING_CHARS) {
            let end = start + len;
            if inside_ascii[end] {
                continue;
            }
            let reading: String = chars[start..end].iter().collect();
            for entry in entries {
                let cost = word_cost(&reading, &entry.surface, entry.left_id, entry.cost);
                nodes.push(Node::new(
                    start, reading.clone(), entry.left_id, entry.right_id, cost, entry.surface.clone(),
                ));
                ending_at[end].push(nodes.len() - 1);
            }
        }
        if let Some(l) = discount.filter(|_| unsplit_max > 0) {
            for end in start + 1..=n.min(start + unsplit_max) {
                if inside_ascii[end] {
                    continue;
                }
                let reading = &kana[offsets[start]..offsets[end]];
                for (surface, bonus) in l.store.unsplit_phrases(reading) {
                    let cost = (i64::from(UNKNOWN_WORD_COST) - bonus) as i32;
                    nodes.push(Node::new(start, reading.to_string(), noun_id, noun_id, cost, surface.to_string()));
                    ending_at[end].push(nodes.len() - 1);
                }
            }
        }
        // 英字の連続の先頭では連続全体を1語にする (未知語ノードの代わり)
        let (end, word_cost) = if is_ascii_alpha(chars[start]) {
            let len = chars[start..].iter().take_while(|&&c| is_ascii_alpha(c)).count();
            (start + len, ASCII_WORD_COST)
        } else {
            // 未知語ノード (1文字をそのまま出力)。どんな入力でも経路が成立する保険
            (start + 1, UNKNOWN_WORD_COST)
        };
        let reading: String = chars[start..end].iter().collect();
        // 英字の連続が正しい表記を持てば、その表記のノードを打ったとおりのノードより安く足す
        // (打ったとおりの候補はその下に残る。docs/design/english-case.md)
        let form = if is_ascii_alpha(chars[start]) {
            user.ascii_form(&reading).filter(|f| *f != reading).map(str::to_string)
        } else {
            None
        };
        nodes.push(Node::new(
            start, reading.clone(), DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, word_cost, reading.clone(),
        ));
        ending_at[end].push(nodes.len() - 1);
        if let Some(form) = form {
            nodes.push(Node::new(
                start, reading, DEFAULT_NOUN_ID, DEFAULT_NOUN_ID, ASCII_WORD_COST - ASCII_FORM_BONUS, form,
            ));
            ending_at[end].push(nodes.len() - 1);
        }
    }
    // 補正の語は通常の語の後ろに足す (ending_at の並びの先頭側は通常の語のまま)
    let states = typo.map_or(0, |t| t.states);
    if let Some(typo) = typo {
        add_typo_nodes(typo, &chars, &inside_ascii, dict, user, &mut nodes, &mut ending_at);
        nodes[0].typo_costs = vec![i64::MAX; states];
        nodes[0].typo_prevs = vec![0; states];
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
            i64::from(SEGMENT_PENALTY) + adjust.get(nodes[i].start).copied().unwrap_or(0)
        };
        let mut best_cost = i64::MAX;
        let mut best_prev = 0;
        // 補正の語を s 個通った状態。補正の語へは s - 1 個通った状態から入り、
        // 通常の語へは同じ状態から進む (補正の状態の数より多く通る経路は作らない)
        let mut typo_costs = vec![i64::MAX; states];
        let mut typo_prevs = vec![0; states];
        for &p in &ending_at[nodes[i].start] {
            let step = i64::from(matrix.get(nodes[p].right_id, nodes[i].left_id))
                + i64::from(nodes[i].word_cost)
                + penalty;
            if nodes[i].typo {
                for s in 0..states {
                    let from = if s == 0 { nodes[p].best_cost } else { nodes[p].typo_costs[s - 1] };
                    if from != i64::MAX && from + step < typo_costs[s] {
                        typo_costs[s] = from + step;
                        typo_prevs[s] = p;
                    }
                }
                continue;
            }
            for s in 0..states {
                let from = nodes[p].typo_costs[s];
                if from != i64::MAX && from + step < typo_costs[s] {
                    typo_costs[s] = from + step;
                    typo_prevs[s] = p;
                }
            }
            if nodes[p].best_cost == i64::MAX {
                continue; // 到達不能な経路
            }
            let cost = nodes[p].best_cost + step;
            if cost < best_cost {
                best_cost = cost;
                best_prev = p;
            }
        }
        nodes[i].penalty = penalty;
        nodes[i].best_cost = best_cost;
        nodes[i].best_prev = best_prev;
        nodes[i].typo_costs = typo_costs;
        nodes[i].typo_prevs = typo_prevs;
    }
    Some(Lattice { nodes, ending_at, typo_states: states })
}

/// 打ったかな (文字の列は chars) の 1 打の誤りを戻した読み (typo.rs) ごとに、変わった所を
/// 覆う辞書の語 (システム辞書・ユーザ辞書・インポート辞書) を、打ったかなの対応する区間の補正の語にして足す。
/// 補正後の読みの区間 [s, e2) の語は、打ったかなの区間 [s, e2 − (補正後の変わった所の終わり −
/// 打ったかなの変わった所の終わり)) に置く。単語コストには戻した誤りのコストを足す
fn add_typo_nodes<'a>(
    typo: &TypoWords,
    chars: &[char],
    inside_ascii: &[bool],
    dict: &'a Dictionary,
    user: &'a UserDict,
    nodes: &mut Vec<Node>,
    ending_at: &mut [Vec<usize>],
) {
    // 打ったかなの区間 → (誤りのコストを足した単語コスト, 表記, left_id, right_id)。区間の並びを
    // 毎回同じにして、同じコストの経路の選び方を一定にする
    let mut words: BTreeMap<(usize, usize), Vec<(i32, &'a str, u16, u16)>> = BTreeMap::new();
    for correction in &typo.corrections {
        let edit_cost = typo.fixed_cost.unwrap_or(correction.cost);
        let fixed: Vec<char> = correction.reading.chars().collect();
        let first = correction.corrected_end.saturating_sub(MAX_READING_CHARS);
        for s in first..=correction.start {
            let mut hit = |len: usize, cost: i16, surface: &'a str, left: u16, right: u16| {
                let end2 = s + len;
                if end2 < correction.corrected_end {
                    return;
                }
                let end = end2 - correction.corrected_end + correction.typed_end;
                if end <= s || inside_ascii[s] || inside_ascii[end] {
                    return;
                }
                words.entry((s, end)).or_default().push((i32::from(cost) + edit_cost, surface, left, right));
            };
            for (len, entries) in dict.common_prefix_search(&fixed[s..], MAX_READING_CHARS) {
                for e in entries {
                    hit(len, e.cost, &e.surface, e.left_id, e.right_id);
                }
            }
            for (len, w) in user.common_prefix_words(&fixed[s..]) {
                hit(len, w.cost, &w.surface, w.left_id, w.right_id);
            }
            for (len, entries) in user.imported_common_prefix(&fixed[s..], MAX_READING_CHARS) {
                for e in entries {
                    hit(len, e.cost, &e.surface, e.left_id, e.right_id);
                }
            }
        }
    }
    for ((start, end), mut list) in words {
        // 同じ語を複数の補正が覆うときは 1 つにする (安定ソートなのでコストの安いものが残る)
        list.sort_by_key(|&(cost, _, _, _)| cost);
        let mut kept: Vec<(i32, &'a str, u16, u16)> = Vec::new();
        for word in list {
            if kept.len() >= MAX_TYPO_WORDS {
                break;
            }
            if !kept.iter().any(|k| (k.1, k.2, k.3) == (word.1, word.2, word.3)) {
                kept.push(word);
            }
        }
        let reading: String = chars[start..end].iter().collect();
        for (cost, surface, left_id, right_id) in kept {
            let mut node = Node::new(start, reading.clone(), left_id, right_id, cost, surface.to_string());
            node.typo = true;
            nodes.push(node);
            ending_at[end].push(nodes.len() - 1);
        }
    }
}

/// 前向きの結果から最小コスト経路のノード index 列 (BOS を含まない) を返す
fn best_path(lattice: &Lattice, matrix: &ConnectionMatrix) -> Option<Vec<usize>> {
    let nodes = &lattice.nodes;
    // 経路を逆順にたどって単語列を作る
    let mut indices: Vec<usize> = Vec::new();
    let mut cursor = best_end(lattice, matrix, 0)?.1;
    while cursor != 0 {
        indices.push(cursor);
        cursor = nodes[cursor].best_prev;
    }
    indices.reverse();
    Some(indices)
}

//// EOS: 位置 n で終わるノードから文末への接続コストを含めて最良を選び、(総コスト, ノード) を返す。
/// state は補正の語を通った数 (0 なら通常の経路)
fn best_end(lattice: &Lattice, matrix: &ConnectionMatrix, state: usize) -> Option<(i64, usize)> {
    let nodes = &lattice.nodes;
    let n = lattice.ending_at.len() - 1;
    let mut best: Option<(i64, usize)> = None;
    for &i in &lattice.ending_at[n] {
        let cost = if state == 0 { nodes[i].best_cost } else { nodes[i].typo_costs[state - 1] };
        if cost == i64::MAX {
            continue;
        }
        let cost = cost + lattice.eos_cost(i, matrix);
        if best.is_none_or(|(c, _)| cost < c) {
            best = Some((cost, i));
        }
    }
    best
}

/// 補正の語を 1 つ以上 (補正の状態の数まで) 通る最小コスト経路の (総コスト, ノード index 列 (BOS を含まない))
fn typo_path(lattice: &Lattice, matrix: &ConnectionMatrix) -> Option<(i64, Vec<usize>)> {
    let nodes = &lattice.nodes;
    let mut best: Option<(i64, usize, usize)> = None;
    for state in 1..=lattice.typo_states {
        if let Some((cost, end)) = best_end(lattice, matrix, state) {
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, end, state));
            }
        }
    }
    let (cost, end, mut state) = best?;
    let mut indices: Vec<usize> = Vec::new();
    let mut cursor = end;
    // 補正の語を通るたびに状態を 1 つ戻し、通っていない状態になったら通常の直前ノードをたどる
    while cursor != 0 {
        indices.push(cursor);
        if state == 0 {
            cursor = nodes[cursor].best_prev;
        } else {
            let prev = nodes[cursor].typo_prevs[state - 1];
            if nodes[cursor].typo {
                state -= 1;
            }
            cursor = prev;
        }
    }
    indices.reverse();
    Some((cost, indices))
}

/// 補正の最良経路の、通常の最良経路とのコストの差 (通常 − 補正。誤りのコストを含む)、
/// N-best と同じ規則で区切った文節 (読みは打ったかなの区間)、補正の最良経路の総コスト。
/// 補正の語を通る経路が無ければ None
fn typo_best(
    lattice: &Lattice,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<(i64, Vec<(String, String)>, i64)> {
    let (normal, _) = best_end(lattice, matrix, 0)?;
    let (cost, indices) = typo_path(lattice, matrix)?;
    let (segments, _) = path_segments(lattice.path_words(&indices), functional);
    Some((normal - cost, segments.into_iter().map(|s| (s.reading, s.surface)).collect(), cost))
}

/// 補正の最良経路の Δ (通常の最良経路とのコストの差を、打ったかな kana の文字数で割った値。
/// TYPO_COST を含む) とノード index 列 (BOS を含まない)。Δ が TYPO_SHOW 未満か、補正の語を通る
/// 経路が無ければ None
fn typo_route(
    lattice: &Lattice,
    matrix: &ConnectionMatrix,
    kana: &str,
) -> Option<(f64, Vec<usize>)> {
    let (normal, _) = best_end(lattice, matrix, 0)?;
    let (cost, indices) = typo_path(lattice, matrix)?;
    let delta = (normal - cost) as f64 / kana.chars().count() as f64;
    (delta >= TYPO_SHOW).then_some((delta, indices))
}

/// 計測用 (examples/typo_eval.rs): 補正の語を 1 つだけ通す文節 UI の補正で、補正の最良経路の、
/// 通常の最良経路とのコストの差 (通常 − 補正。TYPO_COST を含めない) と表記。補正の語を通る経路が
/// 無ければ None
#[allow(dead_code)]
pub fn typo_gain(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Option<(i64, String)> {
    let typo = TypoWords::segment(kana);
    let learned = Learned::full(learning);
    let lattice =
        build_lattice(kana, dict, user, matrix, functional, 0, Some(learned), Some(&typo))?;
    let (gain, segments, _) = typo_best(&lattice, matrix, functional)?;
    let surface = segments.iter().map(|(_, surface)| surface.as_str()).collect();
    Some((gain + i64::from(TYPO_COST), surface))
}

/// 1 語だけの経路を Viterbi と同じ式 (前文脈からの接続・単語コスト・文節境界ペナルティ・文末への接続)
/// で評価したコスト。予測の語の点数に使う (docs/design/unified-suggest.md の E)
pub fn word_path_cost(
    left_context_id: u16,
    left_id: u16,
    right_id: u16,
    cost: i16,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> i64 {
    let penalty = if functional.is_functional(left_id) { 0 } else { i64::from(SEGMENT_PENALTY) };
    i64::from(matrix.get(left_context_id, left_id))
        + i64::from(cost)
        + penalty
        + i64::from(matrix.get(right_id, 0))
}

/// 1 語だけの経路の語 (読み, 表記, left_id) に当てる学習の割引 (ラティスと同じく、自立語の語の学習の割引)
pub fn learning_discount(
    learning: &LearningStore,
    reading: &str,
    surface: &str,
    left_id: u16,
    functional: &FunctionalIds,
) -> i64 {
    if functional.is_functional(left_id) {
        return 0;
    }
    learning.word_bonus(reading, surface)
}

/// `split_path` の経路の 1 語。start・end は読みの文字位置 (chars の index)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitWord {
    pub start: usize,
    pub end: usize,
    pub reading: String,
    pub surface: String,
    pub functional: bool,
}

/// 確定した文節 (読み, 表記) を辞書 (システム辞書・ユーザ辞書・インポート辞書) の語に分け、経路の
/// 自立語の (読み, 表記) を順に返す (docs/design/learning-score.md の A の 1)。経路は `split_path`。
/// 経路が無ければ None
pub fn split_words(
    reading: &str,
    surface: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<Vec<(String, String)>> {
    let path = split_path(reading, surface, dict, user, matrix, functional)?;
    Some(path.into_iter().filter(|w| !w.functional).map(|w| (w.reading, w.surface)).collect())
}

/// 文節 (読み, 表記) を辞書の語に分けた経路のすべての語 (付属語を含む) を順に返す。読みのラティスのうち
/// 表記が文節の表記の対応する位置と一致する語だけを通る経路を、学習の割引なしの Viterbi と同じコストの
/// 最も安いもので選ぶ。未知語・英字の連続は辞書の語でないので通らない。経路が無ければ None
pub fn split_path(
    reading: &str,
    surface: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
) -> Option<Vec<SplitWord>> {
    struct Word<'a> {
        start: usize,
        end: usize,
        surface: &'a str,
        left_id: u16,
        right_id: u16,
        cost: i16,
    }
    let chars: Vec<char> = reading.chars().collect();
    let n = chars.len();
    if n == 0 || surface.is_empty() {
        return None;
    }
    // 開始位置の順に並ぶ (状態はそれより前に終わる語から作られる)
    let mut words: Vec<Word> = Vec::new();
    for start in 0..n {
        let mut hits: Vec<(usize, &str, u16, u16, i16)> = Vec::new();
        for (len, entries) in dict.common_prefix_search(&chars[start..], MAX_READING_CHARS) {
            hits.extend(entries.iter().map(|e| (len, e.surface.as_str(), e.left_id, e.right_id, e.cost)));
        }
        for (len, w) in user.common_prefix_words(&chars[start..]) {
            hits.push((len, w.surface.as_str(), w.left_id, w.right_id, w.cost));
        }
        for (len, entries) in user.imported_common_prefix(&chars[start..], MAX_READING_CHARS) {
            hits.extend(entries.iter().map(|e| (len, e.surface.as_str(), e.left_id, e.right_id, e.cost)));
        }
        for (len, s, left_id, right_id, cost) in hits {
            if surface.contains(s) {
                words.push(Word { start, end: start + len, surface: s, left_id, right_id, cost });
            }
        }
    }

    // 状態 = (語 (None は文頭), 表記のそこまでのバイト数, 文頭からの最小コスト, 直前の状態)
    let mut states: Vec<(Option<usize>, usize, i64, usize)> = vec![(None, 0, 0, 0)];
    let mut ending: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    ending[0].push(0);
    let mut index: HashMap<(usize, usize), usize> = HashMap::new();
    for (w, word) in words.iter().enumerate() {
        let penalty =
            if functional.is_functional(word.left_id) { 0 } else { i64::from(SEGMENT_PENALTY) };
        for k in 0..ending[word.start].len() {
            let s = ending[word.start][k];
            let (prev, offset, cost, _) = states[s];
            if !surface[offset..].starts_with(word.surface) {
                continue;
            }
            let right_id = prev.map_or(0, |p| words[p].right_id);
            let cost = cost
                + i64::from(matrix.get(right_id, word.left_id))
                + i64::from(word.cost)
                + penalty;
            let key = (w, offset + word.surface.len());
            match index.get(&key) {
                Some(&t) if cost < states[t].2 => {
                    states[t].2 = cost;
                    states[t].3 = s;
                }
                Some(_) => {}
                None => {
                    states.push((Some(w), key.1, cost, s));
                    index.insert(key, states.len() - 1);
                    ending[word.end].push(states.len() - 1);
                }
            }
        }
    }
    let last = ending[n]
        .iter()
        .copied()
        .filter(|&s| states[s].1 == surface.len())
        .min_by_key(|&s| {
            let right_id = states[s].0.map_or(0, |w| words[w].right_id);
            (states[s].2 + i64::from(matrix.get(right_id, 0)), s)
        })?;
    let mut result: Vec<SplitWord> = Vec::new();
    let mut cursor = last;
    while let Some(w) = states[cursor].0 {
        let word = &words[w];
        result.push(SplitWord {
            start: word.start,
            end: word.end,
            reading: chars[word.start..word.end].iter().collect(),
            surface: word.surface.to_string(),
            functional: functional.is_functional(word.left_id),
        });
        cursor = states[cursor].3;
    }
    result.reverse();
    Some(result)
}

// ひらがなをカタカナへ変換する (対象外の文字はそのまま)
pub(crate) fn to_katakana(kana: &str) -> String {
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

    /// LEARN・LEARN2 で文節 (読み, 表記) を確定したときと同じに数える (context は直前の文節の表記)
    fn learn(learning: &mut LearningStore, dict: &Dictionary, reading: &str, surface: &str, context: &str) {
        let words =
            split_words(reading, surface, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        learning.learn_segment(reading, surface, context, words.as_deref());
    }

    /// 2 文節以上の確定の読み全体と同じに数える (句の学習だけ)
    fn learn_whole(learning: &mut LearningStore, dict: &Dictionary, reading: &str, surface: &str) {
        let words =
            split_words(reading, surface, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        learning.learn_phrase(reading, surface, crate::learn::PhraseKind::Whole, words.is_some());
    }

    fn dict_from(data: &str) -> Dictionary {
        let mut dict = Dictionary::empty();
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 文節は表記の一致する辞書の語に分け自立語を返す() {
        let dict = sample_dict();
        let split = |reading: &str, surface: &str| {
            split_words(reading, surface, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional())
        };
        // 今日 (自立語) + は (付属語)。付属語は返さない
        assert_eq!(split("きょうは", "今日は"), Some(vec![("きょう".to_string(), "今日".to_string())]));
        // 表記が経路と違っても、表記の一致する語を通る
        assert_eq!(split("きょうは", "京は"), Some(vec![("きょう".to_string(), "京".to_string())]));
        assert_eq!(
            split("きょうははれです", "今日は晴れです"),
            Some(vec![("きょう".to_string(), "今日".to_string()), ("はれ".to_string(), "晴れ".to_string())])
        );
        // 辞書に無い表記・未知語を通る読み・ひらがなのままの表記は分けられない
        assert_eq!(split("きょうは", "教は"), None);
        assert_eq!(split("きょうはx", "今日はx"), None);
        assert_eq!(split("きょうは", "きょうは"), None);
        // 付属語だけの文節は自立語なしで分けられる
        assert_eq!(split("は", "は"), Some(vec![]));
    }

    #[test]
    fn 経路は付属語を含むすべての語を区間とともに返す() {
        let dict = sample_dict();
        let path = |reading: &str, surface: &str| {
            split_path(reading, surface, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional())
        };
        let word = |start: usize, end: usize, reading: &str, surface: &str, functional: bool| SplitWord {
            start,
            end,
            reading: reading.to_string(),
            surface: surface.to_string(),
            functional,
        };
        assert_eq!(
            path("きょうははれです", "今日は晴れです"),
            Some(vec![
                word(0, 3, "きょう", "今日", false),
                word(3, 4, "は", "は", true),
                word(4, 6, "はれ", "晴れ", false),
                word(6, 8, "です", "です", true),
            ])
        );
        assert_eq!(path("は", "は"), Some(vec![word(0, 1, "は", "は", true)]));
        assert_eq!(path("きょうは", "教は"), None);
    }

    #[test]
    fn 分ける経路はコストの最も安いもの() {
        // 今日 (2000) + は と、きょう は で 1 語の 今日は (5000) のどちらも表記が一致する
        let dict = dict_from(
            "きょう\t1\t1\t2000\t今日\nは\t2\t2\t500\tは\nきょうは\t1\t1\t5000\t今日は\n",
        );
        let split = |reading: &str, surface: &str| {
            split_words(reading, surface, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional())
        };
        assert_eq!(split("きょうは", "今日は"), Some(vec![("きょう".to_string(), "今日".to_string())]));
    }

    #[test]
    fn 文節の確定で語の学習は自立語だけを数え辞書に無い表記は句の学習だけ() {
        let dict = sample_dict();
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &dict, "きょうは", "今日は", "");
        assert!(learning.word_stat("きょう", "今日").is_some());
        assert!(learning.word_stat("は", "は").is_none());
        assert_eq!(learning.serial(), 1);
        assert!(learning.phrase_stat("きょうは", "今日は").is_some());
        learn(&mut learning, &dict, "きょうは", "教は", "晴れ");
        // 確定の通し番号は文節ごとに増える
        assert_eq!(learning.serial(), 2);
        assert!(learning.phrase_stat("きょうは", "教は").is_some());
        assert_eq!(learning.unsplit_phrases("きょうは"), vec![("教は", 1500)]);
        // 語 今日、句 今日は・教は、文脈の句 (晴れ, きょうは, 教は)
        assert_eq!(learning.entry_count(), 4);
    }

    /// 同音の語 (きる → 着る / 切る) と前文脈のテスト用の辞書
    fn kiru_dict() -> Dictionary {
        dict_from(
            "きる\t1\t1\t3000\t切る\n\
             きる\t1\t1\t3000\t着る\n\
             ふく\t1\t1\t3000\t服\n\
             かみ\t1\t1\t3000\t紙\n\
             を\t2\t2\t500\tを\n\
             と\t2\t2\t500\tと\n\
             きょう\t1\t1\t2000\t今日\n\
             きょう\t1\t1\t2800\t京\n\
             は\t2\t2\t500\tは\n\
             も\t2\t2\t500\tも\n",
        )
    }

    /// sample_dict の 京 を 3200 にした辞書。語の学習 1 回 (割引 1000) では 今日 (2000) より高く、
    /// 2 回 (上限 1500) で安くなる
    fn learn_dict() -> Dictionary {
        dict_from(
            "きょう\t1\t1\t2000\t今日\n\
             きょう\t1\t1\t3200\t京\n\
             は\t2\t2\t500\tは\n\
             はれ\t1\t1\t3000\t晴れ\n\
             です\t3\t3\t1000\tです\n",
        )
    }

    fn best_surface(kana: &str, ctx: Option<&Context>, dict: &Dictionary, learning: &LearningStore) -> String {
        convert_segments(
            kana, ctx, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(), learning, false,
        )
        .iter()
        .map(|s| s.candidates[0].clone())
        .collect()
    }

    #[test]
    fn 同音の語は学習の多いほうが経路に選ばれる() {
        let dict = kiru_dict();
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &dict, "きる", "着る", "");
        assert_eq!(best_surface("きる", None, &dict, &learning), "着る");
        for _ in 0..3 {
            learn(&mut learning, &dict, "きる", "切る", "");
        }
        assert_eq!(best_surface("きる", None, &dict, &learning), "切る");
        // 1 回だけ別の表記を選んでも、読み全体の学習表記 (句の学習の割引が最大の表記) でよく使う表記が先頭のまま。
        // 語の学習は 2 回で上限 (WORD_MAX) に届くので、経路だけでは 2 回と 3 回を区別しない
        learn(&mut learning, &dict, "きる", "着る", "");
        let got = convert_nbest(
            "きる", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(), &learning, false,
        );
        assert_eq!(got[0].surface(), "切る");
    }

    #[test]
    fn 語の学習は覚えた文節と違う文節にも効く() {
        let dict = kiru_dict();
        let mut learning = LearningStore::in_memory();
        assert_eq!(best_surface("きょうも", None, &dict, &learning), "今日も");
        learn(&mut learning, &dict, "きょうは", "京は", "");
        assert_eq!(best_surface("きょうも", None, &dict, &learning), "京も");
    }

    #[test]
    fn 前文脈は変換の経路に効かない() {
        let dict = kiru_dict();
        let mut learning = LearningStore::in_memory();
        // 着る は前文脈「服を」の後で 1 回、切る は文脈なしで 2 回確定した
        learn(&mut learning, &dict, "ふくを", "服を", "");
        learn(&mut learning, &dict, "きる", "着る", "服を");
        learn(&mut learning, &dict, "きる", "切る", "");
        learn(&mut learning, &dict, "きる", "切る", "");
        let ctx = Context { reading: "ふくを".to_string(), surface: "服を".to_string() };
        assert_eq!(best_surface("きる", Some(&ctx), &dict, &learning), "切る");
        assert_eq!(best_surface("ふくをきる", None, &dict, &learning), "服を切る");
        // 文脈の句の学習は記録される (予測と後続語の予測に使う)
        assert!(learning.ctx_phrase_stat("服を", "きる", "着る").is_some());
    }

    #[test]
    fn 辞書の語に分けられない句は名詞の1語としてラティスに載る() {
        let dict = sample_dict();
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &dict, "まいぐらふぃ", "マイグラフィ", "");
        // 未知語 6 文字 (12000 × 6) より、12000 − 割引の 1 語が安い
        let got = convert_segments(
            "まいぐらふぃは", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
            &learning, false,
        );
        assert_eq!(readings_of(&got), vec!["まいぐらふぃは"]);
        assert_eq!(got[0].candidates[0], "マイグラフィは");
        // 割引なしのラティスには載らない
        let plain = convert_segments(
            "まいぐらふぃは", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
            &LearningStore::in_memory(), false,
        );
        assert_ne!(plain[0].candidates[0], "マイグラフィは");
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
            None,
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &dict,
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        let symbol = c.iter().position(|s| s == "↑").unwrap();
        let katakana = c.iter().position(|s| s == "キョウ").unwrap();
        assert!(c.iter().position(|s| s == "今日").unwrap() < symbol);
        assert!(symbol < katakana);
    }

    /// sample_dict に単漢字 (京 は辞書候補と重複) と記号を足した辞書
    fn dict_with_single_kanji() -> Dictionary {
        let mut dict = sample_dict();
        dict.load_single_kanji_from("きょう\t今京凶\nは\t葉刃\n".as_bytes()).unwrap();
        dict.load_symbols_from("記号\t↑\tきょう\t上矢印 (テスト用の読み)\n".as_bytes()).unwrap();
        dict
    }

    #[test]
    fn 単漢字が文節候補の辞書候補の後_記号の前に重複なく入る() {
        let segments = convert_segments(
            "きょう",
            None,
            &dict_with_single_kanji(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].candidates, ["今日", "京", "今", "凶", "↑", "キョウ", "きょう"]);
    }

    #[test]
    fn 単漢字が全文候補と入力全体の候補にも入る() {
        let dict = dict_with_single_kanji();
        let got = candidates(
            "きょう", &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(got, ["今日", "京", "今", "凶", "↑", "キョウ", "きょう"]);

        let got = convert_nbest(
            "きょう", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
            &LearningStore::in_memory(), false);
        assert_eq!(surfaces_of(&got), ["今日", "京", "今", "凶", "↑", "キョウ", "きょう"]);
        assert!(got.iter().all(|c| !c.protected));
    }

    #[test]
    fn 単漢字は辞書候補の枠に数えない() {
        // 辞書候補だけで MAX_DICT_CANDIDATES を超える読みでも、辞書候補の並びは
        // 単漢字が無いときと同じで、単漢字はその後ろにすべて入る
        let mut data = String::new();
        for i in 0..MAX_DICT_CANDIDATES + 5 {
            data.push_str(&format!("かん\t1\t1\t{}\t缶{i}\n", 100 + i));
        }
        let build = |kanji: bool| {
            let mut dict = Dictionary::empty();
            dict.load_from(data.as_bytes()).unwrap();
            dict.finalize();
            if kanji {
                dict.load_single_kanji_from("かん\t甲乙丙丁戊己庚辛壬癸\n".as_bytes()).unwrap();
            }
            dict
        };
        let segment = |dict: &Dictionary| {
            convert_segments(
                "かん",
                None,
                dict,
                &no_user(),
                &ConnectionMatrix::empty(),
                &sample_functional(),
                &LearningStore::in_memory(),
                false,
            )
            .remove(0)
            .candidates
        };
        let without = segment(&build(false));
        let with = segment(&build(true));
        let dict_part = &without[..without.len() - 2]; // カタカナ・ひらがなを除く
        assert_eq!(dict_part.len(), MAX_DICT_CANDIDATES);
        assert_eq!(&with[..dict_part.len()], dict_part);
        assert_eq!(
            &with[dict_part.len()..dict_part.len() + 10],
            ["甲", "乙", "丙", "丁", "戊", "己", "庚", "辛", "壬", "癸"]
        );

        let nbest = |dict: &Dictionary| {
            surfaces_of(&convert_nbest(
                "かん", None, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
                &LearningStore::in_memory(), false))
        };
        let without = nbest(&build(false));
        let with = nbest(&build(true));
        let dict_part = &without[..without.len() - 2];
        assert_eq!(&with[..dict_part.len()], dict_part);
        assert_eq!(with.len(), without.len() + 10);
    }

    #[test]
    fn 単漢字はラティスに入らず第1候補を変えない() {
        // 「は」の単漢字 (葉・刃) があっても経路は助詞「は」のまま
        let dict = dict_with_single_kanji();
        let best = convert_sentence(
            "きょうははれです", &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(best.unwrap(), "今日は晴れです");
        assert_eq!(dict.lookup("は").len(), 1);
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
            None,
            &sample_dict(),
            &user_with_shortcut("きょう", "mail@example.com"),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert_eq!(segments.len(), 1);
        let c = &segments[0].candidates;
        assert_eq!(c[0], "今日");
        assert_eq!(c[1], "mail@example.com");
    }

    #[test]
    fn 学習表記は経路の表記の後_短縮よみより前に出る() {
        let mut learning = LearningStore::in_memory();
        // 1 回の学習では 京 (4000 − 1000) は 今日 (2000) より高いので、経路は 今日 のまま
        learn(&mut learning, &sample_dict(), "きょう", "京", "");
        let segments = convert_segments(
            "きょう",
            None,
            &sample_dict(),
            &user_with_shortcut("きょう", "mail@example.com"),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &learning,
            false,
        );
        let c = &segments[0].candidates;
        assert_eq!(c[0], "今日");
        assert_eq!(c[1], "京");
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
            None,
            &sample_dict(),
            &user,
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let c = &segments[0].candidates;
        let user_word = c.iter().position(|s| s == "匡").unwrap();
        assert!(c.iter().position(|s| s == "今日").unwrap() < user_word);
        assert!(user_word < c.iter().position(|s| s == "京").unwrap());
    }

    /// インポート辞書を TSV (読み\t表記\t品詞) から読み込んだユーザ辞書を作る
    fn user_with_imported(manual: &str, imported: &str) -> UserDict {
        let mut user = UserDict::empty();
        user.load_from(manual.as_bytes(), &FunctionalIds::empty());
        user.load_imported_from([imported.as_bytes()], &FunctionalIds::empty());
        user
    }

    #[test]
    fn インポート辞書の名詞が文中で変換される() {
        let user = user_with_imported("", "かんべ\t神戸\t姓\n");
        let result = convert_sentence(
            "かんべです", &sample_dict(), &user, &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(result.unwrap(), "神戸です");
    }

    #[test]
    fn インポート辞書の名詞が文節候補と全文候補に入る() {
        // インポート語のコスト5000は 京(4000) より後ろ
        let user = user_with_imported("", "きょう\t匡\t名\n");
        let segments = convert_segments(
            "きょう",
            None,
            &sample_dict(),
            &user,
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let c = &segments[0].candidates;
        let imported = c.iter().position(|s| s == "匡").unwrap();
        assert!(c.iter().position(|s| s == "京").unwrap() < imported);

        let got = candidates(
            "きょう", &sample_dict(), &user, &ConnectionMatrix::empty(), &sample_functional());
        assert!(got.contains(&"匡".to_string()), "{got:?}");
    }

    #[test]
    fn インポート辞書の短縮よみは手動登録の後に入る() {
        let user = user_with_imported(
            "きょう\tmail@example.com\t短縮よみ\n",
            "きょう\timported@example.com\t短縮よみ\nきょう\tmail@example.com\t短縮よみ\n",
        );
        let segments = convert_segments(
            "きょう",
            None,
            &sample_dict(),
            &user,
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let c = &segments[0].candidates;
        assert_eq!(c[1], "mail@example.com");
        assert_eq!(c[2], "imported@example.com");
        assert_eq!(c.iter().filter(|s| *s == "mail@example.com").count(), 1);
    }

    #[test]
    fn 手動登録と同じインポート語は候補に重複しない() {
        // 手動登録 (3000) が 今日(2000) と 京(4000) の間に入り、インポート側は出ない
        let user = user_with_imported("きょう\t匡\t名\n", "きょう\t匡\t人名\n");
        let segments = convert_segments(
            "きょう",
            None,
            &sample_dict(),
            &user,
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let c = &segments[0].candidates;
        assert_eq!(c.iter().filter(|s| *s == "匡").count(), 1);
        let user_word = c.iter().position(|s| s == "匡").unwrap();
        assert!(user_word < c.iter().position(|s| s == "京").unwrap());
    }

    #[test]
    fn 品詞表が無ければ単語ごとに文節になる() {
        let segments = convert_segments(
            "きょうは",
            None,
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &FunctionalIds::empty(),
            &LearningStore::in_memory(),
            false,
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

    /// 英字混じりの読みのテスト用の辞書。英字の連続の一部だけに一致する語 (ub・hub) を入れて、
    /// 英単語が途中で割れないことを確かめる
    fn mixed_dict() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "きょう\t1\t1\t2000\t今日\n\
                    は\t2\t2\t500\tは\n\
                    で\t2\t2\t500\tで\n\
                    さぎょう\t1\t1\t3000\t作業\n\
                    ub\t1\t1\t100\tUB\n\
                    hub\t1\t1\t100\tハブ\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 英字の連続は割れずに1語として変換される() {
        let segments = convert_segments(
            "きょうはgithubでさぎょう",
            None,
            &mixed_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょうは", "githubで", "さぎょう"]);
        let best: String = segments.iter().map(|s| s.candidates[0].as_str()).collect();
        assert_eq!(best, "今日はgithubで作業");

        let got = convert_nbest(
            "きょうはgithubでさぎょう", None, &mixed_dict(), &no_user(),
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory(), false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "今日はgithubで作業");
        // 英字の連続の一部だけに一致する語 (UB・ハブ) で割れた候補は出ない
        assert!(surfaces.iter().all(|s| !s.contains("UB") && !s.contains("ハブ")), "{surfaces:?}");
    }

    /// 英単語辞書の表記 (github → GitHub) を起動時と同じく写したユーザ辞書
    fn user_with_name_forms() -> UserDict {
        let mut user = UserDict::empty();
        user.set_name_forms(HashMap::from([("github".to_string(), "GitHub".to_string())]));
        user
    }

    #[test]
    fn 英字の連続は正しい表記の候補が上位に打ったとおりの候補が下位に出る() {
        let user = user_with_name_forms();
        let got = convert_nbest(
            "きょうはgithubでさぎょう", None, &mixed_dict(), &user,
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory(), false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "今日はGitHubで作業");
        assert!(surfaces.contains(&"今日はgithubで作業".to_string()), "{surfaces:?}");

        let segments = convert_segments(
            "きょうはgithubでさぎょう", None, &mixed_dict(), &user,
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory(), false);
        let best: String = segments.iter().map(|s| s.candidates[0].as_str()).collect();
        assert_eq!(best, "今日はGitHubで作業");

        // 英字だけの読みも補正した表記が先頭
        let got = convert_nbest(
            "github", None, &mixed_dict(), &user, &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory(), false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "GitHub");
        assert!(surfaces.contains(&"github".to_string()), "{surfaces:?}");

        // 打ったとおりが正しい表記なら、ノードを足さない (候補は1つの表記だけ)
        let got = convert_nbest(
            "GitHub", None, &mixed_dict(), &user, &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory(), false);
        assert_eq!(surfaces_of(&got)[0], "GitHub");
        assert!(!surfaces_of(&got).contains(&"github".to_string()));
    }

    #[test]
    fn 学習で打ったとおりの英字の候補が先頭に来る() {
        let user = user_with_name_forms();
        let dict = mixed_dict();
        let mut learning = LearningStore::in_memory();
        // TSF 層は確定した文節ごとの学習と、読み全体の学習を送る
        learn(&mut learning, &dict, "きょうは", "今日は", "");
        learn(&mut learning, &dict, "githubで", "githubで", "今日は");
        learn(&mut learning, &dict, "さぎょう", "作業", "githubで");
        learn_whole(&mut learning, &dict, "きょうはgithubでさぎょう", "今日はgithubで作業");
        let got = convert_nbest(
            "きょうはgithubでさぎょう", None, &dict, &user,
            &ConnectionMatrix::empty(), &sample_functional(), &learning, false);
        assert_eq!(surfaces_of(&got)[0], "今日はgithubで作業");
        // 英字の連続は辞書の語でないので文節は句の学習だけになり、文節の候補では経路の表記の次に出る
        let segments = convert_segments(
            "githubで", None, &dict, &user, &ConnectionMatrix::empty(), &sample_functional(), &learning,
            false,
        );
        assert_eq!(segments[0].candidates[..2], ["GitHubで", "githubで"]);
    }

    #[test]
    fn 英字だけの読みも変換できる() {
        let result = convert_sentence(
            "github", &mixed_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional());
        assert_eq!(result.unwrap(), "github");
        // 連続全体にちょうど一致する辞書語は残る
        let got = convert_nbest(
            "hub", None, &mixed_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory(), false);
        assert!(surfaces_of(&got).contains(&"ハブ".to_string()));
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
            None,
            &Dictionary::empty(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &FunctionalIds::empty(),
            &LearningStore::in_memory(), false
        )
        .is_empty());
    }

    #[test]
    fn 先頭語を入れ替えた文節候補が出る() {
        let segments = convert_segments(
            "きょうは",
            None,
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &dict_with_many_first_word_homophones(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &dict_with_many_first_word_homophones(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert!(segments[0].candidates.contains(&"下".to_string()));
    }

    #[test]
    fn 文節境界を固定して変換できる() {
        let dict = sample_dict();
        let learning = LearningStore::in_memory();

        // 4,4 なら通常の文節分割と同じ
        let segments = convert_segments_fixed(
            "きょうははれです", &[4, 4], None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["きょうは", "はれです"]);
        assert_eq!(segments[0].candidates[0], "今日は");

        // 3,5 なら「きょう / ははれです」で各範囲内を再変換する
        let segments = convert_segments_fixed(
            "きょうははれです", &[3, 5], None, &dict, &no_user(), &ConnectionMatrix::empty(),
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
            None,
            &sample_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
        );
        assert!(empty.is_empty());
    }

    #[test]
    fn 学習した表記は経路の表記の次に出て多く使うと経路になる() {
        let dict = learn_dict();
        let mut learning = LearningStore::in_memory();
        let convert = |learning: &LearningStore| {
            convert_segments(
                "きょうは", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
                learning, false,
            )
        };
        // 1 回: 京 (3200 − 1000) は 今日 (2000) より高いので、経路の 今日は の次に出る
        learn(&mut learning, &dict, "きょうは", "京は", "");
        let segments = convert(&learning);
        assert_eq!(segments[0].candidates[..2], ["今日は", "京は"]);
        // 2 回: 京 (3200 − 1500) が 今日 より安くなり、経路が変わる
        learn(&mut learning, &dict, "きょうは", "京は", "");
        let segments = convert(&learning);
        assert_eq!(segments[0].candidates[..2], ["京は", "今日は"]);
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
            "0\t4\t4\t1900\t0\n\
             1\t4\t4\t1900\t1\n\
             2\t4\t4\t1900\t2\n\
             9\t4\t4\t1900\t9\n\
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
            None,
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &dict,
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
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
            None,
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        let readings: Vec<&str> = segments.iter().map(|s| s.reading.as_str()).collect();
        assert_eq!(readings, vec!["１２じ"]);
        assert_eq!(segments[0].candidates[0], "１２時");
    }

    /// digit_dict での convert_segments の省略用。(読み一覧, 先頭候補一覧) を返す
    fn digit_segments(kana: &str) -> (Vec<String>, Vec<String>) {
        let segments = convert_segments(
            kana,
            None,
            &digit_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        (
            segments.iter().map(|s| s.reading.clone()).collect(),
            segments.iter().map(|s| s.candidates[0].clone()).collect(),
        )
    }

    #[test]
    fn 小数点を挟む数字が1文節にまとまる() {
        // かな入力では「.」が「。」として入るので、数字に挟まれた分だけ小数点で表記する
        let (readings, best) = digit_segments("0。12");
        assert_eq!(readings, vec!["0。12"]);
        assert_eq!(best, vec!["0.12"]);
    }

    #[test]
    fn 分数と座標とハイフンつなぎが1文節にまとまる() {
        // 「/」は「・」、「-」は「ー」として入る
        assert_eq!(digit_segments("1・2").1, vec!["1/2"]);
        assert_eq!(digit_segments("1、2").1, vec!["1,2"]);
        assert_eq!(digit_segments("2029ー09ー01").1, vec!["2029-09-01"]);
    }

    #[test]
    fn 座標の括弧は数字列とは別の文節になる() {
        let (readings, best) = digit_segments("（0、1）");
        assert_eq!(readings, vec!["（", "0、1", "）"]);
        assert_eq!(best, vec!["（", "0,1", "）"]);
    }

    #[test]
    fn 数字が続かない句読点は句点のまま残る() {
        let (readings, best) = digit_segments("12。");
        assert_eq!(readings, vec!["12", "。"]);
        assert_eq!(best, vec!["12", "。"]);
    }

    #[test]
    fn 全角数字では区切り記号も全角になる() {
        let (readings, best) = digit_segments("１。２");
        assert_eq!(readings, vec!["１。２"]);
        assert_eq!(best, vec!["１．２"]);
    }

    #[test]
    fn 区切りを挟む数字でも助数詞の入れ替え候補が出る() {
        let (readings, best) = digit_segments("1。5じ");
        assert_eq!(readings, vec!["1。5じ"]);
        assert_eq!(best, vec!["1.5時"]);
    }

    /// 0〜9 と「えん」(円・縁)・「と」を持つ辞書
    fn number_form_dict() -> Dictionary {
        let mut data = String::new();
        for d in 0..10 {
            data.push_str(&format!("{d}\t4\t4\t1900\t{d}\n"));
        }
        data.push_str("えん\t5\t5\t100\t円\nえん\t5\t5\t200\t縁\nと\t2\t2\t500\tと\n");
        let mut dict = Dictionary::empty();
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 数字文節に数字の書き方の候補が助数詞の入れ替えの後ろに出る() {
        let segments = convert_segments(
            "1234えん",
            None,
            &number_form_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(
            segments[0].candidates,
            vec![
                "1234円",
                "1234縁",
                "千二百三十四円",
                "一二三四円",
                "1,234円",
                "１２３４円",
                "1234エン",
                "1234えん",
            ]
        );
    }

    #[test]
    fn 区切りを含む数字には数字の書き方の候補が出ない() {
        let segments = convert_segments(
            "0。5",
            None,
            &number_form_dict(),
            &no_user(),
            &ConnectionMatrix::empty(),
            &sample_functional(),
            &LearningStore::in_memory(),
            false,
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].candidates, vec!["0.5", "0。5"]);
    }

    #[test]
    fn 文節境界を固定すれば数字も分かれる() {
        // Shift+← などでユーザが数字を手動分割した場合はそのまま尊重する
        let segments = convert_segments_fixed(
            "12",
            &[1, 1],
            None,
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
    fn 前文脈からビタビ一致で文脈idを復元する() {
        // 「きょうは」→ 今日+は が経路と一致するので、末尾語「は」の right_id (2) が返る
        let ctx = Context { reading: "きょうは".to_string(), surface: "今日は".to_string() };
        let id = resolve_context_id(
            &ctx, &sample_dict(), &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(id, 2);
    }

    #[test]
    fn 経路と違う表記でも後方最長一致で文脈idを復元する() {
        // 「京は」は経路 (今日は) と一致しないが、末尾の「は」が辞書と表記一致する
        let ctx = Context { reading: "きょうは".to_string(), surface: "京は".to_string() };
        let id = resolve_context_id(
            &ctx, &sample_dict(), &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(id, 2);
    }

    #[test]
    fn 復元できない前文脈は文脈id0になる() {
        let ctx = Context { reading: "xyz".to_string(), surface: "XYZ".to_string() };
        let id = resolve_context_id(
            &ctx, &sample_dict(), &no_user(), &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(id, 0);
    }

    /// 前文脈テスト用の辞書と連接行列。
    /// 読み「あ」に同コストの2候補 (亜=id1, 阿=id2)、前文脈用に「を」(id3)
    fn context_dict_and_matrix() -> (Dictionary, ConnectionMatrix) {
        let mut dict = Dictionary::empty();
        dict.load_from(
            "あ\t1\t1\t100\t亜\nあ\t2\t2\t100\t阿\nを\t3\t3\t100\tを\n".as_bytes(),
        )
        .unwrap();
        dict.finalize();
        // 4x4 行列: BOS(右0) からは 亜(左1) が高い → 阿が勝つ。
        // を(右3) からは 阿(左2) が高い → 亜が勝つ
        let matrix = ConnectionMatrix::parse(
            "4\n0\n1000\n0\n0\n\
             0\n0\n0\n0\n\
             0\n0\n0\n0\n\
             0\n0\n1000\n0\n",
        )
        .unwrap();
        (dict, matrix)
    }

    #[test]
    fn 前文脈の連接コストで先頭語が入れ替わる() {
        let (dict, matrix) = context_dict_and_matrix();
        let learning = LearningStore::in_memory();

        // 前文脈なし: BOS からの連接コストで「阿」
        let segments = convert_segments(
            "あ", None, &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning, false);
        assert_eq!(segments[0].candidates[0], "阿");

        // 前文脈「を」あり: を(右3) → 亜(左1) の連接が安く「亜」
        let ctx = Context { reading: "を".to_string(), surface: "を".to_string() };
        let segments = convert_segments(
            "あ", Some(&ctx), &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning, false);
        assert_eq!(segments[0].candidates[0], "亜");
    }

    #[test]
    fn 文節境界固定でも前文脈が効く() {
        let (dict, matrix) = context_dict_and_matrix();
        let ctx = Context { reading: "を".to_string(), surface: "を".to_string() };
        let segments = convert_segments_fixed(
            "あ", &[1], Some(&ctx), &dict, &no_user(), &matrix, &FunctionalIds::empty(),
            &LearningStore::in_memory());
        assert_eq!(segments[0].candidates[0], "亜");
    }

    /// 文節境界の学習テスト用の辞書。
    /// 「きょうはいい」は既定では自立語2つの「きょう|はいい」(今日配意) が安くなる
    fn boundary_dict() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "きょう\t1\t1\t2000\t今日\n\
                    は\t2\t2\t500\tは\n\
                    いい\t1\t1\t2000\t良い\n\
                    はいい\t1\t1\t1800\t配意\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    fn readings_of(segments: &[Segment]) -> Vec<&str> {
        segments.iter().map(|s| s.reading.as_str()).collect()
    }

    #[test]
    fn 学習した文節境界で区切りが変わる() {
        let dict = boundary_dict();
        let segments = convert_segments(
            "きょうはいい", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory(), false);
        assert_eq!(readings_of(&segments), vec!["きょう", "はいい"]);

        // 人が伸縮で直した「きょうは|いい」を学習すると、そちらへ寄る
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        learning.record_boundary("いい");
        let segments = convert_segments(
            "きょうはいい", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning, false);
        assert_eq!(readings_of(&segments), vec!["きょうは", "いい"]);
        assert_eq!(segments[0].candidates[0], "今日は");
        assert_eq!(segments[1].candidates[0], "良い");
    }

    #[test]
    fn 学習した境界は入力途中でも効く() {
        // 候補バーは毎打鍵で読み全体を変換し直すため、部分一致でも効く必要がある
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        learning.record_boundary("いい");
        let segments = convert_segments(
            "きょうはい", None, &boundary_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning, false);
        assert_eq!(readings_of(&segments)[0], "きょうは");
    }

    #[test]
    fn 文節長固定の変換は学習した境界に動かされない() {
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        learning.record_boundary("いい");
        let segments = convert_segments_fixed(
            "きょうはいい", &[3, 3], None, &boundary_dict(), &no_user(),
            &ConnectionMatrix::empty(), &sample_functional(), &learning);
        assert_eq!(readings_of(&segments), vec!["きょう", "はいい"]);
    }

    #[test]
    fn 境界の開始位置は下げ内部は上げる() {
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        let chars: Vec<char> = "きょうはいい".chars().collect();
        let b = i64::from(BOUNDARY_BONUS);
        assert_eq!(
            boundary_adjust("きょうはいい", &chars, &learning),
            vec![-b, b, b, b, -b, 0, 0]
        );
    }

    #[test]
    fn 境界学習が無ければ調整しない() {
        let chars: Vec<char> = "きょうは".chars().collect();
        assert!(boundary_adjust("きょうは", &chars, &LearningStore::in_memory()).is_empty());
    }

    /// 前文脈・境界学習なしで N-best を求め、(総コスト, 表記) の列を返す
    fn nbest_of(
        kana: &str,
        dict: &Dictionary,
        matrix: &ConnectionMatrix,
        functional: &FunctionalIds,
    ) -> Vec<(i64, String)> {
        let lattice = build_lattice(kana, dict, &no_user(), matrix, functional, 0, None, None).unwrap();
        nbest_paths(&lattice, matrix, functional)
            .into_iter()
            .map(|(cost, segments, _)| {
                (cost, segments.iter().map(|s| s.surface.as_str()).collect())
            })
            .collect()
    }

    fn surfaces_of(candidates: &[SentenceCandidate]) -> Vec<String> {
        candidates.iter().map(SentenceCandidate::surface).collect()
    }

    #[test]
    fn nbestはコスト順に並び先頭がviterbiの1bestと一致する() {
        let dict = sample_dict();
        let matrix = ConnectionMatrix::empty();
        let functional = sample_functional();
        let got = nbest_of("きょうははれです", &dict, &matrix, &functional);
        let best = convert_sentence("きょうははれです", &dict, &no_user(), &matrix, &functional);
        assert_eq!(Some(got[0].1.clone()), best);
        assert_eq!(got[1].1, "京は晴れです");
        assert!(got.windows(2).all(|w| w[0].0 <= w[1].0), "{got:?}");
    }

    #[test]
    fn 区切りだけが違う同じ表記の経路は1件になる() {
        // A+I と AI は表記の連結が同じ
        let mut dict = Dictionary::empty();
        dict.load_from("あ\t1\t1\t100\tA\nい\t1\t1\t100\tI\nあい\t1\t1\t1500\tAI\n".as_bytes())
            .unwrap();
        dict.finalize();
        let got =
            nbest_of("あい", &dict, &ConnectionMatrix::empty(), &sample_functional());
        let surfaces: Vec<&str> = got.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(surfaces, vec!["AI"]);
    }

    #[test]
    fn 一bestと違う文節が同じ候補は上限件数まで() {
        // 「きょうは|いく」の2文節目だけが違う候補は (4, いく) の同じ分類になる
        // (1-best の「行く」と、上限より2件多い言い換え)
        let mut data = String::from("きょう\t1\t1\t2000\t今日\nは\t2\t2\t500\tは\n");
        for i in 0..MAX_NBEST_PER_CLASS + 3 {
            data.push_str(&format!("いく\t1\t1\t{}\t行{i}\n", 3000 + i * 10));
        }
        let mut dict = Dictionary::empty();
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        let got =
            nbest_of("きょうはいく", &dict, &ConnectionMatrix::empty(), &sample_functional());
        let surfaces: Vec<String> = got.into_iter().map(|(_, s)| s).collect();
        let expected: Vec<String> =
            (0..=MAX_NBEST_PER_CLASS).map(|i| format!("今日は行{i}")).collect();
        assert_eq!(surfaces, expected);
    }

    #[test]
    fn 同じ分類の中は1bestの語を流用した候補が先に来る() {
        // 1-best は 今日は|医者 (7500)。コスト順は 共|歯医者 (7600)・今日は|意者 (7650)・
        // 今日|歯医者 (7700)・共は|医者 (8400)・共は|意者 (8550)。共|歯医者 と 今日|歯医者 は
        // 同じ分類 ((0, きょう), (3, はいしゃ)) で、1-best に無い語は前者が2つ (共・歯医者)、
        // 後者が1つ。他の分類の 今日は|意者・共は|医者 の位置は動かない
        let mut dict = Dictionary::empty();
        dict.load_from(
            "きょう\t1\t1\t2000\t今日\n\
             きょう\t6\t6\t1900\t共\n\
             は\t2\t2\t500\tは\n\
             いしゃ\t1\t1\t3000\t医者\n\
             いしゃ\t1\t1\t3150\t意者\n\
             はいしゃ\t1\t1\t3700\t歯医者\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let functional =
            FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n6 名詞,一般\n".as_bytes())
                .unwrap();
        // 共 (右6) → は (左2) の連接だけ高くして、共は|医者 を 1-best にしない
        let size = 7;
        let mut text = format!("{size}\n");
        for right in 0..size {
            for left in 0..size {
                text.push_str(if right == 6 && left == 2 { "1000\n" } else { "0\n" });
            }
        }
        let matrix = ConnectionMatrix::parse(&text).unwrap();
        let got = nbest_of("きょうはいしゃ", &dict, &matrix, &functional);
        let surfaces: Vec<&str> = got.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(
            surfaces,
            vec!["今日は医者", "今日歯医者", "今日は意者", "共歯医者", "共は医者", "共は意者"]
        );
    }

    #[test]
    fn 一bestからコストが離れた経路は出ない() {
        let mut dict = Dictionary::empty();
        dict.load_from(
            format!("あ\t1\t1\t100\t亜\nあ\t1\t1\t{}\t阿\n", 101 + NBEST_COST_MARGIN).as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let got = nbest_of("あ", &dict, &ConnectionMatrix::empty(), &FunctionalIds::empty());
        let surfaces: Vec<&str> = got.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(surfaces, vec!["亜"]);
    }

    #[test]
    fn nbestは上限件数を超えない() {
        // 4文節がそれぞれ2候補を持つ (16通り。どれも 1-best と違う文節の組が異なる)
        let mut dict = Dictionary::empty();
        dict.load_from(
            "あ\t1\t1\t100\t亜\nあ\t1\t1\t110\t阿\n\
             い\t1\t1\t100\t伊\nい\t1\t1\t110\t意\n\
             う\t1\t1\t100\t宇\nう\t1\t1\t110\t羽\n\
             え\t1\t1\t100\t江\nえ\t1\t1\t110\t絵\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let got =
            nbest_of("あいうえ", &dict, &ConnectionMatrix::empty(), &sample_functional());
        assert_eq!(got.len(), MAX_NBEST.min(16));
        assert_eq!(got[0].1, "亜伊宇江");
        assert!(got.windows(2).all(|w| w[0].0 <= w[1].0), "{got:?}");
    }

    #[test]
    fn 入力全体の候補は読み全体の学習_学習の割引の1best_nbestの順() {
        let dict = learn_dict();
        let mut learning = LearningStore::in_memory();
        learn_whole(&mut learning, &dict, "きょうははれです", "今日は晴れデス");
        // 2 回で 京 が 今日 より安くなる (learn_dict)
        learn(&mut learning, &dict, "きょうは", "京は", "");
        learn(&mut learning, &dict, "きょうは", "京は", "");
        let got = convert_nbest(
            "きょうははれです", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning, false);
        assert_eq!(
            got[0].segments,
            vec![("きょうははれです".to_string(), "今日は晴れデス".to_string())]
        );
        assert_eq!(
            got[1].segments,
            vec![
                ("きょうは".to_string(), "京は".to_string()),
                ("はれです".to_string(), "晴れです".to_string()),
            ]
        );
        assert_eq!(got[2].surface(), "今日は晴れです");
        // 末尾はカタカナ・ひらがな (読み全体の1文節)。表記の重複は無い
        let surfaces = surfaces_of(&got);
        assert_eq!(&surfaces[surfaces.len() - 2..], ["キョウハハレデス", "きょうははれです"]);
        let mut unique = surfaces.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), surfaces.len());
    }

    #[test]
    fn 学習で変わった1bestは補正の候補があるときとrerankのときだけ保護の印が付く() {
        let dict = learn_dict();
        let mut learning = LearningStore::in_memory();
        learn_whole(&mut learning, &dict, "きょうははれです", "今日は晴れデス");
        learn(&mut learning, &dict, "きょうは", "京は", "");
        learn(&mut learning, &dict, "きょうは", "京は", "");
        let got = convert_nbest(
            "きょうははれです", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning, false);
        // 読み全体の学習表記は保護する。学習で表記が変わった 1-best は、補正の候補があるときだけ保護する
        // (補正の候補がある場合は 学習で表記が変わった1bestは補正の候補で2番目に下がらない)
        assert!(got[0].protected);
        assert_eq!(got[1].surface(), "京は晴れです");
        assert!(!got[1].protected);
        // RERANK に渡す候補では、補正の候補が無くても学習で変わった 1-best を保護する
        let reranked = convert_nbest_rerank(
            "きょうははれです", None, &dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(),
            &learning, TypoSource::Off, &typo::TUNING,
        )
        .candidates;
        assert_eq!(reranked[1].surface(), "京は晴れです");
        assert!(reranked[1].protected);
        assert!(!reranked[2].protected);
        assert_eq!(got[2].surface(), "今日は晴れです");
        assert!(!got[2].protected);

        // 学習で表記が変わらなければ 1-best は保護しない
        let got = convert_nbest(
            "きょうははれです", None, &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory(), false);
        assert_eq!(got[0].surface(), "今日は晴れです");
        assert!(!got[0].protected);
    }

    #[test]
    fn ユーザ辞書の語と英字を含む候補に保護の印が付く() {
        let mut dict = Dictionary::empty();
        dict.load_from(
            "きょう\t1\t1\t2000\t今日\nきょう\t1\t1\t4000\t京\nは\t2\t2\t500\tは\n\
             はれ\t1\t1\t3000\t晴れ\nはれ\t1\t1\t3500\tHARE\n"
                .as_bytes(),
        )
        .unwrap();
        dict.finalize();
        let mut user = UserDict::empty();
        user.load_from("はれ\t腫れ\t名詞\n".as_bytes(), &sample_functional());
        let got = convert_nbest(
            "きょうははれ", None, &dict, &user, &ConnectionMatrix::empty(), &sample_functional(),
            &LearningStore::in_memory(), false);
        let find = |surface: &str| {
            got.iter().find(|c| c.surface() == surface).unwrap_or_else(|| {
                panic!("{surface} が候補に無い: {:?}", surfaces_of(&got))
            })
        };
        assert!(find("今日は腫れ").protected);
        assert!(find("今日はHARE").protected);
        assert!(!find("今日は晴れ").protected);
        assert!(!find("京は晴れ").protected);
    }

    #[test]
    fn 入力全体の候補に1bestの数を書き方ごとにそろえた候補が入る() {
        let got = convert_nbest(
            "1234えんと5600えん", None, &number_form_dict(), &no_user(),
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory(), false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "1234円と5600円");
        let expected = [
            "千二百三十四円と五千六百円",
            "一二三四円と五六〇〇円",
            "1,234円と5,600円",
            "１２３４円と５６００円",
        ];
        let start = surfaces.iter().position(|s| s == expected[0]).unwrap_or_else(|| {
            panic!("数字の書き方の候補が無い: {surfaces:?}")
        });
        assert_eq!(&surfaces[start..start + expected.len()], expected);
        // N-best の後ろ、カタカナ・ひらがなの前
        assert!(surfaces[..start].contains(&"1234縁と5600円".to_string()), "{surfaces:?}");
        assert_eq!(surfaces.len(), start + expected.len() + 2, "{surfaces:?}");
        for candidate in &got[start..start + expected.len()] {
            assert!(candidate.protected);
            // 文節は 1-best の文節のまま
            let readings: Vec<&str> = candidate.segments.iter().map(|(r, _)| r.as_str()).collect();
            assert_eq!(readings, vec!["1234えんと", "5600えん"]);
        }
    }

    #[test]
    fn 学習で数字の表記が変わっても数字の書き方の候補が入る() {
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &number_form_dict(), "1234えん", "1,234円", "");
        // 数字を含む表記の句はラティスに載せない (読み全体の学習表記で先頭に来る)
        assert!(learning.unsplit_phrases("1234えん").is_empty());
        let got = convert_nbest(
            "1234えん", None, &number_form_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning, false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "1,234円");
        for expected in ["千二百三十四円", "一二三四円", "１２３４円"] {
            assert!(surfaces.contains(&expected.to_string()), "{expected}: {surfaces:?}");
        }
    }

    #[test]
    fn 入力全体の候補に読み全体の辞書完全一致が入る() {
        let got = convert_nbest(
            "にほんご", None, &sample_dict(), &user_with_shortcut("にほんご", "NIHONGO"),
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory(), false);
        let surfaces = surfaces_of(&got);
        assert_eq!(surfaces[0], "日本語");
        assert!(surfaces.contains(&"NIHONGO".to_string()), "{surfaces:?}");
        assert_eq!(surfaces.last().unwrap(), "にほんご");
    }

    #[test]
    fn 入力全体の候補でも前文脈で先頭語が入れ替わる() {
        let (dict, matrix) = context_dict_and_matrix();
        let learning = LearningStore::in_memory();
        let got = convert_nbest(
            "あ", None, &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning, false);
        assert_eq!(got[0].surface(), "阿");
        let ctx = Context { reading: "を".to_string(), surface: "を".to_string() };
        let got = convert_nbest(
            "あ", Some(&ctx), &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning, false);
        assert_eq!(got[0].surface(), "亜");
    }

    #[test]
    fn 変換できない入力はカタカナとひらがなのみ() {
        // 空辞書では未知語経路が入力そのままを返すため、候補には加えない
        let got = candidates(
            "かな", &Dictionary::empty(), &no_user(), &ConnectionMatrix::empty(),
            &FunctionalIds::empty());
        assert_eq!(got, vec!["カナ", "かな"]);
    }

    /// 補正の候補のテスト用の辞書 (よろしく|お願い|します。ねぎ は打ち間違いの読みの一部に一致する語)
    fn typo_dict() -> Dictionary {
        typo_dict_with("")
    }

    /// typo_dict に TSV の語 extra を足した辞書
    fn typo_dict_with(extra: &str) -> Dictionary {
        dict_from(&format!(
            "よろしく\t1\t1\t3000\tよろしく\n\
             おねがい\t1\t1\t3000\tお願い\n\
             ねぎ\t1\t1\t4000\tネギ\n\
             します\t3\t3\t1000\tします\n\
             きょう\t1\t1\t2000\t今日\n\
             は\t2\t2\t500\tは\n\
             はれ\t1\t1\t3000\t晴れ\n\
             です\t3\t3\t1000\tです\n\
             {extra}"
        ))
    }

    /// 宜しく (3500) を足した辞書。2 回学習すると 3500 − 2377 で よろしく (3000) より安くなる
    const YOROSHIKU: &str = "よろしく\t1\t1\t3500\t宜しく\n";

    /// お・あ・葱 を足した辞書。読み全体 よろしくおねぎあします を辞書の語に分けられる表記
    /// (よろしく尾葱亜します) を学習できるようにする (分けられない表記はラティスに 1 語として載り、
    /// 読み全体の学習表記と 1-best が同じになるため)
    const WHOLE_WORDS: &str =
        "お\t1\t1\t5000\t尾\nあ\t1\t1\t5000\t亜\nねぎ\t1\t1\t6000\t葱\n";

    /// 入力全体の候補の補正の語 (かなを戻した近似、既定の調整値)
    fn sentence_typo(kana: &str) -> TypoWords {
        TypoWords::sentence(kana, TypoSource::Kana, &typo::TUNING).unwrap()
    }

    fn typo_nbest(kana: &str, learning: &LearningStore, typo: bool) -> Vec<SentenceCandidate> {
        typo_nbest_in(&typo_dict(), kana, learning, typo)
    }

    fn typo_nbest_in(dict: &Dictionary, kana: &str, learning: &LearningStore, typo: bool) -> Vec<SentenceCandidate> {
        convert_nbest(
            kana, None, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(), learning, typo)
    }

    #[test]
    fn 補正の語を足しても通常の1bestとnbestは変わらない() {
        let dict = typo_dict();
        let matrix = ConnectionMatrix::empty();
        let functional = sample_functional();
        for kana in ["よろしくおねぎあします", "よろしくおねがいします", "きょうははれです", "きょうははれでう"] {
            let without =
                build_lattice(kana, &dict, &no_user(), &matrix, &functional, 0, None, None).unwrap();
            let with =
                build_lattice(kana, &dict, &no_user(), &matrix, &functional, 0, None, Some(&sentence_typo(kana))).unwrap();
            assert!(with.nodes.iter().any(|n| n.typo), "{kana}: 補正の語が無い");
            let surfaces = |lattice: &Lattice| -> Vec<(i64, String)> {
                nbest_paths(lattice, &matrix, &functional)
                    .into_iter()
                    .map(|(cost, segments, _)| (cost, segments.iter().map(|s| s.surface.as_str()).collect()))
                    .collect()
            };
            let best = |lattice: &Lattice| -> Vec<PathWord> {
                lattice.path_words(&best_path(lattice, &matrix).unwrap())
            };
            let joined = |words: Vec<PathWord>| -> String {
                words.iter().map(|w| w.surface.as_str()).collect()
            };
            assert_eq!(joined(best(&without)), joined(best(&with)), "{kana}");
            assert_eq!(surfaces(&without), surfaces(&with), "{kana}");
        }
    }

    #[test]
    fn 補正の経路は補正の語をちょうど1つ通る() {
        let dict = typo_dict();
        let matrix = ConnectionMatrix::empty();
        let functional = sample_functional();
        let lattice = build_lattice(
            "よろしくおねぎあします", &dict, &no_user(), &matrix, &functional, 0, None,
            Some(&sentence_typo("よろしくおねぎあします")),
        )
        .unwrap();
        let (_, indices) = typo_path(&lattice, &matrix).unwrap();
        assert_eq!(indices.iter().filter(|&&i| lattice.nodes[i].typo).count(), 1);
        let words = lattice.path_words(&indices);
        let surface: String = words.iter().map(|w| w.surface.as_str()).collect();
        let reading: String = words.iter().map(|w| w.reading.as_str()).collect();
        assert_eq!(surface, "よろしくお願いします");
        // ノードの読みは打ったかなの区間
        assert_eq!(reading, "よろしくおねぎあします");
    }

    #[test]
    fn 打ち間違いがはっきりしていれば補正の候補が先頭に出る() {
        let got = typo_nbest("よろしくおねぎあします", &LearningStore::in_memory(), true);
        assert_eq!(got[0].surface(), "よろしくお願いします");
        assert!(got[0].typo && got[0].protected);
        // 文節の読みは打ったかなの区間で、連結すると打ったかなになる
        let readings: Vec<&str> = got[0].segments.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(readings.concat(), "よろしくおねぎあします");
        assert_eq!(readings[0], "よろしく");
        // 元の 1-best は 2 番目。補正の候補は 1 件だけ
        assert!(!got[1].typo);
        assert_eq!(got.iter().filter(|c| c.typo).count(), 1);
    }

    #[test]
    fn 読み全体の学習表記があれば補正の候補はその後ろの1bestの直後に出る() {
        let dict = typo_dict_with(WHOLE_WORDS);
        let mut learning = LearningStore::in_memory();
        learn_whole(&mut learning, &dict, "よろしくおねぎあします", "よろしく尾葱亜します");
        let got = typo_nbest_in(&dict, "よろしくおねぎあします", &learning, true);
        assert_eq!(got[0].surface(), "よろしく尾葱亜します");
        assert_eq!(got[1].surface(), "よろしく尾ネギ亜します");
        assert!(!got[1].typo);
        assert!(got[2].typo);
        assert_eq!(got[2].surface(), "よろしくお願いします");
    }

    #[test]
    fn 学習で表記が変わった1bestは補正の候補で2番目に下がらない() {
        let dict = typo_dict_with(YOROSHIKU);
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &dict, "よろしく", "宜しく", "");
        learn(&mut learning, &dict, "よろしく", "宜しく", "");
        let got = typo_nbest_in(&dict, "よろしくおねぎあします", &learning, true);
        assert!(got[0].surface().starts_with("宜しく"), "{:?}", surfaces_of(&got));
        assert!(!got[0].typo && got[0].protected);
        assert!(got[1].typo);
    }

    #[test]
    fn 打ち間違いの無い読みと補正の設定が無効なら補正の候補は出ない() {
        let learning = LearningStore::in_memory();
        assert!(typo_nbest("よろしくおねがいします", &learning, true).iter().all(|c| !c.typo));
        assert!(typo_nbest("きょうははれです", &learning, true).iter().all(|c| !c.typo));
        let got = typo_nbest("よろしくおねぎあします", &learning, false);
        assert!(got.iter().all(|c| !c.typo));
        assert!(!surfaces_of(&got).contains(&"よろしくお願いします".to_string()));
    }

    #[test]
    fn 三文字未満の読みには補正の候補が出ない() {
        let got = typo_nbest("はr", &LearningStore::in_memory(), true);
        assert!(got.iter().all(|c| !c.typo));
    }

    /// 文節 UI の変換の (読み, 候補) の並び
    fn segment_pairs(segments: &[Segment]) -> Vec<(String, Vec<String>)> {
        segments.iter().map(|s| (s.reading.clone(), s.candidates.clone())).collect()
    }

    fn pairs(list: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        list.iter()
            .map(|(reading, candidates)| {
                (reading.to_string(), candidates.iter().map(|c| c.to_string()).collect())
            })
            .collect()
    }

    fn typo_segments(kana: &str, learning: &LearningStore, typo: bool) -> Vec<Segment> {
        typo_segments_in(&typo_dict(), kana, learning, typo)
    }

    fn typo_segments_in(dict: &Dictionary, kana: &str, learning: &LearningStore, typo: bool) -> Vec<Segment> {
        convert_segments(
            kana, None, dict, &no_user(), &ConnectionMatrix::empty(), &sample_functional(), learning, typo)
    }

    #[test]
    fn 補正なしの文節の変換は今と同じ区切りと候補になる() {
        let learning = LearningStore::in_memory();
        let got = typo_segments("よろしくおねぎあします", &learning, false);
        assert_eq!(
            segment_pairs(&got),
            pairs(&[
                ("よろしく", &["よろしく", "ヨロシク"]),
                ("お", &["お", "オ"]),
                ("ねぎ", &["ネギ", "ねぎ"]),
                ("あします", &["あします", "アシマス"]),
            ])
        );
        assert!(got.iter().all(|s| s.typo.is_none()));
        // 打ち間違いの無い読みは、補正の有無によらず同じ
        for typo in [false, true] {
            let got = typo_segments("よろしくおねがいします", &learning, typo);
            assert_eq!(
                segment_pairs(&got),
                pairs(&[
                    ("よろしく", &["よろしく", "ヨロシク"]),
                    ("おねがいします", &["お願いします", "オネガイシマス", "おねがいします"]),
                ]),
                "typo {typo}"
            );
            assert!(got.iter().all(|s| s.typo.is_none()));
            let got = typo_segments("きょうははれです", &learning, typo);
            assert_eq!(
                segment_pairs(&got),
                pairs(&[
                    ("きょうは", &["今日は", "キョウハ", "きょうは"]),
                    ("はれです", &["晴れです", "ハレデス", "はれです"]),
                ]),
                "typo {typo}"
            );
            assert!(got.iter().all(|s| s.typo.is_none()));
        }
    }

    #[test]
    fn 文節の変換は補正の経路で区切り補正の文節の先頭に補正の表記が出る() {
        let learning = LearningStore::in_memory();
        let got = typo_segments("よろしくおねぎあします", &learning, true);
        assert_eq!(readings_of(&got), vec!["よろしく", "おねぎあします"]);
        assert_eq!(got[0].candidates, vec!["よろしく", "ヨロシク"]);
        assert!(got[0].typo.is_none());
        assert_eq!(got[1].candidates[0], "お願いします");
        assert_eq!(got[1].typo.as_deref(), Some("お願いします"));
        // 2 番目以降は、補正の文節の区間を文節長の固定で変換した通常の候補
        let fixed = convert_segments_fixed(
            "よろしくおねぎあします", &[4, 7], None, &typo_dict(), &no_user(),
            &ConnectionMatrix::empty(), &sample_functional(), &learning);
        assert_eq!(got[1].candidates[1..], fixed[1].candidates[..]);
        assert_eq!(got[1].candidates[1], "おネギあします");
    }

    #[test]
    fn 補正の文節の読みに学習があれば補正の表記は2番目に出る() {
        let mut learning = LearningStore::in_memory();
        // 辞書の語に分けられない表記なので、句の学習の 1 語としてラティスに載り、補正の文節の区間の経路になる
        learn(&mut learning, &typo_dict(), "おねぎあします", "オネギアシマス", "");
        let got = typo_segments("よろしくおねぎあします", &learning, true);
        assert_eq!(readings_of(&got), vec!["よろしく", "おねぎあします"]);
        assert_eq!(got[1].candidates[0], "オネギアシマス");
        assert_eq!(got[1].candidates[1], "お願いします");
        assert_eq!(got[1].typo.as_deref(), Some("お願いします"));
    }

    #[test]
    fn 通常の1bestが学習で変わっていれば補正の表記は2番目に出る() {
        let dict = typo_dict_with(YOROSHIKU);
        let mut learning = LearningStore::in_memory();
        learn(&mut learning, &dict, "よろしく", "宜しく", "");
        learn(&mut learning, &dict, "よろしく", "宜しく", "");
        let got = typo_segments_in(&dict, "よろしくおねぎあします", &learning, true);
        assert_eq!(readings_of(&got), vec!["よろしく", "おねぎあします"]);
        assert_eq!(got[0].candidates[0], "宜しく");
        assert_eq!(got[1].candidates[0], "おネギあします");
        assert_eq!(got[1].candidates[1], "お願いします");
    }

    #[test]
    fn 読み全体の学習表記があれば補正の表記は2番目に出る() {
        let dict = typo_dict_with(WHOLE_WORDS);
        let mut learning = LearningStore::in_memory();
        learn_whole(&mut learning, &dict, "よろしくおねぎあします", "よろしく尾葱亜します");
        let got = typo_segments_in(&dict, "よろしくおねぎあします", &learning, true);
        assert_eq!(readings_of(&got), vec!["よろしく", "おねぎあします"]);
        assert_eq!(got[1].candidates[1], "お願いします");
        assert_eq!(got[1].typo.as_deref(), Some("お願いします"));
    }

    #[test]
    fn 三文字未満の読みの文節の変換には補正が入らない() {
        let learning = LearningStore::in_memory();
        assert_eq!(
            segment_pairs(&typo_segments("はr", &learning, true)),
            segment_pairs(&typo_segments("はr", &learning, false))
        );
        assert!(typo_segments("はr", &learning, true).iter().all(|s| s.typo.is_none()));
    }
}
