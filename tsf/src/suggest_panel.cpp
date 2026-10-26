#include "suggest_panel.h"

#include <d2d1.h>
#include <dwmapi.h>
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
constexpr float kSoftAlpha = 0.16f;        // selection soft でアクセント色を敷く不透明度
constexpr float kTagAlpha = 0.16f;         // 種類のタグの背景の不透明度
constexpr float kTextShadowAlpha = 0.35f;  // text_shadow の影の不透明度
constexpr float kDimTextAlpha = 0.5f;      // 採用範囲の外の文字の不透明度
constexpr float kSegmentAlpha = 0.16f;     // 変換中の入力行で現在文節にアクセント色を敷く不透明度
constexpr float kPageRowRatio = 1.6f;      // 位置の表示 (12/24) の行の高さ (label_text_size に対する比)

// ぼかしの方式の描画先の種類。hardware (DEFAULT) はホストのプロセスに D3D のデバイスを作るため、
// まず software で試し、alpha が失われるときに切り替えて比べる
constexpr D2D1_RENDER_TARGET_TYPE kBlurTargetType = D2D1_RENDER_TARGET_TYPE_SOFTWARE;

const wchar_t* BlurTargetTypeName()
{
    return kBlurTargetType == D2D1_RENDER_TARGET_TYPE_SOFTWARE ? L"software" : L"hardware";
}

// SetWindowCompositionAttribute (user32 の非公開 API) の引数
enum : DWORD {
    kWcaAccentPolicy = 19,
};
enum : int {
    kAccentEnableAcrylicBlurBehind = 4,
};
struct AccentPolicy {
    int accentState;
    DWORD accentFlags;
    DWORD gradientColor;  // AABBGGRR
    DWORD animationId;
};
struct WindowCompositionAttribData {
    DWORD attrib;
    void* data;
    SIZE_T size;
};
using SetWindowCompositionAttributeFn = BOOL(WINAPI*)(HWND, WindowCompositionAttribData*);

// 古い SDK の dwmapi.h には無いため自前で持つ
constexpr DWORD kDwmwaWindowCornerPreference = 33;
constexpr DWORD kDwmwcpRound = 2;

DWORD ToAbgr(const PanelColor& color)
{
    const auto channel = [](float value) {
        return static_cast<DWORD>(std::lround((std::clamp)(value, 0.0f, 1.0f) * 255.0f));
    };
    return (channel(color.a) << 24) | (channel(color.b) << 16) | (channel(color.g) << 8) |
           channel(color.r);
}

// ウィンドウにアクリルのぼかしを当てる。API が無い・失敗したら false
bool ApplyAcrylic(HWND hwnd, const PanelColor& tint)
{
    HMODULE user32 = GetModuleHandleW(L"user32.dll");
    if (user32 == nullptr) {
        return false;
    }
    const auto setAttribute = reinterpret_cast<SetWindowCompositionAttributeFn>(
        reinterpret_cast<void*>(GetProcAddress(user32, "SetWindowCompositionAttribute")));
    if (setAttribute == nullptr) {
        return false;
    }
    AccentPolicy policy = {kAccentEnableAcrylicBlurBehind, 0, ToAbgr(tint), 0};
    WindowCompositionAttribData data = {kWcaAccentPolicy, &policy, sizeof(policy)};
    return setAttribute(hwnd, &data) != FALSE;
}

// Windows の透明効果がオンか (EnableTransparency。値が無ければオン)。
// オフだとアクリルがぼかさず単色になる
bool TransparencyEnabled()
{
    DWORD value = 1;
    DWORD size = sizeof(value);
    if (RegGetValueW(HKEY_CURRENT_USER,
                     L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
                     L"EnableTransparency", RRF_RT_REG_DWORD, nullptr, &value,
                     &size) != ERROR_SUCCESS) {
        value = 1;
    }
    return value != 0;
}

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
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
    float chipRadius;     // 候補の背景の角丸の半径 (chip_shape で決まる)
    float badgeSize;
    float badgeGap;
    float badgeSlot;      // 番号の形と、番号と候補文字列の間を合わせた幅 (番号なしなら 0)
    float labelGap;
    float inputTextSize;  // 入力行の文字の高さ
    int inputRowHeight;
    int rowGap;
    int line;
    float panelRadius;
    float shadowSpread;
    float shadowOffset;
    float tagHeight;
    float tagPadding;     // 種類のタグの左右の余白
    float tagGap;         // 候補文字列と種類のタグの間
    int margin;           // 影のためにパネルの外側に取る余白
    int ghostMargin;      // ghost のにじみのためにパネルの外側に取る余白
};

int Scaled(float ratio, float f)
{
    return (std::max)(1, static_cast<int>(std::lround(ratio * f)));
}

Metrics ComputeMetrics(const PanelTheme& theme, int fontSize)
{
    const bool ghost = theme.style == PanelStyle::Ghost;
    Metrics m = {};
    m.f = static_cast<float>(fontSize);
    m.padding = Scaled(theme.panelPadding, m.f);
    m.chipHeight = Scaled(theme.chipHeight, m.f);
    m.chipPadding = theme.chipPadding * m.f;
    m.chipGap = theme.chipGap * m.f;
    const float half = static_cast<float>(m.chipHeight) / 2;
    switch (theme.chipShape) {
    case ChipShape::Pill:
        m.chipRadius = half;
        break;
    case ChipShape::Rounded:
        m.chipRadius = (std::min)(theme.chipRadius * m.f, half);
        break;
    default:
        m.chipRadius = 0.0f;
        break;
    }
    m.badgeSize = theme.badgeSize * m.f;
    m.badgeGap = theme.badgeGap * m.f;
    m.badgeSlot = theme.badgeShape == BadgeShape::None ? 0.0f : m.badgeSize + m.badgeGap;
    m.labelGap = theme.labelGap * m.f;
    m.inputTextSize = (ghost ? theme.ghostTextSize : theme.inputTextSize) * m.f;
    // ghost では入力行の文字を大きくするので、行の高さも文字の高さの比で伸ばす
    m.inputRowHeight = Scaled(
        ghost ? theme.inputRowHeight * theme.ghostTextSize / theme.inputTextSize
              : theme.inputRowHeight,
        m.f);
    m.rowGap = Scaled(theme.rowGap, m.f);
    m.line = (std::max)(1, static_cast<int>(std::lround(theme.lineWidthPx)));
    m.panelRadius = theme.panelRadius * m.f;
    m.shadowSpread = theme.shadowSize * m.f;
    m.shadowOffset = theme.shadowOffset * m.f;
    m.tagHeight = theme.tagTextSize * m.f * 1.6f;
    m.tagPadding = theme.tagTextSize * m.f * 0.55f;
    m.tagGap = m.badgeGap;
    m.margin = static_cast<int>(std::ceil(m.shadowSpread + m.shadowOffset));
    m.ghostMargin = Scaled(1.5f, m.f);
    return m;
}

const wchar_t* KindTagLabel(SuggestPanel::ItemKind kind)
{
    switch (kind) {
    case SuggestPanel::ItemKind::Ascii:
        return L"英字";
    case SuggestPanel::ItemKind::Prediction:
        return L"予測";
    case SuggestPanel::ItemKind::Partial:
        return L"文節";
    case SuggestPanel::ItemKind::NextWord:
        return L"次の語";
    case SuggestPanel::ItemKind::Typo:
        return L"補正";
    default:
        return L"変換";
    }
}

const PanelColor& KindTagColor(const PanelTheme& theme, SuggestPanel::ItemKind kind)
{
    switch (kind) {
    case SuggestPanel::ItemKind::Ascii:
        return theme.tagAscii;
    case SuggestPanel::ItemKind::Prediction:
        return theme.tagPrediction;
    case SuggestPanel::ItemKind::Partial:
        return theme.tagPartial;
    case SuggestPanel::ItemKind::NextWord:
        return theme.tagNextWord;
    case SuggestPanel::ItemKind::Typo:
        return theme.tagTypo;
    default:
        return theme.tagConversion;
    }
}

// 縦積みで n 行を出すときのパネルの高さ。候補が無ければ入力行だけにする
int ColumnHeight(const Metrics& m, bool hasInput, size_t n)
{
    if (n == 0) {
        return m.padding * 2 + (hasInput ? m.inputRowHeight : 0);
    }
    const int header = hasInput ? m.inputRowHeight + m.rowGap * 2 + m.line : 0;
    const int count = static_cast<int>(n);
    return m.padding * 2 + header + m.chipHeight * count +
           static_cast<int>(std::lround(m.chipGap * static_cast<float>(count - 1)));
}

// 変換の候補一覧の位置の表示 (12/24) の行の高さ
int PageRowHeight(const PanelTheme& theme, const Metrics& m)
{
    return Scaled(theme.labelTextSize * kPageRowRatio, m.f);
}

// 画面中央に出す候補バーのパネルの幅
int CenterWidth(const PanelTheme& theme, const Metrics& m, int workWidth)
{
    return static_cast<int>(
        (std::min)(theme.centerWidth * m.f, static_cast<float>(workWidth) * theme.centerWidthRatio));
}

// 画面中央に出す候補バーのパネルの上端。SuggestPanel::Show の Center と同じ規則で求める
// (変換の候補一覧の上端をそろえ、候補選択に入っても入力行を動かさないため)
int CenterBarTop(const PanelTheme& theme, const Metrics& m, const RECT& work, bool hasInput)
{
    const int workHeight = static_cast<int>(work.bottom - work.top);
    if (theme.layout == PanelLayout::Column) {
        size_t fullRows = 0;
        while (fullRows < SuggestPanel::kMaxItems &&
               ColumnHeight(m, hasInput, fullRows + 1) <= workHeight) {
            ++fullRows;
        }
        return static_cast<int>(work.top) +
               (std::max)(0, (workHeight - ColumnHeight(m, hasInput, fullRows)) / 2);
    }
    const int header = hasInput ? m.inputRowHeight + m.rowGap * 2 + m.line : 0;
    const int height = m.padding * 2 + m.chipHeight + header;
    return static_cast<int>(work.top) + (workHeight - height) / 2;
}

// パネル (影を除く) の角丸の半径
float BodyRadius(const Metrics& m, float height)
{
    return (std::min)(m.panelRadius, height / 2);
}

