// Mozc OSS 辞書の読み込みと検索
//
// フォーマット: 読み\t左文脈ID\t右文脈ID\tコスト\t表記 (TSV、UTF-8)
// 辞書ファイル (dictionary00.txt 〜 dictionary09.txt) はリポジトリに含めず、
// references/mozc/ (git 管理外) から読み込む。パスは main.rs を参照。
//
// 検索は fst (有限状態トランスデューサ) による。読みをキー、entries 内の
// 位置を値とする fst::Map で、完全一致 (lookup) と予測入力用の前方一致
// (predict_prefix) を同じ構造で捌く。キーの共通接頭辞・接尾辞が圧縮される
// ため、HashMap + ソート済み読み配列だった旧実装よりメモリが数十MB少ない。
// load_from でエントリを積み、finalize() で fst を構築すると検索可能になる。

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use fst::automaton::Str;
use fst::raw::Output;
use fst::{Automaton, IntoStreamer, Streamer};

/// 辞書の1エントリ (読みは Dictionary のキー側に持つ)
pub struct Entry {
    /// 左文脈ID (連接コスト計算で使用。Viterbi 導入まで未使用)
    #[allow(dead_code)]
    pub left_id: u16,
    /// 右文脈ID (同上)
    #[allow(dead_code)]
    pub right_id: u16,
    /// 単語コスト (小さいほど確からしい)
    pub cost: i16,
    /// 表記 (変換結果)
    pub surface: String,
}

/// 前方一致検索で走査する読みの上限。短い接頭辞 (「きょ」等) の巨大な
/// 一致範囲を予測のたびに全走査しないための安全弁
const MAX_PREFIX_SCAN: usize = 20_000;

/// タイプミス補正 (fuzzy_predict_prefix) で収集する読みの上限。
/// 編集候補ごとの部分木の合計は大きくなりうるため早めに打ち切る安全弁
const FUZZY_SCAN_LIMIT: usize = 5_000;

/// fst::Map の値に entries 内の位置を詰める ((開始index << 32) | 件数)
fn pack(start: usize, len: usize) -> u64 {
    debug_assert!(start < (1 << 32) && len < (1 << 32));
    ((start as u64) << 32) | (len as u64)
}

fn unpack(packed: u64) -> (usize, usize) {
    ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize)
}

/// UTF-8 の先頭バイトから文字のバイト長を返す
fn utf8_char_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// fst のノードから1文字分 (UTF-8 バイト列) たどる。途中で遷移が無ければ None
fn advance_char<'f>(
    f: &'f fst::raw::Fst<Vec<u8>>,
    mut node: fst::raw::Node<'f>,
    mut out: Output,
    ch: char,
) -> Option<(fst::raw::Node<'f>, Output)> {
    let mut buf = [0u8; 4];
    for &b in ch.encode_utf8(&mut buf).as_bytes() {
        let t = node.find_input(b)?;
        let tr = node.transition(t);
        out = out.cat(tr.out);
        node = f.node(tr.addr);
    }
    Some((node, out))
}

/// ノードの子を1文字 (UTF-8 1文字分の遷移列) 単位で列挙する。
/// コールバックは (文字, 文字を消費し終えたノード, 累積 Output) を受け取る
fn for_each_child_char<'f>(
    f: &'f fst::raw::Fst<Vec<u8>>,
    node: fst::raw::Node<'f>,
    out: Output,
    cb: &mut impl FnMut(char, fst::raw::Node<'f>, Output),
) {
    fn rec<'f>(
        f: &'f fst::raw::Fst<Vec<u8>>,
        node: fst::raw::Node<'f>,
        out: Output,
        buf: &mut [u8; 4],
        depth: usize,
        need: usize,
        cb: &mut impl FnMut(char, fst::raw::Node<'f>, Output),
    ) {
        for tr in node.transitions() {
            buf[depth] = tr.inp;
            let need = if depth == 0 { utf8_char_len(tr.inp) } else { need };
            let next = f.node(tr.addr);
            let out = out.cat(tr.out);
            if depth + 1 == need {
                // 辞書のキーは正しい UTF-8 なので、need バイト揃えば必ず1文字になる
                if let Some(ch) = std::str::from_utf8(&buf[..need]).ok().and_then(|s| s.chars().next()) {
                    cb(ch, next, out);
                }
            } else {
                rec(f, next, out, buf, depth + 1, need, cb);
            }
        }
    }
    rec(f, node, out, &mut [0u8; 4], 0, 0, cb)
}

