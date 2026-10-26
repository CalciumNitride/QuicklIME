// QuicklIME 設定ツール
//
// config.tsv (エンジンと TSF 層が読む共通の設定ファイル) を編集する。値を変えるたびに
// 300 ms まとめてから書き、エンジンへ RELOADCONFIG を送って即時反映する
// (エンジン未起動時はファイルへの書き込みのみ。TSF 層は次のフォーカス切替時に
// 更新時刻の変化を検知して自動反映する)。
//
// 使い方: quicklime-config.exe (引数なし。TSF 層が Ctrl+F12 などで起動する)
//         quicklime-config.exe --add-language-list
//         (画面を出さずに、起動した利用者の言語の一覧に QuicklIME を足して終わる。インストーラが使う)

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::ptr::null_mut;
use std::rc::Rc;
use std::time::Duration;

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer,
    TimerMode, VecModel,
};

use quicklime_gui::apps::{
    AppSetting, IME_START_LABELS, add_app_setting, app_summary, exe_file_name, merge_app_settings,
    running_app_choices, split_app_settings,
};
use quicklime_gui::config::{
    CANDIDATE_POSITIONS, Config, FONT_SIZE_RANGE, INPUT_BUFFERS, LLM_BACKENDS, MAX_PRED_ITEMS,
    MIN_SUGGEST_CHARS_RANGE, PUNCT_ITEMS, apply_input_buffer_choice, default_debug_log_path,
    input_buffer_choice, theme_choices, theme_dir, user_theme_names,
};
use quicklime_gui::keys::{
    KEY_ITEMS, KeyAssign, KeyCombo, STATES, check_captured_key, default_key_assigns,
    effective_keys, find_key_conflict, working_states,
};
use quicklime_gui::preview::{self, Preview, Sample};
use quicklime_gui::win32::{self, CaptureEvent};

slint::include_modules!();

const WINDOW_TITLE: &str = "QuicklIME の設定";
const MUTEX_NAME: &str = "QuicklimeConfig";
/// 値を変えてから書き込むまでの時間。コンボを続けて切り替えたときに、書き込みと
/// LLM の子プロセスの起動・終了を繰り返さないため
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// 「見た目」ページの番号 (settings.slint の pages の並び)
const APPEARANCE_PAGE: i32 = 3;

/// スペース (space) の選択肢。先頭が full
const SPACE_ITEMS: [&str; 2] = ["全角スペース", "半角スペース"];
/// 数字 (digits) の選択肢。先頭が half
const DIGITS_ITEMS: [&str; 2] = ["半角", "全角"];

/// 画面の状態と、書き込む設定のもと
struct State {
    window: slint::Weak<SettingsWindow>,
    /// 画面で変えた値 (キー割当・アプリ・入力中の読み・診断ログは別に持ち、書くときに入れる)
    config: Config,
    /// 読み込んだ panel_input (「入力パネルだけに出す」ではこれをそのまま書く)
    loaded_panel_input: bool,
    /// 「入力中の読み」の選択肢 (INPUT_BUFFERS の添字)
    input_buffer: usize,
    /// 「入力パネルのテーマ」の選択肢 ((panel_theme の値, 表示名))
    themes: Vec<(String, String)>,
    fonts: Vec<String>,
    debug_log: bool,
    /// 確定した診断ログの出力先 (入力欄の打鍵ごとには変えない)
    debug_log_path: String,
    keys: Vec<KeyAssign>,
    /// 機能ごとの、キーを足せなかった理由
    key_messages: Vec<String>,
    key_model: Rc<VecModel<KeyFunction>>,
    /// 取り込み中の (機能, 行)。行は 0 = 状態ごとに変えないときの行、1 以降 = 働く状態の順
    capture: Option<(usize, usize)>,
    apps: Vec<AppSetting>,
    app_model: Rc<VecModel<AppEntry>>,
    /// [実行中のアプリから追加] の一覧 ((exe 名, ウィンドウタイトル))
    running: Vec<(String, String)>,
    /// 書いていない変更がある
    dirty: bool,
    save_timer: Timer,
}

type Shared = Rc<RefCell<State>>;

