// QuicklIME 設定ツール
//
// config.tsv (エンジンと TSF 層が読む共通の設定ファイル) を編集する
// Win32 ダイアログ。保存時にエンジンへ RELOADCONFIG を送って即時反映する
// (エンジン未起動時はファイルへの書き込みのみ。TSF 層は次のフォーカス
// 切替時に更新時刻の変化を検知して自動反映する)。
//
// 使い方: quicklime-config.exe (引数なし。TSF 層が Ctrl+F12 などで起動する)

#![windows_subsystem = "windows"]

use std::cell::{Cell, RefCell};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, CreateFontW, EnumFontFamiliesExW, GetDC, HFONT, LOGFONTW, ReleaseDC,
    TEXTMETRICW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, ICC_LISTVIEW_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_STATE, LVIF_TEXT, LVIR_BOUNDS,
    LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETEALLITEMS, LVM_ENSUREVISIBLE,
    LVM_GETITEMRECT, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_INSERTITEMW,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMSTATE, LVM_SETITEMTEXTW, LVNI_SELECTED,
    LVS_EX_FULLROWSELECT, LVS_NOSORTHEADER, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL,
    NM_DBLCLK, NMHDR, NMITEMACTIVATE, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_GETCURSEL,
    TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_ESCAPE,
    VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[path = "../edit_keys.rs"]
mod edit_keys;

#[link(name = "imm32")]
unsafe extern "system" {
    // キー取り込みダイアログで IME を無効にする (IME が打鍵を食べると WM_KEYDOWN で
    // 元のキーが届かない)。windows-sys の Win32_UI_Input_Ime を有効にせずに使うため自前で宣言する
    fn ImmAssociateContextEx(hwnd: HWND, himc: isize, flags: u32) -> i32;
}

// コントロールID
const ID_CHECK_LEARNING: i32 = 100;
const ID_CHECK_SUGGEST: i32 = 101;
const ID_CHECK_TYPO: i32 = 102;
const ID_COMBO_MAX_PRED: i32 = 103;
const ID_COMBO_MIN_CHARS: i32 = 104;
const ID_COMBO_SPACE: i32 = 105;
const ID_COMBO_PUNCT: i32 = 106;
const ID_COMBO_DIGITS: i32 = 107;
const ID_COMBO_FONT: i32 = 108;
const ID_COMBO_FONT_SIZE: i32 = 109;
const ID_CHECK_MODELESS: i32 = 112;
const ID_CHECK_CANDIDATE_BAR: i32 = 113;
const ID_CHECK_SEGMENT_UI: i32 = 114;
const ID_CHECK_LLM: i32 = 115;
const ID_COMBO_LLM_BACKEND: i32 = 116;
const ID_CHECK_NEXT_WORD: i32 = 117;
const ID_LIST_KEYS: i32 = 120;
const ID_BUTTON_KEY_EDIT: i32 = 121;
const ID_BUTTON_KEY_DEFAULT: i32 = 122;
const ID_LIST_APPS: i32 = 123;
const ID_BUTTON_APP_RUNNING: i32 = 124;
const ID_BUTTON_APP_FILE: i32 = 125;
const ID_BUTTON_APP_REMOVE: i32 = 126;
const ID_BUTTON_APP_DEFAULT: i32 = 127;
const ID_CHECK_DEBUG_LOG: i32 = 128;
const ID_EDIT_DEBUG_LOG: i32 = 129;
const ID_TAB: i32 = 130;
const ID_BUTTON_SAVE: i32 = 140;
const ID_BUTTON_CANCEL: i32 = 141;
// キー割当の編集ダイアログ。区画 k (0 = 基本の割当、1〜4 = STATES[k - 1] の上書き) ごとに +k*10
const ID_EDIT_LIST_BASE: i32 = 200;
const ID_EDIT_ADD_BASE: i32 = 201;
const ID_EDIT_REMOVE_BASE: i32 = 202;
const ID_EDIT_OVERRIDE_BASE: i32 = 203;
// キー取り込みダイアログ
const ID_CAPTURE_MESSAGE: i32 = 300;
// 実行中のアプリから選ぶダイアログ
const ID_PICK_LIST: i32 = 400;

/// メイン画面のタブ (並びが TAB_PAGES の添字)
const TAB_NAMES: [&str; 6] = ["変換", "入力", "表示", "キー割当", "アプリ", "開発用"];

/// LLM のバックエンド: (設定上の名前, 表示名)
const LLM_BACKENDS: [(&str, &str); 2] = [("cpu", "CPU"), ("vulkan", "GPU (Vulkan)")];

/// キー割当の照合に使う入力状態: (設定上の名前, 表示名)。
/// 並びは TSF 層 (tsf/src/config.h の KeyState) と合わせる
const STATES: [(&str, &str); 4] = [
    ("idle", "入力なし"),
    ("run", "run 中"),
    ("candidate", "候補選択中"),
    ("suggest", "サジェスト選択中"),
];
const IN_IDLE: u8 = 1 << 0;
const IN_RUN: u8 = 1 << 1;
const IN_CANDIDATE: u8 = 1 << 2;
const IN_SUGGEST: u8 = 1 << 3;
const IN_INPUT: u8 = IN_RUN | IN_CANDIDATE | IN_SUGGEST;

/// キー割当の機能一覧: (設定キー名, 表示名, 働く状態, 既定の割当)。
/// 並び・働く状態・既定は TSF 層 (tsf/src/config.h の KeyFunc) と合わせる
const KEY_ITEMS: [(&str, &str, u8, &str); 17] = [
    ("key.convert", "変換", IN_IDLE | IN_INPUT, "Convert"),
    ("key.next_candidate", "次候補", IN_CANDIDATE, "Space"),
    ("key.prev_candidate", "前候補", IN_CANDIDATE, "Shift+Space"),
    ("key.commit_run", "確定", IN_INPUT, "NonConvert"),
    ("key.convert_symbol", "記号・日付変換", IN_INPUT, "F4"),
    ("key.convert_user", "ユーザ語変換", IN_INPUT, "F5"),
    ("key.to_hiragana", "ひらがな変換", IN_IDLE | IN_INPUT, "F6"),
    ("key.to_katakana", "カタカナ変換", IN_IDLE | IN_INPUT, "F7"),
    ("key.to_half_katakana", "半角カタカナ変換", IN_IDLE | IN_INPUT, "F8"),
    ("key.to_full_ascii", "全角英字変換", IN_IDLE | IN_INPUT, "F9"),
    ("key.to_half_ascii", "半角英字変換", IN_IDLE | IN_INPUT, "F10"),
    ("key.undo_commit", "確定アンドゥ", IN_IDLE, "Ctrl+Backspace"),
    ("key.register_word", "単語登録", IN_IDLE, "Ctrl+F7"),
    ("key.open_config", "設定を開く", IN_IDLE, "Ctrl+F12"),
    ("key.insert_newline", "改行", IN_IDLE | IN_INPUT, "Ctrl+J"),
    ("key.next_word_next", "次の語の予測: 次", IN_IDLE | IN_SUGGEST, "Ctrl+Down"),
    ("key.next_word_prev", "次の語の予測: 前", IN_IDLE | IN_SUGGEST, "Ctrl+Up"),
];

/// 改行キーが働くアプリの既定 (設定 newline_apps の行が無いとき。TSF 層と合わせる)
const DEFAULT_NEWLINE_APPS: &str = "Discord.exe,slack.exe,ms-teams.exe,Teams.exe,LINE.exe";

/// 英字・数字・F1〜F24 以外の名前を持つキー: (表記, 仮想キーコード)
const NAMED_KEYS: [(&str, u16); 18] = [
    ("Space", 0x20),
    ("Enter", 0x0D),
    ("Esc", 0x1B),
    ("Tab", 0x09),
    ("Backspace", 0x08),
    ("Delete", 0x2E),
    ("Insert", 0x2D),
    ("Home", 0x24),
    ("End", 0x23),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("Up", 0x26),
    ("Down", 0x28),
    ("Left", 0x25),
    ("Right", 0x27),
    ("Convert", 0x1C),
    ("NonConvert", 0x1D),
    ("Kana", 0x15),
];

/// 句読点の選択肢 (設定値そのまま表示する)
const PUNCT_ITEMS: [&str; 4] = ["、。", "，．", "、．", "，。"];

/// 修飾キーの組と仮想キー
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct KeyCombo {
    ctrl: bool,
    alt: bool,
    shift: bool,
    vk: u16,
}

/// 1機能のキー割当。overrides[s] が Some の状態では keys の代わりにそれを使う
/// (空なら、その状態では割当なし)
#[derive(Clone, PartialEq, Debug, Default)]
struct KeyAssign {
    keys: Vec<KeyCombo>,
    overrides: [Option<Vec<KeyCombo>>; 4],
}

/// キー名を仮想キーコードにする
fn parse_key_name(name: &str) -> Option<u16> {
    let bytes = name.as_bytes();
    if bytes.len() == 1 && (bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return Some(bytes[0] as u16);
    }
    if let Some(number) = name.strip_prefix('F') {
        if (1..=2).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_digit()) {
            let n: u16 = number.parse().ok()?;
            return (1..=24).contains(&n).then_some(0x70 + n - 1);
        }
    }
    if let Some(&(_, vk)) = NAMED_KEYS.iter().find(|(key, _)| *key == name) {
        return Some(vk);
    }
    let hex = name.strip_prefix("VK_")?;
    if hex.len() != 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let vk = u16::from_str_radix(hex, 16).ok()?;
    (vk != 0).then_some(vk)
}

/// 仮想キーコードの表記 (名前の無いキーは VK_xx)
fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from(vk as u8).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => NAMED_KEYS
            .iter()
            .find(|(_, code)| *code == vk)
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| format!("VK_{vk:02X}")),
    }
}

/// コア操作のキー (Enter・Esc・Tab・Backspace・Delete・矢印・Home・End・PageUp・PageDown)
fn is_core_vk(vk: u16) -> bool {
    matches!(vk, 0x0D | 0x1B | 0x09 | 0x08 | 0x2E | 0x25..=0x28 | 0x24 | 0x23 | 0x21 | 0x22)
}

