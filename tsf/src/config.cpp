#include "config.h"

#include <fstream>

namespace {

// 設定ファイルのパス。優先順: QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv
std::wstring ConfigPath()
{
    wchar_t buf[MAX_PATH];
    DWORD len = GetEnvironmentVariableW(L"QUICKLIME_CONFIG_FILE", buf, MAX_PATH);
    if (len > 0 && len < MAX_PATH) {
        return std::wstring(buf, len);
    }
    len = GetEnvironmentVariableW(L"APPDATA", buf, MAX_PATH);
    if (len > 0 && len < MAX_PATH) {
        return std::wstring(buf, len) + L"\\QuicklIME\\config.tsv";
    }
    return {};
}

std::wstring Utf8ToWide(const std::string& utf8)
{
    if (utf8.empty()) {
        return {};
    }
    const int len = MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()),
                                        nullptr, 0);
    if (len <= 0) {
        return {};
    }
    std::wstring wide(static_cast<size_t>(len), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, utf8.data(), static_cast<int>(utf8.size()), wide.data(), len);
    return wide;
}

// "0"/"1" を bool にする。それ以外は変更しない
void ParseBool(const std::wstring& value, bool& out)
{
    if (value == L"0") {
        out = false;
    } else if (value == L"1") {
        out = true;
    }
}

// "full"/"half" を全角フラグにする。それ以外は変更しない
void ParseWidth(const std::wstring& value, bool& out)
{
    if (value == L"full") {
        out = true;
    } else if (value == L"half") {
        out = false;
    }
}

// 整数として読めれば [minValue, maxValue] に収めて設定する。読めなければ変更しない
void ParseClamped(const std::wstring& value, int minValue, int maxValue, int& out)
{
    if (value.empty()) {
        return;
    }
    int n = 0;
    for (const wchar_t c : value) {
        if (c < L'0' || c > L'9' || n > 100000) {
            return;
        }
        n = n * 10 + (c - L'0');
    }
    out = min(max(n, minValue), maxValue);
}

// 句読点の組 (読点+句点の2文字)。対応する4通り以外は変更しない
void ParsePunctuation(const std::wstring& value, TsfConfig& config)
{
    if (value != L"、。" && value != L"，．" && value != L"、．" && value != L"，。") {
        return;
    }
    config.punctComma = value.substr(0, 1);
    config.punctPeriod = value.substr(1, 1);
}

// 設定キー名 → 機能の対応 (key.* のパース用)
const struct {
    const wchar_t* name;
    KeyFunc func;
} kKeyNames[] = {
    {L"key.convert_symbol", KeyFunc::ConvertSymbol},
    {L"key.convert_user", KeyFunc::ConvertUser},
    {L"key.to_hiragana", KeyFunc::ToHiragana},
    {L"key.to_katakana", KeyFunc::ToKatakana},
    {L"key.to_half_katakana", KeyFunc::ToHalfKatakana},
    {L"key.to_full_ascii", KeyFunc::ToFullAscii},
    {L"key.to_half_ascii", KeyFunc::ToHalfAscii},
    {L"key.undo_commit", KeyFunc::UndoCommit},
    {L"key.register_word", KeyFunc::RegisterWord},
    {L"key.open_config", KeyFunc::OpenConfig},
};

// "F1"〜"F12" を仮想キーコードにする。それ以外は 0
UINT ParseFunctionVk(const std::wstring& name)
{
    if (name.size() < 2 || name.size() > 3 || name[0] != L'F') {
        return 0;
    }
    int n = 0;
    for (size_t i = 1; i < name.size(); ++i) {
        if (name[i] < L'0' || name[i] > L'9') {
            return 0;
        }
        n = n * 10 + (name[i] - L'0');
    }
    return (n >= 1 && n <= 12) ? (VK_F1 + n - 1) : 0;
}

