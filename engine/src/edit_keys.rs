// GUI ツール (設定ツール・単語登録ツール) の EDIT コントロールに Ctrl+A と Ctrl+Backspace を足す
//
// Common Controls v6 の manifest を持たないため EDIT は v5 で、どちらのキーにも対応していない。
// メッセージループで WM_KEYDOWN を先取りして処理する。

use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, VK_BACK, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextLengthW, GetWindowTextW, MSG, SendMessageW, WM_KEYDOWN,
};

/// Ctrl+Backspace で消す範囲の先頭 (UTF-16 単位) を返す。
/// キャレットから左へ、空白 → 区切り記号 → 語の文字の連続、の順に飛ばした位置
pub fn word_delete_start(text: &[u16], caret: usize) -> usize {
    let caret = caret.min(text.len());
    // 不正なサロゲート (キャレットがペアの途中にある場合を含む) は 1 単位の区切り記号として扱う
    let chars: Vec<(char, usize)> = char::decode_utf16(text[..caret].iter().copied())
        .map(|r| match r {
            Ok(c) => (c, c.len_utf16()),
            Err(_) => (char::REPLACEMENT_CHARACTER, 1),
        })
        .collect();

    let mut pos = caret;
    let mut i = chars.len();
    let mut skip = |pred: &dyn Fn(char) -> bool| {
        while i > 0 && pred(chars[i - 1].0) {
            i -= 1;
            pos -= chars[i].1;
        }
    };
    skip(&|c| c.is_whitespace());
    skip(&|c| !c.is_whitespace() && !c.is_alphanumeric());
    skip(&|c| c.is_alphanumeric());
    pos
}

/// フォーカス中の EDIT への Ctrl+A / Ctrl+Backspace を処理する。
/// 処理した場合は true を返すので、呼び出し側は TranslateMessage 等に渡さないこと
/// (渡すと WM_CHAR の制御文字が EDIT に届いてビープが鳴る)
pub unsafe fn handle_key(msg: &MSG) -> bool {
    unsafe {
        if msg.message != WM_KEYDOWN {
            return false;
        }
        let vk = msg.wParam;
        if vk != b'A' as usize && vk != VK_BACK as usize {
            return false;
        }
        let pressed = |vk: u16| (GetKeyState(vk as i32) as u16 & 0x8000) != 0;
        if !pressed(VK_CONTROL) || pressed(VK_SHIFT) || pressed(VK_MENU) {
            return false;
        }
        let focus = GetFocus();
        if focus.is_null() {
            return false;
        }
        let mut class = [0u16; 16];
        let len = GetClassNameW(focus, class.as_mut_ptr(), class.len() as i32);
        if len <= 0 || !String::from_utf16_lossy(&class[..len as usize]).eq_ignore_ascii_case("Edit") {
            return false;
        }

        if vk == b'A' as usize {
            SendMessageW(focus, EM_SETSEL, 0, -1);
            return true;
        }

        let mut sel_start: u32 = 0;
        let mut sel_end: u32 = 0;
        SendMessageW(
            focus,
            EM_GETSEL,
            &mut sel_start as *mut u32 as usize,
            &mut sel_end as *mut u32 as isize,
        );
        if sel_start == sel_end {
            let text_len = GetWindowTextLengthW(focus).max(0) as usize;
            let mut buf = vec![0u16; text_len + 1];
            let copied = GetWindowTextW(focus, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let start = word_delete_start(&buf[..copied], sel_end as usize);
            if start == sel_end as usize {
                return true;
            }
            SendMessageW(focus, EM_SETSEL, start, sel_end as isize);
        }
        let empty = [0u16; 1];
        // fCanUndo = TRUE で EDIT の Undo (Ctrl+Z) から戻せるようにする
        SendMessageW(focus, EM_REPLACESEL, 1, empty.as_ptr() as isize);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::word_delete_start;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// 末尾にキャレットを置いて 1 回消した後の文字列
    fn delete_once(s: &str) -> String {
        let text = w(s);
        let start = word_delete_start(&text, text.len());
        String::from_utf16(&text[..start]).unwrap()
    }

    #[test]
    fn path_steps() {
        assert_eq!(delete_once(r"C:\Users\foo\debug.log"), r"C:\Users\foo\debug.");
        assert_eq!(delete_once(r"C:\Users\foo\debug."), r"C:\Users\foo\");
        assert_eq!(delete_once(r"C:\Users\foo\"), r"C:\Users\");
    }

    #[test]
    fn whitespace_before_word() {
        assert_eq!(delete_once("hello world  "), "hello ");
        assert_eq!(delete_once("hello - "), "hello ");
    }

    #[test]
    fn caret_at_start() {
        assert_eq!(word_delete_start(&w("abc"), 0), 0);
        assert_eq!(word_delete_start(&[], 0), 0);
    }

    #[test]
    fn caret_in_middle() {
        let text = w("foo.bar baz");
        // "foo.bar| baz"
        assert_eq!(word_delete_start(&text, 7), 4);
    }

    #[test]
    fn japanese() {
        assert_eq!(delete_once("今日は、晴れ"), "今日は、");
        assert_eq!(delete_once("今日は、"), "");
        assert_eq!(delete_once("ファイル　名前"), "ファイル　");
    }

    #[test]
    fn surrogate_pair() {
        // U+20BB7 (𠮷) はサロゲートペア。語の文字として 2 単位まとめて消す
        assert_eq!(delete_once("ab.\u{20BB7}野家"), "ab.");
        // 絵文字 (U+1F600) は区切り記号として 2 単位まとめて消す
        assert_eq!(delete_once("abc\u{1F600}"), "");
        let text = w("x \u{1F600}");
        assert_eq!(word_delete_start(&text, text.len()), 2);
        // キャレットがペアの途中でもペアの前半だけを消し、ペアをまたいだ位置は返さない
        let text = w("ab\u{1F600}");
        assert_eq!(word_delete_start(&text, 3), 0);
    }
}
