// config.tsv (エンジンと TSF 層が読む共通の設定ファイル) の読み書き

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use crate::apps::{DEFAULT_NEWLINE_APPS, ImeStart, format_ime_start_apps, parse_app_list, parse_ime_start_apps};
use crate::keys::{KEY_ITEMS, KeyAssign, STATES, default_key_assigns, format_key_list, parse_key_list};

/// LLM のバックエンド: (設定上の名前, 表示名)
pub const LLM_BACKENDS: [(&str, &str); 2] = [("cpu", "CPU"), ("vulkan", "GPU (Vulkan)")];
/// 候補の表示位置 (candidate_position の値, 表示名)
pub const CANDIDATE_POSITIONS: [(&str, &str); 2] = [("caret", "キャレットの下"), ("center", "画面中央")];
/// 入力中の読みを出す場所 (表示名, panel_buffer, panel_input)。先頭が既定。
/// panel_input の None は読み込んだ値のまま書く (input_buffer panel では panel_input が効かず、
/// 手で書いた値を保存しただけで変えないため)
pub const INPUT_BUFFERS: [(&str, bool, Option<bool>); 3] = [
    ("文書に下線付きで出す", false, Some(false)),
    ("文書と入力パネルの両方に出す", false, Some(true)),
    ("入力パネルだけに出す", true, None),
];
/// 組み込みのテーマ (panel_theme の値, 表示名)。TSF 層 (tsf/src/panel_theme.cpp) と合わせる
pub const BUILTIN_THEMES: [(&str, &str); 4] = [
    ("capsule", "カプセル (横一列)"),
    ("capsule-column", "カプセル (縦積み)"),
    ("capsule-blur", "カプセル (ぼかし)"),
    ("ghost", "ゴースト"),
];

/// 句読点の選択肢 (設定値そのまま表示する)
pub const PUNCT_ITEMS: [&str; 4] = ["、。", "，．", "、．", "，。"];

/// 予測の件数 (max_predictions) の選択肢
pub const MAX_PRED_ITEMS: [u32; 4] = [9, 18, 27, 36];
/// 予測を表示し始める文字数 (min_suggest_chars) の範囲
pub const MIN_SUGGEST_CHARS_RANGE: std::ops::RangeInclusive<u32> = 1..=5;
/// 入力パネルのフォントサイズ (candidate_font_size) の範囲
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<u32> = 10..=40;

/// max_predictions を 9 の倍数へ切り上げて [9, 36] に丸める (エンジンの config.rs と同じ規則)
pub fn round_max_predictions(n: u32) -> u32 {
    n.clamp(9, 36).div_ceil(9) * 9
}

/// candidate_font に書ける面名か (LOGFONT の面名は 32 要素 (終端込み) に収まる必要がある)
pub fn is_valid_font_name(name: &str) -> bool {
    !name.is_empty() && name.encode_utf16().count() < 32
}

