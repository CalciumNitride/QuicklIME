// QuicklIME 設定ツール
//
// config.tsv (エンジンと TSF 層が読む共通の設定ファイル) を編集する
// Win32 ダイアログ。保存時にエンジンへ RELOADCONFIG を送って即時反映する
// (エンジン未起動時はファイルへの書き込みのみ。TSF 層は次のフォーカス
// 切替時に更新時刻の変化を検知して自動反映する)。
//
// 使い方: quicklime-config.exe (引数なし。TSF 層が Ctrl+F12 などで起動する)

#![windows_subsystem = "windows"]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, CreateFontW, EnumFontFamiliesExW, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Controls::BST_CHECKED;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

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
const ID_COMBO_KEY_BASE: i32 = 120; // +0〜9 (KEY_ITEMS の並び順)
const ID_BUTTON_SAVE: i32 = 140;
const ID_BUTTON_CANCEL: i32 = 141;

/// キー割当の機能一覧: (設定キー名, 表示名, Ctrl 併用か)。
/// 並びと制約は TSF 層 (tsf/src/config.h の KeyFunc) と合わせる
const KEY_ITEMS: [(&str, &str, bool); 10] = [
    ("key.convert_symbol", "記号・日付変換", false),
    ("key.convert_user", "ユーザ語変換", false),
    ("key.to_hiragana", "ひらがな変換", false),
    ("key.to_katakana", "カタカナ変換", false),
    ("key.to_half_katakana", "半角カタカナ変換", false),
    ("key.to_full_ascii", "全角英字変換", false),
    ("key.to_half_ascii", "半角英字変換", false),
    ("key.undo_commit", "確定アンドゥ", true),
    ("key.register_word", "単語登録", true),
    ("key.open_config", "設定を開く", true),
];

/// 句読点の選択肢 (設定値そのまま表示する)
const PUNCT_ITEMS: [&str; 4] = ["、。", "，．", "、．", "，。"];

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
    candidate_font: String,
    candidate_font_size: u32, // 10-40
    keys: [String; 10],       // "F4" / "Ctrl+F7" / "none" (KEY_ITEMS の並び順)
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
            candidate_font: "Yu Gothic UI".to_string(),
            candidate_font_size: 18,
            keys: [
                "F4", "F5", "F6", "F7", "F8", "F9", "F10", "Ctrl+Backspace", "Ctrl+F7",
                "Ctrl+F12",
            ]
            .map(String::from),
        }
    }
}

/// 設定ファイルのパス。優先順: QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv
fn config_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("QUICKLIME_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("config.tsv"))
}

