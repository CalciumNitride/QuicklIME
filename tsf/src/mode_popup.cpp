#include "mode_popup.h"

#include <d2d1.h>
#include <dwrite.h>
#include <shellscalingapi.h>

#include <algorithm>
#include <cmath>

#include "globals.h"

namespace {

constexpr wchar_t kWindowClassName[] = L"QuicklIMEModePopup";
constexpr UINT_PTR kHideTimerId = 1;
constexpr UINT kHideMs = 1000;
// 四角の一辺 (96dpi での px)
constexpr int kSide96 = 28;
// 「あ」の字形の外接矩形の長辺の、四角の一辺に対する比
constexpr float kInkRatio = 0.45f;

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
}

bool EnsureWindowClass()
{
    static bool registered = false;
    if (registered) {
        return true;
    }
    WNDCLASSEXW wc = {};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = ModePopup::WndProc;
    wc.hInstance = globals::dllInstance;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.lpszClassName = kWindowClassName;
    if (RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return false;
    }
    registered = true;
    return true;
}

// モニタの DPI。GetDpiForMonitor は shcore.dll にあり、リンクせずに探して使う
// (見つからない・失敗したときは画面の DC の DPI)
UINT MonitorDpi(HMONITOR monitor, HDC screen)
{
    using GetDpiForMonitorFn = HRESULT(WINAPI*)(HMONITOR, MONITOR_DPI_TYPE, UINT*, UINT*);
    static GetDpiForMonitorFn getDpi = []() -> GetDpiForMonitorFn {
        HMODULE shcore = LoadLibraryExW(L"shcore.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
        return shcore != nullptr ? reinterpret_cast<GetDpiForMonitorFn>(
                                       GetProcAddress(shcore, "GetDpiForMonitor"))
                                 : nullptr;
    }();
    UINT dpiX = 0;
    UINT dpiY = 0;
    if (getDpi != nullptr && monitor != nullptr &&
        SUCCEEDED(getDpi(monitor, MDT_EFFECTIVE_DPI, &dpiX, &dpiY)) && dpiY != 0) {
        return dpiY;
    }
    const int dpi = GetDeviceCaps(screen, LOGPIXELSY);
    return dpi > 0 ? static_cast<UINT>(dpi) : 96;
}

D2D1_COLOR_F ToD2D(const PanelColor& color)
{
    return D2D1::ColorF(color.r, color.g, color.b, color.a);
}

} // namespace

ModePopup::ModePopup() : hwnd_(nullptr), factory_(nullptr), target_(nullptr), brush_(nullptr)
{
}

ModePopup::~ModePopup()
{
    Hide();
    ReleaseTarget();
    SafeRelease(&factory_);
}

bool ModePopup::EnsureTarget()
{
    if (target_ != nullptr) {
        return true;
    }
    // TextService はスレッドごとに1インスタンスで、一時表示も同じスレッドからしか触らない
    if (factory_ == nullptr &&
        FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory_))) {
        factory_ = nullptr;
        return false;
    }
    // 候補パネルと同じく、ホストのプロセスに D3D のデバイスを作らないようソフトウェアで描き、
    // DPI は 96 に固定して DIP を px として扱う
    const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), 96.0f,
        96.0f);
    if (FAILED(factory_->CreateDCRenderTarget(&properties, &target_))) {
        target_ = nullptr;
        return false;
    }
    // layered window で透明な背景ごと合成するので、ClearType の色のにじみを出さない
    target_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    if (FAILED(target_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0, 1), &brush_))) {
        ReleaseTarget();
        return false;
    }
    return true;
}

void ModePopup::ReleaseTarget()
{
    SafeRelease(&brush_);
    SafeRelease(&target_);
}