/// ノード以下の部分木から (読み, パック値) を集める。key は現在のパス (バイト列)
fn collect_subtree(
    f: &fst::raw::Fst<Vec<u8>>,
    node: fst::raw::Node,
    out: Output,
    key: &mut Vec<u8>,
    found: &mut Vec<(String, u64)>,
) {
    if found.len() >= FUZZY_SCAN_LIMIT {
        return;
    }
    if node.is_final() {
        let reading = String::from_utf8(key.clone()).expect("キーは読み文字列 (UTF-8)");
        found.push((reading, out.cat(node.final_output()).value()));
    }
    for tr in node.transitions() {
        key.push(tr.inp);
        collect_subtree(f, f.node(tr.addr), out.cat(tr.out), key, found);
        key.pop();
    }
}

/// タイプミス補正の曖昧一致走査。chars[pos..] を「1箇所までの誤り」を許してたどり、
/// 入力を消費し終えた地点の部分木から読みを収集する。
/// 許す誤り (edited が false のとき1回だけ): 1かな置換 (隣接キーの打ち間違い相当)、
/// 隣接かなの入れ替え、1かなの脱落 (「ん」抜け等)、1かなの余分 (二度打ち等)
fn fuzzy_walk<'f>(
    f: &'f fst::raw::Fst<Vec<u8>>,
    node: fst::raw::Node<'f>,
    out: Output,
    chars: &[char],
    pos: usize,
    edited: bool,
    key: &mut Vec<u8>,
    found: &mut Vec<(String, u64)>,
) {
    if found.len() >= FUZZY_SCAN_LIMIT {
        return;
    }
    if pos == chars.len() {
        // 未編集でここに来た = ただの前方一致なので predict_prefix 側が拾う
        if edited {
            collect_subtree(f, node, out, key, found);
        }
        return;
    }
    let c = chars[pos];
    // そのまま一致して先へ
    if let Some((n2, o2)) = advance_char(f, node, out, c) {
        let klen = key.len();
        let mut buf = [0u8; 4];
        key.extend(c.encode_utf8(&mut buf).as_bytes());
        fuzzy_walk(f, n2, o2, chars, pos + 1, edited, key, found);
        key.truncate(klen);
    }
    if edited {
        return; // 誤りは1箇所まで
    }
    // 置換と脱落: 辞書側の子をすべて試す
    for_each_child_char(f, node, out, &mut |d, n2, o2| {
        let klen = key.len();
        let mut buf = [0u8; 4];
        key.extend(d.encode_utf8(&mut buf).as_bytes());
        if d != c {
            // 置換: 入力の c は d の打ち間違いとみなして入力も1文字進める
            fuzzy_walk(f, n2, o2, chars, pos + 1, true, key, found);
        }
        // 脱落: 入力から d が抜けているとみなして入力位置は据え置き
        fuzzy_walk(f, n2, o2, chars, pos, true, key, found);
        key.truncate(klen);
    });
    // 隣接かなの入れ替え: c と次の文字を逆順にたどる
    if pos + 1 < chars.len() && c != chars[pos + 1] {
        if let Some((n2, o2)) = advance_char(f, node, out, chars[pos + 1]) {
            if let Some((n3, o3)) = advance_char(f, n2, o2, c) {
                let klen = key.len();
                let mut buf = [0u8; 4];
                key.extend(chars[pos + 1].encode_utf8(&mut buf).as_bytes());
                key.extend(c.encode_utf8(&mut buf).as_bytes());
                fuzzy_walk(f, n3, o3, chars, pos + 2, true, key, found);
                key.truncate(klen);
            }
        }
    }
    // 余分: 入力の c が余計な1文字とみなして読み飛ばす (ノードは進めない)
    fuzzy_walk(f, node, out, chars, pos + 1, true, key, found);
}

pub struct Dictionary {
    /// 読み → entries 内の位置 (pack/unpack 参照)。finalize() で構築する
    fst: fst::Map<Vec<u8>>,
    /// 全エントリ本体。同じ読みのエントリは TSV 記載順で連続配置し、
    /// fst の値でスライスを切り出す
    entries: Vec<Entry>,
    /// finalize 前の一時バッファ (読み, エントリ)。finalize で消費されて空になる
    staging: Vec<(String, Entry)>,
    entry_count: usize,
    /// 記号辞書 (読み → 記号のリスト、symbol.tsv の記載順)。
    /// 連接情報を持たないため Viterbi には載せず、文節候補の末尾に追記する
    symbols: HashMap<String, Vec<String>>,
    symbol_count: usize,
}

