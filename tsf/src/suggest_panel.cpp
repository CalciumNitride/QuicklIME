#include "suggest_panel.h"

#include <d2d1.h>
#include <dwrite.h>

#include <algorithm>
#include <cmath>

#include "debug_log.h"
#include "globals.h"

namespace {

constexpr wchar_t kWindowClassName[] = L"QuicklIMESuggestPanel";
constexpr wchar_t kFallbackFont[] = L"Yu Gothic UI";
constexpr wchar_t kNextWordLabel[] = L"次の語";
constexpr UINT_PTR kTimerId = 1;  // StartTimer のタイマー (ウィンドウごとに1つ)
constexpr int kAnchorGap = 2;     // キャレット行とパネルの間

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
}

PanelColor Rgb(unsigned rgb, float alpha = 1.0f)
{
    return {static_cast<float>((rgb >> 16) & 0xFF) / 255.0f,
            static_cast<float>((rgb >> 8) & 0xFF) / 255.0f,
            static_cast<float>(rgb & 0xFF) / 255.0f, alpha};
}

D2D1_COLOR_F ToD2D(const PanelColor& color)
{
    return D2D1::ColorF(color.r, color.g, color.b, color.a);
}

// f に比例させた寸法 (px)
struct Metrics {
    float f;
    int padding;
    int chipHeight;
    float chipPadding;
    float chipGap;
    float badgeSize;
    float badgeGap;
    float labelGap;
    int inputRowHeight;
    int rowGap;
    int line;
    float panelRadius;
    float shadowSpread;
    float shadowOffset;
    int margin;  // 影のためにパネルの外側に取る余白
};

int Scaled(float ratio, float f)
{
    return (std::max)(1, static_cast<int>(std::lround(ratio * f)));
}

Metrics ComputeMetrics(const PanelTheme& theme, int fontSize)
{
    Metrics m = {};
    m.f = static_cast<float>(fontSize);
    m.padding = Scaled(theme.panelPadding, m.f);
    m.chipHeight = Scaled(theme.chipHeight, m.f);
    m.chipPadding = theme.chipPadding * m.f;
    m.chipGap = theme.chipGap * m.f;
    m.badgeSize = theme.badgeSize * m.f;
    m.badgeGap = theme.badgeGap * m.f;
    m.labelGap = theme.labelGap * m.f;
    m.inputRowHeight = Scaled(theme.inputRowHeight, m.f);
    m.rowGap = Scaled(theme.rowGap, m.f);
    m.line = (std::max)(1, static_cast<int>(std::lround(theme.lineWidthPx)));
    m.panelRadius = theme.panelRadius * m.f;
    m.shadowSpread = theme.shadowSize * m.f;
    m.shadowOffset = theme.shadowOffset * m.f;
    m.margin = static_cast<int>(std::ceil(m.shadowSpread + m.shadowOffset));
    return m;
}

// パネル (影を除く) の角丸の半径。カプセルは両端を半円にする
float BodyRadius(bool capsule, const Metrics& m, float height)
{
    return capsule ? height / 2 : (std::min)(m.panelRadius, height / 2);
}

// ウィンドウクラスを一度だけ登録する
bool EnsureWindowClass()
{
    static bool registered = false;
    if (registered) {
        return true;
    }

    WNDCLASSEXW wc = {};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = SuggestPanel::WndProc;
    wc.hInstance = globals::dllInstance;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.lpszClassName = kWindowClassName;
    if (RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return false;
    }
    registered = true;
    return true;
}

RECT WorkAreaOf(HMONITOR monitor)
{
    MONITORINFO info = {};
    info.cbSize = sizeof(info);
    if (monitor != nullptr && GetMonitorInfoW(monitor, &info)) {
        return info.rcWork;
    }
    RECT work = {};
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
    return work;
}

// 基準矩形 anchor を基準に (x, y) に置く width x height の矩形を、anchor を含むモニタの
// 作業領域に収めた位置を返す。下にはみ出すなら anchor の上側、それでも収まらなければ (anchor が
// タスクバーに重なる位置にあるときなど) 作業領域の下端に寄せる
POINT FitToWorkArea(const RECT& anchor, int x, int y, int width, int height)
{
    const POINT origin = {anchor.left, anchor.top};
    const RECT work = WorkAreaOf(MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST));
    if (x + width > work.right) {
        x = static_cast<int>(work.right) - width;
    }
    x = (std::max)(x, static_cast<int>(work.left));
    if (y + height > work.bottom) {
        y = static_cast<int>(anchor.top) - height - kAnchorGap;
        if (y < work.top || y + height > work.bottom) {
            y = static_cast<int>(work.bottom) - height;
        }
    }
    y = (std::max)(y, static_cast<int>(work.top));
    return {x, y};
}