/// 設定ファイルの内容 (エンジン向け + TSF 層向けの全キー)
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub learning: bool,
    pub suggest: bool,
    pub typo_correction: bool,
    pub max_predictions: u32,   // 9 / 18 / 27 / 36
    pub min_suggest_chars: u32, // 1-5
    pub space_full: bool,
    pub punctuation: String,
    pub digits_full: bool,
    pub modeless: bool,
    /// 入力中の読みを文書に書かずパネルだけに持つ (input_buffer の panel。それ以外は composition)
    pub panel_buffer: bool,
    /// 数字・電話番号・メールアドレス・URL などの入力欄 (InputScope で判定) ではキーを食べない
    pub input_scope_passthrough: bool,
    pub candidate_bar: bool,
    pub next_word: bool,
    /// 入力パネルのテーマ名 (組み込みのテーマか、テーマフォルダの <名前>.tsv)
    pub panel_theme: String,
    /// 候補の表示位置 ("caret" / "center")
    pub candidate_position: String,
    /// 入力中の読みを文書と入力パネルの両方に出す (input_buffer composition のときだけ効く)
    pub panel_input: bool,
    /// 切替時とフォーカス移動時に、キャレットの近くに入力モードを一時表示する
    pub mode_popup: bool,
    pub segment_ui: bool,
    pub llm: bool,
    /// LLM のバックエンド ("cpu" / "vulkan")
    pub llm_backend: String,
    pub candidate_font: String,
    pub candidate_font_size: u32, // 10-40
    pub keys: Vec<KeyAssign>,     // KEY_ITEMS の並び順
    /// 改行キーが働くアプリの exe 名
    pub newline_apps: Vec<String>,
    /// 起動時の IME をオン/オフにするアプリ (ime_start_apps。自動のアプリは含めない)
    pub ime_start_apps: Vec<(String, ImeStart)>,
    /// 学習を記録しないアプリの exe 名
    pub no_learning_apps: Vec<String>,
    /// 診断ログの出力先 (空なら出さない)
    pub debug_log: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            learning: true,
            suggest: true,
            typo_correction: true,
            max_predictions: 9,
            min_suggest_chars: 2,
            space_full: true,
            punctuation: "、。".to_string(),
            digits_full: false,
            modeless: false,
            panel_buffer: false,
            input_scope_passthrough: true,
            candidate_bar: true,
            next_word: true,
            panel_theme: BUILTIN_THEMES[0].0.to_string(),
            candidate_position: "caret".to_string(),
            panel_input: false,
            mode_popup: true,
            segment_ui: false,
            llm: false,
            llm_backend: "cpu".to_string(),
            candidate_font: "Yu Gothic UI".to_string(),
            candidate_font_size: 18,
            keys: default_key_assigns(),
            newline_apps: parse_app_list(DEFAULT_NEWLINE_APPS),
            ime_start_apps: Vec::new(),
            no_learning_apps: Vec::new(),
            debug_log: String::new(),
        }
    }
}

/// 設定ファイルのパス。優先順: QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv
pub fn config_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("QUICKLIME_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("config.tsv"))
}

/// テーマフォルダ。優先順: QUICKLIME_THEME_DIR > %APPDATA%\QuicklIME\themes (TSF 層と同じ)
pub fn theme_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("QUICKLIME_THEME_DIR") {
        return Ok(PathBuf::from(path));
    }
    let appdata =
        std::env::var("APPDATA").map_err(|_| "テーマのフォルダを特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("themes"))
}

/// テーマフォルダの *.tsv のファイル名 (拡張子なし)。フォルダが無ければ空
pub fn user_theme_names(dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let is_tsv = path.extension()?.to_str()?.eq_ignore_ascii_case("tsv");
            (is_tsv && path.is_file()).then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect()
}

/// 「入力パネルのテーマ」の選択肢 ((panel_theme の値, 表示名))。組み込みのテーマに続けて、ユーザの
/// テーマを名前順に並べる。組み込みと同名のファイル (Windows のファイル名と同じく大文字小文字を
/// 区別しない) は組み込みの項目だけにする。今の設定値が無ければ (ファイルを消したときなど)
/// 選び直さずに保存しても変わらないよう末尾に足す
pub fn theme_choices(user_names: Vec<String>, current: &str) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> =
        BUILTIN_THEMES.iter().map(|(name, label)| (name.to_string(), label.to_string())).collect();
    let mut users: Vec<String> = Vec::new();
    for name in user_names {
        let lower = name.to_lowercase();
        if name.is_empty()
            || BUILTIN_THEMES.iter().any(|(builtin, _)| builtin.to_lowercase() == lower)
            || users.iter().any(|user| user.to_lowercase() == lower)
        {
            continue;
        }
        users.push(name);
    }
    users.sort_by_key(|name| name.to_lowercase());
    choices.extend(users.into_iter().map(|name| (name.clone(), name)));
    if !choices.iter().any(|(name, _)| name == current) {
        choices.push((current.to_string(), current.to_string()));
    }
    choices
}

