// 学習 (確定履歴) の記録と永続化
//
// 「読み → 最後に確定した表記」を記憶し、候補順の調整に使う。
// 永続化は TSV (読み\t表記) の追記ログ方式で、読み込み時は後の行が優先される。
// 保存先: %APPDATA%\QuicklIME\learning.tsv (QUICKLIME_LEARN_FILE で上書き可)
//
// 文脈学習: 「(直前文節の表記, 読み) → 表記」も別に記憶し、同音異義語の
// 使い分け (「服を|着る」「紙を|切る」) に使う。永続化は learning_context.tsv
// (文脈\t読み\t表記) の追記ログで、既存の learning.tsv の形式は変えない
//
// 境界学習: 人が文節伸縮で直した分割の文節読みを記憶し、変換時の文節境界を
// その位置へ寄せるのに使う。永続化は learning_boundary.tsv (1行1読み)

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// 文脈キー (直前文節の表記) の最大文字数。
/// 長文の貼り付けなど異常に長い文脈は末尾だけをキーにする
const MAX_CONTEXT_KEY_CHARS: usize = 16;

/// 境界学習に記録する文節読みの最小文字数。
/// 「は」のような1文字の付属語まで境界にすると、どこでも切れる方向に効いてしまう
pub const MIN_BOUNDARY_CHARS: usize = 2;

pub struct LearningStore {
    /// 読み → (表記, 記録順の連番)。連番が大きいほど新しい確定
    map: HashMap<String, (String, u64)>,
    /// (前文脈の表記, 読み) → (表記, 記録順の連番)。文脈付きの学習
    ctx_map: HashMap<(String, String), (String, u64)>,
    /// 文脈キー → その文脈で記録した読み。打鍵ごとの文脈での引き当てで ctx_map を走査しないため
    ctx_index: HashMap<String, HashSet<String>>,
    /// 追記先ファイル。None ならメモリ上のみ (保存失敗時・テスト時)
    path: Option<PathBuf>,
    /// 人が文節伸縮で直した分割の文節読み
    boundary: HashSet<String>,
    /// 文脈付き学習の追記先ファイル (learning_context.tsv)
    ctx_path: Option<PathBuf>,
    /// 境界学習の追記先ファイル (learning_boundary.tsv)
    boundary_path: Option<PathBuf>,
    /// boundary に入っている読みの最大文字数。変換時の部分文字列探索の上限に使う
    boundary_max: usize,
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
        let ctx_path = sibling_path(&path, "_context");
        let boundary_path = sibling_path(&path, "_boundary");
        let mut store = LearningStore {
            map: HashMap::new(),
            ctx_map: HashMap::new(),
            ctx_index: HashMap::new(),
            boundary: HashSet::new(),
            path: Some(path.clone()),
            ctx_path: Some(ctx_path.clone()),
            boundary_path: Some(boundary_path.clone()),
            boundary_max: 0,
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
        if let Ok(file) = File::open(&boundary_path) {
            store.load_boundary_from(BufReader::new(file));
            eprintln!(
                "境界学習データを読み込みました: {} 件 [{}]",
                store.boundary.len(),
                boundary_path.display()
            );
        }
        store
    }

    /// テスト用: メモリ上のみのストア
    pub fn in_memory() -> Self {
        LearningStore {
            map: HashMap::new(),
            ctx_map: HashMap::new(),
            ctx_index: HashMap::new(),
            boundary: HashSet::new(),
            path: None,
            ctx_path: None,
            boundary_path: None,
            boundary_max: 0,
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
                    let key = context_key(context);
                    self.ctx_index.entry(key.clone()).or_default().insert(reading.to_string());
                    self.ctx_map.insert((key, reading.to_string()), (surface.to_string(), self.seq));
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
        self.ctx_index.entry(context.clone()).or_default().insert(reading.to_string());
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

    /// 前文脈 context の後に確定した (読み, 表記) のうち、読みが prefix で始まるものを
    /// 記録が新しい順に返す (prefix が空なら全件)。表記の重複は先勝ちで除く
    pub fn predict_context(&self, context: &str, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        let key = context_key(context);
        let Some(readings) = self.ctx_index.get(&key) else {
            return Vec::new();
        };
        let mut matches: Vec<(&str, &str, u64)> = readings
            .iter()
            .filter(|reading| reading.starts_with(prefix))
            .filter_map(|reading| {
                let (surface, seq) = self.ctx_map.get(&(key.clone(), reading.clone()))?;
                Some((reading.as_str(), surface.as_str(), *seq))
            })
            .collect();
        matches.sort_by(|a, b| b.2.cmp(&a.2));
        let mut results: Vec<(&str, &str)> = Vec::new();
        for (reading, surface, _) in matches {
            if results.len() >= limit {
                break;
            }
            if !results.iter().any(|(_, s)| *s == surface) {
                results.push((reading, surface));
            }
        }
        results
    }

    /// 境界学習の追記ログ (1行1読み) を読み込む
    pub fn load_boundary_from(&mut self, reader: impl BufRead) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            self.insert_boundary(line.trim_end());
        }
    }