bool FontExists(IDWriteFactory* factory, const std::wstring& face)
{
    IDWriteFontCollection* fonts = nullptr;
    if (FAILED(factory->GetSystemFontCollection(&fonts, FALSE)) || fonts == nullptr) {
        return false;
    }
    UINT32 index = 0;
    BOOL exists = FALSE;
    const bool found = SUCCEEDED(fonts->FindFamilyName(face.c_str(), &index, &exists)) && exists;
    fonts->Release();
    return found;
}

IDWriteTextFormat* CreateFormat(IDWriteFactory* factory, const std::wstring& face, float size,
                                DWRITE_TEXT_ALIGNMENT alignment)
{
    IDWriteTextFormat* format = nullptr;
    if (FAILED(factory->CreateTextFormat(face.c_str(), nullptr, DWRITE_FONT_WEIGHT_NORMAL,
                                         DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL,
                                         size, L"ja-jp", &format))) {
        return nullptr;
    }
    format->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
    format->SetTextAlignment(alignment);
    format->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    return format;
}

} // namespace

// ---- PanelTheme ----

PanelTheme PanelTheme::Dark()
{
    PanelTheme theme;
    theme.background = Rgb(0x1C1C22, 0.92f);
    theme.border = Rgb(0xFFFFFF, 0.08f);
    theme.text = Rgb(0xF2F2F6);
    theme.secondary = Rgb(0xA0A0AD);
    theme.chip = Rgb(0xFFFFFF, 0.07f);
    theme.separator = Rgb(0xFFFFFF, 0.09f);
    theme.accent = Rgb(0x2B86EE);
    theme.selectedText = Rgb(0xFFFFFF);
    theme.nextWord = Rgb(0x3CC48A);
    theme.shadow = Rgb(0x000000, 0.45f);
    return theme;
}

PanelTheme PanelTheme::Light()
{
    PanelTheme theme;
    theme.background = Rgb(0xFCFCFE, 0.92f);
    theme.border = Rgb(0x000000, 0.08f);
    theme.text = Rgb(0x16161C);
    theme.secondary = Rgb(0x6B6B78);
    theme.chip = Rgb(0x000000, 0.045f);
    theme.separator = Rgb(0x000000, 0.08f);
    theme.accent = Rgb(0x2B86EE);
    theme.selectedText = Rgb(0xFFFFFF);
    theme.nextWord = Rgb(0x2E9E6B);
    theme.shadow = Rgb(0x000000, 0.28f);
    return theme;
}

PanelTheme PanelTheme::ForAppMode()
{
    DWORD value = 1;
    DWORD size = sizeof(value);
    if (RegGetValueW(HKEY_CURRENT_USER,
                     L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                     L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &value,
                     &size) != ERROR_SUCCESS) {
        value = 1;
    }
    return value != 0 ? Light() : Dark();
}

// ---- SuggestPanel ----

SuggestPanel::SuggestPanel()
    : hwnd_(nullptr), destroying_(false), d2dFactory_(nullptr), dwriteFactory_(nullptr),
      target_(nullptr), brush_(nullptr), dotStroke_(nullptr), textFormat_(nullptr),
      inputFormat_(nullptr), badgeFormat_(nullptr), labelFormat_(nullptr), fontSize_(18),
      theme_(PanelTheme::Light()), kind_(Kind::Panel), selection_(0), hasInput_(false),
      bodyX_(0), bodyY_(0), bodyWidth_(0), bodyHeight_(0), margin_(0)
{
    SetFont(kFallbackFont, 18);
}

SuggestPanel::~SuggestPanel()
{
    // ロガーは所有者 (TextService) のメンバを参照するため、所有者の破棄中には呼ばない
    logger_ = nullptr;
    Hide();
    ReleaseTarget();
    SafeRelease(&textFormat_);
    SafeRelease(&inputFormat_);
    SafeRelease(&badgeFormat_);
    SafeRelease(&labelFormat_);
    SafeRelease(&dotStroke_);
    SafeRelease(&dwriteFactory_);
    SafeRelease(&d2dFactory_);
}

bool SuggestPanel::EnsureFactories()
{
    if (dwriteFactory_ == nullptr &&
        FAILED(DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
                                   reinterpret_cast<IUnknown**>(&dwriteFactory_)))) {
        dwriteFactory_ = nullptr;
    }
    // TextService はスレッドごとに1インスタンスで、パネルも同じスレッドからしか触らない
    if (d2dFactory_ == nullptr &&
        FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2dFactory_))) {
        d2dFactory_ = nullptr;
    }
    if (d2dFactory_ != nullptr && dotStroke_ == nullptr) {
        const float dashes[] = {2.0f, 2.0f};
        const D2D1_STROKE_STYLE_PROPERTIES properties = D2D1::StrokeStyleProperties(
            D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_FLAT, D2D1_LINE_JOIN_MITER,
            10.0f, D2D1_DASH_STYLE_CUSTOM, 0.0f);
        if (FAILED(d2dFactory_->CreateStrokeStyle(properties, dashes, ARRAYSIZE(dashes),
                                                  &dotStroke_))) {
            dotStroke_ = nullptr;
        }
    }
    return dwriteFactory_ != nullptr && d2dFactory_ != nullptr;
}

