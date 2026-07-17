// ユーザ辞書 (userdict.tsv) の読み込み・検索・登録
//
// フォーマット: 読み\t表記\t品詞[\tコメント] (Mozc ユーザ辞書エクスポート互換 TSV)。
// 品詞列が無い行は短縮よみとみなし、# 始まりの行はコメント。
// 対応する品詞:
//   短縮よみ: 連接情報を持たない定型文 (メールアドレス等)。
//             変換・予測の候補列へそのまま挿入する
//   名詞系 (名詞/固有名詞/人名/姓/名/地名/組織):
//             id.def から文脈IDを解決し、Viterbi のラティスに通常の辞書語と
//             同様に載せる (文中でも変換される)
// それ以外の品詞の行は無視する (動詞などの活用展開は未対応)。
// 登録 (ADDWORD) の追記先ファイルとメモリ内容を一体で管理し、
// エンジンの再起動なしで登録を反映する。
// 保存先: %APPDATA%\QuicklIME\userdict.tsv (QUICKLIME_USER_DICT_FILE で上書き可)

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::pos::{FunctionalIds, DEFAULT_NOUN_ID};

/// 短縮よみの品詞名 (ファイル上の表記)
pub const SHORTCUT_POS: &str = "短縮よみ";

/// ユーザ登録した名詞系単語の単語コスト。
/// 同じ読みの一般語より候補上位に来やすい中位の値 (並びは学習でさらに調整される)
const USER_WORD_COST: i16 = 3000;

/// ユーザ辞書の品詞名 → id.def の品詞パス (前方一致)。未対応の品詞は None
fn noun_id_prefix(pos: &str) -> Option<&'static str> {
    Some(match pos {
        "名詞" => "名詞,一般",
        "固有名詞" => "名詞,固有名詞,一般",
        "人名" => "名詞,固有名詞,人名,一般",
        "姓" => "名詞,固有名詞,人名,姓",
        "名" => "名詞,固有名詞,人名,名",
        "地名" => "名詞,固有名詞,地域,一般",
        "組織" => "名詞,固有名詞,組織",
        _ => return None,
    })
}

/// ユーザ登録した名詞系の1単語
pub struct UserWord {
    pub reading: String,
    pub surface: String,
    /// 品詞名 (ファイル記載のまま。「名詞」「人名」等)。変換には使わない
    #[allow(dead_code)]
    pub pos: String,
    pub left_id: u16,
    pub right_id: u16,
    pub cost: i16,
}

pub struct UserDict {
    /// 短縮よみ (読み, 表記)。ファイル記載順
    shortcuts: Vec<(String, String)>,
    /// 名詞系の単語。ファイル記載順
    words: Vec<UserWord>,
    /// 登録 (add) の追記先ファイル。None ならメモリ上のみ (テスト時)
    path: Option<PathBuf>,
}

impl UserDict {
    /// 空のユーザ辞書 (メモリ上のみ。テスト用)
    pub fn empty() -> Self {
        UserDict { shortcuts: Vec::new(), words: Vec::new(), path: None }
    }

    /// 既定のパスから読み込む。ファイルが無ければ空で始める (初回登録時に作られる)
    pub fn load_default(functional: &FunctionalIds) -> Self {
        let Some(path) = default_path() else {
            eprintln!("ユーザ辞書の保存先を特定できません。登録はこのセッション限りになります");
            return UserDict::empty();
        };
        let mut dict =
            UserDict { shortcuts: Vec::new(), words: Vec::new(), path: Some(path.clone()) };
        if let Ok(file) = File::open(&path) {
            dict.load_from(BufReader::new(file), functional);
            eprintln!(
                "ユーザ辞書を読み込みました: 短縮よみ {} 件・単語 {} 件 [{}]",
                dict.shortcut_count(),
                dict.word_count(),
                path.display()
            );
        }
        dict
    }