// 尻尾を付けるパネルの左端を Placement::x から左へずらす量。TailTipX は尻尾を角丸の外
// (左端 + 半径 + 半幅) に止めるため、ずらさないと打ち始めのキャレットを指せない
int PointerLeftShift(const Metrics& m, int height, int pointerHeight)
{
    if (pointerHeight <= 0) {
        return 0;
    }
    return static_cast<int>(std::ceil(BodyRadius(m, static_cast<float>(height)))) +
           pointerHeight;
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
// 作業領域に収めた位置を返す。下にはみ出すなら anchor の上側 (aboveGap だけ離す)、それでも
// 収まらなければ (anchor がタスクバーに重なる位置にあるときなど) 作業領域の下端に寄せる
POINT FitToWorkArea(const RECT& anchor, int x, int y, int width, int height,
                    int aboveGap = kAnchorGap)
{
    const POINT origin = {anchor.left, anchor.top};
    const RECT work = WorkAreaOf(MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST));
    if (x + width > work.right) {
        x = static_cast<int>(work.right) - width;
    }
    x = (std::max)(x, static_cast<int>(work.left));
    if (y + height > work.bottom) {
        y = static_cast<int>(anchor.top) - height - aboveGap;
        if (y < work.top || y + height > work.bottom) {
            y = static_cast<int>(work.bottom) - height;
        }
    }
    y = (std::max)(y, static_cast<int>(work.top));
    return {x, y};
}

// 角丸矩形 rect に、上辺 (top) か下辺に二等辺三角形の尻尾を足した輪郭。尻尾の先端は x = tipX で
// 辺から tailHeight だけ外側、底辺は高さの 2 倍。作れなければ nullptr
ID2D1PathGeometry* CreateBalloonGeometry(ID2D1Factory* factory, const D2D1_RECT_F& rect,
                                         float radius, bool top, float tipX, float tailHeight)
{
    ID2D1PathGeometry* geometry = nullptr;
    if (FAILED(factory->CreatePathGeometry(&geometry))) {
        return nullptr;
    }
    ID2D1GeometrySink* sink = nullptr;
    if (FAILED(geometry->Open(&sink))) {
        geometry->Release();
        return nullptr;
    }
    const float r = (std::clamp)(radius, 0.0f,
                                 (std::min)(rect.right - rect.left, rect.bottom - rect.top) / 2);
    const float h = (std::max)(0.0f, tailHeight);
    const auto corner = [&](float x, float y) {
        if (r > 0.0f) {
            sink->AddArc(D2D1::ArcSegment(D2D1::Point2F(x, y), D2D1::SizeF(r, r), 0.0f,
                                          D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_ARC_SIZE_SMALL));
        } else {
            sink->AddLine(D2D1::Point2F(x, y));
        }
    };
    sink->BeginFigure(D2D1::Point2F(rect.left + r, rect.top), D2D1_FIGURE_BEGIN_FILLED);
    if (top) {
        sink->AddLine(D2D1::Point2F(tipX - h, rect.top));
        sink->AddLine(D2D1::Point2F(tipX, rect.top - h));
        sink->AddLine(D2D1::Point2F(tipX + h, rect.top));
    }
    sink->AddLine(D2D1::Point2F(rect.right - r, rect.top));
    corner(rect.right, rect.top + r);
    sink->AddLine(D2D1::Point2F(rect.right, rect.bottom - r));
    corner(rect.right - r, rect.bottom);
    if (!top) {
        sink->AddLine(D2D1::Point2F(tipX + h, rect.bottom));
        sink->AddLine(D2D1::Point2F(tipX, rect.bottom + h));
        sink->AddLine(D2D1::Point2F(tipX - h, rect.bottom));
    }
    sink->AddLine(D2D1::Point2F(rect.left + r, rect.bottom));
    corner(rect.left, rect.bottom - r);
    sink->AddLine(D2D1::Point2F(rect.left, rect.top + r));
    corner(rect.left + r, rect.top);
    sink->EndFigure(D2D1_FIGURE_END_CLOSED);
    const HRESULT hr = sink->Close();
    sink->Release();
    if (FAILED(hr)) {
        geometry->Release();
        return nullptr;
    }
    return geometry;
}

// 輪郭を outset だけ外へ広げた (負なら内へ寄せた) ときの尻尾の高さ。尻尾の斜辺は 45 度なので、
// 斜辺を outset だけ平行に動かすと、先端は辺から見て outset * (√2 - 1) だけ余分に伸びる
float OffsetTailHeight(float tailHeight, float outset)
{
    return tailHeight + outset * 0.41421356f;
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
                                DWRITE_TEXT_ALIGNMENT alignment,
                                DWRITE_FONT_WEIGHT weight = DWRITE_FONT_WEIGHT_NORMAL)
{
    IDWriteTextFormat* format = nullptr;
    // テーマで文字の高さの比率を 0 にできるが、DirectWrite は 0 の大きさを受け付けない
    if (FAILED(factory->CreateTextFormat(face.c_str(), nullptr, weight, DWRITE_FONT_STYLE_NORMAL,
                                         DWRITE_FONT_STRETCH_NORMAL, (std::max)(size, 1.0f),
                                         L"ja-jp", &format))) {
        return nullptr;
    }
    format->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
    format->SetTextAlignment(alignment);
    format->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    return format;
}

// 描く矩形に収まらない文字列の末尾を「…」で切る
void SetEllipsisTrimming(IDWriteFactory* factory, IDWriteTextFormat* format)
{
    IDWriteInlineObject* sign = nullptr;
    if (FAILED(factory->CreateEllipsisTrimmingSign(format, &sign))) {
        return;
    }
    const DWRITE_TRIMMING trimming = {DWRITE_TRIMMING_GRANULARITY_CHARACTER, 0, 0};
    format->SetTrimming(&trimming, sign);
    sign->Release();
}

} // namespace

// ---- SuggestPanel ----

SuggestPanel::SuggestPanel()
    : hwnd_(nullptr), owner_(nullptr), destroying_(false), d2dFactory_(nullptr),
      dwriteFactory_(nullptr),
      target_(nullptr), hwndTarget_(nullptr), drawTarget_(nullptr), brush_(nullptr),
      dotStroke_(nullptr), textFormat_(nullptr), boldFormat_(nullptr), inputFormat_(nullptr),
      badgeFormat_(nullptr), labelFormat_(nullptr), tagFormat_(nullptr), fontSize_(18),
      theme_(PanelTheme::Defaults(false)), blur_(false), blurFailed_(false), blurActive_(false),
      offscreen_(false), snapshotWidth_(0), snapshotHeight_(0), conversion_(false), inputTop_(0.0f), separator_(false),
      separatorTop_(0.0f), pageTop_(0.0f), pageInline_(false), pageLeft_(0.0f), pageStart_(0),
      selection_(0), page_(0), pageLabelWidth_(0.0f), inputWidth_(0.0f), placementCaret_{},
      work_{}, pointerHeight_(0), columnSide_(ColumnSide::None), hasInput_(false),
      bodyX_(0), bodyY_(0), bodyWidth_(0), bodyHeight_(0), margin_(0), tail_(TailSide::None),
      tailHeight_(0), markerX_(0)
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
    SafeRelease(&boldFormat_);
    SafeRelease(&inputFormat_);
    SafeRelease(&badgeFormat_);
    SafeRelease(&labelFormat_);
    SafeRelease(&tagFormat_);
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
    ReleaseHwndTarget();
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
    if (target_ != nullptr) {
        SafeRelease(&brush_);
        SafeRelease(&target_);
    }
}

bool SuggestPanel::EnsureHwndTarget()
{
    if (hwndTarget_ != nullptr) {
        return true;
    }
    if (hwnd_ == nullptr || !EnsureFactories()) {
        return false;
    }
    ReleaseTarget();
    // DPI は layered window の方式と同じく 96 に固定する
    const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
        kBlurTargetType,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), 96.0f,
        96.0f);
    const D2D1_HWND_RENDER_TARGET_PROPERTIES hwndProperties = D2D1::HwndRenderTargetProperties(
        hwnd_, D2D1::SizeU(static_cast<UINT32>(bodyWidth_), static_cast<UINT32>(bodyHeight_)));
    if (FAILED(d2dFactory_->CreateHwndRenderTarget(&properties, &hwndProperties, &hwndTarget_))) {
        hwndTarget_ = nullptr;
        return false;
    }
    if (FAILED(hwndTarget_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0, 1), &brush_))) {
        ReleaseHwndTarget();
        return false;
    }
    // 下地がぼかしの半透明な色のため、layered window と同じくグレースケールにする
    hwndTarget_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    return true;
}

void SuggestPanel::ReleaseHwndTarget()
{
    if (hwndTarget_ != nullptr) {
        SafeRelease(&brush_);
        SafeRelease(&hwndTarget_);
    }
}

void SuggestPanel::SetBlur(bool enabled)
{
    if (enabled == blur_) {
        return;
    }
    blur_ = enabled;
    if (enabled) {
        blurFailed_ = false;
    }
    // 方式によってウィンドウの作り方が違うため、次の表示で作り直す
    if (hwnd_ != nullptr) {
        Hide();
    }
}

void SuggestPanel::SetFont(const std::wstring& face, int height)
{
    if (!EnsureFactories()) {
        return;
    }
    const std::wstring actual = FontExists(dwriteFactory_, face) ? face : kFallbackFont;
    const float f = static_cast<float>(height);
    const float inputSize =
        theme_.style == PanelStyle::Ghost ? theme_.ghostTextSize : theme_.inputTextSize;
    IDWriteTextFormat* text =
        CreateFormat(dwriteFactory_, actual, f, DWRITE_TEXT_ALIGNMENT_LEADING);
    IDWriteTextFormat* bold = CreateFormat(dwriteFactory_, actual, f,
                                           DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_FONT_WEIGHT_BOLD);
    IDWriteTextFormat* input =
        CreateFormat(dwriteFactory_, actual, inputSize * f, DWRITE_TEXT_ALIGNMENT_LEADING);
    IDWriteTextFormat* badge = CreateFormat(dwriteFactory_, actual, theme_.badgeTextSize * f,
                                            DWRITE_TEXT_ALIGNMENT_CENTER);
    IDWriteTextFormat* label = CreateFormat(dwriteFactory_, actual, theme_.labelTextSize * f,
                                            DWRITE_TEXT_ALIGNMENT_LEADING);
    IDWriteTextFormat* tag = CreateFormat(dwriteFactory_, actual, theme_.tagTextSize * f,
                                          DWRITE_TEXT_ALIGNMENT_CENTER);
    if (text == nullptr || bold == nullptr || input == nullptr || badge == nullptr ||
        label == nullptr || tag == nullptr) {
        SafeRelease(&text);
        SafeRelease(&bold);
        SafeRelease(&input);
        SafeRelease(&badge);
        SafeRelease(&label);
        SafeRelease(&tag);
        return;
    }
    SetEllipsisTrimming(dwriteFactory_, text);
    SetEllipsisTrimming(dwriteFactory_, bold);
    SafeRelease(&textFormat_);
    SafeRelease(&boldFormat_);
    SafeRelease(&inputFormat_);
    SafeRelease(&badgeFormat_);
    SafeRelease(&labelFormat_);
    SafeRelease(&tagFormat_);
    textFormat_ = text;
    boldFormat_ = bold;
    inputFormat_ = input;
    badgeFormat_ = badge;
    labelFormat_ = label;
    tagFormat_ = tag;
    face_ = face;
    fontSize_ = height;
}