bool SuggestPanel::EnsureTarget()
{
    if (target_ != nullptr) {
        return true;
    }
    if (!EnsureFactories()) {
        return false;
    }
    // ホストのプロセス (ゲームなど) に D3D のデバイスを作らないよう、ソフトウェアで描く。
    // DPI は 96 に固定し、DIP を px として扱う (DPI による拡大は行わない)
    const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), 96.0f,
        96.0f);
    if (FAILED(d2dFactory_->CreateDCRenderTarget(&properties, &target_))) {
        target_ = nullptr;
        return false;
    }
    if (FAILED(target_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0, 1), &brush_))) {
        ReleaseTarget();
        return false;
    }
    // 透過する背景の上では ClearType の色付きの縁が浮くため、グレースケールにする
    target_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    return true;
}

void SuggestPanel::ReleaseTarget()
{
    SafeRelease(&brush_);
    SafeRelease(&target_);
}

void SuggestPanel::SetFont(const std::wstring& face, int height)
{
    if (!EnsureFactories()) {
        return;
    }
    const std::wstring actual = FontExists(dwriteFactory_, face) ? face : kFallbackFont;
    const float f = static_cast<float>(height);
    IDWriteTextFormat* text =
        CreateFormat(dwriteFactory_, actual, f, DWRITE_TEXT_ALIGNMENT_LEADING);
    IDWriteTextFormat* input = CreateFormat(dwriteFactory_, actual, theme_.inputTextSize * f,
                                            DWRITE_TEXT_ALIGNMENT_LEADING);
    IDWriteTextFormat* badge = CreateFormat(dwriteFactory_, actual, theme_.badgeTextSize * f,
                                            DWRITE_TEXT_ALIGNMENT_CENTER);
    IDWriteTextFormat* label = CreateFormat(dwriteFactory_, actual, theme_.labelTextSize * f,
                                            DWRITE_TEXT_ALIGNMENT_LEADING);
    if (text == nullptr || input == nullptr || badge == nullptr || label == nullptr) {
        SafeRelease(&text);
        SafeRelease(&input);
        SafeRelease(&badge);
        SafeRelease(&label);
        return;
    }
    SafeRelease(&textFormat_);
    SafeRelease(&inputFormat_);
    SafeRelease(&badgeFormat_);
    SafeRelease(&labelFormat_);
    textFormat_ = text;
    inputFormat_ = input;
    badgeFormat_ = badge;
    labelFormat_ = label;
    face_ = face;
    fontSize_ = height;
}

void SuggestPanel::SetTheme(const PanelTheme& theme)
{
    // 文字の大きさの比率も theme が持つので、書式を作り直す
    theme_ = theme;
    SetFont(face_, fontSize_);
    if (hwnd_ != nullptr) {
        Render(L"SetTheme", nullptr);
    }
}

float SuggestPanel::TextWidth(const std::wstring& text, IDWriteTextFormat* format) const
{
    if (text.empty() || format == nullptr || dwriteFactory_ == nullptr) {
        return 0.0f;
    }
    IDWriteTextLayout* layout = nullptr;
    if (FAILED(dwriteFactory_->CreateTextLayout(text.c_str(), static_cast<UINT32>(text.size()),
                                                format, 100000.0f, 1000.0f, &layout))) {
        return 0.0f;
    }
    DWRITE_TEXT_METRICS metrics = {};
    const float width =
        SUCCEEDED(layout->GetMetrics(&metrics)) ? metrics.widthIncludingTrailingWhitespace : 0.0f;
    layout->Release();
    return width;
}

