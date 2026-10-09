#include "config.h"

#include <algorithm>
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

#ifndef VK_IME_ON
#define VK_IME_ON 0x16
#endif
#ifndef VK_IME_OFF
#define VK_IME_OFF 0x1A
#endif

// 設定キー名 → 機能の対応 (key.* のパース用。KeyFunc の並び順)
const wchar_t* const kKeyFuncNames[kKeyFuncCount] = {
    L"key.convert",
    L"key.next_candidate",
    L"key.prev_candidate",
    L"key.commit_run",
    L"key.convert_symbol",
    L"key.convert_user",
    L"key.to_hiragana",
    L"key.to_katakana",
    L"key.to_half_katakana",
    L"key.to_full_ascii",
    L"key.to_half_ascii",
    L"key.undo_commit",
    L"key.register_word",
    L"key.open_config",
};

// 状態別の上書きの接尾辞 (key.<機能>@<状態>。KeyState の並び順)
const wchar_t* const kKeyStateNames[kKeyStateCount] = {
    L"idle",
    L"run",
    L"candidate",
    L"suggest",
};

// 英字・数字以外の名前を持つキー
const struct {
    const wchar_t* name;
    UINT vk;
} kNamedKeys[] = {
    {L"Space", VK_SPACE},       {L"Enter", VK_RETURN},     {L"Esc", VK_ESCAPE},
    {L"Tab", VK_TAB},           {L"Backspace", VK_BACK},   {L"Delete", VK_DELETE},
    {L"Insert", VK_INSERT},     {L"Home", VK_HOME},        {L"End", VK_END},
    {L"PageUp", VK_PRIOR},      {L"PageDown", VK_NEXT},    {L"Up", VK_UP},
    {L"Down", VK_DOWN},         {L"Left", VK_LEFT},        {L"Right", VK_RIGHT},
    {L"Convert", VK_CONVERT},   {L"NonConvert", VK_NONCONVERT}, {L"Kana", VK_KANA},
};

int HexDigit(wchar_t c)
{
    if (c >= L'0' && c <= L'9') {
        return c - L'0';
    }
    if (c >= L'A' && c <= L'F') {
        return c - L'A' + 10;
    }
    if (c >= L'a' && c <= L'f') {
        return c - L'a' + 10;
    }
    return -1;
}

// キー名を仮想キーコードにする。読めなければ 0
UINT ParseKeyName(const std::wstring& name)
{
    if (name.size() == 1 && ((name[0] >= L'A' && name[0] <= L'Z') ||
                             (name[0] >= L'0' && name[0] <= L'9'))) {
        return name[0];
    }
    if (name.size() >= 2 && name.size() <= 3 && name[0] == L'F') {
        int n = 0;
        for (size_t i = 1; i < name.size(); ++i) {
            if (name[i] < L'0' || name[i] > L'9') {
                return 0;
            }
            n = n * 10 + (name[i] - L'0');
        }
        return (n >= 1 && n <= 24) ? static_cast<UINT>(VK_F1 + n - 1) : 0;
    }
    for (const auto& entry : kNamedKeys) {
        if (name == entry.name) {
            return entry.vk;
        }
    }
    if (name.size() == 5 && name.rfind(L"VK_", 0) == 0) {
        const int high = HexDigit(name[3]);
        const int low = HexDigit(name[4]);
        if (high >= 0 && low >= 0 && (high * 16 + low) != 0) {
            return static_cast<UINT>(high * 16 + low);
        }
    }
    return 0;
}

// 1つのキーの表記 ("Ctrl+Shift+F7" など。修飾キーの順序は問わない) を読む
bool ParseKeyNotation(const std::wstring& text, KeyBinding* out)
{
    KeyBinding binding;
    size_t start = 0;
    for (;;) {
        const size_t plus = text.find(L'+', start);
        if (plus == std::wstring::npos) {
            break;
        }
        const std::wstring modifier = text.substr(start, plus - start);
        bool* flag = nullptr;
        if (modifier == L"Ctrl") {
            flag = &binding.ctrl;
        } else if (modifier == L"Alt") {
            flag = &binding.alt;
        } else if (modifier == L"Shift") {
            flag = &binding.shift;
        }
        if (flag == nullptr || *flag) {
            return false;
        }
        *flag = true;
        start = plus + 1;
    }
    binding.vk = ParseKeyName(text.substr(start));
    if (binding.vk == 0) {
        return false;
    }
    *out = binding;
    return true;
}

