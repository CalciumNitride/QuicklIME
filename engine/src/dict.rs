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
use std::path::{Path, PathBuf};

use fst::automaton::Str;
use fst::raw::Output;
use fst::{Automaton, IntoStreamer, Streamer};

use crate::typo;

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

//// ノードの子を1文字 (UTF-8 1文字分の遷移列) 単位で列挙する。
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

/// 打鍵を使い切った位置 (読みの接頭辞) と、そこまでのずれ
pub struct FuzzyPrefix {
    pub prefix: String,
    /// ずれのコスト
    pub cost: i32,
    /// ずれの数
    pub edits: usize,
}

/// 打鍵のファジィ照合の fst の走査 (docs/design/unified-suggest.md の C)。かなの辺をたどりながら
/// 綴りと打鍵を DP で照合し、行が空になった (許すずれの数を超えた) 枝を切る
struct FuzzyWalk<'m, 'k> {
    matcher: &'m typo::Matcher<'k>,
    path: Vec<char>,
    rows: Vec<typo::Row>,
    /// 打鍵を使い切った位置
    found: Vec<FuzzyPrefix>,
    /// 返す位置の数
    limit: usize,
    /// 記録した位置のずれのコストの小さいほうから limit 個 (昇順)。limit 個そろえば、それより高い
    /// コストの状態はどの位置も返す位置に入らないので照合から捨てる
    best: Vec<i32>,
    visits: usize,
}

impl FuzzyWalk<'_, '_> {
    /// above は祖先で打鍵を使い切ったときの最小のずれのコスト。これ以上のコストで使い切る子孫の位置は、
    /// 祖先の前方一致に含まれるので記録しない
    fn walk<'f>(&mut self, f: &'f fst::raw::Fst<Vec<u8>>, node: fst::raw::Node<'f>, above: i32) {
        let mut children: Vec<(char, fst::raw::Node<'f>)> = Vec::new();
        for_each_child_char(f, node, Output::zero(), &mut |c, next, _| children.push((c, next)));
        for (c, next) in children {
            if self.visits >= MAX_FUZZY_VISITS {
                return;
            }
            self.path.push(c);
            if !self.matcher.reachable(&self.path, &self.rows) {
                self.path.pop();
                continue;
            }
            self.visits += 1;
            let (row, consumed) = self.matcher.step(&self.path, &self.rows);
            let mut limit = above;
            if let Some((cost, edits)) = consumed {
                if cost < above {
                    self.found.push(FuzzyPrefix { prefix: self.path.iter().collect(), cost, edits });
                    limit = cost;
                    let at = self.best.partition_point(|&b| b <= cost);
                    self.best.insert(at, cost);
                    self.best.truncate(self.limit);
                    if self.best.len() == self.limit {
                        self.matcher.set_bound(self.best[self.limit - 1]);
                    }
                }
            }
            self.rows.push(row);
            if self.matcher.alive(&self.rows) {
                self.walk(f, next, limit);
            }
            self.rows.pop();
            self.path.pop();
        }
    }
}

