// QuicklIME 単語登録ツール
//
// ユーザ辞書へ1件登録する小さな Win32 ダイアログ。
// 入力 (単語・よみ・品詞) を ADDWORD としてエンジンへ送り、再起動なしで反映する。
// エンジンに接続できない場合は userdict.tsv へ直接追記する (反映はエンジン起動時)。
//
// 使い方: quicklime-regword.exe [単語の初期値] [よみの初期値]
//         (TSF 層が Ctrl+F7 で選択テキストを単語の初期値として起動する)

#![windows_subsystem = "windows"]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{COLOR_BTNFACE, CreateFontW};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 品詞コンボの項目 (ADDWORD の品詞フィールドにそのまま使う)
const POS_ITEMS: [&str; 8] = ["名詞", "固有名詞", "人名", "姓", "名", "地名", "組織", "短縮よみ"];

// コントロールID
const ID_EDIT_WORD: i32 = 100;
const ID_EDIT_READING: i32 = 101;
const ID_COMBO_POS: i32 = 102;
const ID_BUTTON_REGISTER: i32 = 110;
const ID_BUTTON_CANCEL: i32 = 111;

/// NUL 終端の UTF-16 文字列を作る
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    // コマンドライン引数: [単語の初期値] [よみの初期値]
    let args: Vec<String> =
        std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let initial_word = args.first().cloned().unwrap_or_default();
    let initial_reading = args.get(1).cloned().unwrap_or_default();

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class_name = wide("QuicklimeRegword");

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
        let margin = scale(12);
        let label_w = scale(52);
        let edit_w = scale(240);
        let row_h = scale(24);
        let row_gap = scale(8);
        let button_w = scale(88);
        let button_h = scale(28);

        let client_w = margin + label_w + row_gap + edit_w + margin;
        let client_h = margin + (row_h + row_gap) * 3 + button_h + margin;

        // クライアント領域からウィンドウ全体の大きさを求め、画面中央に置く
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let mut rect =
            windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let win_w = rect.right - rect.left;
        let win_h = rect.bottom - rect.top;
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let title = wide("QuicklIME 単語登録");
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
            0, 0, 0, 400, 0, 0, 0,
            1,        // DEFAULT_CHARSET
            0, 0, 0, 0,
            face.as_ptr(),
        );

        let create_control =
            |class: &str, text: &str, ctrl_style: u32, ex: u32, x: i32, y: i32, w: i32, h: i32, id: i32| {
                let class = wide(class);
                let text = wide(text);
                let ctrl = CreateWindowExW(
                    ex,
                    class.as_ptr(),
                    text.as_ptr(),
                    ctrl_style,
                    x, y, w, h,
                    hwnd,
                    id as usize as _,
                    instance,
                    null(),
                );
                SendMessageW(ctrl, WM_SETFONT, font as usize, 1);
                ctrl
            };

        let label_style = WS_CHILD | WS_VISIBLE;
        let edit_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32;

        // 1行目: 単語
        let mut y = margin;
        create_control("STATIC", "単語:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        let edit_word = create_control(
            "EDIT", &initial_word, edit_style, WS_EX_CLIENTEDGE,
            margin + label_w + row_gap, y, edit_w, row_h, ID_EDIT_WORD);

        // 2行目: よみ
        y += row_h + row_gap;
        create_control("STATIC", "よみ:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        create_control(
            "EDIT", &initial_reading, edit_style, WS_EX_CLIENTEDGE,
            margin + label_w + row_gap, y, edit_w, row_h, ID_EDIT_READING);

        // 3行目: 品詞 (ドロップダウンの高さは項目一覧の分を含める)
        y += row_h + row_gap;
        create_control("STATIC", "品詞:", label_style, 0, margin, y + scale(3), label_w, row_h, 0);
        let combo = create_control(
            "COMBOBOX", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
            0, margin + label_w + row_gap, y, edit_w, scale(240), ID_COMBO_POS);
        for item in POS_ITEMS {
            let item = wide(item);
            SendMessageW(combo, CB_ADDSTRING, 0, item.as_ptr() as LPARAM);
        }
        SendMessageW(combo, CB_SETCURSEL, 0, 0); // 既定は「名詞」

        // 4行目: ボタン (右寄せ)
        y += row_h + row_gap;
        let button_style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
        create_control(
            "BUTTON", "登録", button_style | BS_DEFPUSHBUTTON as u32, 0,
            client_w - margin - button_w * 2 - row_gap, y, button_w, button_h, ID_BUTTON_REGISTER);
        create_control(
            "BUTTON", "キャンセル", button_style, 0,
            client_w - margin - button_w, y, button_w, button_h, ID_BUTTON_CANCEL);

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        // 単語が引数で埋まっていれば「よみ」から入力を始める
        if initial_word.is_empty() {
            SetFocus(edit_word);
        } else {
            SetFocus(GetDlgItem(hwnd, ID_EDIT_READING));
        }

        let mut msg = std::mem::zeroed::<MSG>();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Enter は登録、Esc は閉じる (フォーカス位置によらない)
            if msg.message == WM_KEYDOWN {
                if msg.wParam == VK_RETURN as usize {
                    SendMessageW(hwnd, WM_COMMAND, ID_BUTTON_REGISTER as usize, 0);
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
                    ID_BUTTON_REGISTER => on_register(hwnd),
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

/// 登録ボタン: 入力を検証してエンジンへ ADDWORD を送る。
/// 成功したらダイアログを閉じ、失敗したらメッセージを出して開いたままにする
fn on_register(hwnd: HWND) {
    let word = get_text(hwnd, ID_EDIT_WORD).trim().to_string();
    let reading = to_hiragana(get_text(hwnd, ID_EDIT_READING).trim());
    let pos = unsafe {
        let combo = GetDlgItem(hwnd, ID_COMBO_POS);
        let index = SendMessageW(combo, CB_GETCURSEL, 0, 0);
        POS_ITEMS.get(index as usize).copied().unwrap_or("名詞")
    };

    if word.is_empty() || reading.is_empty() {
        message_box(hwnd, "単語とよみを入力してください。", MB_ICONWARNING);
        return;
    }

    match register(&reading, &word, pos) {
        Ok(note) => {
            let text = match note {
                Some(note) => format!("登録しました: {reading} → {word} ({pos})\n\n{note}"),
                None => format!("登録しました: {reading} → {word} ({pos})"),
            };
            message_box(hwnd, &text, MB_ICONINFORMATION);
            unsafe { DestroyWindow(hwnd) };
        }
        Err(e) => message_box(hwnd, &format!("登録できませんでした。\n{e}"), MB_ICONWARNING),
    }
}

/// エンジンへ ADDWORD を送る。接続できないときはユーザ辞書ファイルへ直接追記する。
/// 成功時は補足メッセージ (あれば) を返す
fn register(reading: &str, word: &str, pos: &str) -> Result<Option<String>, String> {
    match send_addword(reading, word, pos) {
        Ok(()) => Ok(None),
        Err(SendError::Refused(message)) => Err(message),
        Err(SendError::NotConnected) => {
            // エンジンが起動していない (IME 未使用など)。ファイルへ追記しておけば
            // 次のエンジン起動時に読み込まれる
            append_to_userdict(reading, word, pos)?;
            Ok(Some("エンジンが起動していないため、ユーザ辞書ファイルへ追記しました。\n\
                     反映は次回のエンジン起動時になります。".to_string()))
        }
    }
}

enum SendError {
    /// エンジンに接続できない (未起動)
    NotConnected,
    /// エンジンが登録を拒否した (重複など)
    Refused(String),
}

/// named pipe でエンジンに ADDWORD を送り、応答を確認する
fn send_addword(reading: &str, word: &str, pos: &str) -> Result<(), SendError> {
    let name =
        std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| "quicklime-engine".to_string());
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\{name}"))
        .map_err(|_| SendError::NotConnected)?;
    writeln!(pipe, "ADDWORD\t{reading}\t{word}\t{pos}")
        .map_err(|_| SendError::NotConnected)?;
    let mut response = String::new();
    BufReader::new(pipe)
        .read_line(&mut response)
        .map_err(|_| SendError::NotConnected)?;
    if response.starts_with("OK") {
        Ok(())
    } else {
        let message = response.trim().strip_prefix("ERR\t").unwrap_or("不明な応答").to_string();
        Err(SendError::Refused(message))
    }
}

/// ユーザ辞書ファイルへ直接追記する (エンジン未起動時のフォールバック)。
/// パスの決定はエンジン (userdict.rs) と同じ:
/// QUICKLIME_USER_DICT_FILE > %APPDATA%\QuicklIME\userdict.tsv
fn append_to_userdict(reading: &str, word: &str, pos: &str) -> Result<(), String> {
    let path = if let Ok(path) = std::env::var("QUICKLIME_USER_DICT_FILE") {
        PathBuf::from(path)
    } else {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
        PathBuf::from(appdata).join("QuicklIME").join("userdict.tsv")
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{reading}\t{word}\t{pos}"))
        .map_err(|e| format!("ユーザ辞書ファイルへ書き込めません ({e})"))
}

/// コントロールのテキストを取得する
fn get_text(hwnd: HWND, id: i32) -> String {
    unsafe {
        let ctrl = GetDlgItem(hwnd, id);
        let length = GetWindowTextLengthW(ctrl);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(ctrl, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..copied as usize])
    }
}

fn message_box(hwnd: HWND, text: &str, icon: u32) {
    let text = wide(text);
    let title = wide("QuicklIME 単語登録");
    unsafe { MessageBoxW(hwnd, text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

/// よみのカタカナをひらがなへ正規化する (変換時の読みはひらがなのため)
fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| {
            // カタカナ (ァ U+30A1 〜 ヶ U+30F6) はひらがなと 0x60 差で並んでいる
            if ('ァ'..='ヶ').contains(&c) {
                char::from_u32(c as u32 - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}
