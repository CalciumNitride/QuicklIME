// Slint の外で windows-sys を直接使う処理 (キーの取り込み・ファイル選択・実行中のアプリの列挙・
// フォントの列挙・二重起動の防止・言語の一覧への追加)

use std::cell::RefCell;
use std::rc::Rc;
use std::ptr::{null, null_mut};

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FreeLibrary, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumFontFamiliesExW, GetDC, LOGFONTW, NEWTEXTMETRICW, NTM_PS_OPENTYPE, NTM_TT_OPENTYPE, ReleaseDC,
    TEXTMETRICW, TRUETYPE_FONTTYPE,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows_sys::Win32::UI::Input::Ime::{IACE_DEFAULT, ImmAssociateContextEx, ImmGetContext, ImmReleaseContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, FindWindowW, GW_OWNER, GetWindow,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SW_RESTORE,
    SetForegroundWindow, ShowWindow, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::apps::{exe_file_name, same_app};
use crate::keys::{KeyCombo, is_modifier_vk};

/// NUL 終端の UTF-16 文字列を作る
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Slint のウィンドウの HWND (winit のウィンドウがまだ無ければ None)
pub fn window_hwnd(window: &slint::Window) -> Option<HWND> {
    window
        .with_winit_window(|winit_window| match winit_window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
            _ => None,
        })
        .flatten()
}

/// ウィンドウを前面に出す (TSF 層から起動されたときに、起動元のアプリの後ろに隠れないように)
// HWND はポインタの形のハンドルで、API は参照外ししない (無効な値はエラーになるだけ)
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn bring_to_front(hwnd: HWND) {
    unsafe { SetForegroundWindow(hwnd) };
}

/// 二重起動の防止。名前付き mutex が既にあれば、タイトルが title のウィンドウを前面に出して
/// false を返す。mutex はプロセスの終了まで持ち続ける
pub fn acquire_single_instance(mutex_name: &str, title: &str) -> bool {
    let name = wide(mutex_name);
    unsafe {
        let mutex = CreateMutexW(null(), 0, name.as_ptr());
        if mutex.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
            return true;
        }
        CloseHandle(mutex);
        // winit のウィンドウクラス名は自分で決められないため、タイトルで探す
        let title = wide(title);
        let existing = FindWindowW(null(), title.as_ptr());
        if !existing.is_null() {
            if IsIconic(existing) != 0 {
                ShowWindow(existing, SW_RESTORE);
            }
            SetForegroundWindow(existing);
        }
        false
    }
}

/// 言語の一覧に足す入力プロファイル ("0x<LANGID>:{CLSID}{プロファイルの GUID}")。
/// CLSID とプロファイルの GUID は tsf/src/globals.cpp の kClsid・kProfileGuid と同じ値
const LANGUAGE_LIST_PROFILE: &str =
    "0x0411:{D8FA8028-4371-40E9-8F49-4E465ECE9A41}{C0730986-A430-4595-8D18-A4103718C6C6}";
const ILOT_INSTALL: u32 = 0x0000_0000;

/// input.dll の InstallLayoutOrTip
type InstallLayoutOrTipFn = unsafe extern "system" fn(profile: *const u16, flags: u32) -> i32;

/// 呼び出した利用者の言語の一覧 (日本語のキーボード) に QuicklIME を足す。一覧に日本語が無ければ
/// 日本語ごと足し、すでに入っていれば変わらない。足せたら true
pub fn add_to_language_list() -> bool {
    // input.dll は SDK に import library が無いため、実行時に System32 から読み込む
    let module = unsafe {
        LoadLibraryExW(wide("input.dll").as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32)
    };
    if module.is_null() {
        return false;
    }
    let result = match unsafe { GetProcAddress(module, c"InstallLayoutOrTip".as_ptr().cast()) } {
        Some(function) => {
            let install = unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, InstallLayoutOrTipFn>(function)
            };
            unsafe { install(wide(LANGUAGE_LIST_PROFILE).as_ptr(), ILOT_INSTALL) != 0 }
        }
        None => false,
    };
    unsafe { FreeLibrary(module) };
    result
}