/// 印字キー (英字・数字・記号・テンキーの数字と演算子)
fn is_printable_vk(vk: u16) -> bool {
    matches!(vk, 0x41..=0x5A | 0x30..=0x39 | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 | 0x60..=0x6B | 0x6D..=0x6F)
}

/// 修飾キー単体 (Shift・Ctrl・Alt・Win)
fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

impl KeyCombo {
    /// 1つのキーの表記 ("Ctrl+Shift+F7" など。修飾キーの順序は問わない) を読む
    fn parse(text: &str) -> Option<KeyCombo> {
        let mut parts: Vec<&str> = text.split('+').collect();
        let name = parts.pop()?;
        let mut combo = KeyCombo { ctrl: false, alt: false, shift: false, vk: 0 };
        for modifier in parts {
            let flag = match modifier {
                "Ctrl" => &mut combo.ctrl,
                "Alt" => &mut combo.alt,
                "Shift" => &mut combo.shift,
                _ => return None,
            };
            if *flag {
                return None;
            }
            *flag = true;
        }
        combo.vk = parse_key_name(name)?;
        Some(combo)
    }

    /// 表記 (修飾キーは Ctrl+・Alt+・Shift+ の順)
    fn notation(&self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("Ctrl+");
        }
        if self.alt {
            text.push_str("Alt+");
        }
        if self.shift {
            text.push_str("Shift+");
        }
        text.push_str(&key_name(self.vk));
        text
    }

    /// 機能に割り当てられないキーなら理由を返す。TSF 層 (tsf/src/config.cpp の
    /// IsAssignable) と同じ規則
    fn unassignable_reason(&self) -> Option<&'static str> {
        if is_modifier_vk(self.vk) {
            return Some("修飾キーだけでは割り当てられません");
        }
        if matches!(self.vk, 0x19 | 0xF3 | 0xF4 | 0x16 | 0x1A) {
            return Some("IME の切替キーは割り当てられません");
        }
        // Alt 併用の打鍵は IME の key event sink に届かないアプリがある (メモ帳で確認)
        if self.alt {
            return Some("Alt と組み合わせたキーは割り当てられません");
        }
        if !self.ctrl {
            if is_core_vk(self.vk) {
                return Some(
                    "Enter・Esc・Tab・Backspace・Delete・矢印・Home・End・PageUp・PageDown は、\
                     Ctrl と組み合わせたときだけ割り当てられます",
                );
            }
            if is_printable_vk(self.vk) {
                return Some("英字・数字・記号・テンキーは、Ctrl と組み合わせたときだけ割り当てられます");
            }
        }
        if self.ctrl && !self.shift && (self.vk == 0x4D || self.vk == 0x48) {
            return Some("Ctrl+M・Ctrl+H は Enter・Backspace として働くため割り当てられません");
        }
        None
    }
}

/// キー割当の値 ("<キー>[,<キー>...]" または "none") を読む。読めない・対象外のキーは捨て、
/// 1つも残らなければ None (既定のまま)。"none" は空の一覧
fn parse_key_list(value: &str) -> Option<Vec<KeyCombo>> {
    if value == "none" {
        return Some(Vec::new());
    }
    let mut keys = Vec::new();
    for item in value.split(',') {
        if let Some(key) = KeyCombo::parse(item.trim_matches(' ')) {
            if key.unassignable_reason().is_none() && !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    (!keys.is_empty()).then_some(keys)
}

/// 設定ファイルに書くキー割当の値
fn format_key_list(keys: &[KeyCombo]) -> String {
    if keys.is_empty() {
        return "none".to_string();
    }
    keys.iter().map(KeyCombo::notation).collect::<Vec<_>>().join(",")
}

/// 一覧・編集ダイアログに出すキー割当
fn display_key_list(keys: &[KeyCombo]) -> String {
    if keys.is_empty() {
        return "(なし)".to_string();
    }
    keys.iter().map(KeyCombo::notation).collect::<Vec<_>>().join(", ")
}

fn default_key_assigns() -> Vec<KeyAssign> {
    KEY_ITEMS
        .iter()
        .map(|(_, _, _, default)| KeyAssign {
            keys: parse_key_list(default).unwrap_or_default(),
            overrides: Default::default(),
        })
        .collect()
}

/// 状態 state で機能 index が使うキー (その状態で働かない機能は None)
fn effective_keys(assigns: &[KeyAssign], index: usize, state: usize) -> Option<&[KeyCombo]> {
    if KEY_ITEMS[index].2 & (1 << state) == 0 {
        return None;
    }
    Some(assigns[index].overrides[state].as_deref().unwrap_or(assigns[index].keys.as_slice()))
}

/// 同じ状態で同じキーが複数の機能に割り当てられていれば、その説明を返す
fn find_key_conflict(assigns: &[KeyAssign]) -> Option<String> {
    for (state, (_, state_label)) in STATES.iter().enumerate() {
        for i in 0..KEY_ITEMS.len() {
            let Some(a) = effective_keys(assigns, i, state) else {
                continue;
            };
            for j in i + 1..KEY_ITEMS.len() {
                let Some(b) = effective_keys(assigns, j, state) else {
                    continue;
                };
                if let Some(key) = a.iter().find(|key| b.contains(key)) {
                    return Some(format!(
                        "{state_label}: {} と {} ({})",
                        KEY_ITEMS[i].1,
                        KEY_ITEMS[j].1,
                        key.notation()
                    ));
                }
            }
        }
    }
    None
}

/// 設定ファイルの内容 (エンジン向け + TSF 層向けの全キー)
struct Config {
    learning: bool,
    suggest: bool,
    typo_correction: bool,
    max_predictions: u32,   // 1-8
    min_suggest_chars: u32, // 1-5
    space_full: bool,
    punctuation: String,
    digits_full: bool,
    modeless: bool,
    candidate_bar: bool,
    next_word: bool,
    segment_ui: bool,
    llm: bool,
    /// LLM のバックエンド ("cpu" / "vulkan")
    llm_backend: String,
    candidate_font: String,
    candidate_font_size: u32, // 10-40
    keys: Vec<KeyAssign>,     // KEY_ITEMS の並び順
    /// 改行キーが働くアプリの exe 名
    newline_apps: Vec<String>,
    /// 診断ログの出力先 (空なら出さない)
    debug_log: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            learning: true,
            suggest: true,
            typo_correction: true,
            max_predictions: 8,
            min_suggest_chars: 2,
            space_full: true,
            punctuation: "、。".to_string(),
            digits_full: false,
            modeless: false,
            candidate_bar: true,
            next_word: true,
            segment_ui: false,
            llm: false,
            llm_backend: "cpu".to_string(),
            candidate_font: "Yu Gothic UI".to_string(),
            candidate_font_size: 18,
            keys: default_key_assigns(),
            newline_apps: parse_app_list(DEFAULT_NEWLINE_APPS),
            debug_log: String::new(),
        }
    }
}

