// キー割当の表記の読み書きと検査。並び・規則は TSF 層 (tsf/src/config.h・config.cpp) と合わせる

/// キー割当の照合に使う入力状態: (設定上の名前, 表示名)。
/// 並びは TSF 層 (tsf/src/config.h の KeyState) と合わせる
pub const STATES: [(&str, &str); 4] = [
    ("idle", "入力なし"),
    ("run", "入力中"),
    ("candidate", "候補選択中"),
    ("suggest", "予測選択中"),
];
pub const IN_IDLE: u8 = 1 << 0;
pub const IN_RUN: u8 = 1 << 1;
pub const IN_CANDIDATE: u8 = 1 << 2;
pub const IN_SUGGEST: u8 = 1 << 3;
pub const IN_INPUT: u8 = IN_RUN | IN_CANDIDATE | IN_SUGGEST;

/// キー割当の機能一覧: (設定キー名, 表示名, 働く状態, 既定の割当)。
/// 並び・働く状態・既定は TSF 層 (tsf/src/config.h の KeyFunc) と合わせる
pub const KEY_ITEMS: [(&str, &str, u8, &str); 18] = [
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
    ("key.forget_learning", "学習の削除", IN_SUGGEST, "Ctrl+Delete"),
];

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

/// 修飾キーの組と仮想キー
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub vk: u16,
}

/// 1機能のキー割当。overrides[s] が Some の状態では keys の代わりにそれを使う
/// (空なら、その状態では割当なし)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct KeyAssign {
    pub keys: Vec<KeyCombo>,
    pub overrides: [Option<Vec<KeyCombo>>; 4],
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
pub fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
}

impl KeyCombo {
    /// 1つのキーの表記 ("Ctrl+Shift+F7" など。修飾キーの順序は問わない) を読む
    pub fn parse(text: &str) -> Option<KeyCombo> {
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
    pub fn notation(&self) -> String {
        self.notation_with(key_name(self.vk))
    }

    /// 割り当てられない理由に添える表記。半角/全角キーは押した状態で仮想キーが 0xF3・0xF4・0x19 に
    /// 変わり、VK_F3 などと出すと F3 キーに見えるため名前で出す (どれも割り当てられないキーなので
    /// 設定ファイルの表記には現れない)
    pub fn display_notation(&self) -> String {
        match self.vk {
            0xF3 | 0xF4 | 0x19 => self.notation_with("半角/全角".to_string()),
            _ => self.notation(),
        }
    }

    fn notation_with(&self, name: String) -> String {
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
        text.push_str(&name);
        text
    }

    /// 機能に割り当てられないキーなら理由を返す。TSF 層 (tsf/src/config.cpp の
    /// IsAssignable) と同じ規則
    pub fn unassignable_reason(&self) -> Option<&'static str> {
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
        if self.ctrl && !self.shift && (0x31..=0x39).contains(&self.vk) {
            return Some("Ctrl+1〜9 は候補の番号での採用に使うため割り当てられません");
        }
        None
    }
}

