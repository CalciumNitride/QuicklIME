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
    /// 読み → (表記, 記録順の連番)。連番が大きいほど新しい確定
    map: HashMap<String, (String, u64)>,
    /// 追記先ファイル。None ならメモリ上のみ (保存失敗時・テスト時)
    path: Option<PathBuf>,
    /// 次に振る連番。追記ログの行順が新しさを表すため、フォーマット変更なしで導出できる
    seq: u64,
}

impl LearningStore {
    /// 既定の保存先から読み込む。ファイルが無ければ空の状態で始める
    pub fn load_default() -> Self {
        let Some(path) = default_path() else {
            eprintln!("学習ファイルの保存先を特定できません。学習はこのセッション限りになります");
            return LearningStore { map: HashMap::new(), path: None, seq: 0 };
        };
        let mut store = LearningStore { map: HashMap::new(), path: Some(path.clone()), seq: 0 };
        if let Ok(file) = File::open(&path) {
            store.load_from(BufReader::new(file));
            eprintln!("学習データを読み込みました: {} 件 [{}]", store.map.len(), path.display());
        }
        store
    }

    /// テスト用: メモリ上のみのストア
    pub fn in_memory() -> Self {
        LearningStore { map: HashMap::new(), path: None, seq: 0 }
    }

    /// 追記ログを読み込む (後の行が優先)
    pub fn load_from(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some((reading, surface)) = line.split_once('\t') {
                if !reading.is_empty() && !surface.is_empty() {
                    self.seq += 1;
                    self.map.insert(reading.to_string(), (surface.to_string(), self.seq));
                }
            }
        }
    }

    /// 確定した表記を記録し、ファイルへ追記する
    pub fn record(&mut self, reading: &str, surface: &str) {
        if reading.is_empty() || surface.is_empty() {
            return;
        }
        // 既に同じ内容ならファイル書き込みを省略する (連番だけ更新して新しさを反映する)
        self.seq += 1;
        if let Some(entry) = self.map.get_mut(reading) {
            if entry.0 == surface {
                entry.1 = self.seq;
                return;
            }
        }
        self.map.insert(reading.to_string(), (surface.to_string(), self.seq));

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
        self.map.get(reading).map(|(surface, _)| surface.as_str())
    }

    /// 読みが prefix で始まる履歴を新しい順に返す (予測入力用)。
    /// 件数が小さい (高々数千) ため線形走査で足りる
    pub fn predict_prefix(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        let mut matches: Vec<(&str, &str, u64)> = self
            .map
            .iter()
            .filter(|(reading, _)| reading.starts_with(prefix))
            .map(|(reading, (surface, seq))| (reading.as_str(), surface.as_str(), *seq))
            .collect();
        matches.sort_by(|a, b| b.2.cmp(&a.2));
        matches.truncate(limit);
        matches.into_iter().map(|(reading, surface, _)| (reading, surface)).collect()
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

    #[test]
    fn 前方一致予測は新しい順に返す() {
        let mut store = LearningStore::in_memory();
        store.record("きょう", "今日");
        store.record("きょうと", "京都");
        store.record("はれ", "晴れ");
        assert_eq!(
            store.predict_prefix("きょう", 8),
            vec![("きょうと", "京都"), ("きょう", "今日")]
        );
        // 同じ読みを確定し直すと新しさが更新される
        store.record("きょう", "今日");
        assert_eq!(
            store.predict_prefix("きょう", 8),
            vec![("きょう", "今日"), ("きょうと", "京都")]
        );
    }

    #[test]
    fn 前方一致予測はログの行順でも新しい順になる() {
        let mut store = LearningStore::in_memory();
        store.load_from("きょう\t今日\nきょうと\t京都\n".as_bytes());
        assert_eq!(
            store.predict_prefix("きょう", 8),
            vec![("きょうと", "京都"), ("きょう", "今日")]
        );
    }

    #[test]
    fn 前方一致予測の上限と不一致() {
        let mut store = LearningStore::in_memory();
        store.record("きょう", "今日");
        store.record("きょうと", "京都");
        assert_eq!(store.predict_prefix("きょう", 1), vec![("きょうと", "京都")]);
        assert!(store.predict_prefix("はれ", 8).is_empty());
    }
}