impl Dictionary {
    /// 空の辞書 (辞書ファイルが見つからない場合のフォールバック)
    pub fn empty() -> Self {
        Dictionary {
            fst: fst::MapBuilder::memory().into_map(),
            entries: Vec::new(),
            staging: Vec::new(),
            entry_count: 0,
            symbols: HashMap::new(),
            symbol_count: 0,
        }
    }

    /// ディレクトリから dictionary00.txt 〜 dictionary09.txt を読み込む
    pub fn load(dir: &Path) -> io::Result<Self> {
        let mut dict = Dictionary::empty();
        let mut found = false;
        for i in 0..10 {
            let path = dir.join(format!("dictionary{i:02}.txt"));
            if !path.exists() {
                continue;
            }
            found = true;
            dict.load_from(BufReader::new(File::open(&path)?))?;
        }
        if !found {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("辞書ファイルが見つかりません: {}", dir.display()),
            ));
        }
        dict.finalize();
        Ok(dict)
    }

    /// 積まれたエントリから検索構造 (fst) を構築する。load_from を直接使う場合
    /// (テスト等) は読み込み後に明示的に呼ぶ。これを呼ぶまで lookup も
    /// predict_prefix も空を返す。staging を消費するため呼ぶのは一度きり
    pub fn finalize(&mut self) {
        // 安定ソート必須: 同じ読みのエントリは TSV 記載順 (挿入順) を保つ契約
        self.staging.sort_by(|a, b| a.0.cmp(&b.0));

        let mut builder = fst::MapBuilder::memory();
        let mut entries: Vec<Entry> = Vec::with_capacity(self.staging.len());
        let mut run_start = 0usize;
        let mut prev: Option<String> = None;
        for (reading, entry) in self.staging.drain(..) {
            if prev.as_deref() != Some(reading.as_str()) {
                if let Some(key) = prev.take() {
                    builder
                        .insert(key, pack(run_start, entries.len() - run_start))
                        .expect("読みはソート済みかつ一意");
                }
                run_start = entries.len();
                prev = Some(reading);
            }
            entries.push(entry);
        }
        if let Some(key) = prev {
            builder
                .insert(key, pack(run_start, entries.len() - run_start))
                .expect("読みはソート済みかつ一意");
        }
        self.entries = entries;
        self.fst = builder.into_map();
    }

    /// 1ファイル分のエントリを読み込む (テストからも使う)
    pub fn load_from(&mut self, reader: impl BufRead) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            let mut fields = line.split('\t');
            let (Some(reading), Some(left), Some(right), Some(cost), Some(surface)) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                continue; // 列が足りない行は無視
            };
            let (Ok(left_id), Ok(right_id), Ok(cost)) =
                (left.parse::<u16>(), right.parse::<u16>(), cost.parse::<i16>())
            else {
                continue; // 数値でない行は無視
            };
            self.staging.push((
                reading.to_string(),
                Entry {
                    left_id,
                    right_id,
                    cost,
                    surface: surface.to_string(),
                },
            ));
            self.entry_count += 1;
        }
        Ok(())
    }

    /// Mozc の symbol.tsv (記号辞書) を読み込む。
    /// フォーマット: 品詞\t記号\t読み(空白区切りで複数)\t説明... (先頭行はヘッダ)
    pub fn load_symbols(&mut self, path: &Path) -> io::Result<()> {
        self.load_symbols_from(BufReader::new(File::open(path)?))
    }

    /// 1ファイル分の記号エントリを読み込む (テストからも使う)
    pub fn load_symbols_from(&mut self, reader: impl BufRead) -> io::Result<()> {
        for (i, line) in reader.lines().enumerate() {
            let line = line?;
            if i == 0 && line.starts_with("POS\t") {
                continue; // ヘッダ行
            }
            let mut fields = line.split('\t');
            let (Some(_pos), Some(symbol), Some(readings)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue; // 列が足りない行は無視
            };
            if symbol.is_empty() {
                continue;
            }
            for reading in readings.split(' ').filter(|r| !r.is_empty()) {
                let list = self.symbols.entry(reading.to_string()).or_default();
                if !list.iter().any(|s| s == symbol) {
                    list.push(symbol.to_string());
                    self.symbol_count += 1;
                }
            }
        }
        Ok(())
    }

    /// 読みに完全一致するエントリ一覧を返す (finalize が未実行なら空)
    pub fn lookup(&self, reading: &str) -> &[Entry] {
        match self.fst.get(reading) {
            Some(packed) => {
                let (start, len) = unpack(packed);
                &self.entries[start..start + len]
            }
            None => &[],
        }
    }

    /// 読みの並び suffix の先頭から始まる辞書語を短い順に返す (共通接頭辞検索)。
    /// 戻り値は (一致した文字数, エントリ一覧)。max_chars 文字まで走査する。
    /// Viterbi のラティス構築用で、fst のノードを1文字ずつたどるため
    /// 部分文字列ごとに lookup するより速い (finalize が未実行なら空)
    pub fn common_prefix_search(&self, suffix: &[char], max_chars: usize) -> Vec<(usize, &[Entry])> {
        let fst = self.fst.as_fst();
        let mut node = fst.root();
        let mut out = fst::raw::Output::zero();
        let mut results = Vec::new();
        let mut buf = [0u8; 4];
        for (i, &ch) in suffix.iter().take(max_chars).enumerate() {
            for &b in ch.encode_utf8(&mut buf).as_bytes() {
                let Some(t) = node.find_input(b) else {
                    return results; // この先に一致する読みは無い
                };
                let tr = node.transition(t);
                out = out.cat(tr.out);
                node = fst.node(tr.addr);
            }
            // 読みは必ず文字境界で終わるので、判定は1文字分の遷移が済んでから
            if node.is_final() {
                let (start, len) = unpack(out.cat(node.final_output()).value());
                results.push((i + 1, &self.entries[start..start + len]));
            }
        }
        results
    }

    /// 読みが prefix で始まるエントリをコスト昇順で limit 件まで返す (予測入力用)。
    /// 戻り値は (読み, 表記)。finalize が未実行なら空を返す
    pub fn predict_prefix(&self, prefix: &str, limit: usize) -> Vec<(String, String)> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        // fst はキーを辞書順に返すため、コストの安定ソート後の同点タイブレークが
        // 「読みの辞書順 → 記載順」になる (旧実装のソート済み配列走査と同じ)
        let mut stream = self.fst.search(Str::new(prefix).starts_with()).into_stream();
        let mut readings: Vec<String> = Vec::new();
        let mut hits: Vec<(i16, usize, &str)> = Vec::new();
        while let Some((key, packed)) = stream.next() {
            if readings.len() >= MAX_PREFIX_SCAN {
                break; // 一致範囲が巨大でも全走査しない (安全弁)
            }
            readings.push(String::from_utf8(key.to_vec()).expect("キーは読み文字列 (UTF-8)"));
            let (start, len) = unpack(packed);
            for entry in &self.entries[start..start + len] {
                hits.push((entry.cost, readings.len() - 1, entry.surface.as_str()));
            }
        }
        hits.sort_by_key(|(cost, _, _)| *cost);
        hits.truncate(limit);
        hits.into_iter()
            .map(|(_, idx, surface)| (readings[idx].clone(), surface.to_string()))
            .collect()
    }

    /// タイプミス補正の曖昧前方一致 (予測入力の補充用)。
    /// 読みの先頭が「prefix に1箇所までの誤り (1かな置換・隣接かな入れ替え・
    /// 1かな脱落・1かな余分) を許した文字列」に一致するエントリを、
    /// コスト昇順で limit 件まで返す。戻り値は (読み, 表記)。
    /// 先頭のかなは打ち間違いが稀なうえ誤補正のノイズ源になるため編集対象にしない。
    /// prefix にそのまま前方一致する読み (= predict_prefix が返すもの) は含めない
    pub fn fuzzy_predict_prefix(&self, prefix: &str, limit: usize) -> Vec<(String, String)> {
        let chars: Vec<char> = prefix.chars().collect();
        // 先頭固定 + 編集1箇所には最低2文字必要
        if chars.len() < 2 || limit == 0 {
            return Vec::new();
        }
        let f = self.fst.as_fst();
        let Some((node, out)) = advance_char(f, f.root(), Output::zero(), chars[0]) else {
            return Vec::new();
        };
        let mut key: Vec<u8> = Vec::with_capacity(prefix.len() + 8);
        let mut buf = [0u8; 4];
        key.extend(chars[0].encode_utf8(&mut buf).as_bytes());
        let mut found: Vec<(String, u64)> = Vec::new();
        fuzzy_walk(f, node, out, &chars, 1, false, &mut key, &mut found);

        // 完全一致側と重複する読みを除き、複数の編集経路で重複した読みをまとめる
        found.retain(|(reading, _)| !reading.starts_with(prefix));
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found.dedup_by(|a, b| a.0 == b.0);

        let mut hits: Vec<(i16, usize, &str)> = Vec::new();
        for (idx, (_, packed)) in found.iter().enumerate() {
            let (start, len) = unpack(*packed);
            for entry in &self.entries[start..start + len] {
                hits.push((entry.cost, idx, entry.surface.as_str()));
            }
        }
        // 安定ソートでコスト同点は読みの辞書順 → 記載順 (predict_prefix と同じ規則)
        hits.sort_by_key(|(cost, _, _)| *cost);
        hits.truncate(limit);
        hits.into_iter()
            .map(|(_, idx, surface)| (found[idx].0.clone(), surface.to_string()))
            .collect()
    }

    /// 読みに対応する記号一覧を返す (symbol.tsv の記載順)。
    /// symbol.tsv の読みは半角形 ("(" など) で載っているため、完全一致で
    /// 見つからない場合は全角英数記号を半角に正規化して引き直す
    /// (「（」を変換すると ( ［ 〔 などが出る)
    pub fn lookup_symbols(&self, reading: &str) -> &[String] {
        if let Some(list) = self.symbols.get(reading) {
            return list;
        }
        let normalized = normalize_symbol_reading(reading);
        if normalized != reading {
            if let Some(list) = self.symbols.get(&normalized) {
                return list;
            }
        }
        &[]
    }

    pub fn entry_count(&self) -> usize {
        self.entry_count
    }

    pub fn symbol_count(&self) -> usize {
        self.symbol_count
    }
}