bool IsModifierVk(UINT vk)
{
    switch (vk) {
    case VK_SHIFT:
    case VK_CONTROL:
    case VK_MENU:
    case VK_LSHIFT:
    case VK_RSHIFT:
    case VK_LCONTROL:
    case VK_RCONTROL:
    case VK_LMENU:
    case VK_RMENU:
    case VK_LWIN:
    case VK_RWIN:
        return true;
    default:
        return false;
    }
}

// 印字キー (英字・数字・記号・テンキーの数字と演算子)
bool IsPrintableVk(UINT vk)
{
    return (vk >= 'A' && vk <= 'Z') || (vk >= '0' && vk <= '9') ||
           (vk >= VK_OEM_1 && vk <= VK_OEM_3) || (vk >= VK_OEM_4 && vk <= VK_OEM_8) ||
           vk == VK_OEM_102 || (vk >= VK_NUMPAD0 && vk <= VK_DIVIDE && vk != VK_SEPARATOR);
}

// コア操作のキー (無修飾と Shift 併用は割当の対象外)
bool IsCoreVk(UINT vk)
{
    switch (vk) {
    case VK_RETURN:
    case VK_ESCAPE:
    case VK_TAB:
    case VK_BACK:
    case VK_DELETE:
    case VK_UP:
    case VK_DOWN:
    case VK_LEFT:
    case VK_RIGHT:
    case VK_HOME:
    case VK_END:
    case VK_PRIOR:
    case VK_NEXT:
        return true;
    default:
        return false;
    }
}

bool IsImeToggleVk(UINT vk)
{
    return vk == VK_KANJI || vk == VK_OEM_AUTO || vk == VK_OEM_ENLW || vk == VK_IME_ON ||
           vk == VK_IME_OFF;
}

// 機能に割り当てられるキーか (設定ツールの検査と同じ規則)
bool IsAssignable(const KeyBinding& binding)
{
    // Alt 併用の打鍵は key event sink に届かないアプリがある (メモ帳で確認)
    if (binding.alt || IsModifierVk(binding.vk) || IsImeToggleVk(binding.vk)) {
        return false;
    }
    if (!binding.ctrl && (IsCoreVk(binding.vk) || IsPrintableVk(binding.vk))) {
        return false;
    }
    // Ctrl+M・Ctrl+H は Enter・Backspace の読み替え
    if (binding.ctrl && !binding.shift &&
        (binding.vk == 'M' || binding.vk == 'H')) {
        return false;
    }
    return true;
}

// キー割当の値 ("<キー>[,<キー>...]" または "none") を読む。読めない・対象外のキーは捨て、
// 1つも残らなければ false (呼び出し側は既定のまま)。"none" は空の一覧で true
bool ParseKeyList(const std::wstring& value, std::vector<KeyBinding>* out)
{
    out->clear();
    if (value == L"none") {
        return true;
    }
    size_t start = 0;
    while (start <= value.size()) {
        size_t comma = value.find(L',', start);
        if (comma == std::wstring::npos) {
            comma = value.size();
        }
        std::wstring item = value.substr(start, comma - start);
        const size_t first = item.find_first_not_of(L' ');
        const size_t last = item.find_last_not_of(L' ');
        item = first == std::wstring::npos ? std::wstring() : item.substr(first, last - first + 1);
        KeyBinding binding;
        if (ParseKeyNotation(item, &binding) && IsAssignable(binding) &&
            std::find(out->begin(), out->end(), binding) == out->end()) {
            out->push_back(binding);
        }
        start = comma + 1;
    }
    return !out->empty();
}

// key.<機能>[@<状態>] の行を反映する。上書きはその機能が働く状態にだけ書ける
void ApplyKeyLine(const std::wstring& key, const std::wstring& value, TsfConfig& config)
{
    const size_t at = key.find(L'@');
    const std::wstring funcName = key.substr(0, at);
    size_t funcIndex = kKeyFuncCount;
    for (size_t i = 0; i < kKeyFuncCount; ++i) {
        if (funcName == kKeyFuncNames[i]) {
            funcIndex = i;
            break;
        }
    }
    if (funcIndex == kKeyFuncCount) {
        return;
    }
    KeyAssignment& assignment = config.keys[funcIndex];
    std::vector<KeyBinding> keys;
    if (at == std::wstring::npos) {
        if (ParseKeyList(value, &keys)) {
            assignment.keys = std::move(keys);
        }
        return;
    }
    const std::wstring stateName = key.substr(at + 1);
    for (size_t s = 0; s < kKeyStateCount; ++s) {
        if (stateName != kKeyStateNames[s]) {
            continue;
        }
        if (KeyFuncWorksIn(static_cast<KeyFunc>(funcIndex), static_cast<KeyState>(s)) &&
            ParseKeyList(value, &keys)) {
            assignment.overridden[s] = true;
            assignment.overrides[s] = std::move(keys);
        }
        return;
    }
}

