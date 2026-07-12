#pragma once

#include <windows.h>
#include <msctf.h>

namespace globals {

// QuicklIME テキストサービスの CLSID
// {D8FA8028-4371-40E9-8F49-4E465ECE9A41}
extern const CLSID kClsid;

// 日本語入力プロファイルの GUID
// {C0730986-A430-4595-8D18-A4103718C6C6}
extern const GUID kProfileGuid;

// 日本語
inline constexpr LANGID kLangId = 0x0411;

// 入力方式一覧に表示される名前
inline constexpr wchar_t kDescription[] = L"QuicklIME";

// DLL のインスタンスハンドル (DllMain で設定)
extern HINSTANCE dllInstance;

// DLL のアンロード可否判定用の参照カウント
void DllAddRef();
void DllRelease();
LONG DllRefCount();

} // namespace globals
