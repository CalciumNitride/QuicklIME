// 学習 (確定履歴) の記録と永続化
//
// 「読み → 最後に確定した表記」を記憶し、候補順の調整に使う。
// 永続化は TSV (読み\t表記) の追記ログ方式で、読み込み時は後の行が優先される。
// 保存先: %APPDATA%\QuicklIME\learning.tsv (QUICKLIME_LEARN_FILE で上書き可)

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub struct LearningStore {
    map: HashMap<String, String>,
    /// 追記先ファイル。None ならメモリ上のみ (保存失敗時・テスト時)
    path: Option<PathBuf>,
}

impl LearningStore {
    /// 既定の保存先から読み込む。ファイルが無ければ空の状態で始める
    pub fn load_default() -> Self {
        let Some(path) = default_path() else {
            eprintln!("学習ファイルの保存先を特定できません。学習はこのセッション限りになります");
            return LearningStore { map: HashMap::new(), path: None };
        };
        let mut store = LearningStore { map: HashMap::new(), path: Some(path.clone()) };
        if let Ok(file) = File::open(&path) {
            store.load_from(BufReader::new(file));
            eprintln!("学習データを読み込みました: {} 件 [{}]", store.map.len(), path.display());
        }
        store
    }

    /// テスト用: メモリ上のみのストア
    pub fn in_memory() -> Self {
        LearningStore { map: HashMap::new(), path: None }
    }

    /// 追記ログを読み込む (後の行が優先)
    pub fn load_from(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some((reading, surface)) = line.split_once('\t') {
                if !reading.is_empty() && !surface.is_empty() {
                    self.map.insert(reading.to_string(), surface.to_string());
                }
            }
        }
    }

    /// 確定した表記を記録し、ファイルへ追記する
    pub fn record(&mut self, reading: &str, surface: &str) {
        if reading.is_empty() || surface.is_empty() {
            return;
        }
        // 既に同じ内容なら書き込みを省略する
        if self.map.get(reading).is_some_and(|s| s == surface) {
            return;
        }
        self.map.insert(reading.to_string(), surface.to_string());

        if let Some(path) = &self.path {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{reading}\t{surface}"));
            if let Err(e) = result {
                eprintln!("学習ファイルへの書き込みに失敗しました ({e})");
            }
        }
    }

    /// 読みに対して学習済みの表記を返す
    pub fn get(&self, reading: &str) -> Option<&str> {
        self.map.get(reading).map(String::as_str)
    }
}

/// 既定の学習ファイルパス。優先順: QUICKLIME_LEARN_FILE > %APPDATA%\QuicklIME\learning.tsv
fn default_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("QUICKLIME_LEARN_FILE") {
        return Some(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").ok()?;
    let dir = PathBuf::from(appdata).join("QuicklIME");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("learning.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 記録した表記が引ける() {
        let mut store = LearningStore::in_memory();
        store.record("きょう", "京");
        assert_eq!(store.get("きょう"), Some("京"));
        assert_eq!(store.get("みらい"), None);
    }

    #[test]
    fn 後から記録した表記が優先される() {
        let mut store = LearningStore::in_memory();
        store.record("きょう", "京");
        store.record("きょう", "今日");
        assert_eq!(store.get("きょう"), Some("今日"));
    }

    #[test]
    fn 追記ログは後の行が優先される() {
        let mut store = LearningStore::in_memory();
        store.load_from("きょう\t京\nきょう\t今日\nはれ\t晴れ\n壊れた行\n".as_bytes());
        assert_eq!(store.get("きょう"), Some("今日"));
        assert_eq!(store.get("はれ"), Some("晴れ"));
    }
}