fn string_model<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(|item| item.as_ref().into()).collect();
    ModelRc::new(VecModel::from(items))
}

fn index_of<T: PartialEq>(items: impl IntoIterator<Item = T>, value: T) -> usize {
    items.into_iter().position(|item| item == value).unwrap_or(0)
}

/// 機能 index の行 row (KeyCard の行) が表す状態。0 は状態ごとに変えないときの行で None
fn row_state(index: usize, row: usize) -> Option<usize> {
    row.checked_sub(1).and_then(|r| working_states(index).get(r).copied())
}

fn is_per_state(assign: &KeyAssign) -> bool {
    assign.overrides.iter().any(Option::is_some)
}

fn notations(keys: &[KeyCombo]) -> ModelRc<SharedString> {
    string_model(keys.iter().map(KeyCombo::notation))
}

impl State {
    fn new(window: &SettingsWindow, config: Config) -> Self {
        let mut fonts = win32::font_families();
        if !fonts.contains(&config.candidate_font) {
            // 設定値が列挙に無いフォントでも選び直さずに残せるよう足しておく
            fonts.push(config.candidate_font.clone());
        }
        fonts.sort_by_key(|name| name.to_lowercase());
        let themes = theme_choices(
            theme_dir().map(|dir| user_theme_names(&dir)).unwrap_or_default(),
            &config.panel_theme,
        );
        let apps = merge_app_settings(&config.newline_apps, &config.ime_start_apps, &config.no_learning_apps);
        let keys = config.keys.clone();
        State {
            window: window.as_weak(),
            loaded_panel_input: config.panel_input,
            input_buffer: input_buffer_choice(config.panel_buffer, config.panel_input),
            themes,
            fonts,
            debug_log: !config.debug_log.is_empty(),
            debug_log_path: if config.debug_log.is_empty() {
                default_debug_log_path()
            } else {
                config.debug_log.clone()
            },
            key_messages: vec![String::new(); KEY_ITEMS.len()],
            keys,
            key_model: Rc::new(VecModel::default()),
            capture: None,
            apps,
            app_model: Rc::new(VecModel::default()),
            running: Vec::new(),
            dirty: false,
            save_timer: Timer::default(),
            config,
        }
    }

    /// 画面に値と選択肢を入れる
    fn fill_window(&self, window: &SettingsWindow) {
        let c = &self.config;
        window.set_version(env!("CARGO_PKG_VERSION").into());

        window.set_learning(c.learning);
        window.set_typo_correction(c.typo_correction);
        window.set_segment_ui(c.segment_ui);
        window.set_llm(c.llm);
        window.set_llm_backend_choices(string_model(LLM_BACKENDS.iter().map(|(_, label)| label)));
        window.set_llm_backend_index(
            index_of(LLM_BACKENDS.iter().map(|(name, _)| *name), c.llm_backend.as_str()) as i32,
        );

        window.set_candidate_bar(c.candidate_bar);
        window.set_suggest(c.suggest);
        window.set_next_word(c.next_word);
        window.set_max_predictions_choices(string_model(MAX_PRED_ITEMS.iter().map(u32::to_string)));
        window.set_max_predictions_index(index_of(MAX_PRED_ITEMS, c.max_predictions) as i32);
        window.set_min_suggest_chars_choices(string_model(MIN_SUGGEST_CHARS_RANGE.map(|n| n.to_string())));
        window.set_min_suggest_chars_index(index_of(MIN_SUGGEST_CHARS_RANGE, c.min_suggest_chars) as i32);
        window.set_candidate_position_choices(string_model(
            CANDIDATE_POSITIONS.iter().map(|(_, label)| label),
        ));
        window.set_candidate_position_index(index_of(
            CANDIDATE_POSITIONS.iter().map(|(name, _)| *name),
            c.candidate_position.as_str(),
        ) as i32);

        window.set_input_buffer_choices(string_model(INPUT_BUFFERS.iter().map(|(label, _, _)| label)));
        window.set_input_buffer_index(self.input_buffer as i32);
        window.set_space_choices(string_model(SPACE_ITEMS));
        window.set_space_index(if c.space_full { 0 } else { 1 });
        window.set_punctuation_choices(string_model(PUNCT_ITEMS));
        window.set_punctuation_index(index_of(PUNCT_ITEMS, c.punctuation.as_str()) as i32);
        window.set_digits_choices(string_model(DIGITS_ITEMS));
        window.set_digits_index(if c.digits_full { 1 } else { 0 });
        window.set_modeless(c.modeless);
        window.set_input_scope_passthrough(c.input_scope_passthrough);

        window.set_theme_choices(string_model(self.themes.iter().map(|(_, label)| label)));
        window.set_theme_index(
            index_of(self.themes.iter().map(|(name, _)| name.as_str()), c.panel_theme.as_str()) as i32,
        );
        window.set_font_choices(string_model(&self.fonts));
        window.set_font_index(index_of(self.fonts.iter(), &c.candidate_font) as i32);
        window.set_font_size_choices(string_model(FONT_SIZE_RANGE.map(|n| n.to_string())));
        window.set_font_size_index(index_of(FONT_SIZE_RANGE, c.candidate_font_size) as i32);
        window.set_mode_popup(c.mode_popup);

        window.set_debug_log(self.debug_log);
        window.set_debug_log_path(self.debug_log_path.as_str().into());

        let rows: Vec<KeyFunction> = (0..KEY_ITEMS.len()).map(|i| self.key_function(i)).collect();
        self.key_model.set_vec(rows);
        window.set_key_functions(ModelRc::from(self.key_model.clone()));
        window.set_key_conflict_warning(find_key_conflict(&self.keys).unwrap_or_default().into());

        let rows: Vec<AppEntry> = self.apps.iter().map(app_entry).collect();
        self.app_model.set_vec(rows);
        window.set_apps(ModelRc::from(self.app_model.clone()));
    }