size_t SuggestPanel::Show(const Placement& placement, const std::vector<Item>& items,
                          size_t selection, const InputLine* input)
{
    if (textFormat_ == nullptr ||
        (placement.position == Position::Caret && placement.caret == nullptr)) {
        Hide();
        return 0;
    }
    const Metrics m = ComputeMetrics(theme_, fontSize_);

    std::vector<std::wstring> texts;
    std::vector<float> widths;
    for (size_t i = 0; i < items.size() && i < kMaxItems; ++i) {
        std::wstring text = items[i].partial ? items[i].text + L"…" : items[i].text;
        widths.push_back(m.chipPadding * 2 + m.badgeSize + m.badgeGap +
                         TextWidth(text, textFormat_));
        texts.push_back(std::move(text));
    }

    float inputWidth = 0.0f;
    if (input != nullptr) {
        if (input->text.empty() && input->pending.empty()) {
            inputWidth = TextWidth(input->context, inputFormat_);
            if (input->nextWordLabel) {
                inputWidth += (input->context.empty() ? 0.0f : m.labelGap) +
                              TextWidth(kNextWordLabel, labelFormat_);
            }
        } else {
            inputWidth = TextWidth(input->text, inputFormat_) +
                         TextWidth(input->pending, inputFormat_) + 1.0f + theme_.caretWidthPx;
        }
    }
    const bool hasInput = input != nullptr;
    // 入力行を出すときは、候補が無くてもチップ列の高さを取って入力行の位置を上下させない
    int height = m.padding * 2 + m.chipHeight;
    if (hasInput) {
        height += m.inputRowHeight + m.rowGap * 2 + m.line;
    }

    size_t count = 0;
    float chipsWidth = 0.0f;
    int x = 0;
    int y = 0;
    int width = 0;
    if (placement.position == Position::Center) {
        HMONITOR monitor = nullptr;
        if (placement.caret != nullptr) {
            monitor = MonitorFromRect(placement.caret, MONITOR_DEFAULTTONEAREST);
        } else {
            HWND foreground = GetForegroundWindow();
            monitor = foreground != nullptr
                          ? MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
                          : MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY);
        }
        const RECT work = WorkAreaOf(monitor);
        const int workWidth = static_cast<int>(work.right - work.left);
        const int workHeight = static_cast<int>(work.bottom - work.top);
        // 幅は候補の数によらず固定し、左詰めにして打鍵ごとに候補の位置を動かさない
        width = static_cast<int>(
            (std::min)(theme_.centerWidth * m.f,
                       static_cast<float>(workWidth) * theme_.centerWidthRatio));
        const float available = static_cast<float>(width - m.padding * 2);
        for (size_t i = 0; i < widths.size(); ++i) {
            const float next = chipsWidth + (i > 0 ? m.chipGap : 0.0f) + widths[i];
            if (next > available) {
                break;
            }
            chipsWidth = next;
            ++count;
        }
        x = static_cast<int>(work.left) + (workWidth - width) / 2;
        y = static_cast<int>(work.top) + (workHeight - height) / 2;
    } else {
        const RECT& caret = *placement.caret;
        const POINT origin = {placement.x, caret.bottom};
        const RECT work = WorkAreaOf(MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST));
        x = (std::max)(placement.x, static_cast<int>(work.left));
        for (size_t i = 0; i < widths.size(); ++i) {
            const float next = chipsWidth + (i > 0 ? m.chipGap : 0.0f) + widths[i];
            if (static_cast<float>(x + m.padding * 2) + next > static_cast<float>(work.right)) {
                break;
            }
            chipsWidth = next;
            ++count;
        }
        if (count == 0 && !widths.empty()) {
            // 先頭の候補も入らない位置では、先頭の候補が収まるところまで左へ寄せる
            count = 1;
            chipsWidth = widths[0];
            const int needed = m.padding * 2 + static_cast<int>(std::ceil(chipsWidth));
            x = (std::max)(static_cast<int>(work.left), static_cast<int>(work.right) - needed);
        }
        const int chipsNeeded = m.padding * 2 + static_cast<int>(std::ceil(chipsWidth));
        width = m.padding * 2 + static_cast<int>(std::ceil((std::max)(chipsWidth, inputWidth)));
        width = (std::max)(width, m.chipHeight * 2);
        // 入力行が長いときは作業領域の右端で切る (候補は上で収まる分だけにしてある)
        width = (std::max)((std::min)(width, static_cast<int>(work.right) - x), chipsNeeded);

        // 未完成ローマ字のカプセル (チップと同じ高さ) はキャレットに重ねて出し、打鍵ごとに
        // 出たり消えたりする。カプセルの有無で上下に動かないよう、常にその下端より下に出す
        int top = static_cast<int>(caret.bottom);
        if (placement.belowCapsule) {
            top = (std::max)(top, static_cast<int>(caret.top) + m.chipHeight);
        }
        y = top + kAnchorGap;
        if (y + height > work.bottom) {
            y = static_cast<int>(caret.top) - height - kAnchorGap;
        }
    }

    if (count == 0 && !hasInput) {
        Hide();
        return 0;
    }

    kind_ = Kind::Panel;
    texts.resize(count);
    itemTexts_ = std::move(texts);
    items_.assign(items.begin(), items.begin() + static_cast<std::ptrdiff_t>(count));
    chipX_.clear();
    chipWidth_.clear();
    float chipX = 0.0f;
    for (size_t i = 0; i < count; ++i) {
        chipX_.push_back(chipX);
        chipWidth_.push_back(widths[i]);
        chipX += widths[i] + m.chipGap;
    }
    selection_ = selection;
    hasInput_ = hasInput;
    input_ = hasInput ? *input : InputLine{};
    capsuleText_.clear();
    bodyX_ = x;
    bodyY_ = y;
    bodyWidth_ = width;
    bodyHeight_ = height;
    margin_ = m.margin;
    if (!Render(L"Show", placement.caret)) {
        return 0;
    }
    return count;
}

