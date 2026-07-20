#pragma once

#include <windows.h>

#include <string>

// キー割当を変えられる機能。コア操作 (Enter/Esc/Tab/矢印など) は対象外。
// 先頭7つは composition 中の機能 (無修飾の F1-F12 のみ割当可)、
// 残り3つは composition が無いときの機能 (Ctrl 併用のみ割当可)
enum class KeyFunc {
    ConvertSymbol,   // 記号・日付変換 (既定 F4)
    ConvertUser,     // ユーザ登録語変換 (既定 F5)
    ToHiragana,      // ひらがな変換 (既定 F6)
    ToKatakana,      // カタカナ変換 (既定 F7)
    ToHalfKatakana,  // 半角カタカナ変換 (既定 F8)
    ToFullAscii,     // 全角英字変換 (既定 F9)
    ToHalfAscii,     // 半角英字変換 (既定 F10)
    UndoCommit,      // 確定アンドゥ (既定 Ctrl+Backspace)
    RegisterWord,    // 単語登録ツール起動 (既定 Ctrl+F7)
    OpenConfig,      // 設定ツール起動 (既定 Ctrl+F12)
    None,            // 割当なし (照合の「該当なし」も表す)
};

constexpr size_t kKeyFuncCount = static_cast<size_t>(KeyFunc::None);

// 1機能へのキー割当。vk == 0 は「割当なし (none)」
struct KeyBinding {
    bool ctrl = false;
    UINT vk = 0;
};

// ユーザ設定 (config.tsv) のうち TSF 層で使う項目。
// エンジン向けのキー (learning, suggest など) はエンジンが同じファイルを読む
struct TsfConfig {
    bool spaceFullwidth = true;    // composition が無い Space で全角スペースを入れる
    bool digitsFullwidth = false;  // 数字キー・テンキーの数字を全角で入れる
    std::wstring punctComma = L"、";   // 読点 (VK_OEM_COMMA の非 Shift)
    std::wstring punctPeriod = L"。";  // 句点 (VK_OEM_PERIOD の非 Shift)
    std::wstring candidateFont = L"Yu Gothic UI";  // 候補ウィンドウのフォント名
    int candidateFontSize = 18;    // 候補ウィンドウのフォントの高さ (px)
    bool liveConversion = false;   // ライブ変換 (入力中にかな全体を自動変換して表示)

    // 機能キーの割当 (KeyFunc の並び順)
    KeyBinding keys[kKeyFuncCount] = {
        {false, VK_F4},   // ConvertSymbol
        {false, VK_F5},   // ConvertUser
        {false, VK_F6},   // ToHiragana
        {false, VK_F7},   // ToKatakana
        {false, VK_F8},   // ToHalfKatakana
        {false, VK_F9},   // ToFullAscii
        {false, VK_F10},  // ToHalfAscii
        {true, VK_BACK},  // UndoCommit
        {true, VK_F7},    // RegisterWord
        {true, VK_F12},   // OpenConfig
    };

    // 無修飾の wparam に割当てられた機能 (composition 中の照合)。該当なしは None
    KeyFunc FindPlainFunc(WPARAM wparam) const;
    // Ctrl 併用の wparam に割当てられた機能 (composition 無しの照合)。該当なしは None
    KeyFunc FindCtrlFunc(WPARAM wparam) const;
};

// config.tsv (QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv) のローダ。
// 書き込みは設定ツール (quicklime-config.exe) が行い、TSF 層は読むだけ。
// 形式は「キー\t値」の行ベース TSV (UTF-8、# 始まりはコメント)。
// 未知キーは無視し、不正な値はそのキーだけ既定値のまま (エラーにしない)。
// TextService (スレッドごとに1インスタンス) が所有するためロックは不要
class ConfigLoader {
public:
    // ファイルの更新時刻を確認し、初回と変更時だけ読み直して true を返す。
    // フォーカス切替などの軽いタイミングで毎回呼べる (通常は時刻比較のみ)。
    // ファイルが無い・読めない場合は既定値になる
    bool Refresh();

    const TsfConfig& Get() const { return config_; }

private:
    TsfConfig config_;
    FILETIME lastWrite_ = {};
    bool loaded_ = false;
};