    /// スイッチ・コンボの値を画面から読む
    fn read_window(&mut self, window: &SettingsWindow) {
        let index = |value: i32| usize::try_from(value).unwrap_or(0);
        let c = &mut self.config;
        c.learning = window.get_learning();
        c.typo_correction = window.get_typo_correction();
        c.segment_ui = window.get_segment_ui();
        c.llm = window.get_llm();
        if let Some((name, _)) = LLM_BACKENDS.get(index(window.get_llm_backend_index())) {
            c.llm_backend = name.to_string();
        }

        c.candidate_bar = window.get_candidate_bar();
        c.suggest = window.get_suggest();
        c.next_word = window.get_next_word();
        if let Some(&n) = MAX_PRED_ITEMS.get(index(window.get_max_predictions_index())) {
            c.max_predictions = n;
        }
        if let Some(n) = MIN_SUGGEST_CHARS_RANGE.clone().nth(index(window.get_min_suggest_chars_index())) {
            c.min_suggest_chars = n;
        }
        if let Some((name, _)) = CANDIDATE_POSITIONS.get(index(window.get_candidate_position_index())) {
            c.candidate_position = name.to_string();
        }

        let buffer = index(window.get_input_buffer_index());
        if buffer < INPUT_BUFFERS.len() {
            self.input_buffer = buffer;
        }
        c.space_full = window.get_space_index() == 0;
        if let Some(punct) = PUNCT_ITEMS.get(index(window.get_punctuation_index())) {
            c.punctuation = punct.to_string();
        }
        c.digits_full = window.get_digits_index() == 1;
        c.modeless = window.get_modeless();
        c.input_scope_passthrough = window.get_input_scope_passthrough();

        if let Some((name, _)) = self.themes.get(index(window.get_theme_index())) {
            c.panel_theme = name.clone();
        }
        if let Some(font) = self.fonts.get(index(window.get_font_index())) {
            if quicklime_gui::config::is_valid_font_name(font) {
                c.candidate_font = font.clone();
            }
        }
        if let Some(n) = FONT_SIZE_RANGE.clone().nth(index(window.get_font_size_index())) {
            c.candidate_font_size = n;
        }
        c.mode_popup = window.get_mode_popup();

        self.debug_log = window.get_debug_log();
    }