bool SuggestPanel::ShowCapsule(const RECT& caret, const std::wstring& text)
{
    if (text.empty() || textFormat_ == nullptr) {
        return false;
    }
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const int width =
        static_cast<int>(std::ceil(m.chipPadding * 2 + TextWidth(text, textFormat_)));
    const int height = m.chipHeight;
    const POINT position = FitToWorkArea(caret, caret.left, caret.top, width, height);

    kind_ = Kind::Capsule;
    items_.clear();
    itemTexts_.clear();
    chipX_.clear();
    chipWidth_.clear();
    selection_ = 0;
    hasInput_ = false;
    input_ = InputLine{};
    capsuleText_ = text;
    bodyX_ = position.x;
    bodyY_ = position.y;
    bodyWidth_ = width;
    bodyHeight_ = height;
    margin_ = m.margin;
    return Render(L"ShowCapsule", &caret);
}

bool SuggestPanel::Render(const wchar_t* kind, const RECT* anchor)
{
    if (!EnsureWindowClass()) {
        if (logger_) {
            logger_(std::wstring(kind) + L": ウィンドウクラスの登録に失敗 error=" +
                    std::to_wstring(GetLastError()));
        }
        return false;
    }
    if (!EnsureTarget()) {
        if (logger_) {
            logger_(std::wstring(kind) + L": Direct2D の描画先を作れない");
        }
        Hide();
        return false;
    }
    const int windowWidth = bodyWidth_ + margin_ * 2;
    const int windowHeight = bodyHeight_ + margin_ * 2;

    HDC screen = GetDC(nullptr);
    HDC memory = CreateCompatibleDC(screen);
    BITMAPINFO info = {};
    info.bmiHeader.biSize = sizeof(info.bmiHeader);
    info.bmiHeader.biWidth = windowWidth;
    info.bmiHeader.biHeight = -windowHeight;  // 上から下へ並ぶ DIB
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    void* bits = nullptr;
    HBITMAP bitmap = memory != nullptr
                         ? CreateDIBSection(memory, &info, DIB_RGB_COLORS, &bits, nullptr, 0)
                         : nullptr;
    HGDIOBJ oldBitmap = bitmap != nullptr ? SelectObject(memory, bitmap) : nullptr;

    HRESULT hr = E_FAIL;
    if (bitmap != nullptr) {
        const RECT rect = {0, 0, windowWidth, windowHeight};
        // 描画先を作り直す必要が出たときは1度だけやり直す
        for (int attempt = 0; attempt < 2; ++attempt) {
            if (!EnsureTarget()) {
                break;
            }
            hr = target_->BindDC(memory, &rect);
            if (SUCCEEDED(hr)) {
                target_->BeginDraw();
                target_->Clear(D2D1::ColorF(0, 0, 0, 0));
                Draw();
                hr = target_->EndDraw();
            }
            if (hr != D2DERR_RECREATE_TARGET) {
                break;
            }
            ReleaseTarget();
        }
    }

    bool created = false;
    DWORD createError = ERROR_SUCCESS;
    bool updated = false;
    if (SUCCEEDED(hr)) {
        if (hwnd_ == nullptr) {
            hwnd_ = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                kWindowClassName, L"", WS_POPUP, bodyX_ - margin_, bodyY_ - margin_, windowWidth,
                windowHeight, nullptr, nullptr, globals::dllInstance, this);
            if (hwnd_ == nullptr) {
                createError = GetLastError();
            } else {
                created = true;
            }
        }
        if (hwnd_ != nullptr) {
            POINT destination = {bodyX_ - margin_, bodyY_ - margin_};
            SIZE size = {windowWidth, windowHeight};
            POINT source = {0, 0};
            BLENDFUNCTION blend = {AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
            updated = UpdateLayeredWindow(hwnd_, screen, &destination, &size, memory, &source, 0,
                                          &blend, ULW_ALPHA) != FALSE;
            if (!updated) {
                createError = GetLastError();
            }
            SetWindowPos(hwnd_, HWND_TOPMOST, 0, 0, 0, 0,
                         SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            ShowWindow(hwnd_, SW_SHOWNA);
        }
    }

    if (oldBitmap != nullptr) {
        SelectObject(memory, oldBitmap);
    }
    if (bitmap != nullptr) {
        DeleteObject(bitmap);
    }
    if (memory != nullptr) {
        DeleteDC(memory);
    }
    ReleaseDC(nullptr, screen);

    if (FAILED(hr)) {
        if (logger_) {
            logger_(std::wstring(kind) + L": 描画に失敗 hr=" + DebugHResultText(hr));
        }
        return false;
    }
    LogShow(kind, anchor, created, updated ? ERROR_SUCCESS : createError);
    return hwnd_ != nullptr && updated;
}

void SuggestPanel::Draw()
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float margin = static_cast<float>(margin_);
    const float width = static_cast<float>(bodyWidth_);
    const float height = static_cast<float>(bodyHeight_);
    const float line = static_cast<float>(m.line);
    const float radius = BodyRadius(kind_ == Kind::Capsule, m, height);

    DrawShadow();

    const D2D1_RECT_F body = D2D1::RectF(margin, margin, margin + width, margin + height);
    brush_->SetColor(ToD2D(theme_.background));
    target_->FillRoundedRectangle(D2D1::RoundedRect(body, radius, radius), brush_);
    const float half = line / 2;
    brush_->SetColor(ToD2D(theme_.border));
    target_->DrawRoundedRectangle(
        D2D1::RoundedRect(D2D1::RectF(body.left + half, body.top + half, body.right - half,
                                      body.bottom - half),
                          radius - half, radius - half),
        brush_, line);

    if (kind_ == Kind::Capsule) {
        brush_->SetColor(ToD2D(theme_.text));
        const D2D1_RECT_F rect =
            D2D1::RectF(body.left + m.chipPadding, body.top, body.right, body.bottom);
        target_->DrawText(capsuleText_.c_str(), static_cast<UINT32>(capsuleText_.size()),
                          textFormat_, rect, brush_);
        return;
    }

    const float padding = static_cast<float>(m.padding);
    const float left = body.left + padding;
    const float contentWidth = width - padding * 2;
    float top = body.top + padding;
    if (hasInput_) {
        DrawInputLine(left, top, contentWidth);
        top += static_cast<float>(m.inputRowHeight + m.rowGap);
        brush_->SetColor(ToD2D(theme_.separator));
        target_->FillRectangle(D2D1::RectF(left, top, left + contentWidth, top + line), brush_);
        top += line + static_cast<float>(m.rowGap);
    }
    for (size_t i = 0; i < itemTexts_.size(); ++i) {
        DrawChip(i, left + chipX_[i], top);
    }
}