void SuggestPanel::SetTheme(const PanelTheme& theme)
{
    const bool sameShape = theme_.SameShape(theme);
    // 文字の大きさの比率も theme が持つので、書式を作り直す
    theme_ = theme;
    SetFont(face_, fontSize_);
    if (hwnd_ != nullptr && !sameShape) {
        Hide();
    }
    if (hwnd_ != nullptr) {
        if (blurActive_) {
            ApplyAcrylic(hwnd_, theme_.blurTint);
        }
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
    UseOwner(placement.owner);
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const bool column = theme_.layout == PanelLayout::Column;
    const bool ghost = theme_.style == PanelStyle::Ghost;
    const int pointerHeight = PointerHeight(placement);

    std::vector<std::wstring> texts;
    std::vector<float> widths;  // 切り詰めずに出したときの候補 1 件の幅
    for (const Item& item : items) {
        std::wstring text = item.text;
        float textWidth = TextWidth(text, textFormat_);
        if (theme_.selection == SelectionStyle::Text) {
            // 選択すると太字になるので、選択によって幅が変わらないよう太字の幅も見込む
            textWidth = (std::max)(textWidth, TextWidth(text, boldFormat_));
        }
        float width = m.chipPadding * 2 + m.badgeSlot + textWidth;
        if (column && theme_.kindTag) {
            width += m.tagGap + TextWidth(KindTagLabel(item.kind), tagFormat_) +
                     m.tagPadding * 2;
        }
        widths.push_back(width);
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
        if (ghost && !texts.empty()) {
            // ゴーストは選択によって変わるので、いちばん長いものが入る幅を取る
            float widest = 0.0f;
            for (const std::wstring& text : texts) {
                widest = (std::max)(widest, TextWidth(L"→ " + text, inputFormat_));
            }
            inputWidth += m.labelGap + widest;
        }
    }
    const bool hasInput = input != nullptr;
    if (texts.empty() && !hasInput) {
        Hide();
        return 0;
    }

    HMONITOR monitor = nullptr;
    if (placement.position == Position::Center) {
        if (placement.caret != nullptr) {
            monitor = MonitorFromRect(placement.caret, MONITOR_DEFAULTTONEAREST);
        } else {
            HWND foreground = GetForegroundWindow();
            monitor = foreground != nullptr
                          ? MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
                          : MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY);
        }
    } else {
        const POINT origin = {placement.x, placement.caret->bottom};
        monitor = MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST);
    }
    const RECT work = WorkAreaOf(monitor);
    const int workWidth = static_cast<int>(work.right - work.left);
    const int workHeight = static_cast<int>(work.bottom - work.top);

    // ページの区切りは先頭から順に決め、選択によって変えない (同じ候補は常に同じページ・同じ番号)
    const size_t total = widths.size();
    const std::wstring totalText = std::to_wstring(total);
    // 位置の表示の文字は選択で変わるので、いちばん広いものの幅を取る
    const float labelWidth = (std::max)(TextWidth(L"-/" + totalText, labelFormat_),
                                        TextWidth(totalText + L"/" + totalText, labelFormat_));
    std::vector<size_t> pageStarts;
    float pageLabelWidth = 0.0f;
    ColumnSide side = ColumnSide::None;
    size_t sideRows = 0;
    if (column && placement.position == Position::Caret) {
        sideRows = FitColumn(placement, work, hasInput, total, &side);
    }
    if (total > 0 && column) {
        const int pageRow = static_cast<int>(std::lround(m.chipGap)) + PageRowHeight(theme_, m);
        // n 行を出すときのパネルの高さ。pages なら位置の表示の行を足す
        const auto heightOf = [&](size_t n, bool pages) {
            return ColumnHeight(m, hasInput, n) + (n > 0 && pages ? pageRow : 0);
        };
        const size_t limit = (std::min)(kMaxItems, total);
        size_t pageSize = 1;
        if (sideRows > 0) {
            pageSize = (std::min)(sideRows, limit);
        } else {
            while (pageSize < limit &&
                   heightOf(pageSize + 1, total > pageSize + 1) <= workHeight) {
                ++pageSize;
            }
        }
        for (size_t begin = 0; begin < total; begin += pageSize) {
            pageStarts.push_back(begin);
        }
        if (pageStarts.size() > 1) {
            pageLabelWidth = labelWidth;
        }
    } else if (total > 0) {
        const int height = m.padding * 2 + m.chipHeight +
                           (hasInput ? m.inputRowHeight + m.rowGap * 2 + m.line : 0);
        float available = 0.0f;
        if (placement.position == Position::Center) {
            available = static_cast<float>(CenterWidth(theme_, m, workWidth) - m.padding * 2);
        } else {
            const int x = (std::max)(placement.x - PointerLeftShift(m, height, pointerHeight),
                                     static_cast<int>(work.left));
            available = static_cast<float>(static_cast<int>(work.right) - x - m.padding * 2);
        }
        // begin から reserve (位置の表示の分) を残して幅に収まる件数。1 件も入らなければその 1 件
        const auto fit = [&](size_t begin, float reserve) {
            size_t count = 0;
            float chipsWidth = 0.0f;
            while (begin + count < total && count < kMaxItems) {
                const float next =
                    chipsWidth + (count > 0 ? m.chipGap : 0.0f) + widths[begin + count];
                if (next + reserve > available) {
                    break;
                }
                chipsWidth = next;
                ++count;
            }
            return (std::max)(count, static_cast<size_t>(1));
        };
        if (total <= kMaxItems && fit(0, 0.0f) == total) {
            pageStarts.push_back(0);
        } else {
            pageLabelWidth = labelWidth;
            for (size_t begin = 0; begin < total; begin += fit(begin, m.chipGap + labelWidth)) {
                pageStarts.push_back(begin);
            }
        }
    }

    conversion_ = false;
    allItems_ = items;
    allTexts_ = std::move(texts);
    allWidths_ = std::move(widths);
    pageStarts_ = std::move(pageStarts);
    pageLabelWidth_ = pageLabelWidth;
    inputWidth_ = inputWidth;
    placement_ = placement;
    if (placement.caret != nullptr) {
        placementCaret_ = *placement.caret;
        placement_.caret = &placementCaret_;
    }
    work_ = work;
    pointerHeight_ = pointerHeight;
    columnSide_ = side;
    hasInput_ = hasInput;
    input_ = hasInput ? *input : InputLine{};
    size_t page = 0;
    if (selection < total) {
        page = static_cast<size_t>(
            std::upper_bound(pageStarts_.begin(), pageStarts_.end(), selection) -
            pageStarts_.begin() - 1);
    }
    return LayoutPage(page, selection, L"Show");
}

size_t SuggestPanel::FitColumn(const Placement& placement, const RECT& work, bool hasInput,
                               size_t total, ColumnSide* side) const
{
    *side = ColumnSide::None;
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const int pointerHeight = PointerHeight(placement);
    const int belowGap = pointerHeight > 0 ? pointerHeight : kAnchorGap + TriangleHeight(placement);
    const int aboveGap = pointerHeight > 0 ? pointerHeight : kAnchorGap;
    const int pageRow = static_cast<int>(std::lround(m.chipGap)) + PageRowHeight(theme_, m);
    // n 行を出すときのパネルの高さ。pages なら位置の表示の行を足す
    const auto heightOf = [&](size_t n, bool pages) {
        return ColumnHeight(m, hasInput, n) + (n > 0 && pages ? pageRow : 0);
    };
    // 今の候補の数ではなく出しうる最大の高さ (位置の表示の行を含む) で判定し、打鍵ごとに
    // 候補の数が変わっても出す側を変えない
    const int workHeight = static_cast<int>(work.bottom - work.top);
    size_t fullRows = 1;
    while (fullRows < kMaxItems && heightOf(fullRows + 1, true) <= workHeight) {
        ++fullRows;
    }
    const int fullHeight = heightOf(fullRows, true);

    const RECT& caret = *placement.caret;
    const int below = static_cast<int>(work.bottom) - (static_cast<int>(caret.bottom) + belowGap);
    const int above = static_cast<int>(caret.top) - aboveGap - static_cast<int>(work.top);
    int space = below;
    if (fullHeight <= below) {
        *side = ColumnSide::Below;
    } else if (fullHeight <= above || above > below) {
        *side = ColumnSide::Above;
        space = above;
    } else {
        *side = ColumnSide::Below;
    }

    const size_t limit = (std::max)(static_cast<size_t>(1), (std::min)(kMaxItems, total));
    if (heightOf(1, total > 1) > space) {
        *side = ColumnSide::None;
        return 0;
    }
    size_t rows = 1;
    while (rows < limit && heightOf(rows + 1, total > rows + 1) <= space) {
        ++rows;
    }
    return rows;
}

void SuggestPanel::MirrorColumn(int height)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float panelHeight = static_cast<float>(height);
    // 上端 top・高さ size の行を、パネルの下端から同じ距離の位置へ移す
    const auto mirror = [&](float top, float size) { return panelHeight - top - size; };
    inputTop_ = mirror(inputTop_, static_cast<float>(m.inputRowHeight));
    separatorTop_ = mirror(separatorTop_, static_cast<float>(m.line));
    for (ItemBox& box : itemBoxes_) {
        box.top = mirror(box.top, static_cast<float>(m.chipHeight));
    }
    pageTop_ = mirror(pageTop_, static_cast<float>(PageRowHeight(theme_, m)));
}

std::wstring SuggestPanel::PageText(size_t selection) const
{
    if (pageStarts_.size() <= 1) {
        return {};
    }
    const size_t total = allItems_.size();
    return (selection < total ? std::to_wstring(selection + 1) : std::wstring(L"-")) + L"/" +
           std::to_wstring(total);
}