/// 設定値に合う「入力中の読み」の選択肢 (INPUT_BUFFERS の添字)
pub fn input_buffer_choice(panel_buffer: bool, panel_input: bool) -> usize {
    INPUT_BUFFERS
        .iter()
        .position(|&(_, buffer, input)| {
            buffer == panel_buffer && input.is_none_or(|input| input == panel_input)
        })
        .unwrap_or(0)
}

/// 「入力中の読み」の選択肢 (INPUT_BUFFERS の添字) を設定値に書く。panel_input を書かない選択肢では
/// config の値をそのまま残すので、config には読み込んだ panel_input を入れておく
pub fn apply_input_buffer_choice(config: &mut Config, index: usize) {
    let (_, buffer, input) = INPUT_BUFFERS[index];
    config.panel_buffer = buffer;
    if let Some(input) = input {
        config.panel_input = input;
    }
}

/// 診断ログの既定の出力先 (%APPDATA%\QuicklIME\debug.log)。QUICKLIME_CONFIG_FILE で
/// 設定ファイルを移していても同じ場所にする。APPDATA が無ければ空
pub fn default_debug_log_path() -> String {
    std::env::var("APPDATA")
        .map(|appdata| {
            PathBuf::from(appdata).join("QuicklIME").join("debug.log").to_string_lossy().into_owned()
        })
        .unwrap_or_default()
}

