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

/// fst::Map の値に entries 内の位置を詰める ((開始index << 32) | 件数)
fn pack(start: usize, len: usize) -> u64 {
    debug_assert!(start < (1 << 32) && len < (1 << 32));
    ((start as u64) << 32) | (len as u64)
}

fn unpack(packed: u64) -> (usize, usize) {
    ((packed >> 32) as usize, (packed & 0xFFFF_FFFF) as usize)
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
