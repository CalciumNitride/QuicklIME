// 変換候補の生成
//
// フェーズ4-3: Viterbi の最小コスト経路を品詞情報で文節にまとめ、
// 文節ごとの候補リストを返す。
// 入力全体を1単位とした N-best (convert_nbest) は docs/design/nbest.md を参照。
// 全文一括の候補 (candidates) も互換のため残している。

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::dict::Dictionary;
use crate::learn::{LearningStore, MIN_BOUNDARY_CHARS};
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
    right_id: u16,
}

/// 入力全体の候補 (CONVNBEST の1件)。文節ごとの (読み, 表記) を持つ
pub struct SentenceCandidate {
    pub segments: Vec<(String, String)>,
    /// LLM の並べ替えで位置を動かさない候補 (読み全体の学習表記、学習で表記が変わった 1-best、
    /// ユーザ辞書の語を含む候補、英字を含む候補。docs/design/llm-rerank.md)
    pub protected: bool,
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

/// かな文字列を文節列へ変換する。ctx は直前に確定した文節 (無ければ None)
pub fn convert_segments(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    let ctx_id = ctx.map_or(0, |c| resolve_context_id(c, dict, user, matrix, functional));
    let Some(path) = viterbi_path(kana, dict, user, matrix, functional, ctx_id, Some(learning))
    else {
        return Vec::new();
    };
    segments_from_path(path, ctx, dict, user, functional, learning)
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

/// 1-best 経路から候補リスト付きの文節列を作る (文節ごとの学習を当てる)
fn segments_from_path(
    path: Vec<PathWord>,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<Segment> {
    // 文脈学習は「直前文節の表記」をキーに引く。先頭文節は前文脈の表記、
    // 2文節目以降は直前文節の先頭候補 (既定のまま確定する流れと自己整合する)
    let mut segments: Vec<Segment> = Vec::new();
    let mut prev_surface: Option<String> = ctx.map(|c| c.surface.clone());
    for group in &group_path(path, functional) {
        let segment = segment_from_group(group, dict, user, learning, prev_surface.as_deref());
        prev_surface = Some(segment.candidates[0].clone());
        segments.push(segment);
    }
    segments
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
    let mut prev_surface: Option<String> = ctx.map(|c| c.surface.clone());
    let mut segments = Vec::new();
    let mut begin = 0;
    for &length in lengths {
        let reading: String = chars[begin..begin + length].iter().collect();
        begin += length;

        // 文節の範囲内だけで最小コスト経路を求める
        let group =
            viterbi_path(&reading, dict, user, matrix, functional, ctx_id, None).unwrap_or_else(|| {
                vec![PathWord {
                    reading: reading.clone(),
                    surface: reading.clone(),
                    left_id: DEFAULT_NOUN_ID,
                    right_id: DEFAULT_NOUN_ID,
                }]
            });
        let segment = segment_from_group(&group, dict, user, learning, prev_surface.as_deref());
        let next_ctx = Context {
            reading: segment.reading.clone(),
            surface: segment.candidates[0].clone(),
        };
        ctx_id = resolve_context_id(&next_ctx, dict, user, matrix, functional);
        prev_surface = Some(next_ctx.surface);
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
    learn_ctx: Option<&str>,
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
    // 前文脈に一致する文脈学習があればさらに先頭へ (最優先)。
    // 「服を|着る」「紙を|切る」のような同音異義語の使い分けがここで効く
    if let Some(ctx) = learn_ctx {
        if let Some(learned) = learning.get_ctx(ctx, &reading) {
            result.retain(|s| s != learned);
            result.insert(0, learned.to_string());
        }
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
    let path = viterbi_path(kana, dict, user, matrix, functional, 0, None)?;
    Some(path.into_iter().map(|w| w.surface).collect())
}

/// かな文字列に対する入力全体の候補 (CONVNBEST) を返す。ctx は直前に確定した文節。
/// 並び: 読み全体の学習表記 → 文節ごとの学習を当てた 1-best → N-best の残り (コスト順)
///       → 短縮よみ・読み全体の辞書完全一致 → 記号・カタカナ・ひらがな。
/// 表記が同じ候補は先勝ちで除く
pub fn convert_nbest(
    kana: &str,
    ctx: Option<&Context>,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    learning: &LearningStore,
) -> Vec<SentenceCandidate> {
    let ctx_id = ctx.map_or(0, |c| resolve_context_id(c, dict, user, matrix, functional));
    let Some(lattice) =
        build_lattice(kana, dict, user, matrix, functional, ctx_id, Some(learning))
    else {
        return Vec::new();
    };

    let mut result: Vec<SentenceCandidate> = Vec::new();
    let mut surfaces: Vec<String> = Vec::new();
    // protected は学習・ユーザ辞書語の印。英字を含むかは表記から決まるのでここで足す
    let mut add = |segments: Vec<(String, String)>, protected: bool| -> bool {
        let mut candidate = SentenceCandidate { segments, protected };
        let surface = candidate.surface();
        if surfaces.contains(&surface) {
            return false;
        }
        candidate.protected |= surface.chars().any(|c| c.is_ascii_alphabetic());
        surfaces.push(surface);
        result.push(candidate);
        true
    };
    let whole = |surface: &str| vec![(kana.to_string(), surface.to_string())];

    if let Some(learned) = learning.get(kana) {
        add(whole(learned), true);
    }
    if let Some(indices) = best_path(&lattice, matrix) {
        let words = lattice.path_words(&indices);
        let has_user = words.iter().any(|w| is_user_word(&w.reading, &w.surface, user));
        // 数字列の結合で区切り記号の表記が変わるため、学習の有無は結合後の表記どうしで比べる
        let raw: String = merge_digit_runs(lattice.path_words(&indices))
            .iter()
            .map(|w| w.surface.as_str())
            .collect();
        let segments = segments_from_path(words, ctx, dict, user, functional, learning);
        let segments: Vec<(String, String)> =
            segments.into_iter().map(|s| (s.reading, s.candidates[0].clone())).collect();
        let learned = segments.iter().map(|(_, surface)| surface.as_str()).collect::<String>() != raw;
        add(segments, learned || has_user);
    }
    for (_, path, words) in nbest_paths(&lattice, matrix, functional) {
        let has_user = words.iter().any(|w| is_user_word(&w.reading, &w.surface, user));
        add(path.into_iter().map(|s| (s.reading, s.surface)).collect(), has_user);
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
        if add(whole(surface), from_user) {
            dict_count += 1;
        }
    }
    for symbol in dict.lookup_symbols(kana) {
        add(whole(symbol), false);
    }
    add(whole(&to_katakana(kana)), false);
    add(whole(kana), false);
    result
}

/// (読み, 表記) がユーザ辞書 (手動登録・学習した複合語・インポート辞書) の名詞系の語か
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
}

/// 前向きの Viterbi まで済ませたラティス
struct Lattice {
    /// nodes[0] は BOS
    nodes: Vec<Node>,
    /// ending_at[p] = 位置 p で終わるノードの index 一覧 (BOS は位置 0 で終わる扱い)
    ending_at: Vec<Vec<usize>>,
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
    // (ライブ変換では毎打鍵で呼ばれる)
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
/// learning を渡すと学習済みの文節境界を文節ペナルティに反映する
/// (文節長を固定する変換や前文脈の復元では境界を動かしたくないため None を渡す)
fn viterbi_path(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    left_context_id: u16,
    learning: Option<&LearningStore>,
) -> Option<Vec<PathWord>> {
    let mut lattice =
        build_lattice(kana, dict, user, matrix, functional, left_context_id, learning)?;
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
/// 引数は viterbi_path と同じ。入力が空なら None
fn build_lattice(
    kana: &str,
    dict: &Dictionary,
    user: &UserDict,
    matrix: &ConnectionMatrix,
    functional: &FunctionalIds,
    left_context_id: u16,
    learning: Option<&LearningStore>,
) -> Option<Lattice> {
    let chars: Vec<char> = kana.chars().collect();
    let n = chars.len();
    if n == 0 {
        return None;
    }
    let adjust = learning.map_or_else(Vec::new, |l| boundary_adjust(kana, &chars, l));

    // nodes[0] は BOS (文頭)。right_id に前文脈の文脈IDを入れると、
    // 先頭語への連接コストが「直前確定語 → 先頭語」の値になる
    let mut nodes: Vec<Node> = vec![Node {
        start: 0,
        reading: String::new(),
        left_id: 0,
        right_id: left_context_id,
        word_cost: 0,
        surface: String::new(),
        penalty: 0,
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
                    penalty: 0,
                    best_cost: i64::MAX,
                    best_prev: 0,
                });
                ending_at[end].push(nodes.len() - 1);
            }
        }
        // ユーザ登録の名詞系単語とインポート辞書も通常の辞書語と同様にノードにする
        for (len, word) in user.common_prefix_words(&chars[start..]) {
            let end = start + len;
            nodes.push(Node {
                start,
                reading: word.reading.clone(),
                left_id: word.left_id,
                right_id: word.right_id,
                word_cost: i32::from(word.cost),
                surface: word.surface.clone(),
                penalty: 0,
                best_cost: i64::MAX,
                best_prev: 0,
            });
            ending_at[end].push(nodes.len() - 1);
        }
        for (len, entries) in user.imported_common_prefix(&chars[start..], MAX_READING_CHARS) {
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
                    penalty: 0,
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
            left_id: DEFAULT_NOUN_ID,
            right_id: DEFAULT_NOUN_ID,
            word_cost: UNKNOWN_WORD_COST,
            surface: ch,
            penalty: 0,
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
            i64::from(SEGMENT_PENALTY) + adjust.get(nodes[i].start).copied().unwrap_or(0)
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
        nodes[i].penalty = penalty;
        nodes[i].best_cost = best_cost;
        nodes[i].best_prev = best_prev;
    }
    Some(Lattice { nodes, ending_at })
}

/// 前向きの結果から最小コスト経路のノード index 列 (BOS を含まない) を返す
fn best_path(lattice: &Lattice, matrix: &ConnectionMatrix) -> Option<Vec<usize>> {
    let nodes = &lattice.nodes;
    let n = lattice.ending_at.len() - 1;

    // EOS: 位置 n で終わるノードから文末への接続コストを含めて最良を選ぶ
    let mut best_end: Option<usize> = None;
    let mut best_end_cost = i64::MAX;
    for &i in &lattice.ending_at[n] {
        if nodes[i].best_cost == i64::MAX {
            continue;
        }
        let cost = nodes[i].best_cost + lattice.eos_cost(i, matrix);
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
    Some(indices)
}

/// ひらがなをカタカナへ変換する (対象外の文字はそのまま)
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
            None,
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
    fn 学習済みの表記が文節候補の先頭に来る() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょうは", "京は");
        let segments = convert_segments(
            "きょうは",
            None,
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
            "あ", None, &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning);
        assert_eq!(segments[0].candidates[0], "阿");

        // 前文脈「を」あり: を(右3) → 亜(左1) の連接が安く「亜」
        let ctx = Context { reading: "を".to_string(), surface: "を".to_string() };
        let segments = convert_segments(
            "あ", Some(&ctx), &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning);
        assert_eq!(segments[0].candidates[0], "亜");
    }

    #[test]
    fn 文脈学習が文脈なし学習より優先される() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょう", "今日");
        learning.record_ctx("晴れ", "きょう", "京");
        let dict = sample_dict();

        // 前文脈なし: 文脈なし学習の「今日」が先頭
        let segments = convert_segments(
            "きょう", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        assert_eq!(segments[0].candidates[0], "今日");

        // 前文脈「晴れ」: 文脈学習の「京」が最優先
        let ctx = Context { reading: "はれ".to_string(), surface: "晴れ".to_string() };
        let segments = convert_segments(
            "きょう", Some(&ctx), &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        assert_eq!(segments[0].candidates[0], "京");
        assert_eq!(segments[0].candidates[1], "今日");

        // 一致しない前文脈では文脈なし学習に戻る
        let ctx = Context { reading: "です".to_string(), surface: "です".to_string() };
        let segments = convert_segments(
            "きょう", Some(&ctx), &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        assert_eq!(segments[0].candidates[0], "今日");
    }

    #[test]
    fn 二文節目の文脈学習は直前文節の先頭候補で引く() {
        // 「きょうは|はれです」の2文節目に、文脈「今日は」付きの学習を仕込む
        let mut learning = LearningStore::in_memory();
        learning.record_ctx("今日は", "はれです", "ハレです");
        let segments = convert_segments(
            "きょうははれです", None, &sample_dict(), &no_user(),
            &ConnectionMatrix::empty(), &sample_functional(), &learning);
        assert_eq!(segments[0].candidates[0], "今日は");
        assert_eq!(segments[1].candidates[0], "ハレです");
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
            &sample_functional(), &LearningStore::in_memory());
        assert_eq!(readings_of(&segments), vec!["きょう", "はいい"]);

        // 人が伸縮で直した「きょうは|いい」を学習すると、そちらへ寄る
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        learning.record_boundary("いい");
        let segments = convert_segments(
            "きょうはいい", None, &dict, &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        assert_eq!(readings_of(&segments), vec!["きょうは", "いい"]);
        assert_eq!(segments[0].candidates[0], "今日は");
        assert_eq!(segments[1].candidates[0], "良い");
    }

    #[test]
    fn 学習した境界は入力途中でも効く() {
        // ライブ変換は毎打鍵で読み全体を変換し直すため、部分一致でも効く必要がある
        let mut learning = LearningStore::in_memory();
        learning.record_boundary("きょうは");
        learning.record_boundary("いい");
        let segments = convert_segments(
            "きょうはい", None, &boundary_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
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
        let lattice = build_lattice(kana, dict, &no_user(), matrix, functional, 0, None).unwrap();
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
    fn 入力全体の候補は読み全体の学習_文節学習の1best_nbestの順() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょうははれです", "今日は晴れデス");
        learning.record("きょうは", "京は");
        let got = convert_nbest(
            "きょうははれです", None, &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
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
    fn 学習で決まった候補に保護の印が付く() {
        let mut learning = LearningStore::in_memory();
        learning.record("きょうははれです", "今日は晴れデス");
        learning.record("きょうは", "京は");
        let got = convert_nbest(
            "きょうははれです", None, &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &learning);
        // 読み全体の学習表記と、学習で表記が変わった 1-best は保護する
        assert!(got[0].protected && got[1].protected);
        assert_eq!(got[2].surface(), "今日は晴れです");
        assert!(!got[2].protected);

        // 学習で表記が変わらなければ 1-best は保護しない
        let got = convert_nbest(
            "きょうははれです", None, &sample_dict(), &no_user(), &ConnectionMatrix::empty(),
            &sample_functional(), &LearningStore::in_memory());
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
            &LearningStore::in_memory());
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
    fn 入力全体の候補に読み全体の辞書完全一致が入る() {
        let got = convert_nbest(
            "にほんご", None, &sample_dict(), &user_with_shortcut("にほんご", "NIHONGO"),
            &ConnectionMatrix::empty(), &sample_functional(), &LearningStore::in_memory());
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
            "あ", None, &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning);
        assert_eq!(got[0].surface(), "阿");
        let ctx = Context { reading: "を".to_string(), surface: "を".to_string() };
        let got = convert_nbest(
            "あ", Some(&ctx), &dict, &no_user(), &matrix, &FunctionalIds::empty(), &learning);
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
}
