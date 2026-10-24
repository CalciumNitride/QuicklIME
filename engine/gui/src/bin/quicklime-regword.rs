// QuicklIME 単語登録ツール
//
// ユーザ辞書へ1件登録する小さなウィンドウ。入力 (単語・よみ・品詞) を ADDWORD としてエンジンへ送り、
// 再起動なしで反映する。エンジンに接続できない場合は userdict.tsv へ直接追記する。
// 「インポート...」で他 IME (MS-IME / ATOK / Mozc・Google 日本語入力) のユーザ辞書を取り込む。
//
// 使い方: quicklime-regword.exe [単語の初期値] [よみの初期値]
//         (TSF 層が Ctrl+F7 で選択テキストを単語の初期値として起動する)

#![windows_subsystem = "windows"]

use std::path::Path;
use std::ptr::null_mut;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use quicklime_gui::import;
use quicklime_gui::regword::{POS_ITEMS, import_file, register};
use quicklime_gui::win32;

slint::include_modules!();

/// 登録できてからウィンドウを閉じるまでの時間 (成功の InfoBar を読めるように)
const CLOSE_DELAY: Duration = Duration::from_millis(1500);

fn show_info(window: &RegwordWindow, severity: InfoBarSeverity, title: &str, message: &str) {
    window.set_info_severity(severity);
    window.set_info_title(title.into());
    window.set_info_message(message.into());
}

/// [登録]: 入力を検証してエンジンへ送る。補足が無ければ成功を見せてから閉じ、
/// 補足や失敗があれば InfoBar に出して開いたままにする
fn on_register(window: &RegwordWindow, close_timer: &Timer) {
    let word = window.get_word().trim().to_string();
    let reading = import::to_hiragana(window.get_reading().trim());
    let pos = usize::try_from(window.get_pos_index())
        .ok()
        .and_then(|i| POS_ITEMS.get(i).copied())
        .unwrap_or("名詞");

    if word.is_empty() || reading.is_empty() {
        show_info(window, InfoBarSeverity::Warning, "単語とよみを入力してください", "");
        return;
    }

    let registered = format!("{reading} → {word} ({pos})");
    match register(&reading, &word, pos) {
        Ok(None) => {
            show_info(window, InfoBarSeverity::Success, "登録しました", &registered);
            window.set_closing(true);
            let weak = window.as_weak();
            close_timer.start(TimerMode::SingleShot, CLOSE_DELAY, move || {
                if let Some(window) = weak.upgrade() {
                    let _ = window.hide();
                }
            });
        }
        Ok(Some(note)) => {
            show_info(window, InfoBarSeverity::Success, "登録しました", &format!("{registered}\n{note}"));
        }
        Err(e) => show_info(window, InfoBarSeverity::Error, "登録できませんでした", &e),
    }
}

/// [インポート...]: ファイルを選ばせて取り込み、結果を InfoBar に出す。
/// 入力途中の登録内容を消さないよう、ウィンドウは閉じない
fn on_import(window: &RegwordWindow) {
    let owner = win32::window_hwnd(window.window()).unwrap_or(null_mut());
    let Some(source) = win32::pick_dictionary_file(owner, "インポートする辞書ファイル") else {
        return;
    };
    match import_file(Path::new(&source)) {
        Ok(message) => show_info(window, InfoBarSeverity::Success, "辞書をインポートしました", &message),
        Err(e) => show_info(window, InfoBarSeverity::Error, "インポートできませんでした", &e),
    }
}

fn main() -> Result<(), slint::PlatformError> {
    // コマンドライン引数: [単語の初期値] [よみの初期値]
    let args: Vec<String> =
        std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let initial_word = args.first().cloned().unwrap_or_default();
    let initial_reading = args.get(1).cloned().unwrap_or_default();

    // winit 以外のバックエンドでは HWND を取れない (前面に出す・ファイル選択に使う)
    slint::BackendSelector::new().backend_name("winit".into()).select()?;

    let window = RegwordWindow::new()?;
    let pos: Vec<SharedString> = POS_ITEMS.iter().map(|&p| p.into()).collect();
    window.set_pos_choices(ModelRc::new(VecModel::from(pos)));
    window.set_pos_index(0);
    window.set_word(initial_word.as_str().into());
    window.set_reading(initial_reading.as_str().into());

    let close_timer = Timer::default();
    let weak = window.as_weak();
    window.on_register(move || {
        if let Some(window) = weak.upgrade() {
            on_register(&window, &close_timer);
        }
    });

    let weak = window.as_weak();
    window.on_import(move || {
        if let Some(window) = weak.upgrade() {
            on_import(&window);
        }
    });

    let weak = window.as_weak();
    window.on_cancel(move || {
        if let Some(window) = weak.upgrade() {
            let _ = window.hide();
        }
    });

    window.show()?;
    // winit のウィンドウはイベントループが始まってから作られるため、show() の直後には HWND が無い
    let weak = window.as_weak();
    Timer::single_shot(Duration::ZERO, move || {
        if let Some(hwnd) = weak.upgrade().and_then(|window| win32::window_hwnd(window.window())) {
            win32::bring_to_front(hwnd);
        }
    });
    // 単語が引数で埋まっていれば「よみ」から入力を始める
    if initial_word.is_empty() {
        window.invoke_focus_word();
    } else {
        window.invoke_focus_reading();
    }
    slint::run_event_loop()
}