void SuggestPanel::DrawShadow()
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const int steps = (std::max)(1, theme_.shadowSteps);
    if (theme_.shadow.a <= 0.0f || m.shadowSpread <= 0.0f) {
        return;
    }
    const float margin = static_cast<float>(margin_);
    const float width = static_cast<float>(bodyWidth_);
    const float height = static_cast<float>(bodyHeight_);
    const float radius = BodyRadius(kind_ == Kind::Capsule, m, height);
    // ID2D1DCRenderTarget ではブラーのエフェクトが使えないため、少しずつ広げた角丸矩形を外側から
    // 重ねて近似する。全部が重なるパネルの縁で最大不透明度になるよう、1枚の不透明度を決める
    const float alpha =
        1.0f - std::pow(1.0f - theme_.shadow.a, 1.0f / static_cast<float>(steps));
    PanelColor color = theme_.shadow;
    color.a = alpha;
    brush_->SetColor(ToD2D(color));
    for (int k = steps; k >= 1; --k) {
        const float spread = m.shadowSpread * static_cast<float>(k) / static_cast<float>(steps);
        const D2D1_RECT_F rect = D2D1::RectF(margin - spread, margin - spread + m.shadowOffset,
                                             margin + width + spread,
                                             margin + height + spread + m.shadowOffset);
        target_->FillRoundedRectangle(D2D1::RoundedRect(rect, radius + spread, radius + spread),
                                      brush_);
    }
}

