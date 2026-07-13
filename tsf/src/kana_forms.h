#pragma once

#include <string>

// ファンクションキー (F6-F10) の直接変換で使う文字種変換。
// どの関数も対応しない文字はそのまま残す
namespace kana_forms {

// ひらがな → 全角カタカナ (F7)
std::wstring ToKatakana(const std::wstring& text);

// ひらがな/全角カタカナ → 半角カタカナ、全角英数記号 → 半角 (F8)
std::wstring ToHalfwidth(const std::wstring& text);

// 半角 ASCII (英数記号・スペース) → 全角 (F9)
std::wstring ToFullwidthAscii(const std::wstring& text);

} // namespace kana_forms