    /// 書き込む設定
    fn config_to_save(&self) -> Config {
        let mut config = self.config.clone();
        config.panel_input = self.loaded_panel_input;
        apply_input_buffer_choice(&mut config, self.input_buffer);
        config.keys = self.keys.clone();
        (config.newline_apps, config.ime_start_apps, config.no_learning_apps) =
            split_app_settings(&self.apps);
        config.debug_log = if self.debug_log {
            let path = self.debug_log_path.trim();
            if path.is_empty() { default_debug_log_path() } else { path.to_string() }
        } else {
            String::new()
        };
        config
    }

    /// 書いていない変更があれば書く。失敗したらウィンドウ上部にエラーを出し、書けたら消す
    fn flush(&mut self) {
        self.save_timer.stop();
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let error = self.config_to_save().save().err().unwrap_or_default();
        if let Some(window) = self.window.upgrade() {
            window.set_save_error(error.into());
        }
    }

    // ---- 入力パネルのプレビュー ----

    /// 見本を描く元の値 (テーマ・フォント・サイズ)
    fn preview_source(&self) -> (String, String, u32) {
        let c = &self.config;
        (c.panel_theme.clone(), c.candidate_font.clone(), c.candidate_font_size)
    }

    /// 入力パネルの見本を、画面で選んでいる (保存前の) テーマ・フォント・サイズで描き直す。
    /// 描けなければ見本の代わりに理由を出す
    fn refresh_preview(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let (theme, font, size) = self.preview_source();
        let render = |sample| preview::render(&theme, &font, size, sample);
        match render(Sample::Bar).and_then(|bar| Ok((bar, render(Sample::Conversion)?))) {
            Ok((bar, conversion)) => {
                window.set_preview_bar(preview_image(&bar));
                window.set_preview_conversion(preview_image(&conversion));
                window.set_preview_dark(bar.dark);
                window.set_preview_blur(bar.blur);
                window.set_preview_error(SharedString::default());
            }
            Err(message) => window.set_preview_error(message.into()),
        }
    }

    // ---- キー割当 ----

    fn key_function(&self, index: usize) -> KeyFunction {
        let assign = &self.keys[index];
        let per_state = is_per_state(assign);
        let states = working_states(index);
        let state_rows: Vec<KeyStateRow> = if per_state {
            states
                .iter()
                .map(|&s| KeyStateRow {
                    label: STATES[s].1.into(),
                    keys: notations(effective_keys(&self.keys, index, s).unwrap_or_default()),
                })
                .collect()
        } else {
            Vec::new()
        };
        KeyFunction {
            name: KEY_ITEMS[index].1.into(),
            states: states.iter().map(|&s| STATES[s].1).collect::<Vec<_>>().join("・").into(),
            keys: if per_state { notations(&[]) } else { notations(&assign.keys) },
            per_state,
            state_rows: ModelRc::new(VecModel::from(state_rows)),
            message: self.key_messages[index].as_str().into(),
            is_default: *assign == default_key_assigns()[index],
        }
    }

    /// 機能 index のキー割当を変えた後に、画面と書き込みを合わせる
    fn keys_changed(&mut self, index: Option<usize>) {
        match index {
            Some(i) => self.key_model.set_row_data(i, self.key_function(i)),
            None => {
                for i in 0..KEY_ITEMS.len() {
                    self.key_model.set_row_data(i, self.key_function(i));
                }
            }
        }
        if let Some(window) = self.window.upgrade() {
            window.set_key_conflict_warning(find_key_conflict(&self.keys).unwrap_or_default().into());
        }
        self.dirty = true;
    }

    fn set_key_message(&mut self, index: usize, message: String) {
        if self.key_messages[index] != message {
            self.key_messages[index] = message;
            self.key_model.set_row_data(index, self.key_function(index));
        }
    }

    /// 機能 index の行 row のキー一覧 (状態ごとの行で上書きが無ければ、今効いているキーで作る)
    fn row_keys_mut(&mut self, index: usize, row: usize) -> Option<&mut Vec<KeyCombo>> {
        let assign = &mut self.keys[index];
        if row == 0 {
            return Some(&mut assign.keys);
        }
        let state = row_state(index, row)?;
        let base = assign.keys.clone();
        Some(assign.overrides[state].get_or_insert(base))
    }