/// exe 名が同じアプリか (TSF 層の照合と同じく大文字小文字を区別しない)
fn same_app(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// apps に exe 名 name を足す (すでにあれば足さない)。足した、またはすでにあった位置を返す
fn add_app(apps: &mut Vec<String>, name: &str) -> usize {
    if let Some(index) = apps.iter().position(|app| same_app(app, name)) {
        return index;
    }
    apps.push(name.to_string());
    apps.len() - 1
}

/// newline_apps の値 (exe 名のカンマ区切り) を読む。TSF 層 (tsf/src/config.cpp の
/// ParseAppList) と同じく前後の空白・タブを除き、空の項目は捨てる
fn parse_app_list(value: &str) -> Vec<String> {
    let mut apps = Vec::new();
    for item in value.split(',') {
        let item = item.trim_matches([' ', '\t']);
        if !item.is_empty() {
            add_app(&mut apps, item);
        }
    }
    apps
}

/// パスのファイル名部分
fn exe_file_name(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// 実行中のアプリから選ぶダイアログの選択肢: exe 名で重複をまとめ (タイトルは最初に
/// 見つかったもの)、登録済みの exe を除いて exe 名順に並べる
fn running_app_choices(
    windows: Vec<(String, String)>,
    registered: &[String],
) -> Vec<(String, String)> {
    let mut choices: Vec<(String, String)> = Vec::new();
    for (exe, title) in windows {
        if registered.iter().any(|app| same_app(app, &exe))
            || choices.iter().any(|(chosen, _)| same_app(chosen, &exe))
        {
            continue;
        }
        choices.push((exe, title));
    }
    choices.sort_by_key(|(exe, _)| exe.to_lowercase());
    choices
}

/// 設定ファイルのパス。優先順: QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv
fn config_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("QUICKLIME_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("config.tsv"))
}

/// 診断ログの既定の出力先 (%APPDATA%\QuicklIME\debug.log)。QUICKLIME_CONFIG_FILE で
/// 設定ファイルを移していても同じ場所にする。APPDATA が無ければ空
fn default_debug_log_path() -> String {
    std::env::var("APPDATA")
        .map(|appdata| {
            PathBuf::from(appdata).join("QuicklIME").join("debug.log").to_string_lossy().into_owned()
        })
        .unwrap_or_default()
}

impl Config {
    /// 設定ファイルを読み込む。無い・読めない・不正な値は既定値のまま
    fn load() -> Self {
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

    fn apply(&mut self, key: &str, value: &str) {
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
                    self.max_predictions = n.clamp(1, 8);
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
            "candidate_bar" => parse_bool(&mut self.candidate_bar),
            "next_word" => parse_bool(&mut self.next_word),
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
                // LOGFONT の面名は 32 要素 (終端込み) に収まる必要がある
                if !value.is_empty() && value.encode_utf16().count() < 32 {
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
    fn save(&self) -> Result<(), String> {
        let path = config_path()?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
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
        text.push_str(&format!("segment_ui\t{}\n", self.segment_ui as u32));
        text.push_str("\n# 候補ウィンドウ\n");
        text.push_str(&format!("candidate_bar\t{}\n", self.candidate_bar as u32));
        text.push_str(&format!("next_word\t{}\n", self.next_word as u32));
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
        text.push_str("\n# 開発用 (debug_log は診断ログの出力先。行が無ければ出さない)\n");
        if !self.debug_log.is_empty() {
            text.push_str(&format!("debug_log\t{}\n", self.debug_log));
        }
        std::fs::write(&path, text.as_bytes())
            .map_err(|e| format!("設定ファイルへ書き込めません ({e})"))?;

        // エンジンに再読込を伝える。未起動なら何もしない
        // (ファイルが正なので、次のエンジン起動時に読み込まれる)
        let _ = send_reloadconfig();
        Ok(())
    }
}

/// named pipe でエンジンに RELOADCONFIG を送る
fn send_reloadconfig() -> std::io::Result<()> {
    let name =
        std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| "quicklime-engine".to_string());
    let mut pipe =
        std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\{name}"))?;
    writeln!(pipe, "RELOADCONFIG")?;
    let mut response = String::new();
    BufReader::new(pipe).read_line(&mut response)?;
    Ok(())
}

/// NUL 終端の UTF-16 文字列を作る
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// インストール済みフォントの面名一覧 (縦書き用の @ 始まりは除く)
fn font_families() -> Vec<String> {
    unsafe extern "system" fn enum_proc(
        lf: *const LOGFONTW,
        _tm: *const TEXTMETRICW,
        _font_type: u32,
        lparam: LPARAM,
    ) -> i32 {
        let fonts = unsafe { &mut *(lparam as *mut Vec<String>) };
        let face = unsafe { &(*lf).lfFaceName };
        let len = face.iter().position(|&c| c == 0).unwrap_or(face.len());
        let name = String::from_utf16_lossy(&face[..len]);
        if !name.starts_with('@') && !fonts.contains(&name) {
            fonts.push(name);
        }
        1
    }

    let mut fonts: Vec<String> = Vec::new();
    unsafe {
        let hdc = GetDC(null_mut());
        let mut lf: LOGFONTW = std::mem::zeroed();
        lf.lfCharSet = 1; // DEFAULT_CHARSET: 全 charset の面名を列挙する
        EnumFontFamiliesExW(hdc, &lf, Some(enum_proc), &mut fonts as *mut _ as LPARAM, 0);
        ReleaseDC(null_mut(), hdc);
    }
    fonts
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class_name = wide("QuicklimeConfig");

        // 二重起動防止: 既に開いていればそれを前面に出して終わる
        let existing = FindWindowW(class_name.as_ptr(), null());
        if !existing.is_null() {
            SetForegroundWindow(existing);
            return;
        }

        let config = Config::load();
        KEY_ASSIGNS.with(|keys| *keys.borrow_mut() = config.keys.clone());
        NEWLINE_APPS.with(|apps| *apps.borrow_mut() = config.newline_apps.clone());

        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_TAB_CLASSES,
        };
        InitCommonControlsEx(&controls);

        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(instance, 1 as *const u16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as _,
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
        };
        RegisterClassW(&wc);
        let edit_class = wide(KEY_EDIT_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(key_edit_wndproc),
            hIcon: null_mut(),
            lpszClassName: edit_class.as_ptr(),
            ..wc
        });
        let capture_class = wide(KEY_CAPTURE_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(key_capture_wndproc),
            hIcon: null_mut(),
            lpszClassName: capture_class.as_ptr(),
            ..wc
        });
        let pick_class = wide(APP_PICK_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(app_pick_wndproc),
            hIcon: null_mut(),
            lpszClassName: pick_class.as_ptr(),
            ..wc
        });

        // レイアウト (96dpi 基準の論理ピクセルを DPI でスケールする)
        let dpi = GetDpiForSystem();
        let scale = |v: i32| v * dpi as i32 / 96;
        let margin = scale(16);
        let row_h = scale(24);
        let row_gap = scale(8);
        let label_w = scale(130);
        let ctrl_w = scale(170);
        let button_w = scale(88);
        let button_h = scale(28);
        // タブの枠から中身までの余白
        let page_pad = scale(12);
        let column_w = label_w + row_gap + ctrl_w;
        let key_list_w = scale(440);
        let page_w = column_w.max(key_list_w);

        // 大きさはタブの中身を作ってから決める
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let title = wide("QuicklIME 設定");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return;
        }

        // フォント (UI 既定の Meiryo UI 9pt 相当)
        let face = wide("Meiryo UI");
        let font = CreateFontW(
            -(9 * dpi as i32 / 72),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1, // DEFAULT_CHARSET
            0,
            0,
            0,
            0,
            face.as_ptr(),
        );
        UI_FONT.set(font);
        UI_DPI.set(dpi);

        // 作るコントロールが属するタブ (None はタブによらず常に表示する)
        let current_page: Cell<Option<usize>> = Cell::new(None);
        let pages: RefCell<Vec<Vec<HWND>>> = RefCell::new(vec![Vec::new(); TAB_NAMES.len()]);
        let create_control = |class: &str,
                              text: &str,
                              ctrl_style: u32,
                              ex: u32,
                              x: i32,
                              y: i32,
                              w: i32,
                              h: i32,
                              id: i32| {
            let class = wide(class);
            let text = wide(text);
            let ctrl = CreateWindowExW(
                ex,
                class.as_ptr(),
                text.as_ptr(),
                ctrl_style,
                x,
                y,
                w,
                h,
                hwnd,
                id as usize as _,
                instance,
                null(),
            );
            SendMessageW(ctrl, WM_SETFONT, font as usize, 1);
            if let Some(page) = current_page.get() {
                pages.borrow_mut()[page].push(ctrl);
            }
            ctrl
        };
        let label_style = WS_CHILD | WS_VISIBLE;
        let check_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32;
        let combo_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32;
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        // コンボの高さはドロップダウン一覧の分を含める
        let combo_list_h = scale(280);

        // タブの項目はタブコントロールに重ねて置く兄弟ウィンドウなので、タブコントロールが
        // 項目の上を塗らないよう WS_CLIPSIBLINGS を付ける
        let tab = create_control(
            "SysTabControl32",
            "",
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            margin,
            margin,
            scale(400),
            scale(300),
            ID_TAB,
        );
        for (i, name) in TAB_NAMES.iter().enumerate() {
            let mut text = wide(name);
            let item = TCITEMW { mask: TCIF_TEXT, pszText: text.as_mut_ptr(), ..Default::default() };
            SendMessageW(tab, TCM_INSERTITEMW, i, &item as *const _ as LPARAM);
        }
        SendMessageW(tab, TCM_SETCURSEL, 0, 0);
        // 見出しの高さはフォントで決まるため、枠と表示領域の差はフォントを設定してから測る
        let probe = scale(1000);
        let mut display = RECT { left: 0, top: 0, right: probe, bottom: probe };
        SendMessageW(tab, TCM_ADJUSTRECT, 0, &mut display as *mut _ as LPARAM);
        let page_x = margin + display.left + page_pad;
        let page_y = margin + display.top + page_pad;

        // コンボを作って項目を入れ、現在値を選択する共通処理
        let add_combo = |x: i32, y: i32, id: i32, items: &[&str], current: &str| {
            let combo = create_control("COMBOBOX", "", combo_style, 0, x, y, ctrl_w, combo_list_h, id);
            for item in items {
                let item = wide(item);
                SendMessageW(combo, CB_ADDSTRING, 0, item.as_ptr() as LPARAM);
            }
            let current_w = wide(current);
            if SendMessageW(combo, CB_SELECTSTRING, usize::MAX, current_w.as_ptr() as LPARAM)
                == CB_ERR as isize
            {
                SendMessageW(combo, CB_SETCURSEL, 0, 0);
            }
            combo
        };
        let check = |text: &str, y: i32, id: i32, value: bool| {
            let ctrl = create_control("BUTTON", text, check_style, 0, page_x, y, column_w, row_h, id);
            SendMessageW(ctrl, BM_SETCHECK, value as usize, 0);
        };
        let ctrl_x = page_x + label_w + row_gap;

        // ---- 変換 ----
        current_page.set(Some(0));
        let mut y = page_y;
        check("学習する (確定した表記を優先する)", y, ID_CHECK_LEARNING, config.learning);
        y += row_h + row_gap;
        check("入力中にサジェストを表示する", y, ID_CHECK_SUGGEST, config.suggest);
        y += row_h + row_gap;
        check("タイプミス補正 (曖昧一致の補充)", y, ID_CHECK_TYPO, config.typo_correction);
        y += row_h + row_gap;
        create_control("STATIC", "サジェスト件数:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        let pred_items: Vec<String> = (1..=8).map(|n| n.to_string()).collect();
        let pred_refs: Vec<&str> = pred_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_MAX_PRED, &pred_refs, &config.max_predictions.to_string());
        y += row_h + row_gap;
        create_control("STATIC", "サジェスト開始文字数:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        let chars_items: Vec<String> = (1..=5).map(|n| n.to_string()).collect();
        let chars_refs: Vec<&str> = chars_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_MIN_CHARS, &chars_refs, &config.min_suggest_chars.to_string());
        y += row_h + row_gap;
        check("文節単位で変換する", y, ID_CHECK_SEGMENT_UI, config.segment_ui);
        y += row_h + row_gap;
        check("AI で候補を並べ替える", y, ID_CHECK_LLM, config.llm);
        y += row_h + row_gap;
        create_control("STATIC", "AI の実行環境:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        let backend_labels: Vec<&str> = LLM_BACKENDS.iter().map(|(_, label)| *label).collect();
        let current_backend = LLM_BACKENDS
            .iter()
            .find(|(name, _)| *name == config.llm_backend)
            .map_or(LLM_BACKENDS[0].1, |(_, label)| *label);
        add_combo(ctrl_x, y, ID_COMBO_LLM_BACKEND, &backend_labels, current_backend);
        let convert_h = y + row_h - page_y;

        // ---- 入力 ----
        current_page.set(Some(1));
        let mut y = page_y;
        create_control("STATIC", "スペースキー:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        add_combo(
            ctrl_x,
            y,
            ID_COMBO_SPACE,
            &["全角スペース", "半角スペース"],
            if config.space_full { "全角スペース" } else { "半角スペース" },
        );
        y += row_h + row_gap;
        create_control("STATIC", "句読点:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        add_combo(ctrl_x, y, ID_COMBO_PUNCT, &PUNCT_ITEMS, &config.punctuation);
        y += row_h + row_gap;
        create_control("STATIC", "数字:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        add_combo(
            ctrl_x,
            y,
            ID_COMBO_DIGITS,
            &["半角", "全角"],
            if config.digits_full { "全角" } else { "半角" },
        );
        y += row_h + row_gap;
        check(
            "モードレス入力 (英語の打鍵を自動で判定する)",
            y,
            ID_CHECK_MODELESS,
            config.modeless,
        );

        // ---- 表示 ----
        current_page.set(Some(2));
        let mut y = page_y;
        check("候補バーを表示する", y, ID_CHECK_CANDIDATE_BAR, config.candidate_bar);
        y += row_h + row_gap;
        check("確定の後に次の語を予測する", y, ID_CHECK_NEXT_WORD, config.next_word);
        y += row_h + row_gap;
        create_control("STATIC", "フォント:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        let fonts = font_families();
        let font_combo = create_control(
            "COMBOBOX",
            "",
            combo_style | CBS_SORT as u32,
            0,
            ctrl_x,
            y,
            ctrl_w,
            combo_list_h,
            ID_COMBO_FONT,
        );
        for name in &fonts {
            let item = wide(name);
            SendMessageW(font_combo, CB_ADDSTRING, 0, item.as_ptr() as LPARAM);
        }
        let current_font = wide(&config.candidate_font);
        if SendMessageW(font_combo, CB_SELECTSTRING, usize::MAX, current_font.as_ptr() as LPARAM)
            == CB_ERR as isize
        {
            // 現在の設定値が列挙に無いフォントでも選択できるよう追加しておく
            let index = SendMessageW(font_combo, CB_ADDSTRING, 0, current_font.as_ptr() as LPARAM);
            SendMessageW(font_combo, CB_SETCURSEL, index as usize, 0);
        }
        y += row_h + row_gap;
        create_control("STATIC", "サイズ:", label_style, 0, page_x, y + scale(3), label_w, row_h, 0);
        let size_items: Vec<String> = (10..=40).map(|n| n.to_string()).collect();
        let size_refs: Vec<&str> = size_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_FONT_SIZE, &size_refs, &config.candidate_font_size.to_string());

        // ---- キー割当 ----
        current_page.set(Some(3));
        let list_style = WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_BORDER
            | LVS_REPORT
            | LVS_SINGLESEL
            | LVS_SHOWSELALWAYS
            | LVS_NOSORTHEADER;
        let list = create_control(
            "SysListView32",
            "",
            list_style,
            0,
            page_x,
            page_y,
            key_list_w,
            scale(300),
            ID_LIST_KEYS,
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_FULLROWSELECT as usize,
            LVS_EX_FULLROWSELECT as isize,
        );
        for (i, (title, width)) in
            [("機能", scale(120)), ("キー", scale(130)), ("状態別の上書き", scale(165))]
                .iter()
                .enumerate()
        {
            let mut text = wide(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: *width,
                pszText: text.as_mut_ptr(),
                ..Default::default()
            };
            SendMessageW(list, LVM_INSERTCOLUMNW, i, &column as *const _ as LPARAM);
        }
        refresh_key_list(list);
        select_list_row(list, 0);
        // 全行がスクロールなしで収まる高さにする。行と見出しの高さはフォントで決まるため、
        // 行を入れてから先頭行の位置 (上端 = 見出しの高さ) と高さを測る
        let mut item_rect = RECT { left: LVIR_BOUNDS as i32, top: 0, right: 0, bottom: 0 };
        let key_rows = KEY_ITEMS.len() as i32;
        let key_list_h =
            if SendMessageW(list, LVM_GETITEMRECT, 0, &mut item_rect as *mut _ as LPARAM) != 0 {
                item_rect.top + (item_rect.bottom - item_rect.top) * key_rows
            } else {
                scale(24) + scale(20) * key_rows
            } + scale(4); // 枠線の分と余裕
        MoveWindow(list, page_x, page_y, key_list_w, key_list_h, 0);
        let key_button_y = page_y + key_list_h + row_gap;
        create_control(
            "BUTTON",
            "編集",
            button_style,
            0,
            page_x,
            key_button_y,
            button_w,
            button_h,
            ID_BUTTON_KEY_EDIT,
        );
        create_control(
            "BUTTON",
            "既定に戻す",
            button_style,
            0,
            page_x + button_w + row_gap,
            key_button_y,
            button_w,
            button_h,
            ID_BUTTON_KEY_DEFAULT,
        );
        let keys_h = key_button_y + button_h - page_y;

        // ---- アプリ ----
        // 見出し・説明の下に、左に一覧、右に縦に並べたボタン
        let app_button_w = scale(190);
        let app_list_w = page_w - row_gap - app_button_w;
        let app_list_y = page_y + row_h * 2 + row_gap;
        let app_buttons: [(&str, i32); 4] = [
            ("実行中のアプリから追加...", ID_BUTTON_APP_RUNNING),
            ("ファイルから追加...", ID_BUTTON_APP_FILE),
            ("削除", ID_BUTTON_APP_REMOVE),
            ("既定に戻す", ID_BUTTON_APP_DEFAULT),
        ];
        let app_button_count = app_buttons.len() as i32;
        let apps_min_h = app_list_y - page_y + app_button_count * (button_h + row_gap) - row_gap;
        // 「開発用」タブの注意書きは折り返すため2行分の高さを取る
        let debug_note_h = row_h * 2;
        let debug_h = (row_h + row_gap) * 2 + debug_note_h;
        let page_h = convert_h.max(keys_h).max(apps_min_h).max(debug_h);

        current_page.set(Some(4));
        create_control("STATIC", "改行キーの対象アプリ", label_style, 0, page_x, page_y, page_w, row_h, 0);
        create_control(
            "STATIC",
            "Ctrl+J などの改行キーで Shift+Enter を送るアプリ",
            label_style,
            0,
            page_x,
            page_y + row_h,
            page_w,
            row_h,
            0,
        );
        let app_list = create_control(
            "SysListView32",
            "",
            list_style,
            0,
            page_x,
            app_list_y,
            app_list_w,
            page_y + page_h - app_list_y,
            ID_LIST_APPS,
        );
        SendMessageW(
            app_list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_FULLROWSELECT as usize,
            LVS_EX_FULLROWSELECT as isize,
        );
        let mut column_text = wide("exe 名");
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            // 縦スクロールバーが出ても横スクロールにならない幅
            cx: app_list_w - scale(24),
            pszText: column_text.as_mut_ptr(),
            ..Default::default()
        };
        SendMessageW(app_list, LVM_INSERTCOLUMNW, 0, &column as *const _ as LPARAM);
        refresh_app_list(app_list, Some(0));
        let app_button_x = page_x + app_list_w + row_gap;
        for (i, (text, id)) in app_buttons.iter().enumerate() {
            create_control(
                "BUTTON",
                text,
                button_style,
                0,
                app_button_x,
                app_list_y + i as i32 * (button_h + row_gap),
                app_button_w,
                button_h,
                *id,
            );
        }

        // ---- 開発用 ----
        current_page.set(Some(5));
        let mut y = page_y;
        check("診断ログを出力する", y, ID_CHECK_DEBUG_LOG, !config.debug_log.is_empty());
        y += row_h + row_gap;
        let debug_path = if config.debug_log.is_empty() {
            default_debug_log_path()
        } else {
            config.debug_log.clone()
        };
        create_control(
            "EDIT",
            &debug_path,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            WS_EX_CLIENTEDGE,
            page_x,
            y,
            page_w,
            row_h,
            ID_EDIT_DEBUG_LOG,
        );
        y += row_h + row_gap;
        create_control(
            "STATIC",
            "診断ログは全アプリのキー入力 (パスワード欄の打鍵も含む) を記録します。\
             調査が終わったらオフにしてください。",
            label_style,
            0,
            page_x,
            y,
            page_w,
            debug_note_h,
            0,
        );
        current_page.set(None);

        // ---- タブの枠と下部ボタン (右寄せ) ----
        let tab_w = display.left + page_pad + page_w + page_pad + (probe - display.right);
        let tab_h = display.top + page_pad + page_h + page_pad + (probe - display.bottom);
        MoveWindow(tab, margin, margin, tab_w, tab_h, 0);
        // 作った順では最前面になるため、項目の下に隠れるよう最背面へ回す
        SetWindowPos(tab, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let client_w = margin + tab_w + margin;
        let button_y = margin + tab_h + page_pad;
        let client_h = button_y + button_h + margin;
        create_control(
            "BUTTON",
            "保存",
            button_style | BS_DEFPUSHBUTTON as u32,
            0,
            client_w - margin - button_w * 2 - row_gap,
            button_y,
            button_w,
            button_h,
            ID_BUTTON_SAVE,
        );
        create_control(
            "BUTTON",
            "キャンセル",
            button_style,
            0,
            client_w - margin - button_w,
            button_y,
            button_w,
            button_h,
            ID_BUTTON_CANCEL,
        );
        TAB_PAGES.with(|tab_pages| *tab_pages.borrow_mut() = pages.take());
        show_tab(hwnd, 0);

        let mut rect = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let win_w = rect.right - rect.left;
        let win_h = rect.bottom - rect.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        SetWindowPos(
            hwnd,
            null_mut(),
            (screen_w - win_w) / 2,
            (screen_h - win_h) / 2,
            win_w,
            win_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Enter は保存、Esc は閉じる (フォーカス位置によらない。
            // ドロップダウン展開中のコンボは自前で Enter/Esc を処理するため除く)
            let dropped_combo = {
                let focus = GetFocus();
                !focus.is_null() && SendMessageW(focus, CB_GETDROPPEDSTATE, 0, 0) != 0
            };
            if msg.message == WM_KEYDOWN && !dropped_combo {
                let pressed = |vk: u16| (GetKeyState(vk as i32) as u16 & 0x8000) != 0;
                // Ctrl+Tab は IsDialogMessageW に渡すとフォーカス移動として扱われるため先に取る
                if msg.wParam == VK_TAB as usize && pressed(VK_CONTROL) {
                    let tab = GetDlgItem(hwnd, ID_TAB);
                    let count = TAB_NAMES.len() as isize;
                    let current = SendMessageW(tab, TCM_GETCURSEL, 0, 0).max(0);
                    let step = if pressed(VK_SHIFT) { count - 1 } else { 1 };
                    let next = ((current + step) % count) as usize;
                    SendMessageW(tab, TCM_SETCURSEL, next, 0);
                    show_tab(hwnd, next);
                    continue;
                }
                if msg.wParam == VK_RETURN as usize {
                    SendMessageW(hwnd, WM_COMMAND, ID_BUTTON_SAVE as usize, 0);
                    continue;
                }
                if msg.wParam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                    continue;
                }
            }
            if edit_keys::handle_key(&msg) {
                continue;
            }
            // Tab でのフォーカス移動
            if IsDialogMessageW(hwnd, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (wparam & 0xFFFF) as i32 {
                    ID_BUTTON_SAVE => on_save(hwnd),
                    ID_BUTTON_CANCEL => {
                        DestroyWindow(hwnd);
                    }
                    ID_BUTTON_KEY_EDIT => on_edit_key(hwnd),
                    ID_BUTTON_KEY_DEFAULT => {
                        KEY_ASSIGNS.with(|keys| *keys.borrow_mut() = default_key_assigns());
                        refresh_key_list(GetDlgItem(hwnd, ID_LIST_KEYS));
                    }
                    ID_BUTTON_APP_RUNNING => on_add_running_app(hwnd),
                    ID_BUTTON_APP_FILE => on_add_app_file(hwnd),
                    ID_BUTTON_APP_REMOVE => on_remove_app(hwnd),
                    ID_BUTTON_APP_DEFAULT => {
                        NEWLINE_APPS
                            .with(|apps| *apps.borrow_mut() = parse_app_list(DEFAULT_NEWLINE_APPS));
                        refresh_app_list(GetDlgItem(hwnd, ID_LIST_APPS), Some(0));
                    }
                    _ => {}
                }
                0
            }
            WM_NOTIFY => {
                let header = &*(lparam as *const NMHDR);
                if header.idFrom == ID_TAB as usize && header.code == TCN_SELCHANGE {
                    let index = SendMessageW(header.hwndFrom, TCM_GETCURSEL, 0, 0);
                    if index >= 0 {
                        show_tab(hwnd, index as usize);
                    }
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 保存ボタン: 画面の状態を検証して config.tsv へ書き出し、エンジンへ再読込を伝える
fn on_save(hwnd: HWND) {
    let config = collect(hwnd);

    if let Some(conflict) = find_key_conflict(&config.keys) {
        message_box(
            hwnd,
            &format!("同じ状態で同じキーが複数の機能に割り当てられています。\n{conflict}"),
            MB_ICONWARNING,
        );
        return;
    }

    match config.save() {
        Ok(()) => unsafe {
            DestroyWindow(hwnd);
        },
        Err(e) => message_box(hwnd, &format!("保存できませんでした。\n{e}"), MB_ICONWARNING),
    }
}

/// 画面のコントロールから設定値を集める
fn collect(hwnd: HWND) -> Config {
    let checked = |id: i32| unsafe {
        SendMessageW(GetDlgItem(hwnd, id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize
    };
    let combo_text = |id: i32| -> String {
        unsafe {
            let combo = GetDlgItem(hwnd, id);
            let index = SendMessageW(combo, CB_GETCURSEL, 0, 0);
            if index < 0 {
                return String::new();
            }
            let length = SendMessageW(combo, CB_GETLBTEXTLEN, index as usize, 0);
            if length <= 0 {
                return String::new();
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied =
                SendMessageW(combo, CB_GETLBTEXT, index as usize, buffer.as_mut_ptr() as LPARAM);
            if copied <= 0 {
                return String::new();
            }
            String::from_utf16_lossy(&buffer[..copied as usize])
        }
    };

    let mut config = Config::default();
    config.learning = checked(ID_CHECK_LEARNING);
    config.suggest = checked(ID_CHECK_SUGGEST);
    config.typo_correction = checked(ID_CHECK_TYPO);
    if let Ok(n) = combo_text(ID_COMBO_MAX_PRED).parse::<u32>() {
        config.max_predictions = n.clamp(1, 8);
    }
    if let Ok(n) = combo_text(ID_COMBO_MIN_CHARS).parse::<u32>() {
        config.min_suggest_chars = n.clamp(1, 5);
    }
    config.space_full = combo_text(ID_COMBO_SPACE) == "全角スペース";
    let punct = combo_text(ID_COMBO_PUNCT);
    if PUNCT_ITEMS.contains(&punct.as_str()) {
        config.punctuation = punct;
    }
    config.digits_full = combo_text(ID_COMBO_DIGITS) == "全角";
    config.modeless = checked(ID_CHECK_MODELESS);
    config.candidate_bar = checked(ID_CHECK_CANDIDATE_BAR);
    config.next_word = checked(ID_CHECK_NEXT_WORD);
    config.segment_ui = checked(ID_CHECK_SEGMENT_UI);
    config.llm = checked(ID_CHECK_LLM);
    let backend = combo_text(ID_COMBO_LLM_BACKEND);
    if let Some((name, _)) = LLM_BACKENDS.iter().find(|(_, label)| *label == backend) {
        config.llm_backend = name.to_string();
    }
    let font = combo_text(ID_COMBO_FONT);
    if !font.is_empty() && font.encode_utf16().count() < 32 {
        config.candidate_font = font;
    }
    if let Ok(n) = combo_text(ID_COMBO_FONT_SIZE).parse::<u32>() {
        config.candidate_font_size = n.clamp(10, 40);
    }
    config.keys = KEY_ASSIGNS.with(|keys| keys.borrow().clone());
    config.newline_apps = NEWLINE_APPS.with(|apps| apps.borrow().clone());
    if checked(ID_CHECK_DEBUG_LOG) {
        let path = window_text(unsafe { GetDlgItem(hwnd, ID_EDIT_DEBUG_LOG) });
        let path = path.trim();
        config.debug_log =
            if path.is_empty() { default_debug_log_path() } else { path.to_string() };
    }
    config
}

/// コントロールのテキスト
fn window_text(ctrl: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(ctrl);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(ctrl, buffer.as_mut_ptr(), buffer.len() as i32);
        if copied <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

/// タブ index の項目だけを表示する。フォーカスが隠した項目に残っていれば、表示したタブの
/// 先頭の項目へ移す (隠れた項目にフォーカスがあると打鍵が見えない項目に届く)
fn show_tab(hwnd: HWND, index: usize) {
    TAB_PAGES.with(|pages| {
        let pages = pages.borrow();
        unsafe {
            for (i, page) in pages.iter().enumerate() {
                for &ctrl in page {
                    ShowWindow(ctrl, if i == index { SW_SHOW } else { SW_HIDE });
                }
            }
            let focus = GetFocus();
            if focus.is_null() || IsWindowVisible(focus) != 0 {
                return;
            }
            let first = pages.get(index).and_then(|page| {
                page.iter().copied().find(|&ctrl| {
                    GetWindowLongW(ctrl, GWL_STYLE) as u32 & WS_TABSTOP != 0
                        && IsWindowEnabled(ctrl) != 0
                })
            });
            SetFocus(first.unwrap_or_else(|| GetDlgItem(hwnd, ID_BUTTON_SAVE)));
        }
    });
}

/// 「アプリ」タブの一覧を作り直し、select の行を選ぶ
fn refresh_app_list(list: HWND, select: Option<usize>) {
    let apps = NEWLINE_APPS.with(|apps| apps.borrow().clone());
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
        for (i, app) in apps.iter().enumerate() {
            let mut text = wide(app);
            let item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: i as i32,
                pszText: text.as_mut_ptr(),
                ..Default::default()
            };
            SendMessageW(list, LVM_INSERTITEMW, 0, &item as *const _ as LPARAM);
        }
        if let Some(row) = select.filter(|&row| row < apps.len()) {
            select_list_row(list, row);
            SendMessageW(list, LVM_ENSUREVISIBLE, row, 0);
        }
    }
}

/// 対象アプリに exe 名を足し (すでにあれば足さない)、その行を選ぶ
fn add_app_to_list(hwnd: HWND, name: &str) {
    let index = NEWLINE_APPS.with(|apps| add_app(&mut apps.borrow_mut(), name));
    refresh_app_list(unsafe { GetDlgItem(hwnd, ID_LIST_APPS) }, Some(index));
}

/// 「実行中のアプリから追加」ボタン
fn on_add_running_app(hwnd: HWND) {
    if let Some(exe) = open_app_pick_dialog(hwnd) {
        add_app_to_list(hwnd, &exe);
    }
    unsafe { SetFocus(GetDlgItem(hwnd, ID_LIST_APPS)) };
}

/// 「ファイルから追加」ボタン: 選んだ exe のファイル名部分を足す
fn on_add_app_file(hwnd: HWND) {
    let filter: Vec<u16> = "実行ファイル (*.exe)\0*.exe\0\0".encode_utf16().collect();
    let title = wide("ファイルから追加");
    let mut file = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return;
    }
    let length = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    if let Some(name) = exe_file_name(&String::from_utf16_lossy(&file[..length])) {
        add_app_to_list(hwnd, &name);
    }
    unsafe { SetFocus(GetDlgItem(hwnd, ID_LIST_APPS)) };
}

/// 「削除」ボタン: 選んでいる exe 名を外し、次の行を選ぶ
fn on_remove_app(hwnd: HWND) {
    unsafe {
        let list = GetDlgItem(hwnd, ID_LIST_APPS);
        let selected = SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
        if selected < 0 {
            return;
        }
        let row = selected as usize;
        let remaining = NEWLINE_APPS.with(|apps| {
            let mut apps = apps.borrow_mut();
            if row < apps.len() {
                apps.remove(row);
            }
            apps.len()
        });
        refresh_app_list(list, (remaining > 0).then(|| row.min(remaining - 1)));
    }
}

fn message_box(hwnd: HWND, text: &str, icon: u32) {
    let text = wide(text);
    let title = wide("QuicklIME 設定");
    unsafe { MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

// ---- キー割当の一覧・編集ダイアログ・キー取り込み ----

const KEY_EDIT_CLASS: &str = "QuicklimeKeyEdit";
const KEY_CAPTURE_CLASS: &str = "QuicklimeKeyCapture";
const CAPTURE_PROMPT: &str = "割り当てるキーを押してください。修飾キーと組み合わせるときは、\
                              修飾キーを押したまま押します。\n(やめるときは「キャンセル」)";

/// 実行中のアプリから選ぶダイアログの選択肢 ((exe 名, ウィンドウタイトル)) と選んだ exe 名
struct AppPick {
    choices: Vec<(String, String)>,
    picked: Option<String>,
}

/// 編集ダイアログで編集中の機能と割当
struct KeyEdit {
    assign: KeyAssign,
    accepted: bool,
}

thread_local! {
    /// 画面上のキー割当 (保存で書き出す。KEY_ITEMS の並び順)
    static KEY_ASSIGNS: RefCell<Vec<KeyAssign>> = const { RefCell::new(Vec::new()) };
    /// 画面上の改行キーの対象アプリ (保存で書き出す)
    static NEWLINE_APPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// タブごとの項目 (TAB_NAMES の並び順)
    static TAB_PAGES: RefCell<Vec<Vec<HWND>>> = const { RefCell::new(Vec::new()) };
    static APP_PICK: RefCell<Option<AppPick>> = const { RefCell::new(None) };
    /// ダイアログの子ウィンドウに使うフォントと DPI (main で設定する)
    static UI_FONT: Cell<HFONT> = const { Cell::new(null_mut()) };
    static UI_DPI: Cell<u32> = const { Cell::new(96) };
    static KEY_EDIT: RefCell<Option<KeyEdit>> = const { RefCell::new(None) };
    /// キー取り込みダイアログで取り込んだキー
    static CAPTURED_KEY: Cell<Option<KeyCombo>> = const { Cell::new(None) };
}

/// 96dpi 基準の論理ピクセルを DPI でスケールする
fn scaled(value: i32) -> i32 {
    value * UI_DPI.get() as i32 / 96
}

/// 一覧の行を現在のキー割当で作り直す (選択中の行は維持する)
fn refresh_key_list(list: HWND) {
    let assigns = KEY_ASSIGNS.with(|keys| keys.borrow().clone());
    unsafe {
        let selected = SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
        SendMessageW(list, LVM_DELETEALLITEMS, 0, 0);
        for (i, (_, label, _, _)) in KEY_ITEMS.iter().enumerate() {
            let overrides: Vec<String> = STATES
                .iter()
                .enumerate()
                .filter_map(|(s, (_, state_label))| {
                    let keys = assigns[i].overrides[s].as_ref()?;
                    Some(format!("{state_label}: {}", display_key_list(keys)))
                })
                .collect();
            let texts = [label.to_string(), display_key_list(&assigns[i].keys), overrides.join(" / ")];
            for (column, text) in texts.iter().enumerate() {
                let mut text = wide(text);
                let item = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: i as i32,
                    iSubItem: column as i32,
                    pszText: text.as_mut_ptr(),
                    ..Default::default()
                };
                let message = if column == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
                SendMessageW(list, message, i, &item as *const _ as LPARAM);
            }
        }
        if selected >= 0 {
            select_list_row(list, selected as usize);
        }
    }
}

fn select_list_row(list: HWND, row: usize) {
    let item = LVITEMW {
        mask: LVIF_STATE,
        state: LVIS_SELECTED | LVIS_FOCUSED,
        stateMask: LVIS_SELECTED | LVIS_FOCUSED,
        ..Default::default()
    };
    unsafe { SendMessageW(list, LVM_SETITEMSTATE, row, &item as *const _ as LPARAM) };
}

/// 「編集」ボタン: 一覧で選んでいる機能の編集ダイアログを開く
fn on_edit_key(hwnd: HWND) {
    unsafe {
        let list = GetDlgItem(hwnd, ID_LIST_KEYS);
        let selected = SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
        if selected < 0 {
            return;
        }
        let index = selected as usize;
        if let Some(assign) = open_key_edit_dialog(hwnd, index) {
            KEY_ASSIGNS.with(|keys| keys.borrow_mut()[index] = assign);
            refresh_key_list(list);
        }
        SetFocus(list);
    }
}

unsafe fn create_child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
) -> HWND {
    let class = wide(class);
    let text = wide(text);
    unsafe {
        let ctrl = CreateWindowExW(
            0,
            class.as_ptr(),
            text.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            parent,
            id as usize as _,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(ctrl, WM_SETFONT, UI_FONT.get() as usize, 1);
        ctrl
    }
}

/// owner の中央に、クライアント領域が client_w x client_h のモーダル用ウィンドウを作る
unsafe fn create_modal_window(
    owner: HWND,
    class: &str,
    title: &str,
    client_w: i32,
    client_h: i32,
) -> HWND {
    let class = wide(class);
    let title = wide(title);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    unsafe {
        let mut rect = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectEx(&mut rect, style, 0, WS_EX_DLGMODALFRAME);
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let mut owner_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetWindowRect(owner, &mut owner_rect);
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title.as_ptr(),
            style,
            owner_rect.left + (owner_rect.right - owner_rect.left - w) / 2,
            owner_rect.top + (owner_rect.bottom - owner_rect.top - h) / 2,
            w,
            h,
            owner,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    }
}

/// owner を無効にして hwnd を表示し、hwnd が閉じるまでメッセージを回す。
/// dialog_keys なら Tab・Enter・Esc をダイアログの操作として扱う
unsafe fn run_modal(owner: HWND, hwnd: HWND, dialog_keys: bool) {
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        let mut msg = std::mem::zeroed::<MSG>();
        while IsWindow(hwnd) != 0 {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result == 0 {
                PostQuitMessage(msg.wParam as i32);
                break;
            }
            if result < 0 {
                break;
            }
            if dialog_keys {
                if IsDialogMessageW(hwnd, &msg) != 0 {
                    continue;
                }
                TranslateMessage(&msg);
            }
            DispatchMessageW(&msg);
        }
    }
}

/// モーダル用ウィンドウを閉じる。owner を無効のまま閉じると別のアプリが前面に出るため、
/// 先に owner を有効に戻す
unsafe fn close_modal(hwnd: HWND) {
    unsafe {
        EnableWindow(GetWindow(hwnd, GW_OWNER), 1);
        DestroyWindow(hwnd);
    }
}

fn with_key_edit<R>(f: impl FnOnce(&mut KeyEdit) -> R) -> Option<R> {
    KEY_EDIT.with(|edit| edit.borrow_mut().as_mut().map(f))
}

/// 編集ダイアログの区画 k (0 = 基本の割当、1〜4 = STATES[k - 1] の上書き) のキー一覧。
/// 上書きしていない区画は None
fn section_keys(assign: &mut KeyAssign, k: usize) -> Option<&mut Vec<KeyCombo>> {
    if k == 0 { Some(&mut assign.keys) } else { assign.overrides[k - 1].as_mut() }
}

/// 機能 index の割当を編集するダイアログ。OK で閉じたら編集後の割当を返す
fn open_key_edit_dialog(owner: HWND, index: usize) -> Option<KeyAssign> {
    let (_, label, states, _) = KEY_ITEMS[index];
    let assign = KEY_ASSIGNS.with(|keys| keys.borrow()[index].clone());
    let sections: Vec<usize> = std::iter::once(0)
        .chain((0..STATES.len()).filter(|s| states & (1 << s) != 0).map(|s| s + 1))
        .collect();
    KEY_EDIT.with(|edit| *edit.borrow_mut() = Some(KeyEdit { assign, accepted: false }));

    let margin = scaled(16);
    let gap = scaled(8);
    let row_h = scaled(24);
    let list_w = scaled(240);
    let list_h = scaled(72);
    let button_w = scaled(88);
    let button_h = scaled(28);
    let section_h = row_h + gap + list_h + scaled(16);
    let client_w = margin + list_w + gap + button_w + margin;
    let client_h = margin + section_h * sections.len() as i32 + button_h + margin;
    unsafe {
        let hwnd = create_modal_window(
            owner,
            KEY_EDIT_CLASS,
            &format!("キー割当の編集: {label}"),
            client_w,
            client_h,
        );
        if hwnd.is_null() {
            KEY_EDIT.with(|edit| edit.borrow_mut().take());
            return None;
        }
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        let mut y = margin;
        for &k in &sections {
            let offset = k as i32 * 10;
            if k == 0 {
                create_child(hwnd, "STATIC", "基本の割当", WS_CHILD | WS_VISIBLE, margin, y + scaled(3), list_w, row_h, 0);
            } else {
                let check = create_child(
                    hwnd,
                    "BUTTON",
                    &format!("{}で上書きする", STATES[k - 1].1),
                    button_style | BS_AUTOCHECKBOX as u32,
                    margin,
                    y,
                    list_w + gap + button_w,
                    row_h,
                    ID_EDIT_OVERRIDE_BASE + offset,
                );
                let overridden =
                    with_key_edit(|edit| edit.assign.overrides[k - 1].is_some()).unwrap_or(false);
                SendMessageW(check, BM_SETCHECK, overridden as usize, 0);
            }
            y += row_h + gap;
            create_child(
                hwnd,
                "LISTBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WS_VSCROLL,
                margin,
                y,
                list_w,
                list_h,
                ID_EDIT_LIST_BASE + offset,
            );
            let button_x = margin + list_w + gap;
            create_child(hwnd, "BUTTON", "追加", button_style, button_x, y, button_w, button_h, ID_EDIT_ADD_BASE + offset);
            create_child(
                hwnd,
                "BUTTON",
                "削除",
                button_style,
                button_x,
                y + button_h + gap,
                button_w,
                button_h,
                ID_EDIT_REMOVE_BASE + offset,
            );
            refresh_edit_section(hwnd, k);
            y += list_h + scaled(16);
        }
        let button_y = client_h - margin - button_h;
        create_child(
            hwnd,
            "BUTTON",
            "OK",
            button_style | BS_DEFPUSHBUTTON as u32,
            client_w - margin - button_w * 2 - gap,
            button_y,
            button_w,
            button_h,
            IDOK,
        );
        create_child(hwnd, "BUTTON", "キャンセル", button_style, client_w - margin - button_w, button_y, button_w, button_h, IDCANCEL);
        run_modal(owner, hwnd, true);
    }
    let edit = KEY_EDIT.with(|edit| edit.borrow_mut().take())?;
    edit.accepted.then_some(edit.assign)
}

/// 編集ダイアログの区画 k の一覧を作り直し、上書きしていない区画の操作を無効にする
unsafe fn refresh_edit_section(hwnd: HWND, k: usize) {
    let offset = k as i32 * 10;
    let keys = with_key_edit(|edit| section_keys(&mut edit.assign, k).cloned()).flatten();
    unsafe {
        let list = GetDlgItem(hwnd, ID_EDIT_LIST_BASE + offset);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        for key in keys.iter().flatten() {
            let text = wide(&key.notation());
            SendMessageW(list, LB_ADDSTRING, 0, text.as_ptr() as LPARAM);
        }
        for id in [ID_EDIT_LIST_BASE, ID_EDIT_ADD_BASE, ID_EDIT_REMOVE_BASE] {
            EnableWindow(GetDlgItem(hwnd, id + offset), keys.is_some() as i32);
        }
    }
}

unsafe extern "system" fn key_edit_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                if ((wparam >> 16) & 0xFFFF) as u32 != BN_CLICKED {
                    return 0;
                }
                match (wparam & 0xFFFF) as i32 {
                    IDOK => {
                        with_key_edit(|edit| edit.accepted = true);
                        close_modal(hwnd);
                    }
                    IDCANCEL => close_modal(hwnd),
                    id if id >= ID_EDIT_LIST_BASE => on_edit_section_command(hwnd, id),
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                close_modal(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 編集ダイアログの区画ごとのボタン (追加・削除・上書きする)
unsafe fn on_edit_section_command(hwnd: HWND, id: i32) {
    let k = ((id - ID_EDIT_LIST_BASE) / 10) as usize;
    let offset = k as i32 * 10;
    unsafe {
        match id - offset {
            ID_EDIT_ADD_BASE => {
                let Some(key) = open_key_capture_dialog(hwnd) else {
                    return;
                };
                with_key_edit(|edit| {
                    if let Some(keys) = section_keys(&mut edit.assign, k) {
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                });
            }
            ID_EDIT_REMOVE_BASE => {
                let selected =
                    SendMessageW(GetDlgItem(hwnd, ID_EDIT_LIST_BASE + offset), LB_GETCURSEL, 0, 0);
                if selected < 0 {
                    return;
                }
                with_key_edit(|edit| {
                    if let Some(keys) = section_keys(&mut edit.assign, k) {
                        if (selected as usize) < keys.len() {
                            keys.remove(selected as usize);
                        }
                    }
                });
            }
            ID_EDIT_OVERRIDE_BASE if k > 0 => {
                let checked = SendMessageW(GetDlgItem(hwnd, id), BM_GETCHECK, 0, 0)
                    == BST_CHECKED as isize;
                // 上書きを始めるときは基本の割当を写して始める (チェックだけでは動作を変えない)
                with_key_edit(|edit| {
                    let base = edit.assign.keys.clone();
                    let entry = &mut edit.assign.overrides[k - 1];
                    if !checked {
                        *entry = None;
                    } else if entry.is_none() {
                        *entry = Some(base);
                    }
                });
            }
            _ => return,
        }
        refresh_edit_section(hwnd, k);
    }
}

/// 押したキーを取り込むダイアログ。取り込めたキーを返す (キャンセルなら None)
fn open_key_capture_dialog(owner: HWND) -> Option<KeyCombo> {
    CAPTURED_KEY.set(None);
    let margin = scaled(16);
    let gap = scaled(8);
    let text_w = scaled(360);
    let text_h = scaled(72);
    let button_w = scaled(88);
    let button_h = scaled(28);
    let client_w = margin + text_w + margin;
    let client_h = margin + text_h + gap + button_h + margin;
    unsafe {
        let hwnd = create_modal_window(owner, KEY_CAPTURE_CLASS, "キーの取り込み", client_w, client_h);
        if hwnd.is_null() {
            return None;
        }
        ImmAssociateContextEx(hwnd, 0, 0);
        create_child(hwnd, "STATIC", CAPTURE_PROMPT, WS_CHILD | WS_VISIBLE, margin, margin, text_w, text_h, ID_CAPTURE_MESSAGE);
        // 打鍵をすべてこのウィンドウで受けるため、ボタンには Tab でフォーカスを移さない
        create_child(
            hwnd,
            "BUTTON",
            "キャンセル",
            WS_CHILD | WS_VISIBLE,
            client_w - margin - button_w,
            margin + text_h + gap,
            button_w,
            button_h,
            IDCANCEL,
        );
        run_modal(owner, hwnd, false);
    }
    CAPTURED_KEY.get()
}

unsafe extern "system" fn key_capture_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                on_capture_key(hwnd, wparam as u16);
                0
            }
            // Alt・F10 を離したときのメニュー起動や、WM_SYSCHAR の警告音を出さない
            WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR => 0,
            WM_ACTIVATE => {
                // 前面に戻ったときに打鍵をこのウィンドウで受ける
                if (wparam & 0xFFFF) != 0 {
                    SetFocus(hwnd);
                }
                0
            }
            WM_COMMAND => {
                if (wparam & 0xFFFF) as i32 == IDCANCEL {
                    close_modal(hwnd);
                }
                0
            }
            WM_CLOSE => {
                close_modal(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 取り込みダイアログで押されたキー。割り当てられるキーなら取り込んで閉じ、
/// 割り当てられないキーなら理由を表示する
unsafe fn on_capture_key(hwnd: HWND, vk: u16) {
    // 修飾キーの押し始めは、続けて押すキーを待つ
    if is_modifier_vk(vk) {
        return;
    }
    let pressed = |key: u16| unsafe { (GetKeyState(key as i32) as u16 & 0x8000) != 0 };
    let message = if vk == 0xE5 {
        // VK_PROCESSKEY: IME が打鍵を処理した
        "IME がこのキーを処理したため取り込めません".to_string()
    } else if pressed(0x5B) || pressed(0x5C) {
        "Win キーとの組み合わせは割り当てられません".to_string()
    } else {
        let key = KeyCombo { ctrl: pressed(0x11), alt: pressed(0x12), shift: pressed(0x10), vk };
        match key.unassignable_reason() {
            None => {
                CAPTURED_KEY.set(Some(key));
                unsafe { close_modal(hwnd) };
                return;
            }
            Some(reason) => format!("{}: {reason}", key.notation()),
        }
    };
    let text = wide(&message);
    unsafe { SetWindowTextW(GetDlgItem(hwnd, ID_CAPTURE_MESSAGE), text.as_ptr()) };
}

// ---- 実行中のアプリから選ぶダイアログ ----

const APP_PICK_CLASS: &str = "QuicklimeAppPick";

/// プロセス pid の exe のファイル名。取れない (権限不足など) なら None
fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        exe_file_name(&String::from_utf16_lossy(&buffer[..size as usize]))
    }
}

fn window_process_id(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid
}

/// parent のトップレベルウィンドウ (parent が null のとき) または子ウィンドウの一覧
fn list_windows(parent: HWND) -> Vec<HWND> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
        let windows = unsafe { &mut *(lparam as *mut Vec<HWND>) };
        windows.push(hwnd);
        1
    }

    let mut windows: Vec<HWND> = Vec::new();
    let lparam = &mut windows as *mut _ as LPARAM;
    unsafe {
        if parent.is_null() {
            EnumWindows(Some(enum_proc), lparam);
        } else {
            EnumChildWindows(parent, Some(enum_proc), lparam);
        }
    }
    windows
}

/// 見えているトップレベルウィンドウ (表示中・所有者なし・タイトルが空でない) を持つ
/// プロセスの (exe 名, ウィンドウタイトル)。設定ツール自身は除く。重複はまとめない
fn visible_app_windows() -> Vec<(String, String)> {
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut apps = Vec::new();
    for hwnd in list_windows(null_mut()) {
        let title = unsafe {
            if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
                continue;
            }
            let length = GetWindowTextLengthW(hwnd);
            if length <= 0 {
                continue;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
        };
        if title.is_empty() {
            continue;
        }
        let pid = window_process_id(hwnd);
        if pid == own_pid {
            continue;
        }
        let Some(mut exe) = process_exe_name(pid) else {
            continue;
        };
        // ストアアプリの枠のウィンドウは ApplicationFrameHost.exe が持ち、アプリ本体は
        // 別プロセスの子ウィンドウとして入っている
        if same_app(&exe, "ApplicationFrameHost.exe") {
            let app = list_windows(hwnd)
                .into_iter()
                .map(window_process_id)
                .find(|&child| child != 0 && child != pid)
                .and_then(process_exe_name);
            let Some(app) = app else {
                continue;
            };
            exe = app;
        }
        apps.push((exe, title));
    }
    apps
}

/// 実行中のアプリから exe を1つ選ぶダイアログ。選んだ exe 名を返す (キャンセルなら None)
fn open_app_pick_dialog(owner: HWND) -> Option<String> {
    let registered = NEWLINE_APPS.with(|apps| apps.borrow().clone());
    let choices = running_app_choices(visible_app_windows(), &registered);
    let margin = scaled(16);
    let gap = scaled(8);
    let list_w = scaled(480);
    let list_h = scaled(280);
    let exe_column_w = scaled(160);
    let button_w = scaled(88);
    let button_h = scaled(28);
    let client_w = margin + list_w + margin;
    let client_h = margin + list_h + gap + button_h + margin;
    unsafe {
        let hwnd = create_modal_window(owner, APP_PICK_CLASS, "実行中のアプリから追加", client_w, client_h);
        if hwnd.is_null() {
            return None;
        }
        let list = create_child(
            hwnd,
            "SysListView32",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | LVS_REPORT
                | LVS_SINGLESEL
                | LVS_SHOWSELALWAYS
                | LVS_NOSORTHEADER,
            margin,
            margin,
            list_w,
            list_h,
            ID_PICK_LIST,
        );
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            LVS_EX_FULLROWSELECT as usize,
            LVS_EX_FULLROWSELECT as isize,
        );
        // タイトルの列は、縦スクロールバーが出ても横スクロールにならない幅
        let columns = [("exe 名", exe_column_w), ("ウィンドウタイトル", list_w - exe_column_w - scaled(24))];
        for (i, (title, width)) in columns.iter().enumerate() {
            let mut text = wide(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: *width,
                pszText: text.as_mut_ptr(),
                ..Default::default()
            };
            SendMessageW(list, LVM_INSERTCOLUMNW, i, &column as *const _ as LPARAM);
        }
        for (i, (exe, title)) in choices.iter().enumerate() {
            for (column, text) in [exe, title].iter().enumerate() {
                let mut text = wide(text);
                let item = LVITEMW {
                    mask: LVIF_TEXT,
                    iItem: i as i32,
                    iSubItem: column as i32,
                    pszText: text.as_mut_ptr(),
                    ..Default::default()
                };
                let message = if column == 0 { LVM_INSERTITEMW } else { LVM_SETITEMTEXTW };
                SendMessageW(list, message, i, &item as *const _ as LPARAM);
            }
        }
        if !choices.is_empty() {
            select_list_row(list, 0);
        }
        APP_PICK.with(|pick| *pick.borrow_mut() = Some(AppPick { choices, picked: None }));
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        let button_y = client_h - margin - button_h;
        create_child(
            hwnd,
            "BUTTON",
            "追加",
            button_style | BS_DEFPUSHBUTTON as u32,
            client_w - margin - button_w * 2 - gap,
            button_y,
            button_w,
            button_h,
            IDOK,
        );
        create_child(hwnd, "BUTTON", "キャンセル", button_style, client_w - margin - button_w, button_y, button_w, button_h, IDCANCEL);
        run_modal(owner, hwnd, true);
    }
    APP_PICK.with(|pick| pick.borrow_mut().take())?.picked
}

/// 選択肢の row 行目を選んだ exe としてダイアログを閉じる
unsafe fn accept_app_pick(hwnd: HWND, row: usize) {
    APP_PICK.with(|pick| {
        if let Some(pick) = pick.borrow_mut().as_mut() {
            pick.picked = pick.choices.get(row).map(|(exe, _)| exe.clone());
        }
    });
    unsafe { close_modal(hwnd) };
}

unsafe extern "system" fn app_pick_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (wparam & 0xFFFF) as i32 {
                    IDOK => {
                        let list = GetDlgItem(hwnd, ID_PICK_LIST);
                        let selected =
                            SendMessageW(list, LVM_GETNEXTITEM, usize::MAX, LVNI_SELECTED as isize);
                        if selected >= 0 {
                            accept_app_pick(hwnd, selected as usize);
                        }
                    }
                    IDCANCEL => close_modal(hwnd),
                    _ => {}
                }
                0
            }
            WM_NOTIFY => {
                let header = &*(lparam as *const NMHDR);
                if header.idFrom == ID_PICK_LIST as usize && header.code == NM_DBLCLK {
                    // 空いている所のダブルクリック (iItem が -1) では選ばない
                    let activate = &*(lparam as *const NMITEMACTIVATE);
                    if activate.iItem >= 0 {
                        accept_app_pick(hwnd, activate.iItem as usize);
                    }
                }
                0
            }
            WM_CLOSE => {
                close_modal(hwnd);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyCombo {
        KeyCombo::parse(text).unwrap()
    }

    #[test]
    fn reads_legacy_values() {
        for value in ["F4", "Ctrl+F7", "Ctrl+Backspace", "Convert", "Ctrl+Space"] {
            assert_eq!(parse_key_list(value), Some(vec![key(value)]), "{value}");
        }
        assert_eq!(parse_key_list("none"), Some(Vec::new()));
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
    fn parses_app_list_like_tsf() {
        // 前後の空白・タブを除き、空の項目を捨て、大文字小文字だけ違う重複はまとめる
        assert_eq!(
            parse_app_list(" Discord.exe ,\tslack.exe\t,, ,DISCORD.EXE,notepad.exe"),
            ["Discord.exe", "slack.exe", "notepad.exe"]
        );
        assert!(parse_app_list(" , ").is_empty());
        assert_eq!(parse_app_list(DEFAULT_NEWLINE_APPS).join(","), DEFAULT_NEWLINE_APPS);
    }

    #[test]
    fn adds_app_ignoring_case() {
        let mut apps = parse_app_list("Discord.exe,Notepad.exe");
        assert_eq!(add_app(&mut apps, "notepad.exe"), 1);
        assert_eq!(add_app(&mut apps, "NOTEPAD.EXE"), 1);
        assert_eq!(apps, ["Discord.exe", "Notepad.exe"]);
        assert_eq!(add_app(&mut apps, "CalculatorApp.exe"), 2);
        assert_eq!(apps, ["Discord.exe", "Notepad.exe", "CalculatorApp.exe"]);
    }

    #[test]
    fn takes_exe_file_name() {
        assert_eq!(exe_file_name(r"C:\Windows\System32\notepad.exe").as_deref(), Some("notepad.exe"));
        assert_eq!(exe_file_name("notepad.exe").as_deref(), Some("notepad.exe"));
        assert_eq!(exe_file_name(r"C:\"), None);
    }

    #[test]
    fn makes_running_app_choices() {
        let window = |exe: &str, title: &str| (exe.to_string(), title.to_string());
        let windows = vec![
            window("Notepad.exe", "メモ 1"),
            window("explorer.exe", "ドキュメント"),
            window("notepad.exe", "メモ 2"),
            window("discord.exe", "Discord"),
            window("Code.exe", "VS Code"),
        ];
        let registered = parse_app_list("Discord.exe");
        // exe 名でまとめて最初のタイトルを残し、登録済みを除き、大文字小文字によらず exe 名順
        assert_eq!(
            running_app_choices(windows, &registered),
            [window("Code.exe", "VS Code"), window("explorer.exe", "ドキュメント"), window("Notepad.exe", "メモ 1")]
        );
    }

    #[test]
    fn parses_and_formats_notation() {
        assert_eq!(key("Shift+Ctrl+F6").notation(), "Ctrl+Shift+F6");
        assert_eq!(key("Alt+S"), KeyCombo { ctrl: false, alt: true, shift: false, vk: 0x53 });
        assert_eq!(key("VK_e9").notation(), "VK_E9");
        assert_eq!(key("VK_1C").notation(), "Convert");
        assert_eq!(key("F24").vk, 0x87);
        assert_eq!(key("NonConvert").vk, 0x1D);
        assert!(KeyCombo::parse("F25").is_none());
        assert!(KeyCombo::parse("Ctrl+Ctrl+A").is_none());
        assert!(KeyCombo::parse("Win+A").is_none());
        assert!(KeyCombo::parse("VK_00").is_none());
        assert!(KeyCombo::parse("VK_+F").is_none());
        assert_eq!(
            parse_key_list("F7, Ctrl+K,F7"),
            Some(vec![key("F7"), key("Ctrl+K")])
        );
        assert_eq!(format_key_list(&[key("F7"), key("Ctrl+K")]), "F7,Ctrl+K");
        assert_eq!(format_key_list(&[]), "none");
    }

    #[test]
    fn rejects_unassignable_keys() {
        for text in [
            "Enter", "A", "Shift+Tab", "Ctrl+H", "Ctrl+M", "Esc", "Shift+1", "VK_19", "VK_F3", "VK_10", "VK_6B",
            "Alt+S", "Alt+Left", "Alt+F4", "Ctrl+Alt+F7", "Alt+Shift+Space",
        ] {
            assert!(key(text).unassignable_reason().is_some(), "{text}");
        }
        for text in ["Ctrl+Enter", "Ctrl+Left", "Ctrl+Shift+M", "Space", "Shift+Space", "Insert", "Kana", "Ctrl+A", "VK_6C"] {
            assert!(key(text).unassignable_reason().is_none(), "{text}");
        }
        // 対象外だけなら既定のまま
        assert_eq!(parse_key_list("Enter"), None);
        assert_eq!(parse_key_list("Alt+S"), None);
        assert_eq!(parse_key_list("Enter,F11"), Some(vec![key("F11")]));
        assert_eq!(parse_key_list("F4,Alt+S"), Some(vec![key("F4")]));
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

    #[test]
    fn detects_conflicts_per_state() {
        let mut assigns = default_key_assigns();
        assert_eq!(find_key_conflict(&assigns), None);
        // 働く状態が重ならなければ重なりではない (Ctrl+F7 は単語登録 = 入力なし のみ、
        // 記号・日付変換は入力なしでは働かない)
        assigns[4].keys = vec![key("F4"), key("Ctrl+F7")];
        assert_eq!(find_key_conflict(&assigns), None);
        // カタカナ変換は入力なしでも働くので、単語登録と重なる
        assigns[4].keys = vec![key("F4")];
        assigns[7].keys = vec![key("F7"), key("Ctrl+F7")];
        let conflict = find_key_conflict(&assigns).unwrap();
        assert!(conflict.contains("入力なし") && conflict.contains("単語登録"), "{conflict}");
        assigns[7].keys = vec![key("F7")];
        // 候補選択中だけ Space を記号変換にも割り当てる
        assigns[4].overrides[2] = Some(vec![key("Space")]);
        let conflict = find_key_conflict(&assigns).unwrap();
        assert!(conflict.contains("候補選択中") && conflict.contains("次候補"), "{conflict}");
        // 次候補の候補選択中を上書きで外せば解消する
        assigns[1].overrides[2] = Some(Vec::new());
        assert_eq!(find_key_conflict(&assigns), None);
    }
}