    /// 人が文節伸縮で直した分割の文節読みを境界として記録し、ファイルへ追記する
    pub fn record_boundary(&mut self, reading: &str) {
        if !self.insert_boundary(reading) {
            return; // 既知の境界。追記ログが際限なく伸びないよう書き込みを省く
        }
        if let Some(path) = &self.boundary_path {
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{reading}"));
            if let Err(e) = result {
                eprintln!("境界学習ファイルへの書き込みに失敗しました ({e})");
            }
        }
    }

    /// 境界の集合へ追加する。新規に追加できたら true
    fn insert_boundary(&mut self, reading: &str) -> bool {
        let len = reading.chars().count();
        if len < MIN_BOUNDARY_CHARS || !self.boundary.insert(reading.to_string()) {
            return false;
        }
        self.boundary_max = self.boundary_max.max(len);
        true
    }

    /// 読みが学習済みの文節境界かどうか
    pub fn is_boundary(&self, reading: &str) -> bool {
        self.boundary.contains(reading)
    }

    /// 学習済み境界の読みの最大文字数 (0 なら境界学習なし)
    pub fn max_boundary_chars(&self) -> usize {
        self.boundary_max
    }

    /// (読み, 表記) の学習を消す: 文脈なしの学習はその読みの表記が一致するときだけ、
    /// 文脈付きの学習は前文脈によらず読みと表記が一致するものをすべて消す。
    /// 消した項目があったファイルはメモリの内容から書き直す。戻り値は消した件数
    pub fn forget(&mut self, reading: &str, surface: &str) -> usize {
        let mut plain = 0;
        if self.map.get(reading).is_some_and(|(s, _)| s == surface) {
            self.map.remove(reading);
            plain = 1;
        }

        let ctx_keys: Vec<(String, String)> = self
            .ctx_map
            .iter()
            .filter(|((_, r), (s, _))| r == reading && s == surface)
            .map(|(key, _)| key.clone())
            .collect();
        for key in &ctx_keys {
            self.ctx_map.remove(key);
            if let Some(readings) = self.ctx_index.get_mut(&key.0) {
                readings.remove(&key.1);
                if readings.is_empty() {
                    self.ctx_index.remove(&key.0);
                }
            }
        }

        if plain > 0 {
            if let Some(path) = &self.path {
                let mut entries: Vec<(&String, &(String, u64))> = self.map.iter().collect();
                entries.sort_by_key(|(_, (_, seq))| *seq);
                let content: String =
                    entries.iter().map(|(r, (s, _))| format!("{r}\t{s}\n")).collect();
                if let Err(e) = rewrite_file(path, &content) {
                    eprintln!("学習ファイルの書き直しに失敗しました ({e})");
                }
            }
        }
        if !ctx_keys.is_empty() {
            if let Some(path) = &self.ctx_path {
                let mut entries: Vec<(&(String, String), &(String, u64))> =
                    self.ctx_map.iter().collect();
                entries.sort_by_key(|(_, (_, seq))| *seq);
                let content: String = entries
                    .iter()
                    .map(|((c, r), (s, _))| format!("{c}\t{r}\t{s}\n"))
                    .collect();
                if let Err(e) = rewrite_file(path, &content) {
                    eprintln!("文脈学習ファイルの書き直しに失敗しました ({e})");
                }
            }
        }
        plain + ctx_keys.len()
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

/// ファイルの内容を content で置き換える。書き込みの途中で失敗しても元のファイルが
/// 壊れないよう、同じディレクトリの一時ファイルに書いてから rename する
pub fn rewrite_file(path: &Path, content: &str) -> std::io::Result<()> {
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let result = fs::write(&tmp, content).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
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

/// 自動学習した複合語の保存先 (learning.tsv → learning_word.tsv)。
/// ユーザ辞書ではなく学習データなので、学習ファイルと同じディレクトリに置く
pub fn learned_word_path() -> Option<PathBuf> {
    Some(sibling_path(&default_path()?, "_word"))
}

/// 学習ファイルパスから派生ファイルのパスを導出する
/// (learning.tsv + "_context" → learning_context.tsv)。QUICKLIME_LEARN_FILE で
/// 学習ファイルを差し替えたときも同じディレクトリに対で作られる
fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("learning");
    match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => path.with_file_name(format!("{stem}{suffix}.{ext}")),
        None => path.with_file_name(format!("{stem}{suffix}")),
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
    fn 文脈で引くと新しい順に返り表記の重複を除く() {
        let mut store = LearningStore::in_memory();
        store.record_ctx("今日は", "いいてんき", "いい天気");
        store.record_ctx("今日は", "はれ", "晴れ");
        store.record_ctx("今日は", "はれる", "晴れ");
        store.record_ctx("明日は", "あめ", "雨");
        assert_eq!(
            store.predict_context("今日は", "", 8),
            vec![("はれる", "晴れ"), ("いいてんき", "いい天気")]
        );
        // 確定し直すと新しさが更新される
        store.record_ctx("今日は", "いいてんき", "いい天気");
        assert_eq!(
            store.predict_context("今日は", "", 8),
            vec![("いいてんき", "いい天気"), ("はれる", "晴れ")]
        );
        assert_eq!(store.predict_context("今日は", "", 1), vec![("いいてんき", "いい天気")]);
        assert!(store.predict_context("昨日は", "", 8).is_empty());
    }

    #[test]
    fn 文脈での引き当ては読みの前方一致で絞る() {
        let mut store = LearningStore::in_memory();
        store.record_ctx("今日は", "いいてんき", "いい天気");
        store.record_ctx("今日は", "はれ", "晴れ");
        assert_eq!(store.predict_context("今日は", "い", 8), vec![("いいてんき", "いい天気")]);
        assert!(store.predict_context("今日は", "あ", 8).is_empty());
    }

    #[test]
    fn 長い文脈は末尾で切り詰めたキーで引ける() {
        let mut store = LearningStore::in_memory();
        let long = "あ".repeat(30) + "今日は"; // 33文字 → 末尾16文字がキー
        store.record_ctx(&long, "いいてんき", "いい天気");
        let other = "い".repeat(30) + &"あ".repeat(13) + "今日は";
        assert_eq!(store.predict_context(&other, "", 8), vec![("いいてんき", "いい天気")]);
    }

    #[test]
    fn 文脈付きログの読み込みでも文脈で引ける() {
        let mut store = LearningStore::in_memory();
        store.load_ctx_from("今日は\tいいてんき\tいい天気\n今日は\tはれ\t晴れ\n".as_bytes());
        assert_eq!(
            store.predict_context("今日は", "", 8),
            vec![("はれ", "晴れ"), ("いいてんき", "いい天気")]
        );
    }

    #[test]
    fn 派生学習ファイルのパスを導出する() {
        assert_eq!(
            sibling_path(Path::new("C:\\dir\\learning.tsv"), "_context"),
            PathBuf::from("C:\\dir\\learning_context.tsv")
        );
        assert_eq!(
            sibling_path(Path::new("C:\\dir\\learning.tsv"), "_boundary"),
            PathBuf::from("C:\\dir\\learning_boundary.tsv")
        );
        assert_eq!(
            sibling_path(Path::new("learn"), "_context"),
            PathBuf::from("learn_context")
        );
    }

    #[test]
    fn 記録した文節境界が引ける() {
        let mut store = LearningStore::in_memory();
        store.record_boundary("きょうは");
        store.record_boundary("いいてんき");
        assert!(store.is_boundary("きょうは"));
        assert!(store.is_boundary("いいてんき"));
        assert!(!store.is_boundary("きょうはい"));
        assert_eq!(store.max_boundary_chars(), 5);
    }

    #[test]
    fn 短すぎる読みは境界にしない() {
        let mut store = LearningStore::in_memory();
        store.record_boundary("は");
        store.record_boundary("");
        assert!(!store.is_boundary("は"));
        assert_eq!(store.max_boundary_chars(), 0);
    }

    #[test]
    fn 境界の追記ログを読み込む() {
        let mut store = LearningStore::in_memory();
        store.load_boundary_from("きょうは\nいいてんき\nは\n\n".as_bytes());
        assert!(store.is_boundary("きょうは"));
        assert!(store.is_boundary("いいてんき"));
        assert!(!store.is_boundary("は"));
        assert_eq!(store.max_boundary_chars(), 5);
    }

    #[test]
    fn 表記が一致する学習だけを消す() {
        let mut store = LearningStore::in_memory();
        store.record("きょうし", "教師");
        store.record_ctx("今日は", "きょうし", "教師");
        store.record_ctx("明日は", "きょうし", "教師");
        store.record_ctx("昨日は", "きょうし", "講師");
        store.record_ctx("今日は", "はれ", "晴れ");
        // 表記が違えば消えない
        assert_eq!(store.forget("きょうし", "講義"), 0);
        assert_eq!(store.get("きょうし"), Some("教師"));
        // 文脈なし 1 件 + 文脈付き 2 件
        assert_eq!(store.forget("きょうし", "教師"), 3);
        assert_eq!(store.get("きょうし"), None);
        assert_eq!(store.get_ctx("今日は", "きょうし"), None);
        assert_eq!(store.get_ctx("明日は", "きょうし"), None);
        assert_eq!(store.get_ctx("昨日は", "きょうし"), Some("講師"));
        assert_eq!(store.predict_context("今日は", "", 8), vec![("はれ", "晴れ")]);
        assert!(store.predict_context("明日は", "", 8).is_empty());
        assert!(store.predict_prefix("きょう", 8).is_empty());
        // 2 回目は消すものが無い
        assert_eq!(store.forget("きょうし", "教師"), 0);
    }

    #[test]
    fn 消した後に書き直したファイルを読み込み直すと同じ状態になる() {
        let dir = std::env::temp_dir().join(format!("quicklime-forget-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("learning.tsv");
        let ctx_path = dir.join("learning_context.tsv");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&ctx_path);

        let mut store = LearningStore {
            path: Some(path.clone()),
            ctx_path: Some(ctx_path.clone()),
            ..LearningStore::in_memory()
        };
        store.record("きょうし", "講師");
        store.record("きょうし", "教師");
        store.record("きょう", "今日");
        store.record("はれ", "晴れ");
        // 同じ内容の確定し直しはファイルに書かれず、連番だけが新しくなる
        store.record("きょう", "今日");
        store.record_ctx("今日は", "きょうし", "教師");
        store.record_ctx("今日は", "はれ", "晴れ");
        store.record_ctx("明日は", "きょうし", "教師");

        assert_eq!(store.forget("きょうし", "教師"), 3);
        // 以前の表記 (講師) の行も消え、残りは連番の昇順
        assert_eq!(fs::read_to_string(&path).unwrap(), "はれ\t晴れ\nきょう\t今日\n");
        assert_eq!(fs::read_to_string(&ctx_path).unwrap(), "今日は\tはれ\t晴れ\n");
        assert!(!dir.join("learning.tsv.tmp").exists());

        let mut reloaded = LearningStore::in_memory();
        reloaded.load_from(BufReader::new(File::open(&path).unwrap()));
        reloaded.load_ctx_from(BufReader::new(File::open(&ctx_path).unwrap()));
        assert_eq!(reloaded.get("きょうし"), None);
        assert_eq!(reloaded.predict_prefix("", 8), store.predict_prefix("", 8));
        assert_eq!(reloaded.predict_context("今日は", "", 8), vec![("はれ", "晴れ")]);
        assert!(reloaded.predict_context("明日は", "", 8).is_empty());

        // 文脈付きの学習だけが一致したときは learning.tsv を書き直さない
        store.record_ctx("明日は", "あめ", "雨");
        fs::write(&path, "そのまま\t残る\n").unwrap();
        assert_eq!(store.forget("あめ", "雨"), 1);
        assert_eq!(fs::read_to_string(&path).unwrap(), "そのまま\t残る\n");
        assert_eq!(fs::read_to_string(&ctx_path).unwrap(), "今日は\tはれ\t晴れ\n");

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&ctx_path);
        let _ = fs::remove_dir(&dir);
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