size_t SuggestPanel::LayoutPage(size_t page, size_t selection, const wchar_t* kind)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const bool column = theme_.layout == PanelLayout::Column;
    const bool ghost = theme_.style == PanelStyle::Ghost;
    const bool centerAlign = theme_.align == PanelAlign::Center;
    const Placement& placement = placement_;
    const int pointerHeight = pointerHeight_;
    // 尻尾は先端がキャレット行の下端・上端に届くよう、尻尾の高さだけ離す
    const int belowGap = pointerHeight > 0 ? pointerHeight : kAnchorGap + TriangleHeight(placement);
    const int aboveGap = pointerHeight > 0 ? pointerHeight : kAnchorGap;
    const bool hasInput = hasInput_;
    const RECT& work = work_;
    const int workWidth = static_cast<int>(work.right - work.left);
    const int workHeight = static_cast<int>(work.bottom - work.top);
    // 画面中央では幅を候補の数によらず固定し、打鍵ごとに候補の位置を動かさない
    const int centerWidth = CenterWidth(theme_, m, workWidth);

    const size_t total = allWidths_.size();
    const bool pages = pageStarts_.size() > 1;
    size_t begin = 0;
    size_t count = 0;
    if (page < pageStarts_.size()) {
        begin = pageStarts_[page];
        count = (page + 1 < pageStarts_.size() ? pageStarts_[page + 1] : total) - begin;
    }
    const std::vector<float> widths(allWidths_.begin() + static_cast<std::ptrdiff_t>(begin),
                                    allWidths_.begin() + static_cast<std::ptrdiff_t>(begin + count));
    // 入力行と区切り線 (前後の間を含む) の高さ
    const int header = hasInput ? m.inputRowHeight + m.rowGap * 2 + m.line : 0;
    const int pageRow =
        pages ? static_cast<int>(std::lround(m.chipGap)) + PageRowHeight(theme_, m) : 0;
    // 縦積みで n 行を出すときのパネルの高さ。複数ページなら位置の表示の行を足す
    const auto columnHeight = [&](size_t n) {
        return ColumnHeight(m, hasInput, n) + (n > 0 ? pageRow : 0);
    };
    // 横一列のチップの列と位置の表示を合わせた幅
    float chipsWidth = 0.0f;
    for (size_t i = 0; i < widths.size(); ++i) {
        chipsWidth += (i > 0 ? m.chipGap : 0.0f) + widths[i];
    }
    const float rowWidth = chipsWidth + (pages ? m.chipGap + pageLabelWidth_ : 0.0f);

    int x = 0;
    int y = 0;
    int width = 0;
    int height = 0;
    if (column) {
        height = columnHeight(count);
        if (placement.position == Position::Center) {
            width = centerWidth;
            x = static_cast<int>(work.left) + (workWidth - width) / 2;
            // 上端は出せる最大の行数のときの高さで決めて固定し、候補の数が変わっても入力行と
            // 先頭の行を動かさない (下へ伸び縮みする)
            size_t fullRows = 0;
            while (fullRows < kMaxItems && columnHeight(fullRows + 1) <= workHeight) {
                ++fullRows;
            }
            const int fullHeight = (std::max)(columnHeight(fullRows), height);
            y = static_cast<int>(work.top) + (std::max)(0, (workHeight - fullHeight) / 2);
        } else {
            float widest = inputWidth_;
            for (const float rowWidthOfItem : widths) {
                widest = (std::max)(widest, rowWidthOfItem);
            }
            if (pages) {
                widest = (std::max)(widest, m.chipPadding * 2 + pageLabelWidth_);
            }
            width = m.padding * 2 + static_cast<int>(std::ceil(widest));
            width = (std::max)(width, m.chipHeight * 2);
            width = (std::min)(width, workWidth);
            x = (std::max)(placement.x - PointerLeftShift(m, height, pointerHeight),
                           static_cast<int>(work.left));
            if (x + width > work.right) {
                x = (std::max)(static_cast<int>(work.left), static_cast<int>(work.right) - width);
            }
            const RECT& caret = *placement.caret;
            y = static_cast<int>(caret.bottom) + belowGap;
            // 上に出すときは下端をキャレット行の上に揃え、候補が増えると上へ伸ばす
            if (columnSide_ == ColumnSide::Above ||
                (columnSide_ == ColumnSide::None && y + height > work.bottom)) {
                y = static_cast<int>(caret.top) - height - aboveGap;
            }
            y = (std::max)(y, static_cast<int>(work.top));
        }
    } else {
        // 入力行を出すときは、候補が無くてもチップ列の高さを取って入力行の位置を上下させない
        height = m.padding * 2 + m.chipHeight + header;
        if (placement.position == Position::Center) {
            width = centerWidth;
            x = static_cast<int>(work.left) + (workWidth - width) / 2;
            y = static_cast<int>(work.top) + (workHeight - height) / 2;
        } else {
            const RECT& caret = *placement.caret;
            x = (std::max)(placement.x - PointerLeftShift(m, height, pointerHeight),
                           static_cast<int>(work.left));
            const int rowNeeded = m.padding * 2 + static_cast<int>(std::ceil(rowWidth));
            if (count > 0 && x + rowNeeded > work.right) {
                // 1 件だけで幅を超える候補のページは、その候補が収まるところまで左へ寄せる
                x = (std::max)(static_cast<int>(work.left), static_cast<int>(work.right) - rowNeeded);
            }
            width = m.padding * 2 + static_cast<int>(std::ceil((std::max)(rowWidth, inputWidth_)));
            width = (std::max)(width, m.chipHeight * 2);
            // 入力行が長いときは作業領域の右端で切る (候補は上で収まる分だけにしてある)
            width = (std::max)((std::min)(width, static_cast<int>(work.right) - x), rowNeeded);

            y = static_cast<int>(caret.bottom) + belowGap;
            if (y + height > work.bottom) {
                y = static_cast<int>(caret.top) - height - aboveGap;
            }
        }
    }

    itemTexts_.assign(allTexts_.begin() + static_cast<std::ptrdiff_t>(begin),
                      allTexts_.begin() + static_cast<std::ptrdiff_t>(begin + count));
    items_.assign(allItems_.begin() + static_cast<std::ptrdiff_t>(begin),
                  allItems_.begin() + static_cast<std::ptrdiff_t>(begin + count));
    const float padding = static_cast<float>(m.padding);
    const float contentWidth = static_cast<float>(width) - padding * 2;
    inputTop_ = padding;
    separatorTop_ = padding + static_cast<float>(m.inputRowHeight + m.rowGap);
    float itemsTop = padding;
    separator_ = false;
    if (hasInput && (!column || count > 0)) {
        itemsTop += static_cast<float>(header);
        // ghost は枠を持たないので区切り線も引かない (間だけ取る)
        separator_ = !ghost;
    }
    itemBoxes_.clear();
    pageText_ = PageText(selection);
    if (column) {
        for (size_t i = 0; i < count; ++i) {
            // 左揃えでは行の幅をそろえ、中央揃えでは行ごとの幅で中央に置く
            const float boxWidth = centerAlign ? (std::min)(widths[i], contentWidth) : contentWidth;
            const float left = padding + (centerAlign ? (contentWidth - boxWidth) / 2 : 0.0f);
            const float top =
                itemsTop + static_cast<float>(i) * (static_cast<float>(m.chipHeight) + m.chipGap);
            itemBoxes_.push_back({left, std::round(top), boxWidth});
        }
        pageInline_ = false;
        pageTop_ = itemsTop + static_cast<float>(ColumnHeight(m, false, count) - m.padding * 2) +
                   std::round(m.chipGap);
    } else {
        // 1 件だけで幅を超える候補 (画面中央の固定幅など) は、チップを幅に収めて文字を「…」で切る
        const float maxChip =
            (std::max)(0.0f, contentWidth - (pages ? m.chipGap + pageLabelWidth_ : 0.0f));
        float chipX =
            padding + (centerAlign ? (std::max)(0.0f, (contentWidth - rowWidth) / 2) : 0.0f);
        for (size_t i = 0; i < count; ++i) {
            const float boxWidth = (std::min)(widths[i], maxChip);
            itemBoxes_.push_back({chipX, itemsTop, boxWidth});
            chipX += boxWidth + m.chipGap;
        }
        pageInline_ = true;
        pageLeft_ = chipX;
        pageTop_ = itemsTop;
    }
    if (column && columnSide_ == ColumnSide::Above) {
        MirrorColumn(height);
    }
    page_ = page;
    pageStart_ = begin;
    selection_ = selection >= begin && selection < begin + count ? selection - begin : count;
    bodyX_ = x;
    bodyY_ = y;
    bodyWidth_ = width;
    bodyHeight_ = height;
    PlaceTail(placement, pointerHeight);
    margin_ = LayeredMargin();
    if (!Render(kind, placement.caret)) {
        return 0;
    }
    return count;
}

