// 単語登録ツールの画面に依存しない処理 (登録・ユーザ辞書への追記・他 IME の辞書のインポート)
//
// 登録は ADDWORD としてエンジンへ送り、再起動なしで反映する。エンジンに接続できない場合は
// userdict.tsv へ直接追記する (反映はエンジン起動時)。インポートの変換結果は
// imported\<元ファイル名>.tsv に保存し、RELOADUSER でエンジンへ反映する。

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Globalization::{MB_ERR_INVALID_CHARS, MultiByteToWideChar};

use crate::import;

/// 品詞コンボの項目 (ADDWORD の品詞フィールドにそのまま使う)
pub const POS_ITEMS: [&str; 8] = ["名詞", "固有名詞", "人名", "姓", "名", "地名", "組織", "短縮よみ"];

/// エンジンへ ADDWORD を送る。接続できないときはユーザ辞書ファイルへ直接追記する。
/// 成功時は補足メッセージ (あれば) を返す
pub fn register(reading: &str, word: &str, pos: &str) -> Result<Option<String>, String> {
    match send_addword(reading, word, pos) {
        Ok(()) => Ok(None),
        Err(SendError::Refused(message)) => Err(message),
        Err(SendError::NotConnected) => {
            // エンジンが起動していない (IME 未使用など)。ファイルへ追記しておけば
            // 次のエンジン起動時に読み込まれる
            append_to_userdict(reading, word, pos)?;
            Ok(Some(
                "エンジンが起動していないため、ユーザ辞書ファイルへ追記しました。\
                 反映は次回のエンジン起動時になります。"
                    .to_string(),
            ))
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
    send_request(&format!("ADDWORD\t{reading}\t{word}\t{pos}"))
}

/// named pipe でエンジンに1行の要求を送り、応答が OK かを確認する
fn send_request(request: &str) -> Result<(), SendError> {
    let name =
        std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| "quicklime-engine".to_string());
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\{name}"))
        .map_err(|_| SendError::NotConnected)?;
    writeln!(pipe, "{request}").map_err(|_| SendError::NotConnected)?;
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
    let path = userdict_path()?;
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

/// ユーザ辞書ファイルのパス (エンジンの userdict.rs と同じ決め方)
fn userdict_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("QUICKLIME_USER_DICT_FILE") {
        return Ok(PathBuf::from(path));
    }
    let appdata = std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("userdict.tsv"))
}

/// インポート辞書の保存先ディレクトリ (エンジンの userdict.rs と同じ決め方):
/// QUICKLIME_IMPORT_DIR > %APPDATA%\QuicklIME\imported
fn import_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("QUICKLIME_IMPORT_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let appdata = std::env::var("APPDATA").map_err(|_| "保存先を特定できません".to_string())?;
    Ok(PathBuf::from(appdata).join("QuicklIME").join("imported"))
}

/// 辞書ファイルを読み込み・変換して imported\<名前>.tsv へ保存し、エンジンへ反映する。
/// 成功時は結果 (件数・保存先・反映できなかったときの補足) の説明を返す
pub fn import_file(source: &Path) -> Result<String, String> {
    let bytes = std::fs::read(source).map_err(|e| format!("ファイルを読めません ({e})"))?;
    let text = import::decode(&bytes, decode_cp932)?;
    let dir = import_dir()?;
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let file_name = format!("{stem}.tsv");

    let result = import::parse(&text, &existing_pairs(&dir, &file_name));
    let counts = format!(
        "重複 {} / 未対応の品詞 {} / 不正な行 {}",
        group_digits(result.duplicate),
        group_digits(result.unsupported_pos),
        group_digits(result.invalid)
    );
    if result.entries.is_empty() {
        return Err(format!("取り込める語がありませんでした ({counts})。"));
    }

    std::fs::create_dir_all(&dir).map_err(|e| format!("保存先を作れません ({e})"))?;
    let path = dir.join(&file_name);
    std::fs::write(&path, import::to_tsv(&result.entries))
        .map_err(|e| format!("保存できません ({e})"))?;

    let summary = format!(
        "{} 語 ({counts})\n保存先: {}",
        group_digits(result.entries.len()),
        path.display()
    );
    Ok(match send_request("RELOADUSER") {
        Ok(()) => summary,
        Err(SendError::NotConnected) => format!(
            "{summary}\nエンジンが起動していないため、反映は次回のエンジン起動時になります。"
        ),
        Err(SendError::Refused(message)) => format!(
            "{summary}\nエンジンへの反映に失敗しました ({message})。反映は次回のエンジン起動時になります。"
        ),
    })
}

/// 重複判定用に、userdict.tsv と他のインポート済みファイルの (読み, 表記) を集める。
/// 上書き対象の同名ファイル自身は除く (配布辞書の更新で全語が重複扱いになるため)
fn existing_pairs(dir: &Path, own_file_name: &str) -> HashSet<(String, String)> {
    let mut pairs = HashSet::new();
    let read = |path: &Path, pairs: &mut HashSet<(String, String)>| {
        if let Ok(bytes) = std::fs::read(path) {
            import::collect_pairs(&String::from_utf8_lossy(&bytes), pairs);
        }
    };
    if let Ok(path) = userdict_path() {
        read(&path, &mut pairs);
    }
    // Windows のファイル名は大文字小文字を区別しないため、同名判定もそれに合わせる
    let own = own_file_name.to_lowercase();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
                continue;
            };
            if path.is_file() && name.ends_with(".tsv") && name != own {
                read(&path, &mut pairs);
            }
        }
    }
    pairs
}

/// CP932 (Shift_JIS) のバイト列を Win32 API でデコードする。不正なバイト列なら None
fn decode_cp932(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let len = i32::try_from(bytes.len()).ok()?;
    unsafe {
        let needed =
            MultiByteToWideChar(932, MB_ERR_INVALID_CHARS, bytes.as_ptr(), len, null_mut(), 0);
        if needed <= 0 {
            return None;
        }
        let mut buffer = vec![0u16; needed as usize];
        let written = MultiByteToWideChar(
            932, MB_ERR_INVALID_CHARS, bytes.as_ptr(), len, buffer.as_mut_ptr(), needed);
        if written <= 0 {
            return None;
        }
        String::from_utf16(&buffer[..written as usize]).ok()
    }
}

/// 件数を3桁区切りの文字列にする (123456 → 123,456)
fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_jisをデコードする() {
        // 「あ漢」の Shift_JIS
        let sjis = [0x82u8, 0xA0, 0x8A, 0xBF];
        assert_eq!(decode_cp932(&sjis).as_deref(), Some("あ漢"));
        // BOM なしで UTF-8 として不正なら CP932 として読まれる
        assert_eq!(import::decode(&sjis, decode_cp932).unwrap(), "あ漢");
        // 2バイト文字の途中で切れたものは不正
        assert_eq!(decode_cp932(&[0x82]), None);
    }

    #[test]
    fn 件数を3桁区切りにする() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(123456), "123,456");
        assert_eq!(group_digits(1234567), "1,234,567");
    }
}