// キー割当の値をパースする。requireCtrl は機能の文脈が要求する修飾
// (composition 中の機能は無修飾 F1-F12 のみ、composition 無しの機能は
//  Ctrl+F1-F12 / Ctrl+Backspace のみ)。"none" は割当なし。
// 文脈に合わない値・読めない値は変更しない
void ParseKeyBinding(const std::wstring& value, bool requireCtrl, KeyBinding& out)
{
    if (value == L"none") {
        out = KeyBinding{};
        return;
    }
    std::wstring name = value;
    bool ctrl = false;
    if (name.rfind(L"Ctrl+", 0) == 0) {
        ctrl = true;
        name = name.substr(5);
    }
    if (ctrl != requireCtrl) {
        return;
    }
    UINT vk = ParseFunctionVk(name);
    if (vk == 0 && ctrl && name == L"Backspace") {
        vk = VK_BACK;
    }
    if (vk == 0) {
        return;
    }
    out = KeyBinding{ctrl, vk};
}

// 1行「キー\t値」を config へ反映する
void ApplyLine(const std::wstring& key, const std::wstring& value, TsfConfig& config)
{
    if (key.rfind(L"key.", 0) == 0) {
        for (const auto& entry : kKeyNames) {
            if (key == entry.name) {
                const size_t index = static_cast<size_t>(entry.func);
                // 先頭7機能 (composition 中) は無修飾、残りは Ctrl 併用のみ
                const bool requireCtrl = entry.func >= KeyFunc::UndoCommit;
                ParseKeyBinding(value, requireCtrl, config.keys[index]);
                return;
            }
        }
        return;
    }
    if (key == L"space") {
        ParseWidth(value, config.spaceFullwidth);
    } else if (key == L"digits") {
        ParseWidth(value, config.digitsFullwidth);
    } else if (key == L"punctuation") {
        ParsePunctuation(value, config);
    } else if (key == L"candidate_font") {
        // LOGFONT の面名は LF_FACESIZE (32) 未満
        if (!value.empty() && value.size() < LF_FACESIZE) {
            config.candidateFont = value;
        }
    } else if (key == L"candidate_font_size") {
        ParseClamped(value, 10, 40, config.candidateFontSize);
    }
    // 未知キー (エンジン向けを含む) は無視
}

// ファイルを読み込んで config へ反映する。失敗しても既定値のまま続ける
void Load(const std::wstring& path, TsfConfig& config)
{
    if (path.empty()) {
        return;
    }
    std::ifstream file(path.c_str(), std::ios::binary);
    if (!file) {
        return;
    }
    std::string line;
    bool firstLine = true;
    while (std::getline(file, line)) {
        if (!line.empty() && line.back() == '\r') {
            line.pop_back();
        }
        if (firstLine) {
            firstLine = false;
            if (line.rfind("\xEF\xBB\xBF", 0) == 0) { // UTF-8 BOM
                line.erase(0, 3);
            }
        }
        if (line.empty() || line[0] == '#') {
            continue;
        }
        const size_t tab = line.find('\t');
        if (tab == std::string::npos) {
            continue;
        }
        ApplyLine(Utf8ToWide(line.substr(0, tab)), Utf8ToWide(line.substr(tab + 1)), config);
    }
}

} // namespace

KeyFunc TsfConfig::FindPlainFunc(WPARAM wparam) const
{
    for (size_t i = 0; i < kKeyFuncCount; ++i) {
        if (!keys[i].ctrl && keys[i].vk != 0 && keys[i].vk == wparam) {
            return static_cast<KeyFunc>(i);
        }
    }
    return KeyFunc::None;
}

KeyFunc TsfConfig::FindCtrlFunc(WPARAM wparam) const
{
    for (size_t i = 0; i < kKeyFuncCount; ++i) {
        if (keys[i].ctrl && keys[i].vk != 0 && keys[i].vk == wparam) {
            return static_cast<KeyFunc>(i);
        }
    }
    return KeyFunc::None;
}

bool ConfigLoader::Refresh()
{
    const std::wstring path = ConfigPath();
    FILETIME lastWrite = {};
    if (!path.empty()) {
        WIN32_FILE_ATTRIBUTE_DATA attr = {};
        if (GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &attr)) {
            lastWrite = attr.ftLastWriteTime;
        }
    }
    if (loaded_ && CompareFileTime(&lastWrite, &lastWrite_) == 0) {
        return false; // 変更なし (ファイルが無いままの場合を含む)
    }
    lastWrite_ = lastWrite;
    loaded_ = true;
    // 一旦既定値に戻してから読む (ファイルから消えたキーが既定へ戻るように)
    config_ = TsfConfig{};
    Load(path, config_);
    return true;
}