/// 打鍵のファジィ照合でたどる fst のノードの数の上限。長い打鍵や大きい辞書で打鍵を止めないための安全弁
const MAX_FUZZY_VISITS: usize = 200_000;

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
    /// 単漢字辞書 (読み → 漢字のリスト、single_kanji.tsv の記載順)。
    /// ラティスに入れると「あ」→ 亜 のような1字が経路に割り込んで区切りが崩れるため、
    /// 記号と同じく候補リストへの追記だけに使う
    single_kanji: HashMap<String, Vec<String>>,
    single_kanji_count: usize,
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
            single_kanji: HashMap::new(),
            single_kanji_count: 0,
        }
    }

    /// ディレクトリから dictionary00.txt 〜 dictionary09.txt を読み込み、続けて
    /// 追加の辞書 extra (同じ書式。UT 辞書など) を積む。存在しない extra は読み飛ばす
    pub fn load(dir: &Path, extra: &[PathBuf]) -> io::Result<Self> {
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
        for path in extra {
            if path.is_file() {
                dict.load_from(BufReader::new(File::open(path)?))?;
            }
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

    /// 文脈ID・コストを解決済みのエントリを1件積む (インポート辞書用)。
    /// load_from と同じく finalize() を呼ぶまで検索には現れない
    pub fn push_entry(&mut self, reading: &str, entry: Entry) {
        self.staging.push((reading.to_string(), entry));
        self.entry_count += 1;
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

    /// Mozc の single_kanji.tsv (単漢字辞書) を読み込む。
    /// フォーマット: 読み\t漢字を連結した文字列 (# で始まる行と空行は読み飛ばす)
    pub fn load_single_kanji(&mut self, path: &Path) -> io::Result<()> {
        self.load_single_kanji_from(BufReader::new(File::open(path)?))
    }

    /// 1ファイル分の単漢字エントリを読み込む (テストからも使う)
    pub fn load_single_kanji_from(&mut self, reader: impl BufRead) -> io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((reading, kanji)) = line.split_once('\t') else {
                continue; // 列が足りない行は無視
            };
            if reading.is_empty() {
                continue;
            }
            let list = self.single_kanji.entry(reading.to_string()).or_default();
            for c in kanji.chars() {
                let c = c.to_string();
                if !list.contains(&c) {
                    list.push(c);
                    self.single_kanji_count += 1;
                }
            }
        }
        Ok(())
    }

    /// 読みに対応する単漢字の一覧を返す (single_kanji.tsv の記載順)
    pub fn lookup_single_kanji(&self, reading: &str) -> &[String] {
        self.single_kanji.get(reading).map_or(&[], |list| list.as_slice())
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
    /// 戻り値は (読み, 表記)。finalize が未実行なら空を返す。
    /// 予測本体はインポート辞書と混ぜるためコスト付きの predict_prefix_scored を使うので、
    /// これは検索結果を表記で確かめるテスト用に残している
    #[cfg(test)]
    pub fn predict_prefix(&self, prefix: &str, limit: usize) -> Vec<(String, String)> {
        self.predict_prefix_scored(prefix, limit)
            .into_iter()
            .map(|(_, reading, surface)| (reading, surface))
            .collect()
    }

    /// predict_prefix と同じ検索で、コストも付けて (コスト, 読み, 表記) を返す。
    /// 別の辞書 (インポート辞書) の予測候補とコスト順に混ぜるために使う
    pub fn predict_prefix_scored(&self, prefix: &str, limit: usize) -> Vec<(i16, String, String)> {
        self.predict_prefix_entries(prefix, limit)
            .into_iter()
            .map(|(reading, entry)| (entry.cost, reading, entry.surface.clone()))
            .collect()
    }

    /// 読みが prefix で始まるエントリを、読みとともにコスト昇順で limit 件まで返す
    /// (予測の語の経路を評価するため文脈 ID も要る)
    pub fn predict_prefix_entries(&self, prefix: &str, limit: usize) -> Vec<(String, &Entry)> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        // fst はキーを辞書順に返すため、コストの安定ソート後の同点タイブレークが
        // 「読みの辞書順 → 記載順」になる (旧実装のソート済み配列走査と同じ)
        let mut stream = self.fst.search(Str::new(prefix).starts_with()).into_stream();
        let mut readings: Vec<String> = Vec::new();
        let mut hits: Vec<(i16, usize, &Entry)> = Vec::new();
        while let Some((key, packed)) = stream.next() {
            if readings.len() >= MAX_PREFIX_SCAN {
                break; // 一致範囲が巨大でも全走査しない (安全弁)
            }
            readings.push(String::from_utf8(key.to_vec()).expect("キーは読み文字列 (UTF-8)"));
            let (start, len) = unpack(packed);
            for entry in &self.entries[start..start + len] {
                hits.push((entry.cost, readings.len() - 1, entry));
            }
        }
        hits.sort_by_key(|(cost, _, _)| *cost);
        hits.truncate(limit);
        hits.into_iter().map(|(_, idx, entry)| (readings[idx].clone(), entry)).collect()
    }

    /// 打鍵のファジィ照合 (docs/design/unified-suggest.md の C) で、打鍵を使い切った読みの接頭辞を
    /// ずれのコストの安い順 (同じコストは接頭辞の辞書順) に limit 件まで返す。接頭辞は辞書の読みの
    /// 接頭辞で、その先の語は predict_prefix_entries で列挙する
    pub fn fuzzy_prefixes(&self, matcher: &typo::Matcher, limit: usize) -> Vec<FuzzyPrefix> {
        if limit == 0 {
            return Vec::new();
        }
        let f = self.fst.as_fst();
        let mut walk = FuzzyWalk {
            matcher,
            path: Vec::new(),
            rows: vec![matcher.start()],
            found: Vec::new(),
            limit,
            best: Vec::new(),
            visits: 0,
        };
        matcher.set_bound(i32::MAX);
        walk.walk(f, f.root(), i32::MAX);
        matcher.set_bound(i32::MAX);
        let mut found = walk.found;
        found.sort_by(|a, b| (a.cost, &a.prefix).cmp(&(b.cost, &b.prefix)));
        found.truncate(limit);
        found
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

    pub fn single_kanji_count(&self) -> usize {
        self.single_kanji_count
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

    #[test]
    fn 解決済みエントリを積んでfinalize後に引ける() {
        let mut dict = Dictionary::empty();
        dict.push_entry("かんべ", Entry { left_id: 1917, right_id: 1917, cost: 5000, surface: "神戸".to_string() });
        dict.push_entry("かんべ", Entry { left_id: 1851, right_id: 1851, cost: 5000, surface: "寛部".to_string() });
        assert!(dict.lookup("かんべ").is_empty());
        dict.finalize();
        let entries = dict.lookup("かんべ");
        // 同じ読みは積んだ順
        assert_eq!(entries.len(), 2);
        assert_eq!((entries[0].surface.as_str(), entries[0].left_id), ("神戸", 1917));
        assert_eq!(entries[1].surface, "寛部");
        assert_eq!(dict.entry_count(), 2);
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
                    にはん\t100\t100\t5000\t二半\n\
                    ありがとう\t100\t100\t3000\t有難う\n\
                    ありがたい\t100\t100\t3000\t有難い\n";
        dict.load_from(data.as_bytes()).unwrap();
        dict.finalize();
        dict
    }

    /// 打鍵 (かな 1 字ごとの要素を \x1f、未完成のローマ字を \x1e で区切る) で照合した接頭辞の
    /// (接頭辞, ずれの数) の並び
    fn fuzzy(kana: &str, field: &str, edits: usize) -> Vec<(String, usize)> {
        let keys = typo::Keys::parse(kana, field).unwrap();
        let matcher = typo::Matcher::new(&keys, edits, &typo::TUNING);
        sample_for_fuzzy()
            .fuzzy_prefixes(&matcher, 8)
            .into_iter()
            .map(|p| (p.prefix, p.edits))
            .collect()
    }

    #[test]
    fn 未完成のローマ字まで綴りの途中で一致する() {
        // arigat: 「と」「た」の綴りの途中まで打った
        let got = fuzzy("ありが", "a\x1fri\x1fga\x1et", 1);
        assert!(got.contains(&("ありがと".to_string(), 0)), "{got:?}");
        assert!(got.contains(&("ありがた".to_string(), 0)), "{got:?}");
    }

    #[test]
    fn 隣のキーの打ち間違いをずれ1つで照合する() {
        // nihinngo = nihonngo の o を隣の i と打った
        let got = fuzzy("にひんご", "ni\x1fhi\x1fnn\x1fgo", 1);
        assert_eq!(got.first(), Some(&("にほんご".to_string(), 1)), "{got:?}");
        // 許すずれが 0 なら照合しない
        assert!(fuzzy("にひんご", "ni\x1fhi\x1fnn\x1fgo", 0).is_empty());
    }

    #[test]
    fn 打ち終えた読みはその位置で使い切り子孫の位置を数えない() {
        // にほん で使い切ると、にほんご は にほん の前方一致に含まれる
        let got = fuzzy("にほん", "ni\x1fho\x1fnn", 1);
        assert_eq!(got.first(), Some(&("にほん".to_string(), 0)), "{got:?}");
        assert!(!got.iter().any(|(p, _)| p == "にほんご"), "{got:?}");
    }

    #[test]
    fn 先頭のかなの打鍵の誤りは照合しない() {
        // ki は ni の隣のキーではないが、余分・抜けでも先頭のかなの打鍵は直さない
        assert!(fuzzy("けほんご", "ke\x1fho\x1fnn\x1fgo", 2).is_empty());
        // mi (n の隣の m) と打っても、先頭のかなの誤りなので にほんご にならない
        assert!(fuzzy("みほんご", "mi\x1fho\x1fnn\x1fgo", 2).is_empty());
    }

    #[test]
    fn 前方一致の語をコスト順に読みとエントリで返す() {
        let dict = sample_for_fuzzy();
        let got: Vec<(String, &str)> = dict
            .predict_prefix_entries("にほ", 8)
            .into_iter()
            .map(|(r, e)| (r, e.surface.as_str()))
            .collect();
        assert_eq!(got, vec![("にほん".to_string(), "日本"), ("にほんご".to_string(), "日本語")]);
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

    #[test]
    fn 単漢字を読みで記載順に引ける() {
        let mut dict = Dictionary::empty();
        let data = "# コメント行\n\
                    \n\
                    あられ\t霰\n\
                    ひょう\t氷表雹\n\
                    壊れた行\n\
                    ひょう\t豹雹\n";
        dict.load_single_kanji_from(data.as_bytes()).unwrap();
        assert_eq!(dict.lookup_single_kanji("あられ"), ["霰"]);
        // 同じ読みの2行目は後ろに足し、既出の字は重ねない
        assert_eq!(dict.lookup_single_kanji("ひょう"), ["氷", "表", "雹", "豹"]);
        assert!(dict.lookup_single_kanji("ない").is_empty());
        assert!(dict.lookup_single_kanji("# コメント行").is_empty());
        assert_eq!(dict.single_kanji_count(), 5);
    }

    #[test]
    fn 追加の辞書を積んで読み込み存在しないものは読み飛ばす() {
        let dir = std::env::temp_dir().join(format!("quicklime-dict-extra-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dictionary00.txt"), "かんべ\t1851\t1851\t5000\t神戸\n").unwrap();
        let extra = dir.join("extra.txt");
        std::fs::write(&extra, "かんべ\t1921\t1921\t8000\t寛部\nあられ\t1851\t1851\t8999\t霰\n")
            .unwrap();
        let dict = Dictionary::load(&dir, &[extra.clone(), dir.join("missing.txt")]).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(dict.entry_count(), 3);
        // 同じ読みは Mozc 辞書 → extra の順
        let entries = dict.lookup("かんべ");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].surface, "神戸");
        assert_eq!((entries[1].surface.as_str(), entries[1].left_id), ("寛部", 1921));
        assert_eq!(dict.lookup("あられ")[0].surface, "霰");
    }
}