bool ModePopup::Show(const RECT& caret, bool open, const PanelTheme& theme, int fontSize)
{
    IDWriteFontFace* face = font_.Face();
    if (face == nullptr || !EnsureWindowClass() || !EnsureTarget()) {
        Hide();
        return false;
    }
    HDC screen = GetDC(nullptr);
    if (screen == nullptr) {
        Hide();
        return false;
    }
    HMONITOR monitor = MonitorFromRect(&caret, MONITOR_DEFAULTTONEAREST);
    const int side = MulDiv(kSide96, static_cast<int>(MonitorDpi(monitor, screen)), 96);
    const float sideF = static_cast<float>(side);
    IconGlyph glyph;
    if (side < 1 || !PlaceIconGlyph(face, open ? L'あ' : L'A', sideF * kInkRatio, sideF, &glyph)) {
        ReleaseDC(nullptr, screen);
        Hide();
        return false;
    }

    // キャレット行のすぐ下に置き、作業領域の下端からはみ出すときはキャレット行の上に出す。
    // 左右は作業領域に収める
    int x = static_cast<int>(caret.left);
    int y = static_cast<int>(caret.bottom);
    MONITORINFO monitorInfo = {};
    monitorInfo.cbSize = sizeof(monitorInfo);
    if (monitor != nullptr && GetMonitorInfoW(monitor, &monitorInfo)) {
        const RECT& work = monitorInfo.rcWork;
        if (y + side > work.bottom) {
            y = static_cast<int>(caret.top) - side;
        }
        x = (std::max)(static_cast<int>(work.left),
                       (std::min)(x, static_cast<int>(work.right) - side));
    }

    HDC memory = CreateCompatibleDC(screen);
    BITMAPINFO info = {};
    info.bmiHeader.biSize = sizeof(info.bmiHeader);
    info.bmiHeader.biWidth = side;
    info.bmiHeader.biHeight = -side;  // 上から下へ並ぶ DIB
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
        const RECT rect = {0, 0, side, side};
        hr = target_->BindDC(memory, &rect);
        if (SUCCEEDED(hr)) {
            const float radius = (std::min)(theme.panelRadius * static_cast<float>(fontSize),
                                            sideF / 2.0f);
            const float line =
                static_cast<float>((std::max)(1, static_cast<int>(std::lround(theme.lineWidthPx))));
            const float half = line / 2.0f;
            const D2D1_RECT_F body = D2D1::RectF(0.0f, 0.0f, sideF, sideF);
            const D2D1_RECT_F inset = D2D1::RectF(half, half, sideF - half, sideF - half);
            const FLOAT advance = glyph.advance;
            DWRITE_GLYPH_RUN run = {};
            run.fontFace = face;
            run.fontEmSize = glyph.emSize;
            run.glyphCount = 1;
            run.glyphIndices = &glyph.index;
            run.glyphAdvances = &advance;
            target_->BeginDraw();
            target_->Clear(D2D1::ColorF(0, 0, 0, 0));
            brush_->SetColor(ToD2D(theme.background));
            target_->FillRoundedRectangle(D2D1::RoundedRect(body, radius, radius), brush_);
            if (theme.drawBorder) {
                const float insetRadius = (std::max)(0.0f, radius - half);
                brush_->SetColor(ToD2D(theme.border));
                target_->DrawRoundedRectangle(D2D1::RoundedRect(inset, insetRadius, insetRadius),
                                              brush_, line);
            }
            brush_->SetColor(ToD2D(theme.text));
            target_->DrawGlyphRun(D2D1::Point2F(glyph.originX, glyph.originY), &run, brush_);
            hr = target_->EndDraw();
        }
        if (hr == D2DERR_RECREATE_TARGET) {
            // 次の表示で作り直す
            ReleaseTarget();
        }
    }

    bool updated = false;
    if (SUCCEEDED(hr)) {
        if (hwnd_ == nullptr) {
            hwnd_ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW |
                                        WS_EX_LAYERED | WS_EX_TRANSPARENT,
                                    kWindowClassName, L"", WS_POPUP, x, y, side, side, nullptr,
                                    nullptr, globals::dllInstance, nullptr);
            if (hwnd_ != nullptr) {
                SetWindowLongPtrW(hwnd_, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(this));
            }
        }
        if (hwnd_ != nullptr) {
            POINT destination = {x, y};
            SIZE size = {side, side};
            POINT source = {0, 0};
            BLENDFUNCTION blend = {AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
            updated = UpdateLayeredWindow(hwnd_, screen, &destination, &size, memory, &source, 0,
                                          &blend, ULW_ALPHA) != FALSE;
            SetWindowPos(hwnd_, HWND_TOPMOST, 0, 0, 0, 0,
                         SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            ShowWindow(hwnd_, SW_SHOWNA);
            // 同じ ID で設定し直すと、表示中でも 1 秒を数え直す
            updated = updated && SetTimer(hwnd_, kHideTimerId, kHideMs, nullptr) != 0;
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

    if (!updated) {
        Hide();
        return false;
    }
    return true;
}

void ModePopup::Hide()
{
    if (hwnd_ != nullptr) {
        KillTimer(hwnd_, kHideTimerId);
        DestroyWindow(hwnd_);
        hwnd_ = nullptr;
    }
}

LRESULT CALLBACK ModePopup::WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    if (msg == WM_MOUSEACTIVATE) {
        return MA_NOACTIVATE;
    }
    if (msg == WM_TIMER && wparam == kHideTimerId) {
        auto* popup = reinterpret_cast<ModePopup*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        if (popup != nullptr && popup->hwnd_ == hwnd) {
            popup->Hide();
        } else {
            KillTimer(hwnd, kHideTimerId);
        }
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}