/// 記号辞書を引くための読みの正規化: 全角英数記号を半角にする。
/// TSF 層は記号キーを全角形 (「（」など) で未確定文字列に入れるが、
/// symbol.tsv の読みは半角形で載っているため
fn normalize_symbol_reading(reading: &str) -> String {
    reading
        .chars()
        .map(|c| match c {
            // 全角 ASCII (！ U+FF01 〜 ～ U+FF5E) → 半角
            '\u{FF01}'..='\u{FF5E}' => {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            }
            '　' => ' ',
            '￥' => '\\',
            '”' | '“' => '"',
            '’' | '‘' => '\'',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "にほんご\t1851\t1851\t3793\t日本語\n\
                    にほんご\t1851\t1851\t7869\tニホンゴ\n\
                    かな\t100\t100\t5000\t仮名\n\
                    壊れた行\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 完全一致で全エントリが引ける() {
        let dict = sample();
        let entries = dict.lookup("にほんご");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].surface, "日本語");
        assert_eq!(entries[0].cost, 3793);
    }

    #[test]
    fn 一致しなければ空スライス() {
        assert!(sample().lookup("そんざいしない").is_empty());
    }

    #[test]
    fn 壊れた行は無視してカウントしない() {
        assert_eq!(sample().entry_count(), 3);
    }

    fn sample_for_prediction() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "きょう\t100\t100\t3000\t今日\n\
                    きょうと\t100\t100\t2000\t京都\n\
                    きょうかい\t100\t100\t4000\t教会\n\
                    きのう\t100\t100\t1000\t昨日\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 前方一致でコスト昇順に引ける() {
        let dict = sample_for_prediction();
        assert_eq!(
            dict.predict_prefix("きょう", 8),
            vec![
                ("きょうと".to_string(), "京都".to_string()),
                ("きょう".to_string(), "今日".to_string()),
                ("きょうかい".to_string(), "教会".to_string()),
            ]
        );
    }

    #[test]
    fn 前方一致の上限と不一致() {
        let dict = sample_for_prediction();
        assert_eq!(dict.predict_prefix("きょう", 1).len(), 1);
        assert_eq!(dict.predict_prefix("きょう", 1)[0].1, "京都");
        assert!(dict.predict_prefix("あめ", 8).is_empty());
        assert!(dict.predict_prefix("", 8).is_empty());
    }

    #[test]
    fn finalize前は完全一致も前方一致も空() {
        let mut dict = Dictionary::empty();
        dict.load_from("きょう\t100\t100\t3000\t今日\n".as_bytes()).unwrap();
        assert!(dict.lookup("きょう").is_empty());
        assert!(dict.predict_prefix("きょう", 8).is_empty());
    }

    fn sample_for_fuzzy() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "にほんご\t100\t100\t3000\t日本語\n\
                    にほん\t100\t100\t2000\t日本\n\
                    にはん\t100\t100\t5000\t二半\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    #[test]
    fn 一かな置換を補正する() {
        // にひんご = にほんご の「ほ→ひ」打ち間違い (o→i の隣接キーミス相当)
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にひんご", 8),
            vec![("にほんご".to_string(), "日本語".to_string())]
        );
    }

    #[test]
    fn 隣接かなの入れ替えを補正する() {
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にんほご", 8),
            vec![("にほんご".to_string(), "日本語".to_string())]
        );
    }

    #[test]
    fn 一かなの脱落を補正する() {
        // にほご = にほんご の「ん」抜け。「にほん」も置換 (ご→ん) として一致する
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にほご", 8),
            vec![
                ("にほん".to_string(), "日本".to_string()),
                ("にほんご".to_string(), "日本語".to_string()),
            ]
        );
    }

    #[test]
    fn 一かな余分を補正する() {
        // にほんごご = 「ご」の二度打ち
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にほんごご", 8),
            vec![("にほんご".to_string(), "日本語".to_string())]
        );
    }

    #[test]
    fn 打ち間違い途中の入力でも補正して前方一致する() {
        // にひん = にほん(ご) を打ち間違えている途中。には(ん) も1かな置換で一致
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にひん", 8),
            vec![
                ("にほん".to_string(), "日本".to_string()),
                ("にほんご".to_string(), "日本語".to_string()),
                ("にはん".to_string(), "二半".to_string()),
            ]
        );
    }

    #[test]
    fn 先頭かなの誤りは補正しない() {
        assert!(sample_for_fuzzy().fuzzy_predict_prefix("けほんご", 8).is_empty());
    }

    #[test]
    fn 完全一致と同じ読みは補正候補に含めない() {
        // 「にほん」への前方一致 (にほん・にほんご) は predict_prefix の担当なので出さない
        assert_eq!(
            sample_for_fuzzy().fuzzy_predict_prefix("にほん", 8),
            vec![("にはん".to_string(), "二半".to_string())]
        );
    }

    fn sample_symbols() -> Dictionary {
        let mut dict = Dictionary::empty();
        let data = "POS\tCHAR\tReading (space separated)\tdescription\n\
                    記号\t→\tやじるし みぎ\t右矢印\n\
                    記号\t←\tやじるし ひだり\t左矢印\n\
                    記号\t→\tやじるし\t重複読みの行\n\
                    記号\t\tよみ\t記号が空の行\n\
                    壊れた行\n";
        dict.load_symbols_from(data.as_bytes()).unwrap();
        dict
    }

    #[test]
    fn 記号を読みで引ける() {
        let dict = sample_symbols();
        assert_eq!(dict.lookup_symbols("やじるし"), ["→", "←"]);
        assert_eq!(dict.lookup_symbols("みぎ"), ["→"]);
        assert!(dict.lookup_symbols("ない").is_empty());
    }

    #[test]
    fn 記号の重複と壊れた行はカウントしない() {
        // → の2重登録・記号が空の行・列不足の行は数えず、有効なのは4件
        assert_eq!(sample_symbols().symbol_count(), 4);
    }

    #[test]
    fn 全角の読みは半角に正規化して記号を引ける() {
        let mut dict = Dictionary::empty();
        let data = "記号\t（\t( [\t始め丸括弧\n\
                    記号\t［\t( [\t始め角括弧\n\
                    記号\t”\t\"\t終わりダブルクォート\n";
        dict.load_symbols_from(data.as_bytes()).unwrap();
        // 完全一致 (半角) はそのまま
        assert_eq!(dict.lookup_symbols("("), ["（", "［"]);
        // 全角形は半角に正規化して一致する
        assert_eq!(dict.lookup_symbols("（"), ["（", "［"]);
        assert_eq!(dict.lookup_symbols("［"), ["（", "［"]);
        assert_eq!(dict.lookup_symbols("”"), ["”"]);
        assert!(dict.lookup_symbols("ない").is_empty());
    }
}
