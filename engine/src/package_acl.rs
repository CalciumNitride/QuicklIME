// AppContainer のアプリ (ALL APPLICATION PACKAGES) にファイル・フォルダの読み取りを許す ACE を足す
//
// AppContainer で動く TSF 層 (スタートメニューの検索欄など) が config.tsv・romaji.tsv・themes を
// 読めるようにするために使う。エンジンと設定ツールで共有する (gui crate は #[path] で取り込む)。

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    GetSecurityDescriptorControl, INHERIT_ONLY_ACE, NO_INHERITANCE, OBJECT_INHERIT_ACE,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, UNPROTECTED_DACL_SECURITY_INFORMATION,
};

/// ALL APPLICATION PACKAGES
const APP_PACKAGES_SID: &str = "S-1-15-2-1";
// 次の 2 つは windows-sys の別の feature (Storage_FileSystem・System_SystemServices) にあり、
// 定数 1 つずつのために有効にしない
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

/// LocalAlloc で確保されて返るメモリ (SID・記述子・ACL) を解放する
struct LocalGuard(*mut core::ffi::c_void);

impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

fn to_wide(text: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn app_packages_sid() -> io::Result<(PSID, LocalGuard)> {
    let text = to_wide(APP_PACKAGES_SID.as_ref());
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((sid, LocalGuard(sid)))
}

/// path の今の DACL に、ALL APPLICATION PACKAGES の読み取り (FILE_GENERIC_READ) の ACE を足す。
/// 今の ACE は残し、同じ ACE がすでにあれば何もしない。フォルダには中のファイルとフォルダへ
/// 継承させる ACE (OI|CI) を付け、今ある中身にも継承させる
pub fn grant_app_packages_read(path: &Path) -> io::Result<()> {
    let is_dir = path.is_dir();
    let name = to_wide(path.as_os_str());
    let (sid, _sid_guard) = app_packages_sid()?;
    unsafe {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let err = GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        );
        if err != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        let _sd_guard = LocalGuard(sd);
        // NULL DACL は誰にでもすべてを許す。ここに足すと、足した ACE 以外を拒む DACL になってしまう
        if dacl.is_null() || has_read_ace(dacl, sid, is_dir) {
            return Ok(());
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        if GetSecurityDescriptorControl(sd, &mut control, &mut revision) == 0 {
            return Err(io::Error::last_os_error());
        }
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_GENERIC_READ,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: if is_dir { SUB_CONTAINERS_AND_OBJECTS_INHERIT } else { NO_INHERITANCE },
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid.cast(),
                ..Default::default()
            },
        };
        let mut new_dacl: *mut ACL = std::ptr::null_mut();
        let err = SetEntriesInAclW(1, &access, dacl, &mut new_dacl);
        if err != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        let _new_dacl_guard = LocalGuard(new_dacl.cast());
        // 親からの継承を受けるかどうかを変えないよう、今の状態をそのまま指定する
        let protection = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let err = SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | protection,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            new_dacl,
            std::ptr::null(),
        );
        if err != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
    Ok(())
}