size_t SuggestPanel::ShowConversion(const Placement& placement,
                                    const std::vector<std::wstring>& candidates,
                                    const std::vector<bool>& typo, size_t selection,
                                    const InputLine* input)
{
    if (textFormat_ == nullptr ||
        (placement.position == Position::Caret && placement.caret == nullptr) ||
        (candidates.empty() && input == nullptr)) {
        Hide();
        return 0;
    }
    UseOwner(placement.owner);
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const bool ghost = theme_.style == PanelStyle::Ghost;
    const bool centerAlign = theme_.align == PanelAlign::Center;
    const bool hasInput = input != nullptr;
    const size_t total = candidates.size();
    const int pointerHeight = PointerHeight(placement);
    const int belowGap = pointerHeight > 0 ? pointerHeight : kAnchorGap + TriangleHeight(placement);
    const int aboveGap = pointerHeight > 0 ? pointerHeight : kAnchorGap;
    const int pageRow = static_cast<int>(std::lround(m.chipGap)) + PageRowHeight(theme_, m);
    // n 行を出すときのパネルの高さ。pages なら位置の表示の行を足す
    const auto heightOf = [&](size_t n, bool pages) {
        return ColumnHeight(m, hasInput, n) + (n > 0 && pages ? pageRow : 0);
    };

    HMONITOR monitor = nullptr;
    if (placement.position == Position::Center) {
        if (placement.caret != nullptr) {
            monitor = MonitorFromRect(placement.caret, MONITOR_DEFAULTTONEAREST);
        } else {
            HWND foreground = GetForegroundWindow();
            monitor = foreground != nullptr
                          ? MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
                          : MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY);
        }
    } else {
        // FitToWorkArea と同じモニタで行数を決める
        const POINT origin = {placement.caret->left, placement.caret->top};
        monitor = MonitorFromPoint(origin, MONITOR_DEFAULTTONEAREST);
    }
    const RECT work = WorkAreaOf(monitor);
    const int workWidth = static_cast<int>(work.right - work.left);
    const int workHeight = static_cast<int>(work.bottom - work.top);

    ColumnSide side = ColumnSide::None;
    size_t pageSize = 0;
    if (placement.position == Position::Caret) {
        pageSize = FitColumn(placement, work, hasInput, total, &side);
    }
    if (pageSize == 0) {
        // 作業領域に kMaxItems 行が収まらなければ、収まる行数を 1 ページにしてページ送りで
        // 全候補に届かせる
        const size_t limit = (std::max)(static_cast<size_t>(1), (std::min)(kMaxItems, total));
        pageSize = 1;
        while (pageSize < limit && heightOf(pageSize + 1, total > pageSize + 1) <= workHeight) {
            ++pageSize;
        }
    }
    const bool pages = total > pageSize;
    const size_t current = total == 0 ? 0 : (std::min)(selection, total - 1);
    const size_t begin = current / pageSize * pageSize;
    const size_t count = (std::min)(begin + pageSize, total) - begin;

    std::vector<std::wstring> texts;
    std::vector<ItemKind> kinds;
    std::vector<float> widths;  // 切り詰めずに出したときの行の幅
    for (size_t i = begin; i < begin + count; ++i) {
        const ItemKind kind = i < typo.size() && typo[i] ? ItemKind::Typo : ItemKind::Conversion;
        float textWidth = TextWidth(candidates[i], textFormat_);
        if (theme_.selection == SelectionStyle::Text) {
            // 選択すると太字になるので、選択によって幅が変わらないよう太字の幅も見込む
            textWidth = (std::max)(textWidth, TextWidth(candidates[i], boldFormat_));
        }
        float width = m.chipPadding * 2 + m.badgeSlot + textWidth;
        if (kind == ItemKind::Typo) {
            width += m.tagGap + TextWidth(KindTagLabel(kind), tagFormat_) + m.tagPadding * 2;
        }
        widths.push_back(width);
        texts.push_back(candidates[i]);
        kinds.push_back(kind);
    }
    std::wstring pageText;
    if (pages) {
        pageText = std::to_wstring(current + 1) + L"/" + std::to_wstring(total);
    }

    const int height = heightOf(count, pages);
    int x = 0;
    int y = 0;
    int width = 0;
    if (placement.position == Position::Center) {
        width = static_cast<int>((std::min)(
            theme_.centerWidth * m.f, static_cast<float>(workWidth) * theme_.centerWidthRatio));
        x = static_cast<int>(work.left) + (workWidth - width) / 2;
        y = CenterBarTop(theme_, m, work, hasInput);
        if (y + height > work.bottom) {
            y = static_cast<int>(work.bottom) - height;
        }
        y = (std::max)(y, static_cast<int>(work.top));
    } else {
        float widest = 0.0f;
        for (const float rowWidth : widths) {
            widest = (std::max)(widest, rowWidth);
        }
        if (!pageText.empty()) {
            widest = (std::max)(widest, m.chipPadding * 2 + TextWidth(pageText, labelFormat_));
        }
        if (hasInput) {
            float inputWidth = TextWidth(input->excluded, inputFormat_);
            for (const std::wstring& segment : input->segments) {
                inputWidth += TextWidth(segment, inputFormat_);
            }
            widest = (std::max)(widest, inputWidth);
        }
        width = m.padding * 2 + static_cast<int>(std::ceil(widest));
        width = (std::max)(width, m.chipHeight * 2);
        width = (std::min)(width, workWidth);
        const RECT& anchor = *placement.caret;
        const POINT position =
            FitToWorkArea(anchor, placement.x - PointerLeftShift(m, height, pointerHeight),
                          static_cast<int>(anchor.bottom) + belowGap, width, height, aboveGap);
        x = position.x;
        y = position.y;
        if (side == ColumnSide::Below) {
            y = static_cast<int>(anchor.bottom) + belowGap;
        } else if (side == ColumnSide::Above) {
            y = static_cast<int>(anchor.top) - height - aboveGap;
        }
    }

    conversion_ = true;
    itemTexts_ = std::move(texts);
    items_.clear();
    for (size_t i = 0; i < itemTexts_.size(); ++i) {
        items_.push_back({itemTexts_[i], kinds[i]});
    }
    const float padding = static_cast<float>(m.padding);
    const float contentWidth = static_cast<float>(width) - padding * 2;
    inputTop_ = padding;
    separatorTop_ = padding + static_cast<float>(m.inputRowHeight + m.rowGap);
    float itemsTop = padding;
    separator_ = false;
    if (hasInput && count > 0) {
        itemsTop += static_cast<float>(m.inputRowHeight + m.rowGap * 2 + m.line);
        // ghost は枠を持たないので区切り線も引かない (間だけ取る)
        separator_ = !ghost;
    }
    itemBoxes_.clear();
    for (size_t i = 0; i < count; ++i) {
        // 左揃えでは行の幅をそろえ、中央揃えでは行ごとの幅で中央に置く
        const float boxWidth = centerAlign ? (std::min)(widths[i], contentWidth) : contentWidth;
        const float left = padding + (centerAlign ? (contentWidth - boxWidth) / 2 : 0.0f);
        const float top =
            itemsTop + static_cast<float>(i) * (static_cast<float>(m.chipHeight) + m.chipGap);
        itemBoxes_.push_back({left, std::round(top), boxWidth});
    }
    pageText_ = std::move(pageText);
    pageInline_ = false;
    pageTop_ = itemsTop + static_cast<float>(ColumnHeight(m, false, count) - m.padding * 2) +
               std::round(m.chipGap);
    columnSide_ = side;
    if (side == ColumnSide::Above) {
        MirrorColumn(height);
    }
    selection_ = current - begin;
    hasInput_ = hasInput;
    input_ = hasInput ? *input : InputLine{};
    bodyX_ = x;
    bodyY_ = y;
    bodyWidth_ = width;
    bodyHeight_ = height;
    PlaceTail(placement, pointerHeight);
    margin_ = LayeredMargin();
    if (!Render(L"変換の候補", placement.caret)) {
        return 0;
    }
    return count > 0 ? pageSize : 0;
}