    /// ファイルから読み直す (RELOADUSER 用。手動編集の反映)。
    /// パスが無ければ (メモリ上のみなら) 何もしない
    pub fn reload(&mut self, functional: &FunctionalIds) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.shortcuts.clear();
        self.words.clear();
        if let Ok(file) = File::open(&path) {
            self.load_from(BufReader::new(file), functional);
        }
    }

    /// 1ファイル分のエントリを読み込む (テストからも使う)。不正な行は無視する
    pub fn load_from(&mut self, reader: impl BufRead, functional: &FunctionalIds) {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if line.starts_with('#') {
                continue; // コメント行
            }
            let mut fields = line.split('\t');
            let (Some(reading), Some(surface)) = (fields.next(), fields.next()) else {
                continue; // 列が足りない行は無視
            };
            // 品詞列が無い行は短縮よみとみなす (旧フォーマット互換)
            let pos = fields.next().unwrap_or(SHORTCUT_POS);
            let _ = self.insert(reading, surface, pos, functional);
        }
    }

    /// 1件登録する: メモリへ反映し、ユーザ辞書ファイルへ追記する (ADDWORD 用)。
    /// 失敗はエラーメッセージで返す (空・未対応の品詞・登録済み)
    pub fn add(
        &mut self,
        reading: &str,
        surface: &str,
        pos: &str,
        functional: &FunctionalIds,
    ) -> Result<(), String> {
        self.insert(reading, surface, pos, functional)?;
        if let Some(path) = &self.path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let result = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| writeln!(file, "{reading}\t{surface}\t{pos}"));
            if let Err(e) = result {
                // メモリには反映済みなので登録自体は成立させる (このセッション限りになる)
                eprintln!("ユーザ辞書への書き込みに失敗しました ({e})");
            }
        }
        Ok(())
    }

    /// メモリへ1件追加する (ファイルには書かない)
    fn insert(
        &mut self,
        reading: &str,
        surface: &str,
        pos: &str,
        functional: &FunctionalIds,
    ) -> Result<(), String> {
        if reading.is_empty() || surface.is_empty() {
            return Err("読みまたは表記が空です".to_string());
        }
        if pos == SHORTCUT_POS {
            let pair = (reading.to_string(), surface.to_string());
            if self.shortcuts.contains(&pair) {
                return Err("すでに登録されています".to_string());
            }
            self.shortcuts.push(pair);
            return Ok(());
        }
        let Some(prefix) = noun_id_prefix(pos) else {
            return Err(format!("未対応の品詞です: {pos}"));
        };
        if self.words.iter().any(|w| w.reading == reading && w.surface == surface) {
            return Err("すでに登録されています".to_string());
        }
        // id.def が無い・引けない場合は一般名詞のIDで代用する
        let id = functional.find_id(prefix).unwrap_or(DEFAULT_NOUN_ID);
        self.words.push(UserWord {
            reading: reading.to_string(),
            surface: surface.to_string(),
            pos: pos.to_string(),
            left_id: id,
            right_id: id,
            cost: USER_WORD_COST,
        });
        Ok(())
    }

    /// 読みに完全一致する短縮よみの表記一覧を返す (ファイル記載順)
    pub fn lookup_shortcuts(&self, reading: &str) -> Vec<&str> {
        self.shortcuts
            .iter()
            .filter(|(r, _)| r.as_str() == reading)
            .map(|(_, s)| s.as_str())
            .collect()
    }

    /// 読みが prefix で始まる短縮よみを記載順に limit 件まで返す (予測入力用)
    pub fn shortcut_prefix(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        self.shortcuts
            .iter()
            .filter(|(r, _)| r.starts_with(prefix))
            .take(limit)
            .map(|(r, s)| (r.as_str(), s.as_str()))
            .collect()
    }

    /// 読みに完全一致する名詞系単語を返す (ファイル記載順)
    pub fn lookup_words(&self, reading: &str) -> Vec<&UserWord> {
        self.words.iter().filter(|w| w.reading == reading).collect()
    }

    /// 読みの並び suffix の先頭から始まる名詞系単語を返す (Viterbi ラティス構築用)。
    /// 戻り値は (一致した文字数, 単語)。件数は高々数千なので線形走査で足りる
    pub fn common_prefix_words(&self, suffix: &[char]) -> Vec<(usize, &UserWord)> {
        self.words
            .iter()
            .filter_map(|w| {
                let mut len = 0;
                for (i, rc) in w.reading.chars().enumerate() {
                    if suffix.get(i) != Some(&rc) {
                        return None;
                    }
                    len = i + 1;
                }
                (len > 0).then_some((len, w))
            })
            .collect()
    }

    /// 読みが prefix で始まる名詞系単語を記載順に limit 件まで返す (予測入力用)
    pub fn word_prefix(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        self.words
            .iter()
            .filter(|w| w.reading.starts_with(prefix))
            .take(limit)
            .map(|w| (w.reading.as_str(), w.surface.as_str()))
            .collect()
    }

    pub fn shortcut_count(&self) -> usize {
        self.shortcuts.len()
    }

    pub fn word_count(&self) -> usize {
        self.words.len()
    }
}