    fn stop_capture(&mut self) {
        win32::stop_key_capture();
        self.capture = None;
        if let Some(window) = self.window.upgrade() {
            window.set_capture_function(-1);
            window.set_capture_row(-1);
        }
    }

    fn cancel_capture_of(&mut self, index: usize) {
        if self.capture.is_some_and(|(f, _)| f == index) {
            self.stop_capture();
        }
    }

    /// 取り込んだキー。足せないキーは理由を出して取り込みを続ける
    fn on_captured(&mut self, event: CaptureEvent) {
        let Some((index, row)) = self.capture else {
            return;
        };
        // Esc は取り込みをやめる操作に使う (Ctrl と組み合わせない Esc は元から割り当てられないので失うものがない)
        if let CaptureEvent::Key(KeyCombo { ctrl: false, vk: 0x1B, .. }) = event {
            self.stop_capture();
            self.set_key_message(index, String::new());
            return;
        }
        let message = match event {
            CaptureEvent::ProcessedByIme => "IME がこのキーを処理したため取り込めません".to_string(),
            CaptureEvent::WithWin => "Win キーとの組み合わせは割り当てられません".to_string(),
            CaptureEvent::Key(key) => {
                let states = match row_state(index, row) {
                    Some(state) => vec![state],
                    None => working_states(index),
                };
                match check_captured_key(&self.keys, index, &states, &key) {
                    Err(message) => message,
                    Ok(()) => {
                        if let Some(keys) = self.row_keys_mut(index, row) {
                            if !keys.contains(&key) {
                                keys.push(key);
                            }
                        }
                        self.stop_capture();
                        self.key_messages[index].clear();
                        self.keys_changed(Some(index));
                        return;
                    }
                }
            }
        };
        self.set_key_message(index, message);
    }

    // ---- アプリ ----

    fn add_app(&mut self, exe: &str) {
        let before = self.apps.len();
        let index = add_app_setting(&mut self.apps, exe);
        if index == before {
            self.app_model.push(app_entry(&self.apps[index]));
            self.dirty = true;
        }
    }

    fn app_changed(&mut self, index: usize, change: impl FnOnce(&mut AppSetting)) {
        let Some(app) = self.apps.get_mut(index) else {
            return;
        };
        change(app);
        self.app_model.set_row_data(index, app_entry(app));
        self.dirty = true;
    }
}

fn preview_image(preview: &Preview) -> Image {
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&preview.rgba, preview.width, preview.height);
    Image::from_rgba8_premultiplied(buffer)
}

fn app_entry(app: &AppSetting) -> AppEntry {
    AppEntry {
        exe: app.exe.as_str().into(),
        summary: app_summary(app).into(),
        newline: app.newline,
        ime_start: IME_START_LABELS.iter().position(|(s, _)| *s == app.ime_start).unwrap_or(0) as i32,
        no_learning: app.no_learning,
    }
}

/// 変更を書き込みの待ちに入れる (SAVE_DELAY の間に次の変更があれば待ち直す)
fn schedule_save(state: &Shared) {
    let s = state.borrow_mut();
    if !s.dirty {
        return;
    }
    let weak = Rc::downgrade(state);
    s.save_timer.start(TimerMode::SingleShot, SAVE_DELAY, move || {
        if let Some(state) = weak.upgrade() {
            state.borrow_mut().flush();
        }
    });
}

/// 「テーマのフォルダを開く」: フォルダが無ければ作ってからエクスプローラーで開く。
/// 失敗したらウィンドウ上部にエラーを出し、開けたら消す
fn open_theme_dir(window: &SettingsWindow) {
    let result = theme_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("テーマのフォルダを作れません ({e})\n{}", dir.display()))?;
        std::process::Command::new("explorer.exe")
            .arg(&dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("テーマのフォルダを開けません ({e})\n{}", dir.display()))
    });
    window.set_theme_dir_error(result.err().unwrap_or_default().into());
}