impl Config {
    /// 設定ファイルを読み込む。無い・読めない・不正な値は既定値のまま
    pub fn load() -> Self {
        let mut config = Config::default();
        let Ok(path) = config_path() else {
            return config;
        };
        let Ok(file) = std::fs::File::open(&path) else {
            return config;
        };
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                break;
            };
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };
            config.apply(key, value);
        }
        config
    }

    pub fn apply(&mut self, key: &str, value: &str) {
        let parse_bool = |out: &mut bool| match value {
            "0" => *out = false,
            "1" => *out = true,
            _ => {}
        };
        match key {
            "learning" => parse_bool(&mut self.learning),
            "suggest" => parse_bool(&mut self.suggest),
            "typo_correction" => parse_bool(&mut self.typo_correction),
            "max_predictions" => {
                if let Ok(n) = value.parse::<u32>() {
                    self.max_predictions = round_max_predictions(n);
                }
            }
            "min_suggest_chars" => {
                if let Ok(n) = value.parse::<u32>() {
                    self.min_suggest_chars = n.clamp(1, 5);
                }
            }
            "space" => match value {
                "full" => self.space_full = true,
                "half" => self.space_full = false,
                _ => {}
            },
            "digits" => match value {
                "full" => self.digits_full = true,
                "half" => self.digits_full = false,
                _ => {}
            },
            "modeless" => parse_bool(&mut self.modeless),
            // TSF 層と同じく panel 以外 (document などの不正な値を含む) は composition
            "input_buffer" => self.panel_buffer = value == "panel",
            "input_scope_passthrough" => parse_bool(&mut self.input_scope_passthrough),
            "candidate_bar" => parse_bool(&mut self.candidate_bar),
            "next_word" => parse_bool(&mut self.next_word),
            "panel_theme" => {
                if !value.is_empty() {
                    self.panel_theme = value.to_string();
                }
            }
            "candidate_position" => {
                if CANDIDATE_POSITIONS.iter().any(|(name, _)| *name == value) {
                    self.candidate_position = value.to_string();
                }
            }
            "panel_input" => parse_bool(&mut self.panel_input),
            "mode_popup" => parse_bool(&mut self.mode_popup),
            "segment_ui" => parse_bool(&mut self.segment_ui),
            "llm" => parse_bool(&mut self.llm),
            "llm_backend" => {
                if LLM_BACKENDS.iter().any(|(name, _)| *name == value) {
                    self.llm_backend = value.to_string();
                }
            }
            "punctuation" => {
                if PUNCT_ITEMS.contains(&value) {
                    self.punctuation = value.to_string();
                }
            }
            "candidate_font" => {
                if is_valid_font_name(value) {
                    self.candidate_font = value.to_string();
                }
            }
            "candidate_font_size" => {
                if let Ok(n) = value.parse::<u32>() {
                    self.candidate_font_size = n.clamp(10, 40);
                }
            }
            // 空の値は「対象なし」としてそのまま持つ
            "newline_apps" => self.newline_apps = parse_app_list(value),
            "ime_start_apps" => self.ime_start_apps = parse_ime_start_apps(value),
            "no_learning_apps" => self.no_learning_apps = parse_app_list(value),
            "debug_log" => self.debug_log = value.to_string(),
            _ => self.apply_key(key, value),
        }
    }

    /// key.<機能>[@<状態>] の行を反映する。上書きはその機能が働く状態にだけ書ける。
    /// TSF 層のパース (tsf/src/config.cpp) と同じ規則
    fn apply_key(&mut self, key: &str, value: &str) {
        let (name, state) = match key.split_once('@') {
            Some((name, state)) => (name, Some(state)),
            None => (key, None),
        };
        let Some(index) = KEY_ITEMS.iter().position(|item| item.0 == name) else {
            return;
        };
        let state = match state {
            None => None,
            Some(state) => match STATES.iter().position(|(s, _)| *s == state) {
                Some(s) if KEY_ITEMS[index].2 & (1 << s) != 0 => Some(s),
                _ => return,
            },
        };
        let Some(keys) = parse_key_list(value) else {
            return;
        };
        match state {
            None => self.keys[index].keys = keys,
            Some(s) => self.keys[index].overrides[s] = Some(keys),
        }
    }

    /// 全キーをコメント付きで書き出し、エンジンへ RELOADCONFIG を送る
    pub fn save(&self) -> Result<(), String> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, self.file_text().as_bytes())
            .map_err(|e| format!("設定ファイルへ書き込めません ({e})"))?;

        // エンジンに再読込を伝える。未起動なら何もしない
        // (ファイルが正なので、次のエンジン起動時に読み込まれる)
        let _ = send_reloadconfig();
        Ok(())
    }

    /// config.tsv に書く内容
    pub fn file_text(&self) -> String {
        let mut text = String::new();
        text.push_str("# QuicklIME 設定 (quicklime-config.exe が生成)\n");
        text.push_str("# 形式: キー<TAB>値。# 始まりはコメント\n");
        text.push_str("\n# 変換エンジン\n");
        text.push_str(&format!("learning\t{}\n", self.learning as u32));
        text.push_str(&format!("suggest\t{}\n", self.suggest as u32));
        text.push_str(&format!("typo_correction\t{}\n", self.typo_correction as u32));
        text.push_str(&format!("max_predictions\t{}\n", self.max_predictions));
        text.push_str(&format!("min_suggest_chars\t{}\n", self.min_suggest_chars));
        text.push_str(&format!("llm\t{}\n", self.llm as u32));
        text.push_str(&format!("llm_backend\t{}\n", self.llm_backend));
        text.push_str("\n# 入力挙動\n");
        text.push_str(&format!("space\t{}\n", if self.space_full { "full" } else { "half" }));
        text.push_str(&format!("punctuation\t{}\n", self.punctuation));
        text.push_str(&format!("digits\t{}\n", if self.digits_full { "full" } else { "half" }));
        text.push_str(&format!("modeless\t{}\n", self.modeless as u32));
        text.push_str(&format!(
            "input_buffer\t{}\n",
            if self.panel_buffer { "panel" } else { "composition" }
        ));
        text.push_str(&format!("segment_ui\t{}\n", self.segment_ui as u32));
        text.push_str(&format!(
            "input_scope_passthrough\t{}\n",
            self.input_scope_passthrough as u32
        ));
        text.push_str("\n# 候補ウィンドウ\n");
        text.push_str(&format!("candidate_bar\t{}\n", self.candidate_bar as u32));
        text.push_str(&format!("next_word\t{}\n", self.next_word as u32));
        text.push_str(&format!("panel_theme\t{}\n", self.panel_theme));
        text.push_str(&format!("candidate_position\t{}\n", self.candidate_position));
        text.push_str(&format!("panel_input\t{}\n", self.panel_input as u32));
        text.push_str(&format!("mode_popup\t{}\n", self.mode_popup as u32));
        text.push_str(&format!("candidate_font\t{}\n", self.candidate_font));
        text.push_str(&format!("candidate_font_size\t{}\n", self.candidate_font_size));
        text.push_str("\n# キー割当 (key.<機能>@<状態> は状態別の上書き)\n");
        for (i, (name, _, _, _)) in KEY_ITEMS.iter().enumerate() {
            text.push_str(&format!("{}\t{}\n", name, format_key_list(&self.keys[i].keys)));
            for (s, (state, _)) in STATES.iter().enumerate() {
                if let Some(keys) = &self.keys[i].overrides[s] {
                    text.push_str(&format!("{name}@{state}\t{}\n", format_key_list(keys)));
                }
            }
        }
        text.push_str("\n# 改行キー (key.insert_newline) が働くアプリの exe 名 (カンマ区切り)\n");
        text.push_str(&format!("newline_apps\t{}\n", self.newline_apps.join(",")));
        text.push_str("\n# 起動時の IME の状態 (exe名:on|off のカンマ区切り。無いアプリは自動)\n");
        text.push_str(&format!(
            "ime_start_apps\t{}\n",
            format_ime_start_apps(&self.ime_start_apps)
        ));
        text.push_str("\n# 学習を記録しないアプリの exe 名 (カンマ区切り)\n");
        text.push_str(&format!("no_learning_apps\t{}\n", self.no_learning_apps.join(",")));
        text.push_str("\n# 開発用 (debug_log は診断ログの出力先。行が無ければ出さない)\n");
        if !self.debug_log.is_empty() {
            text.push_str(&format!("debug_log\t{}\n", self.debug_log));
        }
        text
    }
}

