// アプリごとの設定 (newline_apps・ime_start_apps・no_learning_apps。docs/spec/tools.md「アプリのページ」)

/// 改行キーが働くアプリの既定 (設定 newline_apps の行が無いとき。TSF 層と合わせる)
pub const DEFAULT_NEWLINE_APPS: &str = "Discord.exe,slack.exe,ms-teams.exe,Teams.exe,LINE.exe";

/// 起動時の IME の状態の表示名 (コンボの項目の並び)
pub const IME_START_LABELS: [(ImeStart, &str); 3] =
    [(ImeStart::Auto, "自動"), (ImeStart::On, "オン"), (ImeStart::Off, "オフ")];

/// アプリの起動時の IME の状態 (docs/spec/input.md「起動時の IME の状態」)。自動は ime_start_apps に
/// 載せないことで表す
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ImeStart {
    #[default]
    Auto,
    On,
    Off,
}

/// アプリごとの設定
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppSetting {
    pub exe: String,
    /// 改行キーが働く (newline_apps に載る)
    pub newline: bool,
    pub ime_start: ImeStart,
    /// 学習を記録しない (no_learning_apps に載る)
    pub no_learning: bool,
}

/// exe 名が同じアプリか (TSF 層の照合と同じく大文字小文字を区別しない)
pub fn same_app(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// apps に exe 名 name を足す (すでにあれば足さない)。足した、またはすでにあった位置を返す
pub fn add_app(apps: &mut Vec<String>, name: &str) -> usize {
    if let Some(index) = apps.iter().position(|app| same_app(app, name)) {
        return index;
    }
    apps.push(name.to_string());
    apps.len() - 1
}

/// newline_apps の値 (exe 名のカンマ区切り) を読む。TSF 層 (tsf/src/config.cpp の
/// ParseAppList) と同じく前後の空白・タブを除き、空の項目は捨てる
pub fn parse_app_list(value: &str) -> Vec<String> {
    let mut apps = Vec::new();
    for item in value.split(',') {
        let item = item.trim_matches([' ', '\t']);
        if !item.is_empty() {
            add_app(&mut apps, item);
        }
    }
    apps
}

/// ime_start_apps の値 (exe名:on|off のカンマ区切り) を読む。TSF 層 (tsf/src/config.cpp の
/// ParseImeStartApps) と同じく、区切りは最後の `:` で、exe 名と状態から前後の空白・タブを除き、
/// 状態の on / off は大文字小文字を区別しない。状態が不正な項目・exe 名が空の項目は捨て、
/// 同じ exe 名が重なれば最初の項目を使う
pub fn parse_ime_start_apps(value: &str) -> Vec<(String, ImeStart)> {
    let mut apps: Vec<(String, ImeStart)> = Vec::new();
    for item in value.split(',') {
        let Some((exe, state)) = item.rsplit_once(':') else {
            continue;
        };
        let exe = exe.trim_matches([' ', '\t']);
        let state = state.trim_matches([' ', '\t']);
        let state = if state.eq_ignore_ascii_case("on") {
            ImeStart::On
        } else if state.eq_ignore_ascii_case("off") {
            ImeStart::Off
        } else {
            continue;
        };
        if exe.is_empty() || apps.iter().any(|(app, _)| same_app(app, exe)) {
            continue;
        }
        apps.push((exe.to_string(), state));
    }
    apps
}

/// ime_start_apps の値を作る (自動の項目は書かない)
pub fn format_ime_start_apps(apps: &[(String, ImeStart)]) -> String {
    apps.iter()
        .filter_map(|(exe, state)| match state {
            ImeStart::On => Some(format!("{exe}:on")),
            ImeStart::Off => Some(format!("{exe}:off")),
            ImeStart::Auto => None,
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// newline_apps・ime_start_apps・no_learning_apps からアプリごとの設定の一覧を作る。
/// newline_apps の順に並べ、続けて後のキーにだけあるアプリをその順に足す
/// (exe 名の表記は先に出たものを使う)
pub fn merge_app_settings(
    newline_apps: &[String],
    ime_start_apps: &[(String, ImeStart)],
    no_learning_apps: &[String],
) -> Vec<AppSetting> {
    let mut apps: Vec<AppSetting> = Vec::new();
    for exe in newline_apps {
        let index = add_app_setting(&mut apps, exe);
        apps[index].newline = true;
    }
    for (exe, state) in ime_start_apps {
        let index = add_app_setting(&mut apps, exe);
        if apps[index].ime_start == ImeStart::Auto {
            apps[index].ime_start = *state;
        }
    }
    for exe in no_learning_apps {
        let index = add_app_setting(&mut apps, exe);
        apps[index].no_learning = true;
    }
    apps
}

/// アプリごとの設定の一覧を、一覧の順のまま newline_apps・ime_start_apps・no_learning_apps に分ける
pub fn split_app_settings(
    apps: &[AppSetting],
) -> (Vec<String>, Vec<(String, ImeStart)>, Vec<String>) {
    let newline_apps = apps.iter().filter(|app| app.newline).map(|app| app.exe.clone()).collect();
    let ime_start_apps = apps
        .iter()
        .filter(|app| app.ime_start != ImeStart::Auto)
        .map(|app| (app.exe.clone(), app.ime_start))
        .collect();
    let no_learning_apps =
        apps.iter().filter(|app| app.no_learning).map(|app| app.exe.clone()).collect();
    (newline_apps, ime_start_apps, no_learning_apps)
}

/// アプリの一覧の既定: 改行キーの既定のアプリだけ (改行キーあり・自動)
pub fn default_app_settings() -> Vec<AppSetting> {
    merge_app_settings(&parse_app_list(DEFAULT_NEWLINE_APPS), &[], &[])
}

/// apps に exe 名 name のアプリを「改行キーなし・自動・学習する」で足す (すでにあれば足さない)。
/// 足した、またはすでにあった位置を返す
pub fn add_app_setting(apps: &mut Vec<AppSetting>, name: &str) -> usize {
    if let Some(index) = apps.iter().position(|app| same_app(&app.exe, name)) {
        return index;
    }
    apps.push(AppSetting {
        exe: name.to_string(),
        newline: false,
        ime_start: ImeStart::Auto,
        no_learning: false,
    });
    apps.len() - 1
}

/// 起動時の IME の状態の表示名
pub fn ime_start_label(state: ImeStart) -> &'static str {
    IME_START_LABELS.iter().find(|(s, _)| *s == state).map_or("", |(_, label)| *label)
}

/// 閉じたカードに出す、「改行キーなし・自動・学習する」から変えた設定の要約
pub fn app_summary(app: &AppSetting) -> String {
    let mut parts: Vec<String> = Vec::new();
    if app.newline {
        parts.push("改行キーで Shift+Enter".to_string());
    }
    if app.ime_start != ImeStart::Auto {
        parts.push(format!("起動時の IME: {}", ime_start_label(app.ime_start)));
    }
    if app.no_learning {
        parts.push("学習しない".to_string());
    }
    parts.join("・")
}

/// パスのファイル名部分
pub fn exe_file_name(path: &str) -> Option<String> {
    let name = std::path::Path::new(path).file_name()?.to_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// 実行中のアプリから選ぶ一覧の選択肢: exe 名で重複をまとめ (タイトルは最初に
/// 見つかったもの)、登録済みの exe を除いて exe 名順に並べる
pub fn running_app_choices(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exe: &str, newline: bool, ime_start: ImeStart, no_learning: bool) -> AppSetting {
        AppSetting { exe: exe.to_string(), newline, ime_start, no_learning }
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
    fn parses_ime_start_apps_like_tsf() {
        let app = |exe: &str, state: ImeStart| (exe.to_string(), state);
        assert_eq!(
            parse_ime_start_apps("Notepad.exe:on,WindowsTerminal.exe:OFF"),
            [app("Notepad.exe", ImeStart::On), app("WindowsTerminal.exe", ImeStart::Off)]
        );
        // 状態が不正な項目・exe 名が空の項目は捨て、exe 名と状態の前後の空白・タブは除く
        assert_eq!(
            parse_ime_start_apps("Notepad.exe:yes,:on, Notepad.exe : off ,\tCode.exe\t:\tOn\t"),
            [app("Notepad.exe", ImeStart::Off), app("Code.exe", ImeStart::On)]
        );
        // 重なれば最初の項目 (大文字小文字は区別しない)
        assert_eq!(
            parse_ime_start_apps("Notepad.exe:on,notepad.exe:off"),
            [app("Notepad.exe", ImeStart::On)]
        );
        // 区切りは最後の `:`。`:` の無い項目は捨てる
        assert_eq!(parse_ime_start_apps("a:b.exe:on,Notepad.exe"), [app("a:b.exe", ImeStart::On)]);
        assert!(parse_ime_start_apps("").is_empty());
    }

    #[test]
    fn adds_app_setting_as_default() {
        let mut apps = default_app_settings();
        assert_eq!(add_app_setting(&mut apps, "discord.exe"), 0);
        assert_eq!(apps.len(), 5);
        assert_eq!(add_app_setting(&mut apps, "Notepad.exe"), 5);
        assert_eq!(apps[5], app("Notepad.exe", false, ImeStart::Auto, false));
        assert_eq!(ime_start_label(ImeStart::Auto), "自動");
        assert_eq!(ime_start_label(ImeStart::On), "オン");
        assert_eq!(ime_start_label(ImeStart::Off), "オフ");
    }

    #[test]
    fn summarizes_app_settings() {
        assert_eq!(app_summary(&app("a.exe", false, ImeStart::Auto, false)), "");
        assert_eq!(app_summary(&app("a.exe", true, ImeStart::Auto, false)), "改行キーで Shift+Enter");
        assert_eq!(
            app_summary(&app("a.exe", false, ImeStart::Off, true)),
            "起動時の IME: オフ・学習しない"
        );
        assert_eq!(
            app_summary(&app("a.exe", true, ImeStart::On, true)),
            "改行キーで Shift+Enter・起動時の IME: オン・学習しない"
        );
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
}
