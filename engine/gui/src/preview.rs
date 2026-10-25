// 設定ツールの「見た目」ページに出す入力パネルの見本 (docs/spec/tools.md「入力パネルの見本」)。
// アプリが実際に読む TSF の DLL を読み込み、その描画で見本を描かせる

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_SUCCESS, GetLastError};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW};

use crate::win32::wide;

/// QuicklIME のテキストサービスの CLSID (tsf/src/globals.cpp の kClsid)
const CLSID: &str = "{D8FA8028-4371-40E9-8F49-4E465ECE9A41}";
const PREVIEW_FUNCTION: &[u8] = b"QuicklimePanelPreview\0";

/// tsf/src/preview.cpp の QuicklimePanelPreview
type PreviewFn = unsafe extern "system" fn(
    theme: *const u16,
    font: *const u16,
    font_size: i32,
    sample: i32,
    pixels: *mut u8,
    capacity: u32,
    width: *mut u32,
    height: *mut u32,
    dark: *mut i32,
    blur: *mut i32,
) -> i32;

/// 見本の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// 入力パネル
    Bar = 0,
    /// 変換の候補一覧
    Conversion = 1,
}

/// 描いた見本
pub struct Preview {
    pub width: u32,
    pub height: u32,
    /// 上から下へ並ぶ RGBA (premultiplied alpha)
    pub rgba: Vec<u8>,
    /// テーマをダークの配色で解決した
    pub dark: bool,
    /// テーマのぼかしが on (見本はぼかしの無い描画で描いてある)
    pub blur: bool,
}

/// HKCR\CLSID\{CLSID}\InprocServer32 の既定値 (アプリが読む DLL のパス)
fn registered_dll_path() -> Option<String> {
    let key = wide(&format!("CLSID\\{CLSID}\\InprocServer32"));
    let mut buffer = vec![0u16; 1024];
    loop {
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CLASSES_ROOT,
                key.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status == ERROR_SUCCESS {
            let length = (size as usize / 2).min(buffer.len());
            let value = &buffer[..length];
            let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
            let path = String::from_utf16_lossy(&value[..end]);
            return (!path.is_empty()).then_some(path);
        }
        if status != ERROR_MORE_DATA {
            return None;
        }
        buffer.resize((size as usize).div_ceil(2) + 1, 0);
    }
}

/// 登録されている DLL のプレビューの関数
fn load_preview_function() -> Result<PreviewFn, String> {
    let path = registered_dll_path()
        .ok_or("QuicklIME が登録されていないため、プレビューを表示できません")?;
    // 解放しない。TSF が同じ DLL をこのプロセスに読み込んでいることがあり (同じパスなので同じ
    // モジュールが返る)、DLL 自身も読み込まれたら常駐する前提で作られている
    let module = unsafe { LoadLibraryW(wide(&path).as_ptr()) };
    if module.is_null() {
        let error = unsafe { GetLastError() };
        return Err(format!(
            "QuicklIME の本体を読み込めないため、プレビューを表示できません (エラー {error})\n{path}"
        ));
    }
    let function = unsafe { GetProcAddress(module, PREVIEW_FUNCTION.as_ptr()) }.ok_or_else(|| {
        format!(
            "インストールされている QuicklIME はプレビューに対応していない版です。QuicklIME を更新してください\n{path}"
        )
    })?;
    Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, PreviewFn>(function) })
}

/// 上から下へ並ぶ BGRA を RGBA に並べ替える (premultiplied はそのまま)
pub fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect()
}

/// テーマ theme (panel_theme の値)・フォント font・大きさ font_size (px) で sample の見本を描く。
/// 描けなければ画面に出す理由の文を返す
pub fn render(theme: &str, font: &str, font_size: u32, sample: Sample) -> Result<Preview, String> {
    let function = load_preview_function()?;
    let theme = wide(theme);
    let font = wide(font);
    let font_size = i32::try_from(font_size).unwrap_or(i32::MAX);
    let mut pixels: Vec<u8> = Vec::new();
    // 1 回目で大きさを聞き、足りなければその大きさで描き直す
    for _ in 0..2 {
        let (mut width, mut height, mut dark, mut blur) = (0u32, 0u32, 0i32, 0i32);
        let hr = unsafe {
            function(
                theme.as_ptr(),
                font.as_ptr(),
                font_size,
                sample as i32,
                if pixels.is_empty() { std::ptr::null_mut() } else { pixels.as_mut_ptr() },
                pixels.len() as u32,
                &mut width,
                &mut height,
                &mut dark,
                &mut blur,
            )
        };
        let needed = width as usize * height as usize * 4;
        if hr == 0 && needed <= pixels.len() {
            pixels.truncate(needed);
            return Ok(Preview {
                width,
                height,
                rgba: bgra_to_rgba(&pixels),
                dark: dark != 0,
                blur: blur != 0,
            });
        }
        // HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)
        if hr as u32 != 0x8007_0000 | ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            return Err(format!("プレビューを描けませんでした (エラー 0x{:08X})", hr as u32));
        }
        pixels = vec![0; needed];
    }
    Err("プレビューを描けませんでした".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_rgba_swaps_red_and_blue() {
        let bgra = [1, 2, 3, 4, 10, 20, 30, 40];
        assert_eq!(bgra_to_rgba(&bgra), vec![3, 2, 1, 4, 30, 20, 10, 40]);
    }
}