/// キー割当の表記が文脈 (Ctrl 併用かどうか) に合う正しい形かどうか。
/// TSF 層のパース (tsf/src/config.cpp) と同じ規則
fn valid_key_notation(value: &str, ctrl: bool) -> bool {
    if value == "none" {
        return true;
    }
    let name = match (value.strip_prefix("Ctrl+"), ctrl) {
        (Some(rest), true) => rest,
        (None, false) => value,
        _ => return false,
    };
    if ctrl && name == "Backspace" {
        return true;
    }
    matches!(name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()), Some(1..=12))
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
            _ => {
                for (i, (name, _, ctrl)) in KEY_ITEMS.iter().enumerate() {
                    if key == *name && valid_key_notation(value, *ctrl) {
                        self.keys[i] = value.to_string();
                    }
                }
            }
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
        text.push_str("\n# 入力挙動\n");
        text.push_str(&format!("space\t{}\n", if self.space_full { "full" } else { "half" }));
        text.push_str(&format!("punctuation\t{}\n", self.punctuation));
        text.push_str(&format!("digits\t{}\n", if self.digits_full { "full" } else { "half" }));
        text.push_str("\n# 候補ウィンドウ\n");
        text.push_str(&format!("candidate_font\t{}\n", self.candidate_font));
        text.push_str(&format!("candidate_font_size\t{}\n", self.candidate_font_size));
        text.push_str("\n# キー割当\n");
        for (i, (name, _, _)) in KEY_ITEMS.iter().enumerate() {
            text.push_str(&format!("{}\t{}\n", name, self.keys[i]));
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

        // レイアウト (96dpi 基準の論理ピクセルを DPI でスケールする)
        let dpi = GetDpiForSystem();
        let scale = |v: i32| v * dpi as i32 / 96;
        let margin = scale(16);
        let row_h = scale(24);
        let row_gap = scale(8);
        let section_gap = scale(16);
        let label_w = scale(130);
        let ctrl_w = scale(170);
        let button_w = scale(88);
        let button_h = scale(28);
        let col_gap = scale(32);

        let left_w = label_w + row_gap + ctrl_w;
        let right_x = margin + left_w + col_gap;
        let right_w = label_w + row_gap + ctrl_w;
        let client_w = right_x + right_w + margin;
        // 左カラム: 見出し3 + 項目10行 + 見出し前の隙間、右カラム: 見出し1 + 10行。
        // 高さは行数の多い左カラム基準
        let left_rows = 13;
        let client_h =
            margin + left_rows * (row_h + row_gap) + section_gap * 2 + button_h + margin;

        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let mut rect = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: client_w,
            bottom: client_h,
        };
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let win_w = rect.right - rect.left;
        let win_h = rect.bottom - rect.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let title = wide("QuicklIME 設定");
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            (screen_w - win_w) / 2,
            (screen_h - win_h) / 2,
            win_w,
            win_h,
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
            ctrl
        };
        let label_style = WS_CHILD | WS_VISIBLE;
        let check_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32;
        let combo_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32;
        // コンボの高さはドロップダウン一覧の分を含める
        let combo_list_h = scale(280);

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

        // ---- 左カラム ----
        let ctrl_x = margin + label_w + row_gap;
        let mut y = margin;

        create_control("STATIC", "変換", label_style, 0, margin, y, left_w, row_h, 0);
        y += row_h + row_gap;
        let check = |text: &str, y: i32, id: i32, value: bool| {
            let ctrl = create_control("BUTTON", text, check_style, 0, margin, y, left_w, row_h, id);
            SendMessageW(ctrl, BM_SETCHECK, value as usize, 0);
        };
        check("学習する (確定した表記を優先する)", y, ID_CHECK_LEARNING, config.learning);
        y += row_h + row_gap;
        check("入力中にサジェストを表示する", y, ID_CHECK_SUGGEST, config.suggest);
        y += row_h + row_gap;
        check("タイプミス補正 (曖昧一致の補充)", y, ID_CHECK_TYPO, config.typo_correction);
        y += row_h + row_gap;
        create_control("STATIC", "サジェスト件数:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        let pred_items: Vec<String> = (1..=8).map(|n| n.to_string()).collect();
        let pred_refs: Vec<&str> = pred_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_MAX_PRED, &pred_refs, &config.max_predictions.to_string());
        y += row_h + row_gap;
        create_control("STATIC", "サジェスト開始文字数:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        let chars_items: Vec<String> = (1..=5).map(|n| n.to_string()).collect();
        let chars_refs: Vec<&str> = chars_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_MIN_CHARS, &chars_refs, &config.min_suggest_chars.to_string());

        y += row_h + row_gap + section_gap;
        create_control("STATIC", "入力", label_style, 0, margin, y, left_w, row_h, 0);
        y += row_h + row_gap;
        create_control("STATIC", "スペースキー:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        add_combo(
            ctrl_x,
            y,
            ID_COMBO_SPACE,
            &["全角スペース", "半角スペース"],
            if config.space_full { "全角スペース" } else { "半角スペース" },
        );
        y += row_h + row_gap;
        create_control("STATIC", "句読点:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        add_combo(ctrl_x, y, ID_COMBO_PUNCT, &PUNCT_ITEMS, &config.punctuation);
        y += row_h + row_gap;
        create_control("STATIC", "数字:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        add_combo(
            ctrl_x,
            y,
            ID_COMBO_DIGITS,
            &["半角", "全角"],
            if config.digits_full { "全角" } else { "半角" },
        );

        y += row_h + row_gap + section_gap;
        create_control("STATIC", "候補ウィンドウ", label_style, 0, margin, y, left_w, row_h, 0);
        y += row_h + row_gap;
        create_control("STATIC", "フォント:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
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
        create_control("STATIC", "サイズ:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        let size_items: Vec<String> = (10..=40).map(|n| n.to_string()).collect();
        let size_refs: Vec<&str> = size_items.iter().map(String::as_str).collect();
        add_combo(ctrl_x, y, ID_COMBO_FONT_SIZE, &size_refs, &config.candidate_font_size.to_string());

        // ---- 右カラム: キー割当 ----
        let key_ctrl_x = right_x + label_w + row_gap;
        let mut y = margin;
        create_control("STATIC", "キー割当", label_style, 0, right_x, y, right_w, row_h, 0);
        // 割当の選択肢 (Ctrl 併用の機能とそれ以外で異なる)
        let plain_items: Vec<String> =
            std::iter::once("none".to_string()).chain((1..=12).map(|n| format!("F{n}"))).collect();
        let ctrl_items: Vec<String> = std::iter::once("none".to_string())
            .chain(std::iter::once("Ctrl+Backspace".to_string()))
            .chain((1..=12).map(|n| format!("Ctrl+F{n}")))
            .collect();
        for (i, (_, label, ctrl)) in KEY_ITEMS.iter().enumerate() {
            y += row_h + row_gap;
            create_control("STATIC", &format!("{label}:"), label_style, 0, right_x, y + scale(3), label_w, row_h, 0);
            let items = if *ctrl { &ctrl_items } else { &plain_items };
            let refs: Vec<&str> = items.iter().map(String::as_str).collect();
            add_combo(key_ctrl_x, y, ID_COMBO_KEY_BASE + i as i32, &refs, &config.keys[i]);
        }

        // ---- 下部ボタン (右寄せ) ----
        let button_y = client_h - margin - button_h;
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
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
                if msg.wParam == VK_RETURN as usize {
                    SendMessageW(hwnd, WM_COMMAND, ID_BUTTON_SAVE as usize, 0);
                    continue;
                }
                if msg.wParam == VK_ESCAPE as usize {
                    DestroyWindow(hwnd);
                    continue;
                }
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
                    _ => {}
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

    // キー割当の重複を拒否する (無修飾と Ctrl 併用は別空間なので独立に確認)
    for (i, (_, label_a, ctrl_a)) in KEY_ITEMS.iter().enumerate() {
        for (j, (_, label_b, ctrl_b)) in KEY_ITEMS.iter().enumerate().skip(i + 1) {
            if ctrl_a == ctrl_b && config.keys[i] != "none" && config.keys[i] == config.keys[j] {
                message_box(
                    hwnd,
                    &format!(
                        "キー割当が重複しています: {} と {} ({})",
                        label_a, label_b, config.keys[j]
                    ),
                    MB_ICONWARNING,
                );
                return;
            }
        }
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
    let font = combo_text(ID_COMBO_FONT);
    if !font.is_empty() && font.encode_utf16().count() < 32 {
        config.candidate_font = font;
    }
    if let Ok(n) = combo_text(ID_COMBO_FONT_SIZE).parse::<u32>() {
        config.candidate_font_size = n.clamp(10, 40);
    }
    for (i, (_, _, ctrl)) in KEY_ITEMS.iter().enumerate() {
        let value = combo_text(ID_COMBO_KEY_BASE + i as i32);
        if valid_key_notation(&value, *ctrl) {
            config.keys[i] = value;
        }
    }
    config
}

fn message_box(hwnd: HWND, text: &str, icon: u32) {
    let text = wide(text);
    let title = wide("QuicklIME 設定");
    unsafe { MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}