/// named pipe でエンジンに RELOADCONFIG を送る
pub fn send_reloadconfig() -> std::io::Result<()> {
    let name =
        std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| "quicklime-engine".to_string());
    let mut pipe =
        std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\{name}"))?;
    writeln!(pipe, "RELOADCONFIG")?;
    let mut response = String::new();
    BufReader::new(pipe).read_line(&mut response)?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{AppSetting, default_app_settings, merge_app_settings, split_app_settings};
    use crate::keys::KeyCombo;

    fn key(text: &str) -> KeyCombo {
        KeyCombo::parse(text).unwrap()
    }

    #[test]
    fn reads_llm_settings() {
        let mut config = Config::default();
        config.apply("llm", "1");
        config.apply("llm_backend", "vulkan");
        assert!(config.llm);
        assert_eq!(config.llm_backend, "vulkan");
        // 未対応のバックエンドは読み飛ばす
        config.apply("llm_backend", "cuda");
        assert_eq!(config.llm_backend, "vulkan");
    }

    #[test]
    fn reads_newline_settings() {
        // 行が無ければ既定のリスト
        let mut config = Config::default();
        assert_eq!(
            config.newline_apps,
            ["Discord.exe", "slack.exe", "ms-teams.exe", "Teams.exe", "LINE.exe"]
        );
        let index = KEY_ITEMS.iter().position(|item| item.0 == "key.insert_newline").unwrap();
        assert_eq!(config.keys[index].keys, vec![key("Ctrl+J")]);
        // 全状態で働くので、どの状態でも上書きできる
        for (s, (state, _)) in STATES.iter().enumerate() {
            config.apply(&format!("key.insert_newline@{state}"), "Ctrl+Enter");
            assert_eq!(config.keys[index].overrides[s], Some(vec![key("Ctrl+Enter")]));
        }
        config.apply("newline_apps", "Discord.exe, notepad.exe");
        assert_eq!(config.newline_apps, ["Discord.exe", "notepad.exe"]);
        // 空の値は対象なしとして既定に戻さない
        config.apply("newline_apps", "");
        assert!(config.newline_apps.is_empty());
    }

    #[test]
    fn reads_next_word_settings() {
        let mut config = Config::default();
        assert!(config.next_word);
        config.apply("next_word", "0");
        assert!(!config.next_word);
        let next = KEY_ITEMS.iter().position(|item| item.0 == "key.next_word_next").unwrap();
        let prev = KEY_ITEMS.iter().position(|item| item.0 == "key.next_word_prev").unwrap();
        assert_eq!(config.keys[next].keys, vec![key("Ctrl+Down")]);
        assert_eq!(config.keys[prev].keys, vec![key("Ctrl+Up")]);
        // 入力なしとサジェスト選択中でだけ働く
        config.apply("key.next_word_next@suggest", "Ctrl+N");
        config.apply("key.next_word_next@run", "Ctrl+N");
        assert_eq!(config.keys[next].overrides[3], Some(vec![key("Ctrl+N")]));
        assert_eq!(config.keys[next].overrides[1], None);
    }

    #[test]
    fn reads_panel_settings() {
        let mut config = Config::default();
        assert_eq!(config.candidate_position, "caret");
        assert!(!config.panel_input);
        config.apply("candidate_position", "center");
        config.apply("panel_input", "1");
        assert_eq!(config.candidate_position, "center");
        assert!(config.panel_input);
        // 不正な値は読み飛ばす
        config.apply("candidate_position", "top");
        config.apply("panel_input", "2");
        assert_eq!(config.candidate_position, "center");
        assert!(config.panel_input);
    }

    #[test]
    fn reads_mode_popup() {
        let mut config = Config::default();
        assert!(config.mode_popup);
        assert!(config.file_text().contains("\nmode_popup\t1\n"));
        config.apply("mode_popup", "0");
        assert!(!config.mode_popup);
        assert!(config.file_text().contains("\nmode_popup\t0\n"));
        // 不正な値は読み飛ばす
        config.apply("mode_popup", "2");
        assert!(!config.mode_popup);
        config.apply("mode_popup", "1");
        assert!(config.mode_popup);
    }

    #[test]
    fn reads_input_buffer() {
        let mut config = Config::default();
        assert!(!config.panel_buffer);
        config.apply("input_buffer", "panel");
        assert!(config.panel_buffer);
        config.apply("input_buffer", "composition");
        assert!(!config.panel_buffer);
        // 不正な値は既定の composition にする (document も同じ)
        config.apply("input_buffer", "panel");
        config.apply("input_buffer", "document");
        assert!(!config.panel_buffer);
        config.apply("input_buffer", "panel");
        config.apply("input_buffer", "1");
        assert!(!config.panel_buffer);
        // 書くときは composition / panel
        assert!(Config::default().file_text().contains("
input_buffer	composition
"));
        config.apply("input_buffer", "panel");
        assert!(config.file_text().contains("
input_buffer	panel
"));
    }

    #[test]
    fn input_buffer_choices() {
        let load = |buffer: &str, input: &str| {
            let mut config = Config::default();
            config.apply("input_buffer", buffer);
            config.apply("panel_input", input);
            config
        };
        // 読み込み
        let config = load("composition", "0");
        assert_eq!(input_buffer_choice(config.panel_buffer, config.panel_input), 0);
        let config = load("composition", "1");
        assert_eq!(input_buffer_choice(config.panel_buffer, config.panel_input), 1);
        let config = load("panel", "0");
        assert_eq!(input_buffer_choice(config.panel_buffer, config.panel_input), 2);
        let config = load("panel", "1");
        assert_eq!(input_buffer_choice(config.panel_buffer, config.panel_input), 2);
        // 保存
        let mut config = load("panel", "1");
        apply_input_buffer_choice(&mut config, 0);
        assert!(config.file_text().contains("\ninput_buffer\tcomposition\n"));
        assert!(config.file_text().contains("\npanel_input\t0\n"));
        apply_input_buffer_choice(&mut config, 1);
        assert!(config.file_text().contains("\ninput_buffer\tcomposition\n"));
        assert!(config.file_text().contains("\npanel_input\t1\n"));
        // 入力パネルだけに出すときは、読み込んだ panel_input をそのまま書く
        for input in ["0", "1"] {
            let mut config = load("composition", input);
            apply_input_buffer_choice(&mut config, 2);
            assert!(config.file_text().contains("\ninput_buffer\tpanel\n"));
            assert!(config.file_text().contains(&format!("\npanel_input\t{input}\n")));
        }
    }

    #[test]
    fn reads_input_scope_passthrough() {
        let mut config = Config::default();
        assert!(config.input_scope_passthrough);
        assert!(config.file_text().contains("\ninput_scope_passthrough\t1\n"));
        config.apply("input_scope_passthrough", "0");
        assert!(!config.input_scope_passthrough);
        assert!(config.file_text().contains("\ninput_scope_passthrough\t0\n"));
        config.apply("input_scope_passthrough", "1");
        assert!(config.input_scope_passthrough);
    }

    #[test]
    fn reads_panel_theme() {
        let mut config = Config::default();
        assert_eq!(config.panel_theme, "capsule");
        config.apply("panel_theme", "mine");
        assert_eq!(config.panel_theme, "mine");
        // 空の値は読み飛ばす
        config.apply("panel_theme", "");
        assert_eq!(config.panel_theme, "mine");
    }

    #[test]
    fn makes_theme_choices() {
        let names = |choices: &[(String, String)]| -> Vec<String> {
            choices.iter().map(|(name, _)| name.clone()).collect()
        };
        // 組み込みに続けてユーザのテーマを名前順に。組み込みと同名 (大文字小文字は問わない)・
        // 重複は除く
        let choices = theme_choices(
            vec![
                "mine".to_string(),
                "Capsule".to_string(),
                "ghost".to_string(),
                "Alpha".to_string(),
                "MINE".to_string(),
            ],
            "capsule",
        );
        assert_eq!(
            names(&choices),
            ["capsule", "capsule-column", "capsule-blur", "ghost", "Alpha", "mine"]
        );
        assert_eq!(choices[0].1, "カプセル (横一列)");
        assert_eq!(choices[2].1, "カプセル (ぼかし)");
        assert_eq!(choices[3].1, "ゴースト");
        assert_eq!(choices[4].1, "Alpha");
        // 一覧に無い設定値は末尾に足す
        let choices = theme_choices(Vec::new(), "gone");
        assert_eq!(names(&choices), ["capsule", "capsule-column", "capsule-blur", "ghost", "gone"]);
    }

    #[test]
    fn lists_user_theme_files() {
        let dir = std::env::temp_dir().join(format!("quicklime-theme-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub.tsv")).unwrap();
        for file in ["mine.tsv", "Upper.TSV", "note.txt"] {
            std::fs::write(dir.join(file), "accent\t#E0457B\n").unwrap();
        }
        let mut names = user_theme_names(&dir);
        names.sort();
        assert_eq!(names, ["Upper", "mine"]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(user_theme_names(&dir).is_empty());
    }

    #[test]
    fn merges_app_settings_in_newline_order() {
        let mut config = Config::default();
        config.apply("newline_apps", "Discord.exe");
        config.apply("ime_start_apps", "Notepad.exe:on,discord.exe:off");
        config.apply("no_learning_apps", "notepad.exe, WindowsTerminal.exe");
        let apps = merge_app_settings(
            &config.newline_apps,
            &config.ime_start_apps,
            &config.no_learning_apps,
        );
        let app = |exe: &str, newline: bool, ime_start: ImeStart, no_learning: bool| AppSetting {
            exe: exe.to_string(),
            newline,
            ime_start,
            no_learning,
        };
        assert_eq!(
            apps,
            [
                app("Discord.exe", true, ImeStart::Off, false),
                app("Notepad.exe", false, ImeStart::On, true),
                app("WindowsTerminal.exe", false, ImeStart::Auto, true),
            ]
        );
        // newline_apps の行が無ければ既定のアプリが「改行キーあり・自動・学習する」で並ぶ
        let config = Config::default();
        let apps = merge_app_settings(
            &config.newline_apps,
            &config.ime_start_apps,
            &config.no_learning_apps,
        );
        assert_eq!(apps, default_app_settings());
        assert_eq!(apps.len(), 5);
        assert!(apps
            .iter()
            .all(|app| app.newline && app.ime_start == ImeStart::Auto && !app.no_learning));
    }

    #[test]
    fn splits_app_settings_for_saving() {
        let app = |exe: &str, newline: bool, ime_start: ImeStart, no_learning: bool| AppSetting {
            exe: exe.to_string(),
            newline,
            ime_start,
            no_learning,
        };
        let mut apps = vec![
            app("Discord.exe", true, ImeStart::Off, false),
            app("Notepad.exe", true, ImeStart::Off, true),
            app("WindowsTerminal.exe", false, ImeStart::Auto, true),
            // 既定のままのアプリはどのキーにも書かない
            app("Code.exe", false, ImeStart::Auto, false),
        ];
        let mut config = Config::default();
        (config.newline_apps, config.ime_start_apps, config.no_learning_apps) =
            split_app_settings(&apps);
        let text = config.file_text();
        assert!(text.contains("\nnewline_apps\tDiscord.exe,Notepad.exe\n"));
        assert!(text.contains("\nime_start_apps\tDiscord.exe:off,Notepad.exe:off\n"));
        assert!(text.contains("\nno_learning_apps\tNotepad.exe,WindowsTerminal.exe\n"));
        // 学習しないだけのアプリも、読み直すと一覧に残る
        let mut reloaded = Config::default();
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            if let Some((key, value)) = line.split_once('\t') {
                reloaded.apply(key, value);
            }
        }
        assert_eq!(
            merge_app_settings(
                &reloaded.newline_apps,
                &reloaded.ime_start_apps,
                &reloaded.no_learning_apps,
            ),
            apps[..3]
        );
        // 削除したアプリはどこからも消え、空になれば空の値で書く
        apps.retain(|app| app.exe == "Code.exe");
        (config.newline_apps, config.ime_start_apps, config.no_learning_apps) =
            split_app_settings(&apps);
        let text = config.file_text();
        assert!(text.contains("\nnewline_apps\t\n"));
        assert!(text.contains("\nime_start_apps\t\n"));
        assert!(text.contains("\nno_learning_apps\t\n"));
        // 書いた内容を読み直すと同じ一覧になる (既定のままのアプリは消える)
        let mut reloaded = Config::default();
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            if let Some((key, value)) = line.split_once('\t') {
                reloaded.apply(key, value);
            }
        }
        assert!(merge_app_settings(
            &reloaded.newline_apps,
            &reloaded.ime_start_apps,
            &reloaded.no_learning_apps,
        )
        .is_empty());
    }

    #[test]
    fn applies_overrides_only_to_working_states() {
        let mut config = Config::default();
        config.apply("key.next_candidate@candidate", "none");
        config.apply("key.commit_run@candidate", "Ctrl+Enter");
        config.apply("key.undo_commit@run", "F1");
        config.apply("key.to_katakana@bogus", "F1");
        config.apply("key.to_katakana", "Enter");
        config.apply("key.to_hiragana@idle", "Ctrl+K");
        config.apply("key.convert_symbol@idle", "Ctrl+J");
        config.apply("key.convert", "Convert,Ctrl+Space");
        assert_eq!(config.keys[1].overrides[2], Some(Vec::new()));
        assert_eq!(config.keys[3].overrides[2], Some(vec![key("Ctrl+Enter")]));
        assert_eq!(config.keys[11].overrides[1], None);
        assert_eq!(config.keys[7], KeyAssign { keys: vec![key("F7")], overrides: Default::default() });
        // F6〜F10 は入力なしでも働くので上書きでき、F4・F5 は入力なしでは働かない
        assert_eq!(config.keys[6].overrides[0], Some(vec![key("Ctrl+K")]));
        assert_eq!(config.keys[4].overrides[0], None);
        assert_eq!(config.keys[0].keys, vec![key("Convert"), key("Ctrl+Space")]);
    }
}
