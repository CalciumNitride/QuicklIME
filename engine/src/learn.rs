// 学習 (確定履歴) の記録と永続化
//
// 「読み → 最後に確定した表記」を記憶し、候補順の調整に使う。
// 永続化は TSV (読み\t表記) の追記ログ方式で、読み込み時は後の行が優先される。
// 保存先: %APPDATA%\QuicklIME\learning.tsv (QUICKLIME_LEARN_FILE で上書き可)
//
// 文脈学習: 「(直前文節の表記, 読み) → 表記」も別に記憶し、同音異義語の
// 使い分け (「服を|着る」「紙を|切る」) に使う。永続化は learning_context.tsv
// (文脈\t読み\t表記) の追記ログで、既存の learning.tsv の形式は変えない

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// 文脈キー (直前文節の表記) の最大文字数。
/// 長文の貼り付けなど異常に長い文脈は末尾だけをキーにする
const MAX_CONTEXT_KEY_CHARS: usize = 16;

pub struct LearningStore {
    /// 読み → (表記, 記録順の連番)。連番が大きいほど新しい確定
    map: HashMap<String, (String, u64)>,
    /// (前文脈の表記, 読み) → (表記, 記録順の連番)。文脈付きの学習
    ctx_map: HashMap<(String, String), (String, u64)>,
    /// 追記先ファイル。None ならメモリ上のみ (保存失敗時・テスト時)
    path: Option<PathBuf>,
    /// 文脈付き学習の追記先ファイル (learning_context.tsv)
    ctx_path: Option<PathBuf>,
    /// 次に振る連番。追記ログの行順が新しさを表すため、フォーマット変更なしで導出できる
    seq: u64,
}

impl LearningStore {
    /// 既定の保存先から読み込む。ファイルが無ければ空の状態で始める
    pub fn load_default() -> Self {
        let Some(path) = default_path() else {
            eprintln!("学習ファイルの保存先を特定できません。学習はこのセッション限りになります");
            return LearningStore::in_memory();
        };
        let ctx_path = context_path_for(&path);
        let mut store = LearningStore {
            map: HashMap::new(),
            ctx_map: HashMap::new(),
            path: Some(path.clone()),
            ctx_path: Some(ctx_path.clone()),
            seq: 0,
        };
        if let Ok(file) = File::open(&path) {
            store.load_from(BufReader::new(file));
            eprintln!("学習データを読み込みました: {} 件 [{}]", store.map.len(), path.display());
        }
        if let Ok(file) = File::open(&ctx_path) {
            store.load_ctx_from(BufReader::new(file));
            eprintln!(
                "文脈学習データを読み込みました: {} 件 [{}]",
                store.ctx_map.len(),
                ctx_path.display()
            );
        }
        store
    }

    /// テスト用: メモリ上のみのストア
    pub fn in_memory() -> Self {
        LearningStore {
            map: HashMap::new(),
            ctx_map: HashMap::new(),
            path: None,
            ctx_path: None,
            seq: 0,
        }
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

    /// 文脈付き学習の追記ログ (文脈\t読み\t表記) を読み込む (後の行が優先)
    pub fn load_ctx_from(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            let mut parts = line.splitn(3, '\t');
            if let (Some(context), Some(reading), Some(surface)) =
                (parts.next(), parts.next(), parts.next())
            {
                if !context.is_empty() && !reading.is_empty() && !surface.is_empty() {
                    self.seq += 1;
                    self.ctx_map.insert(
                        (context_key(context), reading.to_string()),
                        (surface.to_string(), self.seq),
                    );
                }
            }
        }
    }

    /// 前文脈付きで確定した表記を記録し、ファイルへ追記する
    pub fn record_ctx(&mut self, context: &str, reading: &str, surface: &str) {
        if context.is_empty() || reading.is_empty() || surface.is_empty() {
            return;
        }
        let key = (context_key(context), reading.to_string());
        // 既に同じ内容ならファイル書き込みを省略する (連番だけ更新して新しさを反映する)
        self.seq += 1;
        if let Some(entry) = self.ctx_map.get_mut(&key) {
            if entry.0 == surface {
                entry.1 = self.seq;
                return;
            }
        }
        let context = key.0.clone();
        self.ctx_map.insert(key, (surface.to_string(), self.seq));

        if let Some(path) = &self.ctx_path {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{context}\t{reading}\t{surface}"));
            if let Err(e) = result {
                eprintln!("文脈学習ファイルへの書き込みに失敗しました ({e})");
            }
        }
    }

    /// (前文脈, 読み) に対して学習済みの表記を返す
    pub fn get_ctx(&self, context: &str, reading: &str) -> Option<&str> {
        self.ctx_map
            .get(&(context_key(context), reading.to_string()))
            .map(|(surface, _)| surface.as_str())
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

/// 文脈キーの正規化: 末尾 MAX_CONTEXT_KEY_CHARS 文字に切り詰める
fn context_key(context: &str) -> String {
    let chars: Vec<char> = context.chars().collect();
    chars[chars.len().saturating_sub(MAX_CONTEXT_KEY_CHARS)..].iter().collect()
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

/// 学習ファイルパスから文脈学習ファイルのパスを導出する
/// (learning.tsv → learning_context.tsv)。QUICKLIME_LEARN_FILE で
/// 学習ファイルを差し替えたときも同じディレクトリに対で作られる
fn context_path_for(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("learning");
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => path.with_file_name(format!("{stem}_context.{ext}")),
        None => path.with_file_name(format!("{stem}_context")),
    }
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
    fn 文脈付きで記録した表記が引ける() {
        let mut store = LearningStore::in_memory();
        store.record_ctx("服を", "きる", "着る");
        store.record_ctx("紙を", "きる", "切る");
        assert_eq!(store.get_ctx("服を", "きる"), Some("着る"));
        assert_eq!(store.get_ctx("紙を", "きる"), Some("切る"));
        // 文脈が違えば引けない。文脈なしの学習にも影響しない
        assert_eq!(store.get_ctx("髪を", "きる"), None);
        assert_eq!(store.get("きる"), None);
    }

    #[test]
    fn 文脈付きログは後の行が優先される() {
        let mut store = LearningStore::in_memory();
        store.load_ctx_from(
            "服を\tきる\t切る\n服を\tきる\t着る\n壊れた行\n文脈\t読みだけ\n".as_bytes(),
        );
        assert_eq!(store.get_ctx("服を", "きる"), Some("着る"));
    }

    #[test]
    fn 長い文脈は末尾で切り詰めてキーになる() {
        let mut store = LearningStore::in_memory();
        let long = "あ".repeat(30) + "服を"; // 32文字 → 末尾16文字がキー
        store.record_ctx(&long, "きる", "着る");
        // 末尾16文字が同じ文脈なら一致する
        let other = "い".repeat(30) + &"あ".repeat(14) + "服を";
        assert_eq!(store.get_ctx(&other, "きる"), Some("着る"));
    }

    #[test]
    fn 文脈学習ファイルのパスを導出する() {
        assert_eq!(
            context_path_for(Path::new("C:\\dir\\learning.tsv")),
            PathBuf::from("C:\\dir\\learning_context.tsv")
        );
        assert_eq!(
            context_path_for(Path::new("learn")),
            PathBuf::from("learn_context")
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