/// キー割当の値 ("<キー>[,<キー>...]" または "none") を読む。読めない・対象外のキーは捨て、
/// 1つも残らなければ None (既定のまま)。"none" は空の一覧
pub fn parse_key_list(value: &str) -> Option<Vec<KeyCombo>> {
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
pub fn format_key_list(keys: &[KeyCombo]) -> String {
    if keys.is_empty() {
        return "none".to_string();
    }
    keys.iter().map(KeyCombo::notation).collect::<Vec<_>>().join(",")
}

pub fn default_key_assigns() -> Vec<KeyAssign> {
    KEY_ITEMS
        .iter()
        .map(|(_, _, _, default)| KeyAssign {
            keys: parse_key_list(default).unwrap_or_default(),
            overrides: Default::default(),
        })
        .collect()
}

/// 機能 index が働く状態 (STATES の添字) の一覧
pub fn working_states(index: usize) -> Vec<usize> {
    (0..STATES.len()).filter(|s| KEY_ITEMS[index].2 & (1 << s) != 0).collect()
}

/// 状態 state で機能 index が使うキー (その状態で働かない機能は None)
pub fn effective_keys(assigns: &[KeyAssign], index: usize, state: usize) -> Option<&[KeyCombo]> {
    if KEY_ITEMS[index].2 & (1 << state) == 0 {
        return None;
    }
    Some(assigns[index].overrides[state].as_deref().unwrap_or(assigns[index].keys.as_slice()))
}

/// 同じ状態で同じキーが複数の機能に割り当てられていれば、その説明を返す
pub fn find_key_conflict(assigns: &[KeyAssign]) -> Option<String> {
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

/// 機能 index の状態 states に key を足すと、同じ状態で別の機能と重なるなら、その説明を返す
pub fn find_conflict_for_key(
    assigns: &[KeyAssign],
    index: usize,
    states: &[usize],
    key: &KeyCombo,
) -> Option<String> {
    for &state in states {
        for other in (0..KEY_ITEMS.len()).filter(|&other| other != index) {
            if effective_keys(assigns, other, state).is_some_and(|keys| keys.contains(key)) {
                return Some(format!(
                    "{}: {} で「{}」に割り当てられています",
                    key.notation(),
                    STATES[state].1,
                    KEY_ITEMS[other].1
                ));
            }
        }
    }
    None
}

/// 取り込んだ key を機能 index の状態 states に足せるか。足せないなら画面に出す理由を返す
pub fn check_captured_key(
    assigns: &[KeyAssign],
    index: usize,
    states: &[usize],
    key: &KeyCombo,
) -> Result<(), String> {
    if let Some(reason) = key.unassignable_reason() {
        return Err(format!("{}: {reason}", key.display_notation()));
    }
    match find_conflict_for_key(assigns, index, states, key) {
        Some(conflict) => Err(conflict),
        None => Ok(()),
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
            "Alt+S", "Alt+Left", "Alt+F4", "Ctrl+Alt+F7", "Alt+Shift+Space", "Ctrl+1", "Ctrl+9",
        ] {
            assert!(key(text).unassignable_reason().is_some(), "{text}");
        }
        for text in [
            "Ctrl+Enter", "Ctrl+Left", "Ctrl+Shift+M", "Space", "Shift+Space", "Insert", "Kana", "Ctrl+A", "VK_6C",
            "Ctrl+0", "Ctrl+Shift+1",
        ] {
            assert!(key(text).unassignable_reason().is_none(), "{text}");
        }
        // 対象外だけなら既定のまま
        assert_eq!(parse_key_list("Enter"), None);
        assert_eq!(parse_key_list("Alt+S"), None);
        assert_eq!(parse_key_list("Enter,F11"), Some(vec![key("F11")]));
        assert_eq!(parse_key_list("F4,Alt+S"), Some(vec![key("F4")]));
    }

    #[test]
    fn shows_hankaku_zenkaku_by_name() {
        for text in ["VK_F3", "VK_F4", "VK_19"] {
            assert_eq!(key(text).display_notation(), "半角/全角", "{text}");
        }
        assert_eq!(key("Ctrl+VK_F4").display_notation(), "Ctrl+半角/全角");
        // 設定ファイルの表記は変えない
        assert_eq!(key("VK_F3").notation(), "VK_F3");
        assert_eq!(key("Alt+A").display_notation(), "Alt+A");
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

    #[test]
    fn detects_conflict_before_adding() {
        let assigns = default_key_assigns();
        // 記号・日付変換に Ctrl+F7 を足しても、単語登録 (入力なしのみ) とは重ならない
        assert_eq!(find_conflict_for_key(&assigns, 4, &working_states(4), &key("Ctrl+F7")), None);
        // カタカナ変換は入力なしでも働くので重なる
        assert_eq!(
            find_conflict_for_key(&assigns, 7, &working_states(7), &key("Ctrl+F7")).as_deref(),
            Some("Ctrl+F7: 入力なし で「単語登録」に割り当てられています")
        );
        // 状態を絞れば重ならない
        assert_eq!(find_conflict_for_key(&assigns, 7, &[1], &key("Ctrl+F7")), None);
        // 自分自身とは重ならない
        assert_eq!(find_conflict_for_key(&assigns, 7, &working_states(7), &key("F7")), None);
        assert_eq!(working_states(1), [2]);
        assert_eq!(working_states(15), [0, 3]);
    }

    #[test]
    fn checks_captured_keys() {
        let mut assigns = default_key_assigns();
        let check = |assigns: &[KeyAssign], index: usize, text: &str| {
            check_captured_key(assigns, index, &working_states(index), &key(text))
        };
        // 割り当てられないキーは、今と同じ理由を表記付きで出す
        assert_eq!(check(&assigns, 4, "Alt+A").unwrap_err(), "Alt+A: Alt と組み合わせたキーは割り当てられません");
        assert_eq!(
            check(&assigns, 4, "A").unwrap_err(),
            "A: 英字・数字・記号・テンキーは、Ctrl と組み合わせたときだけ割り当てられます"
        );
        assert!(check(&assigns, 4, "Esc").unwrap_err().starts_with("Esc: Enter・Esc・Tab"));
        assert_eq!(check(&assigns, 4, "VK_F3").unwrap_err(), "半角/全角: IME の切替キーは割り当てられません");
        assert_eq!(check(&assigns, 4, "VK_19").unwrap_err(), "半角/全角: IME の切替キーは割り当てられません");
        // 衝突するキーは、衝突する機能名を出す
        assert_eq!(
            check(&assigns, 4, "Convert").unwrap_err(),
            "Convert: 入力中 で「変換」に割り当てられています"
        );
        // 割り当てられるキー
        assert_eq!(check(&assigns, 4, "Ctrl+Shift+F7"), Ok(()));
        assert_eq!(check(&assigns, 0, "F4"), Err("F4: 入力中 で「記号・日付変換」に割り当てられています".to_string()));
        assigns[0].keys.clear();
        assigns[3].keys.clear();
        for text in ["Convert", "NonConvert", "Ctrl+Shift+F7"] {
            assert_eq!(check(&assigns, 4, text), Ok(()), "{text}");
        }
    }
}