void SuggestPanel::DrawInputLine(float left, float top, float width)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float rowHeight = static_cast<float>(m.inputRowHeight);
    const float big = 100000.0f;
    target_->PushAxisAlignedClip(D2D1::RectF(left, top, left + width, top + rowHeight),
                                 D2D1_ANTIALIAS_MODE_ALIASED);

    if (input_.text.empty() && input_.pending.empty()) {
        float x = left;
        if (!input_.context.empty()) {
            brush_->SetColor(ToD2D(theme_.secondary));
            target_->DrawText(input_.context.c_str(), static_cast<UINT32>(input_.context.size()),
                              inputFormat_, D2D1::RectF(x, top, x + big, top + rowHeight), brush_);
            x += TextWidth(input_.context, inputFormat_) + m.labelGap;
        }
        if (input_.nextWordLabel) {
            brush_->SetColor(ToD2D(theme_.nextWord));
            target_->DrawText(kNextWordLabel, static_cast<UINT32>(wcslen(kNextWordLabel)),
                              labelFormat_, D2D1::RectF(x, top, x + big, top + rowHeight), brush_);
        }
        target_->PopAxisAlignedClip();
        return;
    }

    // 下線とキャレットの位置は、文字列全体を同じ書式で並べたときの行の寸法から決める
    const std::wstring whole = input_.text + input_.pending;
    float lineTop = top;
    float lineHeight = rowHeight;
    float baseline = rowHeight * 0.75f;
    IDWriteTextLayout* layout = nullptr;
    if (SUCCEEDED(dwriteFactory_->CreateTextLayout(whole.c_str(), static_cast<UINT32>(whole.size()),
                                                   inputFormat_, big, rowHeight, &layout))) {
        DWRITE_TEXT_METRICS metrics = {};
        DWRITE_LINE_METRICS lineMetrics = {};
        UINT32 lineCount = 0;
        if (SUCCEEDED(layout->GetMetrics(&metrics)) &&
            SUCCEEDED(layout->GetLineMetrics(&lineMetrics, 1, &lineCount)) && lineCount >= 1) {
            lineTop = top + metrics.top;
            lineHeight = lineMetrics.height;
            baseline = lineMetrics.baseline;
        }
        layout->Release();
    }
    const float inputSize = theme_.inputTextSize * m.f;
    const float underlineY =
        std::round(lineTop + baseline + (std::max)(2.0f, inputSize * 0.15f));
    const float line = static_cast<float>(m.line);

    float x = left;
    if (!input_.text.empty()) {
        const float textWidth = TextWidth(input_.text, inputFormat_);
        brush_->SetColor(ToD2D(theme_.text));
        target_->DrawText(input_.text.c_str(), static_cast<UINT32>(input_.text.size()),
                          inputFormat_, D2D1::RectF(x, top, x + big, top + rowHeight), brush_);
        brush_->SetColor(ToD2D(theme_.accent));
        target_->FillRectangle(
            D2D1::RectF(x, underlineY, x + textWidth, underlineY + theme_.underlinePx), brush_);
        x += textWidth;
    }
    if (!input_.pending.empty()) {
        const float pendingWidth = TextWidth(input_.pending, inputFormat_);
        brush_->SetColor(ToD2D(theme_.secondary));
        target_->DrawText(input_.pending.c_str(), static_cast<UINT32>(input_.pending.size()),
                          inputFormat_, D2D1::RectF(x, top, x + big, top + rowHeight), brush_);
        const float dotY = underlineY + line / 2;
        target_->DrawLine(D2D1::Point2F(x, dotY), D2D1::Point2F(x + pendingWidth, dotY), brush_,
                          line, dotStroke_);
        x += pendingWidth;
    }
    // キャレットは点滅させない (打鍵ごとの再描画以外にタイマーで描き直さない)
    brush_->SetColor(ToD2D(theme_.accent));
    const float caretX = std::round(x) + 1.0f;
    target_->FillRectangle(D2D1::RectF(caretX, lineTop + lineHeight * 0.12f,
                                       caretX + theme_.caretWidthPx,
                                       lineTop + lineHeight * 0.88f),
                           brush_);
    target_->PopAxisAlignedClip();
}

void SuggestPanel::DrawChip(size_t index, float left, float top)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float width = chipWidth_[index];
    const float height = static_cast<float>(m.chipHeight);
    const float radius = height / 2;
    const float line = static_cast<float>(m.line);
    const float half = line / 2;
    const bool selected = index == selection_;
    const D2D1_RECT_F rect = D2D1::RectF(left, top, left + width, top + height);

    brush_->SetColor(ToD2D(selected ? theme_.accent : theme_.chip));
    target_->FillRoundedRectangle(D2D1::RoundedRect(rect, radius, radius), brush_);
    if (items_[index].nextWord) {
        brush_->SetColor(ToD2D(theme_.nextWord));
        target_->DrawRoundedRectangle(
            D2D1::RoundedRect(D2D1::RectF(rect.left + half, rect.top + half, rect.right - half,
                                          rect.bottom - half),
                              radius - half, radius - half),
            brush_, line);
    }

    const PanelColor& badgeColor = selected ? theme_.selectedText : theme_.secondary;
    const float badgeRadius = m.badgeSize / 2;
    const float centerX = left + m.chipPadding + badgeRadius;
    const float centerY = top + height / 2;
    brush_->SetColor(ToD2D(badgeColor));
    target_->DrawEllipse(
        D2D1::Ellipse(D2D1::Point2F(centerX, centerY), badgeRadius - half, badgeRadius - half),
        brush_, line);
    const std::wstring number = std::to_wstring(index + 1);
    target_->DrawText(number.c_str(), static_cast<UINT32>(number.size()), badgeFormat_,
                      D2D1::RectF(centerX - badgeRadius, centerY - badgeRadius,
                                  centerX + badgeRadius, centerY + badgeRadius),
                      brush_);

    const std::wstring& text = itemTexts_[index];
    const float textX = left + m.chipPadding + m.badgeSize + m.badgeGap;
    brush_->SetColor(ToD2D(selected ? theme_.selectedText : theme_.text));
    target_->DrawText(text.c_str(), static_cast<UINT32>(text.size()), textFormat_,
                      D2D1::RectF(textX, top, rect.right, rect.bottom), brush_);
}

bool SuggestPanel::WindowRect(RECT* rect) const
{
    if (hwnd_ == nullptr) {
        return false;
    }
    *rect = {bodyX_, bodyY_, bodyX_ + bodyWidth_, bodyY_ + bodyHeight_};
    return true;
}

void SuggestPanel::SetSelection(size_t selection)
{
    selection_ = selection;
    if (hwnd_ != nullptr) {
        Render(L"SetSelection", nullptr);
    }
}