bool SuggestPanel::Render(const wchar_t* kind, const RECT* anchor)
{
    if (offscreen_) {
        return RenderOffscreen(kind);
    }
    if (!EnsureWindowClass()) {
        if (logger_) {
            logger_(std::wstring(kind) + L": ウィンドウクラスの登録に失敗 error=" +
                    std::to_wstring(GetLastError()));
        }
        return false;
    }
    bool created = false;
    if (hwnd_ == nullptr && blur_ && !blurFailed_) {
        created = CreateBlurWindow(kind);
    }
    if (blurActive_) {
        // ぼかしの方式では影を DWM に任せるため、ウィンドウの矩形 = パネルの矩形
        margin_ = 0;
        SetWindowPos(hwnd_, HWND_TOPMOST, bodyX_, bodyY_, bodyWidth_, bodyHeight_,
                     SWP_NOACTIVATE);
        ShowWindow(hwnd_, SW_SHOWNA);
        // 打鍵ごとの位置・大きさの変化でちらつかないよう、WM_PAINT を待たずにその場で描く
        const HRESULT hr = DrawBlur();
        ValidateRect(hwnd_, nullptr);
        if (SUCCEEDED(hr)) {
            LogShow(kind, anchor, created, ERROR_SUCCESS);
            return true;
        }
        if (hwndTarget_ != nullptr) {
            if (logger_) {
                logger_(std::wstring(kind) + L": 描画に失敗 hr=" + DebugHResultText(hr));
            }
            return false;
        }
        // 描画先を作り直せなかったときは、このインスタンスではぼかしをやめる
        DestroyBlurWindow();
        blurFailed_ = true;
        created = false;
        margin_ = LayeredMargin();
        if (logger_) {
            logger_(std::wstring(kind) +
                    L": ぼかしをやめて layered で描く (ID2D1HwndRenderTarget を作り直せない hr=" +
                    DebugHResultText(hr) + L")");
        }
    }

    // layered の描画先はぼかしの描画先を解放して作るため、ぼかしの方式では作らない
    if (!EnsureTarget()) {
        if (logger_) {
            logger_(std::wstring(kind) + L": Direct2D の描画先を作れない");
        }
        Hide();
        return false;
    }
    const int windowWidth = bodyWidth_ + margin_ * 2;
    const int windowHeight = bodyHeight_ + margin_ * 2 + TopTailPad() + BottomTailPad();
    const int windowY = bodyY_ - margin_ - TopTailPad();

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
        hr = DrawLayered(memory, {0, 0, windowWidth, windowHeight});
    }

    DWORD createError = ERROR_SUCCESS;
    bool updated = false;
    if (SUCCEEDED(hr)) {
        if (hwnd_ == nullptr) {
            hwnd_ = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                kWindowClassName, L"", WS_POPUP, bodyX_ - margin_, windowY, windowWidth,
                windowHeight, owner_, nullptr, globals::dllInstance, this);
            if (hwnd_ == nullptr) {
                createError = GetLastError();
            } else {
                created = true;
            }
        }
        if (hwnd_ != nullptr) {
            POINT destination = {bodyX_ - margin_, windowY};
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

HRESULT SuggestPanel::DrawLayered(HDC memory, const RECT& rect)
{
    HRESULT hr = E_FAIL;
    // 描画先を作り直す必要が出たときは1度だけやり直す
    for (int attempt = 0; attempt < 2; ++attempt) {
        if (!EnsureTarget()) {
            break;
        }
        hr = target_->BindDC(memory, &rect);
        if (SUCCEEDED(hr)) {
            drawTarget_ = target_;
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
    return hr;
}

bool SuggestPanel::RenderOffscreen(const wchar_t* kind)
{
    snapshot_.clear();
    snapshotWidth_ = 0;
    snapshotHeight_ = 0;
    if (!EnsureTarget()) {
        if (logger_) {
            logger_(std::wstring(kind) + L": Direct2D の描画先を作れない");
        }
        return false;
    }
    const int width = bodyWidth_ + margin_ * 2;
    const int height = bodyHeight_ + margin_ * 2 + TopTailPad() + BottomTailPad();
    if (width <= 0 || height <= 0) {
        return false;
    }

    HDC memory = CreateCompatibleDC(nullptr);
    BITMAPINFO info = {};
    info.bmiHeader.biSize = sizeof(info.bmiHeader);
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;  // 上から下へ並ぶ DIB
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
        hr = DrawLayered(memory, {0, 0, width, height});
    }
    if (SUCCEEDED(hr) && bits != nullptr) {
        GdiFlush();
        const BYTE* begin = static_cast<const BYTE*>(bits);
        snapshot_.assign(begin, begin + static_cast<size_t>(width) * static_cast<size_t>(height) * 4);
        snapshotWidth_ = width;
        snapshotHeight_ = height;
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
    if (FAILED(hr)) {
        if (logger_) {
            logger_(std::wstring(kind) + L": 描画に失敗 hr=" + DebugHResultText(hr));
        }
        return false;
    }
    return !snapshot_.empty();
}

bool SuggestPanel::CreateBlurWindow(const wchar_t* kind)
{
    // sticky は API が使えない理由 (このインスタンスでは再び試さない)
    const auto fallback = [&](const std::wstring& reason, bool sticky) {
        if (hwnd_ != nullptr) {
            DestroyBlurWindow();
        }
        if (sticky) {
            blurFailed_ = true;
        }
        if (logger_) {
            logger_(std::wstring(kind) + L": ぼかしをやめて layered で描く (" + reason + L")");
        }
        return false;
    };

    if (!TransparencyEnabled()) {
        return fallback(L"Windows の透明効果がオフ", false);
    }
    // 作成中に届く WM_ERASEBKGND・WM_PAINT をぼかしの方式として扱うため、先に立てる
    blurActive_ = true;
    hwnd_ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, kWindowClassName,
                            L"", WS_POPUP, bodyX_, bodyY_, bodyWidth_, bodyHeight_, owner_,
                            nullptr, globals::dllInstance, this);
    if (hwnd_ == nullptr) {
        blurActive_ = false;
        return fallback(L"CreateWindowExW 失敗 error=" + std::to_wstring(GetLastError()), false);
    }
    if (!ApplyAcrylic(hwnd_, theme_.blurTint)) {
        return fallback(L"SetWindowCompositionAttribute が無いか失敗", true);
    }
    const DWORD corner = kDwmwcpRound;
    const HRESULT hr =
        DwmSetWindowAttribute(hwnd_, kDwmwaWindowCornerPreference, &corner, sizeof(corner));
    if (FAILED(hr)) {
        return fallback(L"DWMWA_WINDOW_CORNER_PREFERENCE の指定に失敗 hr=" + DebugHResultText(hr),
                        true);
    }
    if (!EnsureHwndTarget()) {
        return fallback(L"ID2D1HwndRenderTarget を作れない", true);
    }
    return true;
}

void SuggestPanel::DestroyBlurWindow()
{
    ReleaseHwndTarget();
    if (hwnd_ != nullptr) {
        destroying_ = true;
        DestroyWindow(hwnd_);
        destroying_ = false;
        hwnd_ = nullptr;
    }
    blurActive_ = false;
}

HRESULT SuggestPanel::DrawBlur()
{
    HRESULT hr = E_FAIL;
    // 描画先を作り直す必要が出たときは1度だけやり直す
    for (int attempt = 0; attempt < 2; ++attempt) {
        if (!EnsureHwndTarget()) {
            break;
        }
        const D2D1_SIZE_U size =
            D2D1::SizeU(static_cast<UINT32>(bodyWidth_), static_cast<UINT32>(bodyHeight_));
        const D2D1_SIZE_U current = hwndTarget_->GetPixelSize();
        if (current.width != size.width || current.height != size.height) {
            hr = hwndTarget_->Resize(size);
            if (FAILED(hr)) {
                break;
            }
        }
        drawTarget_ = hwndTarget_;
        hwndTarget_->BeginDraw();
        hwndTarget_->Clear(D2D1::ColorF(0, 0, 0, 0));
        Draw();
        hr = hwndTarget_->EndDraw();
        if (hr != D2DERR_RECREATE_TARGET) {
            break;
        }
        ReleaseHwndTarget();
    }
    return hr;
}

int SuggestPanel::LayeredMargin() const
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    if (theme_.style == PanelStyle::Ghost) {
        return m.ghostMargin;
    }
    return theme_.drawShadow ? m.margin : 0;
}

bool SuggestPanel::BlurExpected() const
{
    // Render がぼかしのウィンドウを作るのは、ウィンドウが無いときだけ
    return blurActive_ || (hwnd_ == nullptr && blur_ && !blurFailed_);
}

int SuggestPanel::PointerHeight(const Placement& placement) const
{
    // 画面中央ではキャレットから離れており、ghost には尻尾を付ける枠が無く、ぼかしでは DWM の
    // 角丸ウィンドウの外に描けない。描けないときは三角にも代えない (テーマで選んだ見た目と違うため)
    if (!placement.pointer || placement.position != Position::Caret ||
        placement.caret == nullptr || theme_.style == PanelStyle::Ghost || BlurExpected()) {
        return 0;
    }
    return static_cast<int>(std::lround(theme_.pointerSize * static_cast<float>(fontSize_)));
}

int SuggestPanel::TriangleHeight(const Placement& placement) const
{
    if (!placement.belowTriangle || placement.position != Position::Caret) {
        return 0;
    }
    return static_cast<int>(std::lround(theme_.triangleSize * static_cast<float>(fontSize_)));
}

void SuggestPanel::PlaceTail(const Placement& placement, int pointerHeight)
{
    tail_ = TailSide::None;
    tailHeight_ = pointerHeight;
    markerX_ = placement.markerX;
    if (pointerHeight <= 0 || placement.caret == nullptr) {
        return;
    }
    // 作業領域に収めるためにキャレット行に重なったときは、指す向きが決まらないので付けない
    if (bodyY_ >= placement.caret->bottom) {
        tail_ = TailSide::Top;
    } else if (bodyY_ + bodyHeight_ <= placement.caret->top) {
        tail_ = TailSide::Bottom;
    }
}

int SuggestPanel::TopTailPad() const
{
    // 先端の縁のアンチエイリアスの分を 1px 足す
    return tail_ == TailSide::Top && !blurActive_ ? tailHeight_ + 1 : 0;
}

int SuggestPanel::BottomTailPad() const
{
    return tail_ == TailSide::Bottom && !blurActive_ ? tailHeight_ + 1 : 0;
}

float SuggestPanel::TailTipX(float bodyLeft, float bodyRight, float radius) const
{
    const float half = static_cast<float>(tailHeight_);
    const float low = bodyLeft + radius + half;
    const float high = bodyRight - radius - half;
    if (low > high) {
        return (bodyLeft + bodyRight) / 2;
    }
    const float tipX = bodyLeft + static_cast<float>(markerX_ - bodyX_);
    return (std::clamp)(tipX, low, high);
}

std::wstring SuggestPanel::GhostText() const
{
    if (theme_.style != PanelStyle::Ghost || conversion_ || itemTexts_.empty()) {
        return {};
    }
    const size_t index = selection_ < itemTexts_.size() ? selection_ : 0;
    return L"→ " + itemTexts_[index];
}

void SuggestPanel::DrawLabel(const std::wstring& text, IDWriteTextFormat* format, float left,
                             float top, float right, float bottom, const PanelColor& color)
{
    if (text.empty() || format == nullptr) {
        return;
    }
    const UINT32 length = static_cast<UINT32>(text.size());
    if (theme_.textShadow) {
        // 背景の明暗によらず読めるよう、文字と反対の明るさの影を 1px ずらして敷く
        PanelColor shadow = theme_.dark ? PanelColor{0.0f, 0.0f, 0.0f, 1.0f}
                                        : PanelColor{1.0f, 1.0f, 1.0f, 1.0f};
        shadow.a = kTextShadowAlpha * color.a;
        brush_->SetColor(ToD2D(shadow));
        drawTarget_->DrawText(text.c_str(), length, format,
                              D2D1::RectF(left + 1.0f, top + 1.0f, right + 1.0f, bottom + 1.0f),
                              brush_);
    }
    brush_->SetColor(ToD2D(color));
    drawTarget_->DrawText(text.c_str(), length, format, D2D1::RectF(left, top, right, bottom),
                          brush_);
}

void SuggestPanel::Draw()
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float margin = static_cast<float>(margin_);
    const float width = static_cast<float>(bodyWidth_);
    const float height = static_cast<float>(bodyHeight_);
    const float line = static_cast<float>(m.line);
    const float radius = BodyRadius(m, height);
    const float bodyTop = margin + static_cast<float>(TopTailPad());

    const D2D1_RECT_F body = D2D1::RectF(margin, bodyTop, margin + width, bodyTop + height);
    // ぼかしの方式では背景・外周の縁・影を DWM に任せる
    if (!blurActive_) {
        if (theme_.style == PanelStyle::Ghost) {
            DrawScrim();
        } else {
            if (theme_.drawShadow) {
                DrawShadow();
            }
            const float half = line / 2;
            const D2D1_RECT_F inset =
                D2D1::RectF(body.left + half, body.top + half, body.right - half, body.bottom - half);
            if (tail_ != TailSide::None) {
                // 尻尾とパネルを1つの輪郭で塗り・縁取りし、縁を尻尾の底辺の部分で切ってひと続きにする
                const bool up = tail_ == TailSide::Top;
                const float tipX = TailTipX(body.left, body.right, radius);
                const float tail = static_cast<float>(tailHeight_);
                ID2D1PathGeometry* fill =
                    CreateBalloonGeometry(d2dFactory_, body, radius, up, tipX, tail);
                if (fill != nullptr) {
                    brush_->SetColor(ToD2D(theme_.background));
                    drawTarget_->FillGeometry(fill, brush_);
                    fill->Release();
                }
                if (theme_.drawBorder) {
                    ID2D1PathGeometry* outline =
                        CreateBalloonGeometry(d2dFactory_, inset, radius - half, up, tipX,
                                              OffsetTailHeight(tail, -half));
                    if (outline != nullptr) {
                        brush_->SetColor(ToD2D(theme_.border));
                        drawTarget_->DrawGeometry(outline, brush_, line);
                        outline->Release();
                    }
                }
            } else {
                brush_->SetColor(ToD2D(theme_.background));
                drawTarget_->FillRoundedRectangle(D2D1::RoundedRect(body, radius, radius), brush_);
                if (theme_.drawBorder) {
                    brush_->SetColor(ToD2D(theme_.border));
                    drawTarget_->DrawRoundedRectangle(
                        D2D1::RoundedRect(inset, radius - half, radius - half), brush_, line);
                }
            }
        }
    }

    const float padding = static_cast<float>(m.padding);
    const float left = body.left + padding;
    const float contentWidth = width - padding * 2;
    if (hasInput_) {
        const float top = body.top + inputTop_;
        DrawInputLine(left, top, contentWidth);
        if (separator_) {
            const float separatorY = body.top + separatorTop_;
            brush_->SetColor(ToD2D(theme_.separator));
            drawTarget_->FillRectangle(
                D2D1::RectF(left, separatorY, left + contentWidth, separatorY + line), brush_);
        }
    }
    for (size_t i = 0; i < itemBoxes_.size(); ++i) {
        DrawItem(i, body.left, body.top);
    }
    if (!pageText_.empty() && pageInline_) {
        const float pageTop = body.top + pageTop_;
        DrawLabel(pageText_, labelFormat_, body.left + pageLeft_, pageTop, left + contentWidth,
                  pageTop + static_cast<float>(m.chipHeight), theme_.secondary);
    } else if (!pageText_.empty()) {
        float pageX = left + m.chipPadding;
        if (theme_.align == PanelAlign::Center) {
            const float pageWidth = TextWidth(pageText_, labelFormat_);
            pageX = left + (std::max)(0.0f, std::round((contentWidth - pageWidth) / 2));
        }
        const float pageTop = body.top + pageTop_;
        DrawLabel(pageText_, labelFormat_, pageX, pageTop, left + contentWidth,
                  pageTop + static_cast<float>(PageRowHeight(theme_, m)), theme_.secondary);
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
    const float top = margin + static_cast<float>(TopTailPad());
    const float width = static_cast<float>(bodyWidth_);
    const float height = static_cast<float>(bodyHeight_);
    const float radius = BodyRadius(m, height);
    const float tipX = TailTipX(margin, margin + width, radius);
    // ID2D1DCRenderTarget ではブラーのエフェクトが使えないため、少しずつ広げた角丸矩形を外側から
    // 重ねて近似する。全部が重なるパネルの縁で最大不透明度になるよう、1枚の不透明度を決める
    const float alpha =
        1.0f - std::pow(1.0f - theme_.shadow.a, 1.0f / static_cast<float>(steps));
    PanelColor color = theme_.shadow;
    color.a = alpha;
    brush_->SetColor(ToD2D(color));
    for (int k = steps; k >= 1; --k) {
        const float spread = m.shadowSpread * static_cast<float>(k) / static_cast<float>(steps);
        const D2D1_RECT_F rect = D2D1::RectF(margin - spread, top - spread + m.shadowOffset,
                                             margin + width + spread,
                                             top + height + spread + m.shadowOffset);
        if (tail_ == TailSide::None) {
            drawTarget_->FillRoundedRectangle(
                D2D1::RoundedRect(rect, radius + spread, radius + spread), brush_);
            continue;
        }
        // 尻尾と矩形を別々に塗ると重なりで濃くなるため、1つの輪郭にして塗る
        ID2D1PathGeometry* geometry = CreateBalloonGeometry(
            d2dFactory_, rect, radius + spread, tail_ == TailSide::Top, tipX,
            OffsetTailHeight(static_cast<float>(tailHeight_), spread));
        if (geometry != nullptr) {
            drawTarget_->FillGeometry(geometry, brush_);
            geometry->Release();
        }
    }
}

void SuggestPanel::DrawScrim()
{
    if (theme_.scrim.a <= 0.0f) {
        return;
    }
    // ウィンドウ (外側の余白を含む) に内接する楕円で透明まで落とし、ウィンドウの端で切れて
    // 見えないようにする。中心から一様に薄めると文字の位置で背景が透けて読みにくいため、
    // 文字の載る内側は scrim の濃さを保ち、外側だけで透明へ落とす
    const float margin = static_cast<float>(margin_);
    const float radiusX = static_cast<float>(bodyWidth_) / 2 + margin;
    const float radiusY = static_cast<float>(bodyHeight_) / 2 + margin;
    const D2D1_POINT_2F center = D2D1::Point2F(radiusX, radiusY);
    PanelColor clear = theme_.scrim;
    clear.a = 0.0f;
    const D2D1_GRADIENT_STOP stops[] = {
        {0.0f, ToD2D(theme_.scrim)},
        {0.45f, ToD2D(theme_.scrim)},
        {1.0f, ToD2D(clear)},
    };
    ID2D1GradientStopCollection* collection = nullptr;
    if (FAILED(drawTarget_->CreateGradientStopCollection(stops, ARRAYSIZE(stops), &collection))) {
        return;
    }
    ID2D1RadialGradientBrush* gradient = nullptr;
    if (SUCCEEDED(drawTarget_->CreateRadialGradientBrush(
            D2D1::RadialGradientBrushProperties(center, D2D1::Point2F(0, 0), radiusX, radiusY),
            collection, &gradient))) {
        drawTarget_->FillEllipse(D2D1::Ellipse(center, radiusX, radiusY), gradient);
        gradient->Release();
    }
    collection->Release();
}

void SuggestPanel::DrawInputLine(float left, float top, float width)
{
    if (!input_.segments.empty()) {
        DrawConversionInput(left, top, width);
        return;
    }
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float rowHeight = static_cast<float>(m.inputRowHeight);
    const float big = 100000.0f;
    const bool contextOnly = input_.text.empty() && input_.pending.empty();
    const std::wstring ghost = GhostText();
    PanelColor ghostColor = theme_.text;
    ghostColor.a *= theme_.ghostOpacity;

    // 中央揃えでは、ゴーストまで含めた入力行の幅で中央に置く
    float x = left;
    if (theme_.align == PanelAlign::Center) {
        float total = 0.0f;
        if (contextOnly) {
            if (!input_.context.empty()) {
                total += TextWidth(input_.context, inputFormat_) + m.labelGap;
            }
            if (input_.nextWordLabel) {
                total += TextWidth(kNextWordLabel, labelFormat_);
            }
        } else {
            total = std::round(TextWidth(input_.text, inputFormat_) +
                               TextWidth(input_.pending, inputFormat_)) +
                    1.0f + theme_.caretWidthPx;
        }
        if (!ghost.empty()) {
            total += m.labelGap + TextWidth(ghost, inputFormat_);
        }
        x += (std::max)(0.0f, std::round((width - total) / 2));
    }

    drawTarget_->PushAxisAlignedClip(D2D1::RectF(left, top, left + width, top + rowHeight),
                                     D2D1_ANTIALIAS_MODE_ALIASED);

    if (contextOnly) {
        if (!input_.context.empty()) {
            DrawLabel(input_.context, inputFormat_, x, top, x + big, top + rowHeight,
                      theme_.secondary);
            x += TextWidth(input_.context, inputFormat_) + m.labelGap;
        }
        if (input_.nextWordLabel) {
            DrawLabel(kNextWordLabel, labelFormat_, x, top, x + big, top + rowHeight,
                      theme_.nextWord);
            x += TextWidth(kNextWordLabel, labelFormat_);
        }
        if (!ghost.empty()) {
            const float ghostX = x + m.labelGap;
            DrawLabel(ghost, inputFormat_, ghostX, top, ghostX + big, top + rowHeight, ghostColor);
        }
        drawTarget_->PopAxisAlignedClip();
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
    const float underlineY =
        std::round(lineTop + baseline + (std::max)(2.0f, m.inputTextSize * 0.15f));
    const float line = static_cast<float>(m.line);

    if (!input_.text.empty()) {
        const float textWidth = TextWidth(input_.text, inputFormat_);
        DrawLabel(input_.text, inputFormat_, x, top, x + big, top + rowHeight, theme_.text);
        brush_->SetColor(ToD2D(theme_.accent));
        drawTarget_->FillRectangle(
            D2D1::RectF(x, underlineY, x + textWidth, underlineY + theme_.underlinePx), brush_);
        x += textWidth;
    }
    if (!input_.pending.empty()) {
        const float pendingWidth = TextWidth(input_.pending, inputFormat_);
        DrawLabel(input_.pending, inputFormat_, x, top, x + big, top + rowHeight,
                  theme_.secondary);
        brush_->SetColor(ToD2D(theme_.secondary));
        const float dotY = underlineY + line / 2;
        drawTarget_->DrawLine(D2D1::Point2F(x, dotY), D2D1::Point2F(x + pendingWidth, dotY),
                              brush_, line, dotStroke_);
        x += pendingWidth;
    }
    // キャレットは点滅させない (打鍵ごとの再描画以外にタイマーで描き直さない)
    brush_->SetColor(ToD2D(theme_.accent));
    const float caretX = std::round(x) + 1.0f;
    drawTarget_->FillRectangle(D2D1::RectF(caretX, lineTop + lineHeight * 0.12f,
                                           caretX + theme_.caretWidthPx,
                                           lineTop + lineHeight * 0.88f),
                               brush_);
    if (!ghost.empty()) {
        const float ghostX = caretX + theme_.caretWidthPx + m.labelGap;
        DrawLabel(ghost, inputFormat_, ghostX, top, ghostX + big, top + rowHeight, ghostColor);
    }
    drawTarget_->PopAxisAlignedClip();
}

void SuggestPanel::DrawConversionInput(float left, float top, float width)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const float rowHeight = static_cast<float>(m.inputRowHeight);
    const float big = 100000.0f;
    const float line = static_cast<float>(m.line);
    const std::vector<std::wstring>& segments = input_.segments;
    const size_t current = (std::min)(input_.currentSegment, segments.size() - 1);

    // 下線と強調の位置は、文字列全体を同じ書式で並べたときの行の寸法から決める
    std::wstring whole = input_.excluded;
    for (const std::wstring& segment : segments) {
        whole += segment;
    }
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
    const float underlineY =
        std::round(lineTop + baseline + (std::max)(2.0f, m.inputTextSize * 0.15f));

    const float excludedWidth = TextWidth(input_.excluded, inputFormat_);
    std::vector<float> widths;
    float total = excludedWidth;
    float currentStart = excludedWidth;
    for (size_t i = 0; i < segments.size(); ++i) {
        widths.push_back(TextWidth(segments[i], inputFormat_));
        if (i < current) {
            currentStart += widths.back();
        }
        total += widths.back();
    }
    const float currentEnd = currentStart + widths[current];

    // 現在文節が幅に収まらなければ左を「…」で切り、現在文節の末尾を右端にそろえる
    // (現在文節だけで幅を超えるときは、その先頭を「…」の直後にそろえる)
    float x = left;
    float clipLeft = left;
    const bool truncated = currentEnd > width;
    if (truncated) {
        clipLeft = left + TextWidth(L"…", inputFormat_);
        x = left + width - currentEnd;
        if (x + currentStart < clipLeft) {
            x = clipLeft - currentStart;
        }
    } else if (theme_.align == PanelAlign::Center) {
        x += (std::max)(0.0f, std::round((width - total) / 2));
    }

    drawTarget_->PushAxisAlignedClip(D2D1::RectF(left, top, left + width, top + rowHeight),
                                     D2D1_ANTIALIAS_MODE_ALIASED);
    if (truncated) {
        DrawLabel(L"…", inputFormat_, left, top, clipLeft, top + rowHeight, theme_.secondary);
    }
    drawTarget_->PushAxisAlignedClip(D2D1::RectF(clipLeft, top, left + width, top + rowHeight),
                                     D2D1_ANTIALIAS_MODE_ALIASED);

    // 文節の境目では、下線を両側から 1px ずつ縮めて 2px 空ける
    if (!input_.excluded.empty()) {
        DrawLabel(input_.excluded, inputFormat_, x, top, x + big, top + rowHeight,
                  theme_.secondary);
        brush_->SetColor(ToD2D(theme_.secondary));
        const float dotY = underlineY + line / 2;
        drawTarget_->DrawLine(D2D1::Point2F(x, dotY),
                              D2D1::Point2F(x + excludedWidth - 1.0f, dotY), brush_, line,
                              dotStroke_);
    }
    float segmentX = x + excludedWidth;
    for (size_t i = 0; i < segments.size(); ++i) {
        const float segmentWidth = widths[i];
        const float start = segmentX + (i > 0 || !input_.excluded.empty() ? 1.0f : 0.0f);
        const float end = segmentX + segmentWidth - (i + 1 < segments.size() ? 1.0f : 0.0f);
        if (i == current) {
            PanelColor fill = theme_.accent;
            fill.a *= kSegmentAlpha;
            brush_->SetColor(ToD2D(fill));
            drawTarget_->FillRectangle(
                D2D1::RectF(segmentX, lineTop, segmentX + segmentWidth, lineTop + lineHeight),
                brush_);
        }
        DrawLabel(segments[i], inputFormat_, segmentX, top, segmentX + big, top + rowHeight,
                  theme_.text);
        if (i == current) {
            brush_->SetColor(ToD2D(theme_.accent));
            drawTarget_->FillRectangle(
                D2D1::RectF(start, underlineY, end, underlineY + theme_.underlinePx), brush_);
        } else {
            brush_->SetColor(ToD2D(theme_.secondary));
            drawTarget_->FillRectangle(D2D1::RectF(start, underlineY, end, underlineY + line),
                                       brush_);
        }
        segmentX += segmentWidth;
    }
    drawTarget_->PopAxisAlignedClip();
    drawTarget_->PopAxisAlignedClip();
}

void SuggestPanel::DrawItem(size_t index, float originX, float originY)
{
    const Metrics m = ComputeMetrics(theme_, fontSize_);
    const ItemBox& box = itemBoxes_[index];
    // 変換の候補一覧はテーマの layout によらず縦積み
    const bool column = conversion_ || theme_.layout == PanelLayout::Column;
    const bool underline = theme_.chipShape == ChipShape::Underline;
    const SelectionStyle selectionStyle = theme_.selection;
    const float height = static_cast<float>(m.chipHeight);
    const float radius = m.chipRadius;
    const float line = static_cast<float>(m.line);
    const float half = line / 2;
    const bool selected = index == selection_;
    const D2D1_RECT_F rect = D2D1::RectF(originX + box.left, originY + box.top,
                                         originX + box.left + box.width, originY + box.top + height);

    // 横一列では未選択の候補もチップの背景を持ち、縦積みでは選択中の行だけを強調する
    if (!underline) {
        bool filled = true;
        PanelColor fill = theme_.chip;
        if (selected && selectionStyle == SelectionStyle::Fill) {
            fill = theme_.accent;
        } else if (selected && selectionStyle == SelectionStyle::Soft) {
            fill = theme_.accent;
            fill.a *= kSoftAlpha;
        } else if (column) {
            filled = false;
        }
        if (filled) {
            brush_->SetColor(ToD2D(fill));
            drawTarget_->FillRoundedRectangle(D2D1::RoundedRect(rect, radius, radius), brush_);
        }
        const ItemKind itemKind = items_[index].kind;
        if (!column && (itemKind == ItemKind::NextWord || itemKind == ItemKind::Typo)) {
            const float inner = (std::max)(0.0f, radius - half);
            brush_->SetColor(ToD2D(itemKind == ItemKind::Typo ? theme_.typo : theme_.nextWord));
            drawTarget_->DrawRoundedRectangle(
                D2D1::RoundedRect(D2D1::RectF(rect.left + half, rect.top + half,
                                              rect.right - half, rect.bottom - half),
                                  inner, inner),
                brush_, line);
        }
    }

    // underline の fill・soft は下線の色だけをアクセント色にし、文字は未選択と同じにする
    PanelColor textColor = theme_.text;
    PanelColor badgeColor = theme_.secondary;
    bool bold = false;
    if (selected) {
        if (selectionStyle == SelectionStyle::Text) {
            textColor = theme_.accent;
            bold = true;
        } else if (!underline && selectionStyle == SelectionStyle::Fill) {
            textColor = theme_.selectedText;
            badgeColor = theme_.selectedText;
        } else if (!underline && selectionStyle == SelectionStyle::Soft) {
            textColor = theme_.accent;
            badgeColor = theme_.accent;
        }
    }

    const float centerY = rect.top + height / 2;
    float x = rect.left + m.chipPadding;
    if (theme_.badgeShape != BadgeShape::None) {
        const float badgeRadius = m.badgeSize / 2;
        const float centerX = x + badgeRadius;
        brush_->SetColor(ToD2D(badgeColor));
        if (theme_.badgeShape == BadgeShape::Circle) {
            drawTarget_->DrawEllipse(D2D1::Ellipse(D2D1::Point2F(centerX, centerY),
                                                   badgeRadius - half, badgeRadius - half),
                                     brush_, line);
        } else if (theme_.badgeShape == BadgeShape::Square) {
            const float corner = badgeRadius * 0.35f;
            drawTarget_->DrawRoundedRectangle(
                D2D1::RoundedRect(D2D1::RectF(centerX - badgeRadius + half,
                                              centerY - badgeRadius + half,
                                              centerX + badgeRadius - half,
                                              centerY + badgeRadius - half),
                                  corner, corner),
                brush_, line);
        }
        DrawLabel(std::to_wstring(index + 1), badgeFormat_, centerX - badgeRadius,
                  centerY - badgeRadius, centerX + badgeRadius, centerY + badgeRadius, badgeColor);
        x += m.badgeSlot;
    }

    float textRight = rect.right;
    if (column) {
        textRight = rect.right - m.chipPadding;
        // 変換の候補一覧はほぼすべて同じ種類なので種類のタグを出さない。ただし補正の候補は、
        // 縦に積むので縁では目立たずほかに見分ける方法が無いため、kind_tag によらず出す
        const ItemKind kind = items_[index].kind;
        if (conversion_ ? kind == ItemKind::Typo : theme_.kindTag) {
            const std::wstring tag = KindTagLabel(kind);
            const PanelColor& tagColor = KindTagColor(theme_, kind);
            const float tagWidth = TextWidth(tag, tagFormat_) + m.tagPadding * 2;
            const D2D1_RECT_F tagRect =
                D2D1::RectF(textRight - tagWidth, centerY - m.tagHeight / 2, textRight,
                            centerY + m.tagHeight / 2);
            PanelColor tagFill = tagColor;
            tagFill.a = kTagAlpha;
            const float tagRadius = m.tagHeight * 0.25f;
            brush_->SetColor(ToD2D(tagFill));
            drawTarget_->FillRoundedRectangle(D2D1::RoundedRect(tagRect, tagRadius, tagRadius),
                                              brush_);
            DrawLabel(tag, tagFormat_, tagRect.left, tagRect.top, tagRect.right, tagRect.bottom,
                      tagColor);
            textRight = tagRect.left - m.tagGap;
        }
        // 切り詰めずに収まる行で、幅の丸め誤差から「…」が出ないよう少し余裕を持たせる
        textRight += 1.0f;
    }

    const std::wstring& text = itemTexts_[index];
    IDWriteTextFormat* format = bold ? boldFormat_ : textFormat_;
    textRight = (std::max)(textRight, x);
    const size_t dimFrom = items_[index].dimFrom;
    if (dimFrom < text.size()) {
        // 1 つの文字列として描いた位置のまま、採用範囲の外だけを切り分けて薄くする
        // (分けて描くと、収まらないときの末尾の「…」の位置が変わるため)
        const float split =
            (std::min)(x + TextWidth(text.substr(0, dimFrom), format), textRight);
        PanelColor dimColor = textColor;
        dimColor.a *= kDimTextAlpha;
        drawTarget_->PushAxisAlignedClip(D2D1::RectF(rect.left, rect.top, split, rect.bottom),
                                         D2D1_ANTIALIAS_MODE_ALIASED);
        DrawLabel(text, format, x, rect.top, textRight, rect.bottom, textColor);
        drawTarget_->PopAxisAlignedClip();
        drawTarget_->PushAxisAlignedClip(D2D1::RectF(split, rect.top, rect.right, rect.bottom),
                                         D2D1_ANTIALIAS_MODE_ALIASED);
        DrawLabel(text, format, x, rect.top, textRight, rect.bottom, dimColor);
        drawTarget_->PopAxisAlignedClip();
    } else {
        DrawLabel(text, format, x, rect.top, textRight, rect.bottom, textColor);
    }

    if (underline && selected) {
        const float underlineWidth = (std::min)(TextWidth(text, format), textRight - x);
        const float underlineY = std::round(rect.bottom - theme_.underlinePx - height * 0.12f);
        brush_->SetColor(ToD2D(theme_.accent));
        drawTarget_->FillRectangle(
            D2D1::RectF(x, underlineY, x + underlineWidth, underlineY + theme_.underlinePx),
            brush_);
    }
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
    if (conversion_ || pageStarts_.empty()) {
        selection_ = selection;
        if (hwnd_ != nullptr) {
            Render(L"SetSelection", nullptr);
        }
        return;
    }
    if (hwnd_ == nullptr) {
        return;
    }
    size_t page = 0;
    if (selection < allItems_.size()) {
        page = static_cast<size_t>(
            std::upper_bound(pageStarts_.begin(), pageStarts_.end(), selection) -
            pageStarts_.begin() - 1);
    }
    if (page != page_) {
        LayoutPage(page, selection, L"SetSelection");
        return;
    }
    selection_ = selection >= pageStart_ && selection < pageStart_ + items_.size()
                     ? selection - pageStart_
                     : items_.size();
    pageText_ = PageText(selection);
    Render(L"SetSelection", nullptr);
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
        ReleaseHwndTarget();
        // ウィンドウと一緒にタイマーも破棄される
        destroying_ = true;
        DestroyWindow(hwnd_);
        destroying_ = false;
        hwnd_ = nullptr;
    }
    blurActive_ = false;
    timerCallback_ = nullptr;
    conversion_ = false;
    items_.clear();
    itemTexts_.clear();
    itemBoxes_.clear();
    separator_ = false;
    columnSide_ = ColumnSide::None;
    pageText_.clear();
    pageStart_ = 0;
    selection_ = 0;
    allItems_.clear();
    allTexts_.clear();
    allWidths_.clear();
    pageStarts_.clear();
    page_ = 0;
    hasInput_ = false;
    input_ = InputLine{};
    tail_ = TailSide::None;
}

