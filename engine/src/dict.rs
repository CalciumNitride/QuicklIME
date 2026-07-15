// Mozc OSS 辞書の読み込みと検索
//
// フォーマット: 読み\t左文脈ID\t右文脈ID\tコスト\t表記 (TSV、UTF-8)
// 辞書ファイル (dictionary00.txt 〜 dictionary09.txt) はリポジトリに含めず、
// references/mozc/ (git 管理外) から読み込む。パスは main.rs を参照。
//
// 検索は HashMap による完全一致 (lookup) と、予測入力用の前方一致 (predict_prefix)。
// 前方一致はソート済み読み配列 + 二分探索で実装している。読み文字列の複製で
// メモリが数十MB増えるため、性能・メモリが問題になったら yada (ダブル配列) への
// 置き換えを検討する。

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

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

pub struct Dictionary {
    map: HashMap<String, Vec<Entry>>,
    entry_count: usize,
    /// 記号辞書 (読み → 記号のリスト、symbol.tsv の記載順)。
    /// 連接情報を持たないため Viterbi には載せず、文節候補の末尾に追記する
    symbols: HashMap<String, Vec<String>>,
    symbol_count: usize,
    /// 前方一致検索用にソートしたユニーク読みの配列 (build_prediction_index で構築)
    readings_sorted: Vec<String>,
}

impl Dictionary {
    /// 空の辞書 (辞書ファイルが見つからない場合のフォールバック)
    pub fn empty() -> Self {
        Dictionary {
            map: HashMap::new(),
            entry_count: 0,
            symbols: HashMap::new(),
            symbol_count: 0,
            readings_sorted: Vec::new(),
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
        dict.build_prediction_index();
        Ok(dict)
    }

    /// 前方一致検索用の読み配列を構築する。load_from を直接使う場合 (テスト等) は
    /// 読み込み後に明示的に呼ぶ。未構築でも予測が空になるだけで他の検索には影響しない
    pub fn build_prediction_index(&mut self) {
        self.readings_sorted = self.map.keys().cloned().collect();
        self.readings_sorted.sort_unstable();
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
            self.map.entry(reading.to_string()).or_default().push(Entry {
                left_id,
                right_id,
                cost,
                surface: surface.to_string(),
            });
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

    /// 読みに完全一致するエントリ一覧を返す
    pub fn lookup(&self, reading: &str) -> &[Entry] {
        self.map.get(reading).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 読みが prefix で始まるエントリをコスト昇順で limit 件まで返す (予測入力用)。
    /// 戻り値は (読み, 表記)。build_prediction_index が未実行なら空を返す
    pub fn predict_prefix(&self, prefix: &str, limit: usize) -> Vec<(String, String)> {
        if prefix.is_empty() || limit == 0 {
            return Vec::new();
        }
        let start = self.readings_sorted.partition_point(|r| r.as_str() < prefix);
        let mut hits: Vec<(i16, &str, &str)> = Vec::new();
        for reading in self.readings_sorted[start..].iter().take(MAX_PREFIX_SCAN) {
            if !reading.starts_with(prefix) {
                break; // ソート済みなので一致範囲はここで終わり
            }
            for entry in self.lookup(reading) {
                hits.push((entry.cost, reading.as_str(), entry.surface.as_str()));
            }
        }
        // 安定ソートでコスト同点は辞書の記載順を保つ
        hits.sort_by_key(|(cost, _, _)| *cost);
        hits.truncate(limit);
        hits.into_iter()
            .map(|(_, reading, surface)| (reading.to_string(), surface.to_string()))
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
        dict.build_prediction_index();
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
    fn 索引未構築なら前方一致は空() {
        let mut dict = Dictionary::empty();
        dict.load_from("きょう\t100\t100\t3000\t今日\n".as_bytes()).unwrap();
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