bool SuggestPanel::StartTimer(UINT intervalMs, std::function<void()> callback)
{
    if (hwnd_ == nullptr) {
        return false;
    }
    timerCallback_ = std::move(callback);
    if (SetTimer(hwnd_, kTimerId, intervalMs, nullptr) == 0) {
        timerCallback_ = nullptr;
        return false;
    }
    return true;
}

void SuggestPanel::StopTimer()
{
    if (hwnd_ != nullptr) {
        KillTimer(hwnd_, kTimerId);
    }
    timerCallback_ = nullptr;
}

void SuggestPanel::Hide()
{
    if (hwnd_ != nullptr) {
        if (logger_) {
            wchar_t text[64] = {};
            swprintf_s(text, L"Hide: ウィンドウを破棄 hwnd=%p", static_cast<void*>(hwnd_));
            logger_(text);
        }
        // ウィンドウと一緒にタイマーも破棄される
        destroying_ = true;
        DestroyWindow(hwnd_);
        destroying_ = false;
        hwnd_ = nullptr;
    }
    timerCallback_ = nullptr;
    items_.clear();
    itemTexts_.clear();
    chipX_.clear();
    chipWidth_.clear();
    selection_ = 0;
    hasInput_ = false;
    input_ = InputLine{};
    capsuleText_.clear();
}

LRESULT CALLBACK SuggestPanel::WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    if (msg == WM_NCCREATE) {
        // CreateWindowExW の最終引数で渡した this を関連付ける
        auto* cs = reinterpret_cast<CREATESTRUCTW*>(lparam);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(cs->lpCreateParams));
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    auto* self = reinterpret_cast<SuggestPanel*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    switch (msg) {
    case WM_DESTROY:
        if (self != nullptr && self->logger_) {
            wchar_t text[96] = {};
            swprintf_s(text, L"WM_DESTROY を受信 hwnd=%p (%s)", static_cast<void*>(hwnd),
                       self->destroying_ ? L"Hide による破棄" : L"外部からの破棄");
            self->logger_(text);
        }
        break;
    case WM_MOUSEACTIVATE:
        // クリックされてもフォーカスを奪わない
        return MA_NOACTIVATE;
    case WM_TIMER:
        if (self != nullptr && wparam == kTimerId && self->timerCallback_) {
            // callback の中で StopTimer・Hide されても呼び出し中の関数が消えないよう、写しを呼ぶ
            const std::function<void()> callback = self->timerCallback_;
            callback();
        }
        return 0;
    default:
        break;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}

void SuggestPanel::LogShow(const wchar_t* kind, const RECT* anchor, bool created,
                           DWORD error) const
{
    if (!logger_) {
        return;
    }
    const std::wstring line =
        std::wstring(kind) + L": anchor=" + (anchor != nullptr ? DebugRectText(*anchor) : L"なし") +
        L" pos=(" + std::to_wstring(bodyX_) + L"," + std::to_wstring(bodyY_) + L") size=(" +
        std::to_wstring(bodyWidth_) + L"," + std::to_wstring(bodyHeight_) + L") 候補=" +
        std::to_wstring(itemTexts_.size()) + L" 入力行=" + (hasInput_ ? L"1" : L"0");
    if (hwnd_ == nullptr) {
        logger_(line + L" CreateWindowExW 失敗 error=" + std::to_wstring(error));
        return;
    }

    RECT windowRect = {};
    const bool gotRect = GetWindowRect(hwnd_, &windowRect) != FALSE;
    const bool offMonitor =
        gotRect && MonitorFromRect(&windowRect, MONITOR_DEFAULTTONULL) == nullptr;
    const DWORD windowThread = GetWindowThreadProcessId(hwnd_, nullptr);

    HWND foreground = GetForegroundWindow();
    DWORD foregroundProcess = 0;
    bool foregroundTopmost = false;
    if (foreground != nullptr) {
        GetWindowThreadProcessId(foreground, &foregroundProcess);
        foregroundTopmost = (GetWindowLongPtrW(foreground, GWL_EXSTYLE) & WS_EX_TOPMOST) != 0;
    }

    const std::wstring rectText = gotRect ? DebugRectText(windowRect) : L"(取得失敗)";
    wchar_t text[512] = {};
    swprintf_s(text,
               L" %s hwnd=%p visible=%d rect=%s monitor=%s wndTid=%lu curTid=%lu fg=%p "
               L"fgTopmost=%d fgSameProcess=%d",
               created ? L"新規作成" : L"既存", static_cast<void*>(hwnd_),
               IsWindowVisible(hwnd_) ? 1 : 0, rectText.c_str(),
               !gotRect ? L"不明" : (offMonitor ? L"なし" : L"あり"), windowThread,
               GetCurrentThreadId(), static_cast<void*>(foreground), foregroundTopmost ? 1 : 0,
               (foreground != nullptr && foregroundProcess == GetCurrentProcessId()) ? 1 : 0);
    std::wstring message = line + text;
    if (error != ERROR_SUCCESS) {
        message += L" UpdateLayeredWindow 失敗 error=" + std::to_wstring(error);
    }
    logger_(message);
}
