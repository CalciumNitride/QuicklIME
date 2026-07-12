// Mozc OSS 辞書の読み込みと検索
//
// フォーマット: 読み\t左文脈ID\t右文脈ID\tコスト\t表記 (TSV、UTF-8)
// 辞書ファイル (dictionary00.txt 〜 dictionary09.txt) はリポジトリに含めず、
// references/mozc/ (git 管理外) から読み込む。パスは main.rs を参照。
//
// 現状は HashMap による完全一致検索のみ。Viterbi 導入時に共通接頭辞検索が
// 必要になったら yada (ダブル配列) への置き換えを検討する。

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

pub struct Dictionary {
    map: HashMap<String, Vec<Entry>>,
    entry_count: usize,
}

impl Dictionary {
    /// 空の辞書 (辞書ファイルが見つからない場合のフォールバック)
    pub fn empty() -> Self {
        Dictionary { map: HashMap::new(), entry_count: 0 }
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
        Ok(dict)
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

    /// 読みに完全一致するエントリ一覧を返す
    pub fn lookup(&self, reading: &str) -> &[Entry] {
        self.map.get(reading).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn entry_count(&self) -> usize {
        self.entry_count
    }
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
}