/// インストール済みフォントの面名一覧 (縦書き用の @ 始まりと、ビットマップ・ベクタのフォントは除く)
pub fn font_families() -> Vec<String> {
    unsafe extern "system" fn enum_proc(
        lf: *const LOGFONTW,
        tm: *const TEXTMETRICW,
        font_type: u32,
        lparam: LPARAM,
    ) -> i32 {
        // ビットマップ・ベクタのフォントは入力パネル (DirectWrite) で描けない。CFF の OpenType は
        // TRUETYPE_FONTTYPE が立たないため ntmFlags で拾う。tm はどの種類のフォントでも
        // NEWTEXTMETRICW を先頭に持つ構造体を指す
        let ntm_flags = unsafe { (*(tm as *const NEWTEXTMETRICW)).ntmFlags };
        if font_type & TRUETYPE_FONTTYPE == 0 && ntm_flags & (NTM_PS_OPENTYPE | NTM_TT_OPENTYPE) == 0 {
            return 1;
        }
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

/// exe を選ぶファイル選択ダイアログ。選んだファイルのパスを返す (キャンセルなら None)
pub fn pick_exe_file(owner: HWND, title: &str) -> Option<String> {
    open_file_dialog(owner, title, "実行ファイル (*.exe)\0*.exe\0\0", 0)
}

/// インポートする辞書ファイルを選ぶファイル選択ダイアログ (キャンセルなら None)
pub fn pick_dictionary_file(owner: HWND, title: &str) -> Option<String> {
    open_file_dialog(
        owner,
        title,
        "テキスト (*.txt;*.tsv)\0*.txt;*.tsv\0すべてのファイル (*.*)\0*.*\0\0",
        OFN_HIDEREADONLY,
    )
}

/// filter は「表示名\0パターン\0...\0\0」の形
fn open_file_dialog(owner: HWND, title: &str, filter: &str, flags: u32) -> Option<String> {
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let title = wide(title);
    let mut file = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        nFilterIndex: 1,
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        lpstrTitle: title.as_ptr(),
        // 作業ディレクトリが変わると相対パスの環境変数指定 (QUICKLIME_*) の解決先がずれる
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | flags,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(String::from_utf16_lossy(&file[..length]))
}

// ---- 実行中のアプリの列挙 ----

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
pub fn visible_app_windows() -> Vec<(String, String)> {
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

// ---- キーの取り込み ----

/// 取り込み中に押されたキー
#[derive(Clone, Copy, Debug)]
pub enum CaptureEvent {
    Key(KeyCombo),
    /// IME が打鍵を処理した (VK_PROCESSKEY)
    ProcessedByIme,
    /// Win キーとの組み合わせ
    WithWin,
}

const CAPTURE_SUBCLASS_ID: usize = 1;
const VK_PROCESSKEY: u16 = 0xE5;

struct Capture {
    hwnd: HWND,
    /// 取り込みを始めた時点で IME が付いていたか (終わったらこの状態に戻す)
    ime_enabled: bool,
    handler: Rc<dyn Fn(CaptureEvent)>,
}

thread_local! {
    static CAPTURE: RefCell<Option<Capture>> = const { RefCell::new(None) };
}

/// hwnd のキー入力の取り込みを始める。押されたキーは handler に渡す (stop_key_capture まで続く)
// HWND はポインタの形のハンドルで、API は参照外ししない (無効な値はエラーになるだけ)
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn start_key_capture(hwnd: HWND, handler: impl Fn(CaptureEvent) + 'static) -> bool {
    stop_key_capture();
    unsafe {
        if SetWindowSubclass(hwnd, Some(capture_subclass_proc), CAPTURE_SUBCLASS_ID, 0) == 0 {
            return false;
        }
        let context = ImmGetContext(hwnd);
        let ime_enabled = !context.is_null();
        if ime_enabled {
            ImmReleaseContext(hwnd, context);
        }
        // IME が打鍵を食べると WM_KEYDOWN で元のキーが届かない
        ImmAssociateContextEx(hwnd, null_mut(), 0);
        CAPTURE.set(Some(Capture { hwnd, ime_enabled, handler: Rc::new(handler) }));
    }
    true
}

/// 取り込みを終え、IME を取り込みを始めた時点の状態に戻す。
/// winit はフォーカスのある部品に応じて IME を付け外ししており、常に付けて戻すと
/// ボタンにフォーカスがあるときにも IME が効いてしまう
pub fn stop_key_capture() {
    let Some(capture) = CAPTURE.take() else {
        return;
    };
    unsafe {
        RemoveWindowSubclass(capture.hwnd, Some(capture_subclass_proc), CAPTURE_SUBCLASS_ID);
        if capture.ime_enabled {
            ImmAssociateContextEx(capture.hwnd, null_mut(), IACE_DEFAULT);
        }
    }
}

/// 取り込み中に押されたキー。修飾キーの押し始めは、続けて押すキーを待つ
fn on_capture_key(vk: u16) {
    if is_modifier_vk(vk) {
        return;
    }
    let pressed = |key: i32| unsafe { (GetKeyState(key) as u16 & 0x8000) != 0 };
    let event = if vk == VK_PROCESSKEY {
        CaptureEvent::ProcessedByIme
    } else if pressed(0x5B) || pressed(0x5C) {
        CaptureEvent::WithWin
    } else {
        CaptureEvent::Key(KeyCombo { ctrl: pressed(0x11), alt: pressed(0x12), shift: pressed(0x10), vk })
    };
    // 修飾キーの状態は WM_KEYDOWN を処理している今読む必要があるが、画面の更新はウィンドウ
    // プロシージャの外 (イベントループ) で行う
    let _ = slint::invoke_from_event_loop(move || {
        // handler の中で stop_key_capture を呼べるよう、借用を外してから呼ぶ
        let handler = CAPTURE.with_borrow(|capture| capture.as_ref().map(|c| c.handler.clone()));
        if let Some(handler) = handler {
            handler(event);
        }
    });
}

// Slint のキーイベントでは Convert・NonConvert・VK_PROCESSKEY を確実には受けられないため、
// winit のウィンドウプロシージャより前で仮想キーを読む
unsafe extern "system" fn capture_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            on_capture_key(wparam as u16);
            0
        }
        // Alt・F10 を離したときのメニュー起動や、WM_SYSCHAR の警告音を出さない
        WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR => 0,
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}