// 1行「キー\t値」を config へ反映する
void ApplyLine(const std::wstring& key, const std::wstring& value, TsfConfig& config)
{
    if (key.rfind(L"key.", 0) == 0) {
        ApplyKeyLine(key, value, config);
        return;
    }
    if (key == L"debug_log") {
        config.debugLog = value;
    } else if (key == L"space") {
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
    } else if (key == L"modeless") {
        ParseBool(value, config.modeless);
    } else if (key == L"candidate_bar") {
        ParseBool(value, config.candidateBar);
    } else if (key == L"min_suggest_chars") {
        ParseClamped(value, 1, 5, config.minSuggestChars);
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

bool KeyFuncWorksIn(KeyFunc func, KeyState state)
{
    switch (func) {
    case KeyFunc::Convert:
        return true;
    case KeyFunc::NextCandidate:
    case KeyFunc::PrevCandidate:
        return state == KeyState::Candidate;
    case KeyFunc::UndoCommit:
    case KeyFunc::RegisterWord:
    case KeyFunc::OpenConfig:
        return state == KeyState::Idle;
    case KeyFunc::None:
        return false;
    default:
        // CommitRun と文字種変換・記号変換・ユーザ語変換
        return state != KeyState::Idle;
    }
}

std::array<KeyAssignment, kKeyFuncCount> TsfConfig::DefaultKeyAssignments()
{
    std::array<KeyAssignment, kKeyFuncCount> keys;
    const auto set = [&keys](KeyFunc func, KeyBinding binding) {
        keys[static_cast<size_t>(func)].keys = {binding};
    };
    set(KeyFunc::Convert, {false, false, false, VK_CONVERT});
    set(KeyFunc::NextCandidate, {false, false, false, VK_SPACE});
    set(KeyFunc::PrevCandidate, {false, false, true, VK_SPACE});
    set(KeyFunc::CommitRun, {false, false, false, VK_NONCONVERT});
    set(KeyFunc::ConvertSymbol, {false, false, false, VK_F4});
    set(KeyFunc::ConvertUser, {false, false, false, VK_F5});
    set(KeyFunc::ToHiragana, {false, false, false, VK_F6});
    set(KeyFunc::ToKatakana, {false, false, false, VK_F7});
    set(KeyFunc::ToHalfKatakana, {false, false, false, VK_F8});
    set(KeyFunc::ToFullAscii, {false, false, false, VK_F9});
    set(KeyFunc::ToHalfAscii, {false, false, false, VK_F10});
    set(KeyFunc::UndoCommit, {true, false, false, VK_BACK});
    set(KeyFunc::RegisterWord, {true, false, false, VK_F7});
    set(KeyFunc::OpenConfig, {true, false, false, VK_F12});
    return keys;
}

KeyMatch TsfConfig::FindFunc(KeyState state, const KeyBinding& pressed) const
{
    const size_t s = static_cast<size_t>(state);
    const auto contains = [](const std::vector<KeyBinding>& list, const KeyBinding& key) {
        return std::find(list.begin(), list.end(), key) != list.end();
    };
    // 状態別の上書きで一致したものを、基本の割当で一致したものより優先する
    for (const bool overridePass : {true, false}) {
        for (size_t i = 0; i < kKeyFuncCount; ++i) {
            const KeyFunc func = static_cast<KeyFunc>(i);
            const KeyAssignment& assignment = keys[i];
            if (!KeyFuncWorksIn(func, state) || assignment.overridden[s] != overridePass) {
                continue;
            }
            const auto& list = overridePass ? assignment.overrides[s] : assignment.keys;
            if (contains(list, pressed)) {
                return {func, false};
            }
        }
    }
    if (pressed.shift) {
        KeyBinding unshifted = pressed;
        unshifted.shift = false;
        const KeyAssignment& convert = keys[static_cast<size_t>(KeyFunc::Convert)];
        const auto& list = convert.overridden[s] ? convert.overrides[s] : convert.keys;
        if (contains(list, unshifted)) {
            return {KeyFunc::Convert, true};
        }
    }
    return {};
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