fn connect(window: &SettingsWindow, state: &Shared) {
    let s = state.clone();
    let weak = window.as_weak();
    window.on_settings_changed(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        {
            let mut s = s.borrow_mut();
            let before = s.preview_source();
            s.read_window(&window);
            s.dirty = true;
            if s.preview_source() != before {
                s.refresh_preview();
            }
        }
        schedule_save(&s);
    });

    let s = state.clone();
    let weak = window.as_weak();
    window.on_debug_log_path_committed(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        {
            let mut s = s.borrow_mut();
            let path = window.get_debug_log_path().to_string();
            if path == s.debug_log_path {
                return;
            }
            s.debug_log_path = path;
            s.dirty = true;
        }
        schedule_save(&s);
    });

    let weak = window.as_weak();
    window.on_open_theme_dir(move || {
        if let Some(window) = weak.upgrade() {
            open_theme_dir(&window);
        }
    });

    // 取り込み中にページを移ったら、取り込みをやめる
    let s = state.clone();
    let weak = window.as_weak();
    window.on_page_changed(move || {
        let mut st = s.borrow_mut();
        st.stop_capture();
        if weak.upgrade().is_some_and(|window| window.get_current_page() == APPEARANCE_PAGE) {
            st.refresh_preview();
        }
    });

    // ---- キー割当 ----

    let s = state.clone();
    let weak = window.as_weak();
    window.on_add_key(move |function, row| {
        let (Ok(index), Ok(row)) = (usize::try_from(function), usize::try_from(row)) else {
            return;
        };
        let Some(window) = weak.upgrade() else {
            return;
        };
        let mut st = s.borrow_mut();
        st.stop_capture();
        let handler_state = Rc::downgrade(&s);
        let started = win32::window_hwnd(window.window()).is_some_and(|hwnd| {
            win32::start_key_capture(hwnd, move |event| {
                if let Some(state) = handler_state.upgrade() {
                    state.borrow_mut().on_captured(event);
                    schedule_save(&state);
                }
            })
        });
        if !started {
            st.set_key_message(index, "キーの取り込みを始められませんでした".to_string());
            return;
        }
        st.capture = Some((index, row));
        st.set_key_message(index, String::new());
        window.set_capture_function(function);
        window.set_capture_row(row as i32);
    });

    let s = state.clone();
    window.on_cancel_capture(move || s.borrow_mut().stop_capture());

    let s = state.clone();
    window.on_remove_key(move |function, row, key| {
        let (Ok(index), Ok(row), Ok(key)) =
            (usize::try_from(function), usize::try_from(row), usize::try_from(key))
        else {
            return;
        };
        {
            let mut st = s.borrow_mut();
            let Some(keys) = st.row_keys_mut(index, row) else {
                return;
            };
            if key >= keys.len() {
                return;
            }
            keys.remove(key);
            st.key_messages[index].clear();
            st.keys_changed(Some(index));
        }
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_per_state_changed(move |function, on| {
        let Ok(index) = usize::try_from(function) else {
            return;
        };
        {
            let mut st = s.borrow_mut();
            // 行の並びが変わるので、この機能の取り込みはやめる
            st.cancel_capture_of(index);
            let assign = &mut st.keys[index];
            if on {
                // 状態ごとの行は、今の割当を写して始める (オンにしただけでは動作を変えない)
                for state in working_states(index) {
                    if assign.overrides[state].is_none() {
                        assign.overrides[state] = Some(assign.keys.clone());
                    }
                }
            } else {
                assign.overrides = Default::default();
            }
            st.key_messages[index].clear();
            st.keys_changed(Some(index));
        }
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_reset_keys(move |function| {
        let Ok(index) = usize::try_from(function) else {
            return;
        };
        {
            let mut st = s.borrow_mut();
            st.cancel_capture_of(index);
            st.keys[index] = default_key_assigns().swap_remove(index);
            st.key_messages[index].clear();
            st.keys_changed(Some(index));
        }
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_reset_all_keys(move || {
        {
            let mut st = s.borrow_mut();
            st.stop_capture();
            st.keys = default_key_assigns();
            st.key_messages.iter_mut().for_each(String::clear);
            st.keys_changed(None);
        }
        schedule_save(&s);
    });

    // ---- アプリ ----

    let s = state.clone();
    let weak = window.as_weak();
    window.on_refresh_running_apps(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let mut st = s.borrow_mut();
        let registered: Vec<String> = st.apps.iter().map(|app| app.exe.clone()).collect();
        st.running = running_app_choices(win32::visible_app_windows(), &registered);
        let rows: Vec<RunningApp> = st
            .running
            .iter()
            .map(|(exe, title)| RunningApp { exe: exe.as_str().into(), title: title.as_str().into() })
            .collect();
        window.set_running_apps(ModelRc::new(VecModel::from(rows)));
    });

    let s = state.clone();
    window.on_running_app_picked(move |index| {
        {
            let mut st = s.borrow_mut();
            let Some((exe, _)) = usize::try_from(index).ok().and_then(|i| st.running.get(i).cloned())
            else {
                return;
            };
            st.add_app(&exe);
        }
        schedule_save(&s);
    });

    let s = state.clone();
    let weak = window.as_weak();
    window.on_pick_exe(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let owner = win32::window_hwnd(window.window()).unwrap_or(null_mut());
        // ダイアログはメッセージを回すので、状態を借りずに開く
        let Some(path) = win32::pick_exe_file(owner, "exe を選ぶ") else {
            return;
        };
        let Some(exe) = exe_file_name(&path) else {
            return;
        };
        s.borrow_mut().add_app(&exe);
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_app_newline_changed(move |index, on| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        s.borrow_mut().app_changed(index, |app| app.newline = on);
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_app_ime_start_changed(move |index, value| {
        let (Ok(index), Some((ime_start, _))) =
            (usize::try_from(index), usize::try_from(value).ok().and_then(|v| IME_START_LABELS.get(v)))
        else {
            return;
        };
        let ime_start = *ime_start;
        s.borrow_mut().app_changed(index, |app| app.ime_start = ime_start);
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_app_no_learning_changed(move |index, on| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        s.borrow_mut().app_changed(index, |app| app.no_learning = on);
        schedule_save(&s);
    });

    let s = state.clone();
    window.on_remove_app(move |index| {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        {
            let mut st = s.borrow_mut();
            if index >= st.apps.len() {
                return;
            }
            st.apps.remove(index);
            st.app_model.remove(index);
            st.dirty = true;
        }
        schedule_save(&s);
    });

    // 閉じるときに、書いていない変更を書く。入力途中の診断ログの出力先も確定する
    let s = state.clone();
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        let mut st = s.borrow_mut();
        st.stop_capture();
        if let Some(window) = weak.upgrade() {
            let path = window.get_debug_log_path().to_string();
            if path != st.debug_log_path {
                st.debug_log_path = path;
                st.dirty = true;
            }
        }
        st.flush();
        slint::CloseRequestResponse::HideWindow
    });
}

fn main() -> Result<(), slint::PlatformError> {
    // 設定の画面を開いていても動くよう、二重起動の防止より前に見る。失敗しても何も出さない
    // (インストーラは結果を見ず、一覧に出ないときは利用者が手で足す)
    if std::env::args_os().skip(1).any(|a| a == "--add-language-list") {
        win32::add_to_language_list();
        return Ok(());
    }

    if !win32::acquire_single_instance(MUTEX_NAME, WINDOW_TITLE) {
        return Ok(());
    }

    // winit 以外のバックエンドでは HWND を取れない (キーの取り込み・ファイル選択に使う)
    slint::BackendSelector::new().backend_name("winit".into()).select()?;

    let window = SettingsWindow::new()?;
    let state: Shared = Rc::new(RefCell::new(State::new(&window, Config::load())));
    state.borrow().fill_window(&window);
    state.borrow().refresh_preview();
    connect(&window, &state);

    window.show()?;
    // フォーカスを得たら見本を描き直す (テーマファイルを外のエディタで書き換えた場合に読み直す)。
    // winit のイベントの処理中に画面を変えないよう、イベントループに回してから描く
    let weak = Rc::downgrade(&state);
    window.window().on_winit_window_event(move |_, event| {
        if matches!(event, WindowEvent::Focused(true)) {
            let weak = weak.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(state) = weak.upgrade() {
                    state.borrow().refresh_preview();
                }
            });
        }
        EventResult::Propagate
    });
    slint::run_event_loop()?;
    state.borrow_mut().flush();
    Ok(())
}