/// dacl に sid の読み取りを許す ACE (フォルダなら OI|CI の継承つき) が効いた状態であるか
unsafe fn has_read_ace(dacl: *const ACL, sid: PSID, is_dir: bool) -> bool {
    let inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let count = unsafe { (*dacl).AceCount };
    (0..u32::from(count)).any(|i| {
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
            return false;
        }
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let flags = u32::from(ace.Header.AceFlags);
        ace.Header.AceType == ACCESS_ALLOWED_ACE_TYPE
            && flags & INHERIT_ONLY_ACE == 0
            && (!is_dir || flags & inherit == inherit)
            && ace.Mask & FILE_GENERIC_READ == FILE_GENERIC_READ
            && unsafe { EqualSid(std::ptr::from_ref(&ace.SidStart).cast_mut().cast(), sid) } != 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::INHERITED_ACE;

    /// ACE の (SID, 種類, フラグ, アクセス) の一覧
    fn aces(path: &Path) -> Vec<(String, u8, u8, u32)> {
        let name = to_wide(path.as_os_str());
        let mut list = Vec::new();
        unsafe {
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let err = GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            );
            assert_eq!(err, ERROR_SUCCESS);
            let _sd_guard = LocalGuard(sd);
            assert!(!dacl.is_null());
            for i in 0..u32::from((*dacl).AceCount) {
                let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
                assert_ne!(GetAce(dacl, i, &mut ace), 0);
                let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                let mut wide: *mut u16 = std::ptr::null_mut();
                let sid = std::ptr::from_ref(&ace.SidStart).cast_mut().cast();
                assert_ne!(ConvertSidToStringSidW(sid, &mut wide), 0);
                let _wide_guard = LocalGuard(wide.cast());
                let len = (0..).take_while(|&n| *wide.add(n) != 0).count();
                let text = String::from_utf16(std::slice::from_raw_parts(wide, len)).unwrap();
                list.push((text, ace.Header.AceType, ace.Header.AceFlags, ace.Mask));
            }
        }
        list
    }

    fn app_package_aces(path: &Path) -> Vec<(String, u8, u8, u32)> {
        aces(path).into_iter().filter(|a| a.0 == APP_PACKAGES_SID).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quicklime-acl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ファイルに読み取りのaceを足し元のaceを残す() {
        let dir = temp_dir("file");
        let path = dir.join("config.tsv");
        std::fs::write(&path, "learning\t1\n").unwrap();
        let before = aces(&path);
        assert!(app_package_aces(&path).is_empty());

        grant_app_packages_read(&path).unwrap();
        let added = app_package_aces(&path);
        assert_eq!(added.len(), 1, "{added:?}");
        let (_, kind, flags, mask) = added[0];
        assert_eq!(kind, ACCESS_ALLOWED_ACE_TYPE);
        assert_eq!(u32::from(flags) & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE), 0);
        assert_eq!(mask & FILE_GENERIC_READ, FILE_GENERIC_READ);
        let after = aces(&path);
        for ace in &before {
            assert!(after.contains(ace), "元の ACE が消えた: {ace:?} / {after:?}");
        }

        // 2 回目は何も変えない
        grant_app_packages_read(&path).unwrap();
        assert_eq!(aces(&path), after);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn フォルダには継承するaceを足し中のファイルに継承させる() {
        let dir = temp_dir("dir");
        let themes = dir.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        let existing = themes.join("before.tsv");
        std::fs::write(&existing, "").unwrap();
        let before = aces(&themes);

        grant_app_packages_read(&themes).unwrap();
        let added: Vec<_> = app_package_aces(&themes)
            .into_iter()
            .filter(|a| u32::from(a.2) & INHERITED_ACE == 0)
            .collect();
        assert_eq!(added.len(), 1, "{added:?}");
        let inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
        assert_eq!(u32::from(added[0].2) & inherit, inherit);
        let after = aces(&themes);
        for ace in &before {
            assert!(after.contains(ace), "元の ACE が消えた: {ace:?} / {after:?}");
        }

        grant_app_packages_read(&themes).unwrap();
        assert_eq!(aces(&themes), after);

        // 今あるファイルにも、後から作ったファイルとフォルダにも継承される
        let created = themes.join("after.tsv");
        std::fs::write(&created, "").unwrap();
        let sub = themes.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        for path in [&existing, &created, &sub] {
            let inherited = app_package_aces(path);
            assert!(
                inherited.iter().any(|a| u32::from(a.2) & INHERITED_ACE != 0
                    && a.3 & FILE_GENERIC_READ == FILE_GENERIC_READ),
                "{} に継承されていない: {inherited:?}",
                path.display()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 無いパスはエラーを返す() {
        let dir = temp_dir("missing");
        assert!(grant_app_packages_read(&dir.join("romaji.tsv")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
