#include "panel_theme.h"

#include <algorithm>
#include <cmath>
#include <cwchar>
#include <cwctype>
#include <fstream>
#include <iterator>

namespace {

// 組み込みのテーマ (docs/design/panel-theme.md「組み込みテーマの値」)。ユーザのテーマファイルと
// 同じ形式で持ち、同じ ApplyThemeText で解釈する。不透明度の AA は、以前の float の値を
// 255 倍して丸めたもの (描画される 8bit の値が変わらないようにする)
constexpr wchar_t kCapsuleText[] =
    L"style\tcapsule\n"
    L"layout\trow\n"
    L"align\tleft\n"
    L"input_row\tsetting\n"
    L"blur\tallow\n"
    L"chip_shape\tpill\n"
    L"badge_shape\tcircle\n"
    L"selection\tfill\n"
    L"border\ton\n"
    L"shadow\ton\n"
    L"kind_tag\toff\n"
    L"text_shadow\toff\n"
    L"panel_radius\t1.4\n"
    L"panel_padding\t0.6\n"
    L"input_text_size\t1.15\n"
    L"input_row_height\t1.7\n"
    L"row_gap\t0.35\n"
    L"chip_height\t1.9\n"
    L"chip_padding\t0.65\n"
    L"chip_gap\t0.35\n"
    L"badge_size\t0.95\n"
    L"badge_text_size\t0.62\n"
    L"badge_gap\t0.35\n"
    L"label_text_size\t0.62\n"
    L"label_gap\t0.5\n"
    L"shadow_size\t0.8\n"
    L"shadow_offset\t0.2\n"
    L"center_width\t36\n"
    L"center_width_ratio\t0.9\n"
    L"line_width_px\t1\n"
    L"underline_px\t2\n"
    L"caret_width_px\t2\n"
    L"shadow_steps\t10\n"
    L"accent\t#2B86EE\n"
    L"selected_text\t#FFFFFF\n"
    L"dark.background\t#1C1C22EB\n"
    L"dark.border\t#FFFFFF14\n"
    L"dark.text\t#F2F2F6\n"
    L"dark.secondary\t#A0A0AD\n"
    L"dark.chip\t#FFFFFF12\n"
    L"dark.separator\t#FFFFFF17\n"
    L"dark.next_word\t#3CC48A\n"
    L"dark.shadow\t#00000073\n"
    L"dark.blur_tint\t#1C1C2299\n"
    L"light.background\t#FCFCFEEB\n"
    L"light.border\t#00000014\n"
    L"light.text\t#16161C\n"
    L"light.secondary\t#6B6B78\n"
    L"light.chip\t#0000000B\n"
    L"light.separator\t#00000014\n"
    L"light.next_word\t#2E9E6B\n"
    L"light.shadow\t#00000047\n"
    L"light.blur_tint\t#FCFCFEB3\n";

constexpr wchar_t kCapsuleColumnText[] =
    L"base\tcapsule\n"
    L"layout\tcolumn\n"
    L"panel_radius\t0.7\n"
    L"chip_shape\trounded\n"
    L"chip_height\t1.7\n"
    L"badge_shape\tsquare\n"
    L"selection\tsoft\n"
    L"kind_tag\ton\n";

constexpr wchar_t kGhostText[] =
    L"base\tcapsule\n"
    L"style\tghost\n"
    L"align\tcenter\n"
    L"input_row\talways\n"
    L"blur\tnever\n"
    L"chip_shape\tunderline\n"
    L"badge_shape\tplain\n"
    L"selection\ttext\n"
    L"border\toff\n"
    L"shadow\toff\n"
    L"text_shadow\ton\n";

constexpr wchar_t kDefaultTheme[] = L"capsule";

const struct {
    const wchar_t* name;
    const wchar_t* text;
} kBuiltinThemes[] = {
    {L"capsule", kCapsuleText},
    {L"capsule-column", kCapsuleColumnText},
    {L"ghost", kGhostText},
};

// base の入れ子の上限 (組み込みどうしの base に循環は無いが、念のため止める)
constexpr int kMaxBaseDepth = 4;

const struct {
    const wchar_t* name;
    float PanelTheme::*member;
    float minValue;
    float maxValue;
} kNumberKeys[] = {
    {L"panel_radius", &PanelTheme::panelRadius, 0.0f, 10.0f},
    {L"panel_padding", &PanelTheme::panelPadding, 0.0f, 10.0f},
    {L"input_text_size", &PanelTheme::inputTextSize, 0.5f, 5.0f},
    {L"input_row_height", &PanelTheme::inputRowHeight, 0.5f, 5.0f},
    {L"row_gap", &PanelTheme::rowGap, 0.0f, 5.0f},
    {L"chip_height", &PanelTheme::chipHeight, 0.0f, 5.0f},
    {L"chip_padding", &PanelTheme::chipPadding, 0.0f, 5.0f},
    {L"chip_gap", &PanelTheme::chipGap, 0.0f, 5.0f},
    {L"chip_radius", &PanelTheme::chipRadius, 0.0f, 5.0f},
    {L"badge_size", &PanelTheme::badgeSize, 0.0f, 5.0f},
    {L"badge_text_size", &PanelTheme::badgeTextSize, 0.0f, 5.0f},
    {L"badge_gap", &PanelTheme::badgeGap, 0.0f, 5.0f},
    {L"label_text_size", &PanelTheme::labelTextSize, 0.0f, 5.0f},
    {L"label_gap", &PanelTheme::labelGap, 0.0f, 5.0f},
    {L"tag_text_size", &PanelTheme::tagTextSize, 0.3f, 3.0f},
    {L"ghost_text_size", &PanelTheme::ghostTextSize, 0.5f, 5.0f},
    {L"ghost_opacity", &PanelTheme::ghostOpacity, 0.0f, 1.0f},
    {L"shadow_size", &PanelTheme::shadowSize, 0.0f, 5.0f},
    {L"shadow_offset", &PanelTheme::shadowOffset, 0.0f, 5.0f},
    {L"center_width", &PanelTheme::centerWidth, 5.0f, 100.0f},
    {L"center_width_ratio", &PanelTheme::centerWidthRatio, 0.1f, 1.0f},
    {L"line_width_px", &PanelTheme::lineWidthPx, 0.0f, 10.0f},
    {L"underline_px", &PanelTheme::underlinePx, 0.0f, 10.0f},
    {L"caret_width_px", &PanelTheme::caretWidthPx, 0.0f, 10.0f},
};

const struct {
    const wchar_t* name;
    PanelColor PanelTheme::*member;
} kColorKeys[] = {
    {L"background", &PanelTheme::background},
    {L"border", &PanelTheme::border},
    {L"text", &PanelTheme::text},
    {L"secondary", &PanelTheme::secondary},
    {L"chip", &PanelTheme::chip},
    {L"separator", &PanelTheme::separator},
    {L"accent", &PanelTheme::accent},
    {L"selected_text", &PanelTheme::selectedText},
    {L"next_word", &PanelTheme::nextWord},
    {L"shadow", &PanelTheme::shadow},
    {L"blur_tint", &PanelTheme::blurTint},
    {L"scrim", &PanelTheme::scrim},
    {L"tag_ascii", &PanelTheme::tagAscii},
    {L"tag_conversion", &PanelTheme::tagConversion},
    {L"tag_prediction", &PanelTheme::tagPrediction},
    {L"tag_partial", &PanelTheme::tagPartial},
    {L"tag_next_word", &PanelTheme::tagNextWord},
};

PanelColor Rgba(unsigned rgb, unsigned alpha = 0xFF)
{
    return {static_cast<float>((rgb >> 16) & 0xFF) / 255.0f,
            static_cast<float>((rgb >> 8) & 0xFF) / 255.0f,
            static_cast<float>(rgb & 0xFF) / 255.0f, static_cast<float>(alpha) / 255.0f};
}

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

// "#RRGGBB" / "#RRGGBBAA"
bool ParseColor(const std::wstring& value, PanelColor* out)
{
    if ((value.size() != 7 && value.size() != 9) || value[0] != L'#') {
        return false;
    }
    unsigned channels[4] = {0, 0, 0, 0xFF};
    for (size_t i = 0; i * 2 + 1 < value.size(); ++i) {
        const int high = HexDigit(value[1 + i * 2]);
        const int low = HexDigit(value[2 + i * 2]);
        if (high < 0 || low < 0) {
            return false;
        }
        channels[i] = static_cast<unsigned>(high * 16 + low);
    }
    *out = Rgba((channels[0] << 16) | (channels[1] << 8) | channels[2], channels[3]);
    return true;
}

// 10進の数 (符号・小数点を含む) として全体が読めれば true
bool ParseNumber(const std::wstring& value, double* out)
{
    if (value.empty() || iswspace(value[0])) {
        return false;
    }
    wchar_t* end = nullptr;
    const double number = std::wcstod(value.c_str(), &end);
    if (end != value.c_str() + value.size() || !std::isfinite(number)) {
        return false;
    }
    // wcstod は 16 進 (0x..) や inf も読むため、数字・符号・小数点だけを許す
    for (const wchar_t c : value) {
        if (!((c >= L'0' && c <= L'9') || c == L'.' || c == L'-' || c == L'+')) {
            return false;
        }
    }
    *out = number;
    return true;
}

std::wstring NumberText(double value)
{
    wchar_t text[32] = {};
    swprintf_s(text, L"%g", value);
    return text;
}

// 選択肢の値を読む。names[i] が value に一致すれば values[i] を out へ入れて true
template <typename T, size_t N>
bool ParseChoice(const std::wstring& value, const wchar_t* const (&names)[N], const T (&values)[N],
                 T* out)
{
    for (size_t i = 0; i < N; ++i) {
        if (value == names[i]) {
            *out = values[i];
            return true;
        }
    }
    return false;
}

bool ParseOnOff(const std::wstring& value, bool* out)
{
    static const wchar_t* const kNames[] = {L"on", L"off"};
    static const bool kValues[] = {true, false};
    return ParseChoice(value, kNames, kValues, out);
}

// 色以外のキー (border・shadow の on/off は色のキーと同名のため除く) か
bool IsShapeKey(const std::wstring& key)
{
    static const wchar_t* const kNames[] = {
        L"style",       L"layout",    L"align",    L"input_row",   L"blur",
        L"chip_shape",  L"badge_shape", L"selection", L"kind_tag", L"text_shadow",
        L"shadow_steps",
    };
    return std::any_of(std::begin(kNames), std::end(kNames),
                       [&key](const wchar_t* name) { return key == name; }) ||
           std::any_of(std::begin(kNumberKeys), std::end(kNumberKeys),
                       [&key](const auto& entry) { return key == entry.name; });
}

// 色以外のキーを反映する。キーが無ければ false (色のキーとして試す)
bool ApplyShapeKey(const std::wstring& key, const std::wstring& value, PanelTheme* theme,
                   const std::wstring& where, std::vector<std::wstring>* log)
{
    const auto invalid = [&] {
        log->push_back(where + L"不正な値 " + key + L"\t" + value + L" を無視");
    };
    bool ok = true;
    if (key == L"style") {
        static const wchar_t* const kNames[] = {L"capsule", L"ghost"};
        static const PanelStyle kValues[] = {PanelStyle::Capsule, PanelStyle::Ghost};
        ok = ParseChoice(value, kNames, kValues, &theme->style);
    } else if (key == L"layout") {
        static const wchar_t* const kNames[] = {L"row", L"column"};
        static const PanelLayout kValues[] = {PanelLayout::Row, PanelLayout::Column};
        ok = ParseChoice(value, kNames, kValues, &theme->layout);
    } else if (key == L"align") {
        static const wchar_t* const kNames[] = {L"left", L"center"};
        static const PanelAlign kValues[] = {PanelAlign::Left, PanelAlign::Center};
        ok = ParseChoice(value, kNames, kValues, &theme->align);
    } else if (key == L"input_row") {
        static const wchar_t* const kNames[] = {L"setting", L"always"};
        static const bool kValues[] = {false, true};
        ok = ParseChoice(value, kNames, kValues, &theme->inputRowAlways);
    } else if (key == L"blur") {
        static const wchar_t* const kNames[] = {L"allow", L"never"};
        static const bool kValues[] = {true, false};
        ok = ParseChoice(value, kNames, kValues, &theme->blurAllowed);
    } else if (key == L"chip_shape") {
        static const wchar_t* const kNames[] = {L"pill", L"rounded", L"underline"};
        static const ChipShape kValues[] = {ChipShape::Pill, ChipShape::Rounded,
                                            ChipShape::Underline};
        ok = ParseChoice(value, kNames, kValues, &theme->chipShape);
    } else if (key == L"badge_shape") {
        static const wchar_t* const kNames[] = {L"circle", L"square", L"plain", L"none"};
        static const BadgeShape kValues[] = {BadgeShape::Circle, BadgeShape::Square,
                                             BadgeShape::Plain, BadgeShape::None};
        ok = ParseChoice(value, kNames, kValues, &theme->badgeShape);
    } else if (key == L"selection") {
        static const wchar_t* const kNames[] = {L"fill", L"soft", L"text"};
        static const SelectionStyle kValues[] = {SelectionStyle::Fill, SelectionStyle::Soft,
                                                 SelectionStyle::Text};
        ok = ParseChoice(value, kNames, kValues, &theme->selection);
    } else if (key == L"kind_tag") {
        ok = ParseOnOff(value, &theme->kindTag);
    } else if (key == L"text_shadow") {
        ok = ParseOnOff(value, &theme->textShadow);
    } else if ((key == L"border" || key == L"shadow") && !value.empty() && value[0] != L'#') {
        // border・shadow は形 (on/off) と色の両方のキー。# で始まらない値は形として読む
        ok = ParseOnOff(value, key == L"border" ? &theme->drawBorder : &theme->drawShadow);
    } else if (key == L"shadow_steps") {
        double number = 0.0;
        ok = ParseNumber(value, &number);
        if (ok) {
            const double clamped = (std::clamp)(std::round(number), 1.0, 32.0);
            if (clamped != number) {
                log->push_back(where + key + L" を範囲 [1, 32] の整数に丸めた (" + value + L" -> " +
                               NumberText(clamped) + L")");
            }
            theme->shadowSteps = static_cast<int>(clamped);
        }
    } else {
        const auto found = std::find_if(std::begin(kNumberKeys), std::end(kNumberKeys),
                                        [&key](const auto& entry) { return key == entry.name; });
        if (found == std::end(kNumberKeys)) {
            return false;
        }
        double number = 0.0;
        ok = ParseNumber(value, &number);
        if (ok) {
            const double clamped = (std::clamp)(number, static_cast<double>(found->minValue),
                                                static_cast<double>(found->maxValue));
            if (clamped != number) {
                log->push_back(where + key + L" を範囲 [" + NumberText(found->minValue) + L", " +
                               NumberText(found->maxValue) + L"] に丸めた (" + value + L" -> " +
                               NumberText(clamped) + L")");
            }
            theme->*(found->member) = static_cast<float>(clamped);
        }
    }
    if (!ok) {
        invalid();
    }
    return true;
}

// 色のキーを読む。キーが無ければ false。apply が false なら値を確かめるだけで反映しない
bool ApplyColorKey(const std::wstring& key, const std::wstring& value, bool apply,
                   PanelTheme* theme, const std::wstring& where, std::vector<std::wstring>* log)
{
    const auto found = std::find_if(std::begin(kColorKeys), std::end(kColorKeys),
                                    [&key](const auto& entry) { return key == entry.name; });
    if (found == std::end(kColorKeys)) {
        return false;
    }
    PanelColor color;
    if (!ParseColor(value, &color)) {
        log->push_back(where + L"不正な色 " + key + L"\t" + value + L" を無視");
        return true;
    }
    if (apply) {
        theme->*(found->member) = color;
    }
    return true;
}

struct ThemeLine {
    std::wstring key;
    std::wstring value;
    int number;  // 行番号 (1 始まり)
};

std::vector<ThemeLine> SplitThemeLines(const std::wstring& text)
{
    std::vector<ThemeLine> lines;
    size_t start = 0;
    int number = 0;
    while (start < text.size()) {
        size_t end = text.find(L'\n', start);
        if (end == std::wstring::npos) {
            end = text.size();
        }
        std::wstring line = text.substr(start, end - start);
        start = end + 1;
        ++number;
        if (!line.empty() && line.back() == L'\r') {
            line.pop_back();
        }
        if (line.empty() || line[0] == L'#') {
            continue;
        }
        const size_t tab = line.find(L'\t');
        if (tab == std::wstring::npos) {
            lines.push_back({line, L"", number});
            continue;
        }
        lines.push_back({line.substr(0, tab), line.substr(tab + 1), number});
    }
    return lines;
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

// テーマフォルダ。優先順: QUICKLIME_THEME_DIR > %APPDATA%\QuicklIME\themes
std::wstring ThemeDir()
{
    wchar_t buf[MAX_PATH];
    DWORD len = GetEnvironmentVariableW(L"QUICKLIME_THEME_DIR", buf, MAX_PATH);
    if (len > 0 && len < MAX_PATH) {
        return std::wstring(buf, len);
    }
    len = GetEnvironmentVariableW(L"APPDATA", buf, MAX_PATH);
    if (len > 0 && len < MAX_PATH) {
        return std::wstring(buf, len) + L"\\QuicklIME\\themes";
    }
    return {};
}

// テーマ名がファイル名として使えるか (フォルダの外を指す名前は受け付けない)
bool IsValidThemeName(const std::wstring& name)
{
    return !name.empty() && name != L"." && name != L".." &&
           name.find_first_of(L"\\/:*?\"<>|") == std::wstring::npos;
}

bool ReadUtf8File(const std::wstring& path, std::wstring* text)
{
    std::ifstream file(path.c_str(), std::ios::binary);
    if (!file) {
        return false;
    }
    std::string bytes((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    if (bytes.rfind("\xEF\xBB\xBF", 0) == 0) {  // UTF-8 BOM
        bytes.erase(0, 3);
    }
    *text = Utf8ToWide(bytes);
    return true;
}

void ApplyThemeTextAt(const std::wstring& source, const std::wstring& builtinName,
                      const std::wstring& text, bool dark, PanelTheme* theme,
                      std::vector<std::wstring>* log, int depth)
{
    const std::vector<ThemeLine> lines = SplitThemeLines(text);
    const auto where = [&source](const ThemeLine& line) {
        return source + L" " + std::to_wstring(line.number) + L" 行目: ";
    };

    // base は何行目に書いても他のキーより先に重ねる (複数あれば後の行)
    std::wstring base = kDefaultTheme;
    const ThemeLine* baseLine = nullptr;
    for (const ThemeLine& line : lines) {
        if (line.key == L"base") {
            base = line.value;
            baseLine = &line;
        }
    }
    const wchar_t* baseText = BuiltinThemeText(base);
    if (baseText == nullptr) {
        log->push_back(where(*baseLine) + L"組み込みのテーマに無い base " + base + L" を無視 (" +
                       kDefaultTheme + L" を使う)");
        base = kDefaultTheme;
        baseText = BuiltinThemeText(base);
    }
    if (base != builtinName && depth < kMaxBaseDepth) {
        ApplyThemeTextAt(base, base, baseText, dark, theme, log, depth + 1);
    }

    // light./dark. の付いた行は、同じファイルにある付かない行より優先する
    for (const bool prefixedPass : {false, true}) {
        for (const ThemeLine& line : lines) {
            if (line.key == L"base") {
                continue;
            }
            std::wstring key = line.key;
            bool prefixed = false;
            bool matches = true;
            for (const auto& [prefix, forDark] :
                 {std::pair<const wchar_t*, bool>{L"light.", false}, {L"dark.", true}}) {
                const size_t length = wcslen(prefix);
                if (key.compare(0, length, prefix) == 0) {
                    key.erase(0, length);
                    prefixed = true;
                    matches = forDark == dark;
                    break;
                }
            }
            if (prefixed != prefixedPass) {
                continue;
            }
            if (!prefixed && ApplyShapeKey(key, line.value, theme, where(line), log)) {
                continue;
            }
            if (ApplyColorKey(key, line.value, matches, theme, where(line), log)) {
                continue;
            }
            if (prefixed && IsShapeKey(key)) {
                log->push_back(where(line) + L"色以外のキー " + key +
                               L" には light./dark. を付けられないため無視");
                continue;
            }
            log->push_back(where(line) + L"不明なキー " + line.key + L" を無視");
        }
    }
}

} // namespace

bool PanelTheme::SameShape(const PanelTheme& other) const
{
    PanelTheme copy = other;
    for (const auto& entry : kColorKeys) {
        copy.*(entry.member) = this->*(entry.member);
    }
    copy.dark = dark;
    return copy == *this;
}

PanelTheme PanelTheme::Defaults(bool dark)
{
    PanelTheme theme;
    theme.dark = dark;
    theme.accent = Rgba(0x2B86EE);
    theme.selectedText = Rgba(0xFFFFFF);
    theme.tagConversion = Rgba(0x2B86EE);
    if (dark) {
        theme.background = Rgba(0x1C1C22, 0xEB);
        theme.border = Rgba(0xFFFFFF, 0x14);
        theme.text = Rgba(0xF2F2F6);
        theme.secondary = Rgba(0xA0A0AD);
        theme.chip = Rgba(0xFFFFFF, 0x12);
        theme.separator = Rgba(0xFFFFFF, 0x17);
        theme.nextWord = Rgba(0x3CC48A);
        theme.shadow = Rgba(0x000000, 0x73);
        theme.blurTint = Rgba(0x1C1C22, 0x99);
        theme.scrim = Rgba(0x000000, 0xBF);
        theme.tagAscii = Rgba(0xA0A0AD);
        theme.tagPrediction = Rgba(0xA0A0AD);
        theme.tagPartial = Rgba(0xC08CF0);
        theme.tagNextWord = Rgba(0x3CC48A);
    } else {
        theme.background = Rgba(0xFCFCFE, 0xEB);
        theme.border = Rgba(0x000000, 0x14);
        theme.text = Rgba(0x16161C);
        theme.secondary = Rgba(0x6B6B78);
        theme.chip = Rgba(0x000000, 0x0B);
        theme.separator = Rgba(0x000000, 0x14);
        theme.nextWord = Rgba(0x2E9E6B);
        theme.shadow = Rgba(0x000000, 0x47);
        theme.blurTint = Rgba(0xFCFCFE, 0xB3);
        theme.scrim = Rgba(0xFFFFFF, 0xE6);
        theme.tagAscii = Rgba(0x6B6B78);
        theme.tagPrediction = Rgba(0x6B6B78);
        theme.tagPartial = Rgba(0x8A4FC8);
        theme.tagNextWord = Rgba(0x2E9E6B);
    }
    return theme;
}

bool AppsUseLightTheme()
{
    DWORD value = 1;
    DWORD size = sizeof(value);
    if (RegGetValueW(HKEY_CURRENT_USER,
                     L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                     L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &value,
                     &size) != ERROR_SUCCESS) {
        value = 1;
    }
    return value != 0;
}

const wchar_t* BuiltinThemeText(const std::wstring& name)
{
    for (const auto& entry : kBuiltinThemes) {
        if (name == entry.name) {
            return entry.text;
        }
    }
    return nullptr;
}

void ApplyThemeText(const std::wstring& source, const std::wstring& builtinName,
                    const std::wstring& text, bool dark, PanelTheme* theme,
                    std::vector<std::wstring>* log)
{
    ApplyThemeTextAt(source, builtinName, text, dark, theme, log, 0);
}

bool PanelThemeLoader::Refresh(const std::wstring& name, bool force,
                               std::vector<std::wstring>* log)
{
    const bool validName = IsValidThemeName(name);
    const std::wstring dir = ThemeDir();
    const std::wstring path = validName && !dir.empty() ? dir + L"\\" + name + L".tsv" : L"";
    FILETIME lastWrite = {};
    bool exists = false;
    if (!path.empty()) {
        WIN32_FILE_ATTRIBUTE_DATA attr = {};
        if (GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &attr) &&
            (attr.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) == 0) {
            lastWrite = attr.ftLastWriteTime;
            exists = true;
        }
    }
    if (!force && loaded_ && name == name_ && exists == exists_ &&
        CompareFileTime(&lastWrite, &lastWrite_) == 0) {
        return false;
    }
    name_ = name;
    lastWrite_ = lastWrite;
    exists_ = exists;
    loaded_ = true;

    const bool dark = !AppsUseLightTheme();
    PanelTheme theme = PanelTheme::Defaults(dark);
    std::wstring text;
    if (exists && ReadUtf8File(path, &text)) {
        ApplyThemeText(name + L".tsv", L"", text, dark, &theme, log);
        log->push_back(L"テーマファイル " + path + L" を読んだ (" +
                       (dark ? L"ダーク" : L"ライト") + L")");
    } else if (const wchar_t* builtin = validName ? BuiltinThemeText(name) : nullptr) {
        ApplyThemeText(name, name, builtin, dark, &theme, log);
        log->push_back(L"組み込みのテーマ " + name + L" を使う (" + (dark ? L"ダーク" : L"ライト") +
                       L")");
    } else {
        ApplyThemeText(kDefaultTheme, kDefaultTheme, BuiltinThemeText(kDefaultTheme), dark, &theme,
                       log);
        log->push_back(L"テーマ " + name + L" が見つからないため " + kDefaultTheme + L" を使う");
    }
    theme_ = theme;
    return true;
}