/// 既定のユーザ辞書パス。優先順: QUICKLIME_USER_DICT_FILE > %APPDATA%\QuicklIME\userdict.tsv
fn default_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("QUICKLIME_USER_DICT_FILE") {
        return Some(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("QuicklIME").join("userdict.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_functional() -> FunctionalIds {
        let data = "1850 名詞,サ変接続,*,*,*,*,*\n\
                    1851 名詞,一般,*,*,*,*,*\n\
                    1916 名詞,固有名詞,人名,一般,*,*,*\n\
                    1917 名詞,固有名詞,人名,姓,*,*,*\n";
        FunctionalIds::load_from(data.as_bytes()).unwrap()
    }

    fn sample() -> UserDict {
        let mut dict = UserDict::empty();
        let data = "# コメント行\n\
                    めーる\tmail@example.com\t短縮よみ\t自宅メール\n\
                    めーる\tsecond@example.jp\t短縮よみ\n\
                    じゅうしょ\t東京都千代田区\t短縮よみ\n\
                    かんべ\t神戸\t姓\n\
                    くいっくる\tQuicklIME\t固有名詞\n\
                    たべる\t食べる\t動詞\n\
                    ひんしなし\tじかに書いた行\n\
                    よみだけ\n";
        dict.load_from(data.as_bytes(), &sample_functional());
        dict
    }

    #[test]
    fn 短縮よみを読みで引ける() {
        let dict = sample();
        // 同じ読みの複数登録はファイル記載順
        assert_eq!(dict.lookup_shortcuts("めーる"), ["mail@example.com", "second@example.jp"]);
        assert_eq!(dict.lookup_shortcuts("じゅうしょ"), ["東京都千代田区"]);
        assert!(dict.lookup_shortcuts("ない").is_empty());
    }

    #[test]
    fn 品詞列なしは短縮よみで壊れた行は無視する() {
        let dict = sample();
        assert_eq!(dict.lookup_shortcuts("ひんしなし"), ["じかに書いた行"]);
        // めーる×2 + じゅうしょ + ひんしなし の4件
        assert_eq!(dict.shortcut_count(), 4);
    }

    #[test]
    fn 名詞系は文脈idを解決して単語になる() {
        let dict = sample();
        let words = dict.lookup_words("かんべ");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].surface, "神戸");
        assert_eq!(words[0].pos, "姓");
        assert_eq!(words[0].left_id, 1917); // 名詞,固有名詞,人名,姓
        assert_eq!(words[0].right_id, 1917);
        assert!(dict.lookup_shortcuts("かんべ").is_empty());
    }

    #[test]
    fn 品詞パスが引けなければ一般名詞のidで代用する() {
        let mut dict = UserDict::empty();
        // FunctionalIds が空 = id.def なし
        dict.load_from("かんべ\t神戸\t姓\n".as_bytes(), &FunctionalIds::empty());
        assert_eq!(dict.lookup_words("かんべ")[0].left_id, DEFAULT_NOUN_ID);
    }

    #[test]
    fn 未対応の品詞は無視する() {
        let dict = sample();
        assert!(dict.lookup_words("たべる").is_empty());
        // かんべ + くいっくる の2件
        assert_eq!(dict.word_count(), 2);
    }

    #[test]
    fn 短縮よみの前方一致は記載順で上限つき() {
        let dict = sample();
        assert_eq!(
            dict.shortcut_prefix("めー", 8),
            vec![("めーる", "mail@example.com"), ("めーる", "second@example.jp")]
        );
        assert_eq!(dict.shortcut_prefix("めー", 1).len(), 1);
        assert!(dict.shortcut_prefix("ない", 8).is_empty());
    }

    #[test]
    fn 名詞系の前方一致は記載順で上限つき() {
        let dict = sample();
        assert_eq!(dict.word_prefix("かん", 8), vec![("かんべ", "神戸")]);
        assert!(dict.word_prefix("ない", 8).is_empty());
    }

    #[test]
    fn 読み並びの先頭から一致する単語を引ける() {
        let dict = sample();
        let suffix: Vec<char> = "かんべさん".chars().collect();
        let hits = dict.common_prefix_words(&suffix);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 3); // 「かんべ」の3文字
        assert_eq!(hits[0].1.surface, "神戸");
        // 途中からは一致しない
        assert!(dict.common_prefix_words(&suffix[1..]).is_empty());
    }

    #[test]
    fn 登録がメモリへ即時反映される() {
        let mut dict = UserDict::empty();
        let functional = sample_functional();
        dict.add("てすと", "テスト表記", "名詞", &functional).unwrap();
        dict.add("めーる", "mail@example.com", "短縮よみ", &functional).unwrap();
        assert_eq!(dict.lookup_words("てすと")[0].surface, "テスト表記");
        assert_eq!(dict.lookup_words("てすと")[0].left_id, 1851);
        assert_eq!(dict.lookup_shortcuts("めーる"), ["mail@example.com"]);
    }

    #[test]
    fn 登録の検証エラー() {
        let mut dict = UserDict::empty();
        let functional = sample_functional();
        assert!(dict.add("", "表記", "名詞", &functional).is_err());
        assert!(dict.add("よみ", "", "名詞", &functional).is_err());
        assert!(dict.add("よみ", "表記", "動詞", &functional).is_err());
        // 重複 (短縮よみ・名詞系とも)
        dict.add("よみ", "表記", "名詞", &functional).unwrap();
        assert!(dict.add("よみ", "表記", "名詞", &functional).is_err());
        dict.add("よみ", "定型", "短縮よみ", &functional).unwrap();
        assert!(dict.add("よみ", "定型", "短縮よみ", &functional).is_err());
    }

    #[test]
    fn 登録がファイルへ追記され再読込で引ける() {
        let dir = std::env::temp_dir().join(format!("quicklime-userdict-test-{}", std::process::id()));
        let path = dir.join("userdict.tsv");
        let _ = std::fs::remove_file(&path);
        let functional = sample_functional();

        let mut dict =
            UserDict { shortcuts: Vec::new(), words: Vec::new(), path: Some(path.clone()) };
        dict.add("かんべ", "神戸", "姓", &functional).unwrap();
        dict.add("めーる", "mail@example.com", "短縮よみ", &functional).unwrap();

        // ファイルには TSV で追記されている
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "かんべ\t神戸\t姓\nめーる\tmail@example.com\t短縮よみ\n");

        // reload で読み直しても同じ内容
        dict.reload(&functional);
        assert_eq!(dict.lookup_words("かんべ")[0].surface, "神戸");
        assert_eq!(dict.lookup_shortcuts("めーる"), ["mail@example.com"]);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
