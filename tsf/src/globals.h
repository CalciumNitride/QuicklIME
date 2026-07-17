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

// IMEオン/オフをトグルする preserved key (半角/全角キー系) の GUID
// {5C1B3F6E-9A2D-4E8B-B7C4-2F0D8A61E395}
extern const GUID kPreservedKeyToggleGuid;

// IMEオン専用キー (VK_IME_ON) の preserved key GUID
// {8E4A70D2-06C3-4D5B-9F1A-C58B37A2D6E1}
extern const GUID kPreservedKeyImeOnGuid;

// IMEオフ専用キー (VK_IME_OFF) の preserved key GUID
// {3B9D51C7-E842-4F06-A1B3-7D64F90C25A8}
extern const GUID kPreservedKeyImeOffGuid;

// F10 (半角英字変換) の preserved key GUID。F10 は Windows 仕様で WM_SYSKEYDOWN と
// なり、非 TSF アプリ (WezTerm 等) では通常のキーイベント経路 (OnKeyDown) に
// 届かないことがあるため、preserved key として配送前に受け取る
// {94E1D5A3-7B26-4A0C-B05E-1F836CD9427A}
extern const GUID kPreservedKeyF10Guid;

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