void SuggestPanel::UseOwner(HWND owner)
{
    if (owner == owner_) {
        return;
    }
    // 作った後にオーナーを付け替える公式の方法は無いため、作ったウィンドウを使い回さずに作り直す
    if (hwnd_ != nullptr) {
        Hide();
    }
    owner_ = owner;
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
        if (self != nullptr && self->hwnd_ == hwnd) {
            self->hwnd_ = nullptr;
            if (!self->destroying_) {
                // オーナーと一緒に OS が破棄したときも、Hide した後と同じ状態にして次の表示で作り直す。
                // ぼかしの描画先はこのウィンドウに結び付いているので先に手放す
                self->ReleaseHwndTarget();
                self->Hide();
            }
        }
        break;
    case WM_MOUSEACTIVATE:
        // クリックされてもフォーカスを奪わない
        return MA_NOACTIVATE;
    case WM_ERASEBKGND:
        if (self != nullptr && self->blurActive_) {
            // ちらつきを避けるため背景は消さない (全面を DrawBlur で描き直す)
            return 1;
        }
        break;
    case WM_PAINT:
        if (self != nullptr && self->blurActive_) {
            PAINTSTRUCT paint = {};
            BeginPaint(hwnd, &paint);
            // 作成中で描画先がまだ無いときは描かずに済ませる (作成後に Render が描く)
            if (self->hwndTarget_ != nullptr) {
                self->DrawBlur();
            }
            EndPaint(hwnd, &paint);
            return 0;
        }
        break;
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
        std::to_wstring(itemTexts_.size()) + L" 入力行=" + (hasInput_ ? L"1" : L"0") +
        L" 方式=" + (blurActive_ ? std::wstring(L"blur(") + BlurTargetTypeName() + L")"
                                 : std::wstring(L"layered"));
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
    if (!blurActive_ && error != ERROR_SUCCESS) {
        message += L" UpdateLayeredWindow 失敗 error=" + std::to_wstring(error);
    }
    logger_(message);
}
