#include "caret_marker.h"

#include <d2d1.h>

#include <cmath>

#include "globals.h"

namespace {

constexpr wchar_t kWindowClassName[] = L"QuicklIMECaretMarker";
// 斜辺のアンチエイリアスが切れないよう、三角の左右と下に取る余白 (px)
constexpr int kPad = 1;

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
    wc.lpfnWndProc = CaretMarker::WndProc;
    wc.hInstance = globals::dllInstance;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.lpszClassName = kWindowClassName;
    if (RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return false;
    }
    registered = true;
    return true;
}

} // namespace

CaretMarker::CaretMarker()
    : hwnd_(nullptr), owner_(nullptr), factory_(nullptr), target_(nullptr), brush_(nullptr)
{
}

CaretMarker::~CaretMarker()
{
    Hide();
    ReleaseTarget();
    SafeRelease(&factory_);
}

bool CaretMarker::EnsureTarget()
{
    if (target_ != nullptr) {
        return true;
    }
    // TextService はスレッドごとに1インスタンスで、印も同じスレッドからしか触らない
    if (factory_ == nullptr &&
        FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &factory_))) {
        factory_ = nullptr;
        return false;
    }
    // 入力パネルと同じく、ホストのプロセスに D3D のデバイスを作らないようソフトウェアで描き、
    // DPI は 96 に固定して DIP を px として扱う
    const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), 96.0f,
        96.0f);
    if (FAILED(factory_->CreateDCRenderTarget(&properties, &target_))) {
        target_ = nullptr;
        return false;
    }
    if (FAILED(target_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0, 1), &brush_))) {
        ReleaseTarget();
        return false;
    }
    return true;
}

void CaretMarker::ReleaseTarget()
{
    SafeRelease(&brush_);
    SafeRelease(&target_);
}

bool CaretMarker::Show(const RECT& caret, int height, const PanelColor& color, HWND owner)
{
    if (height < 1 || !EnsureWindowClass() || !EnsureTarget()) {
        Hide();
        return false;
    }
    // 正三角形の底辺の半分 (高さ / √3)
    const float halfBase = static_cast<float>(height) / 1.7320508f;
    const int halfWidth = static_cast<int>(std::ceil(halfBase));
    const int windowWidth = halfWidth * 2 + kPad * 2;
    const int windowHeight = height + kPad;
    const float tipX = static_cast<float>(halfWidth + kPad);
    const float baseY = static_cast<float>(height);

    ID2D1PathGeometry* triangle = nullptr;
    if (FAILED(factory_->CreatePathGeometry(&triangle))) {
        Hide();
        return false;
    }
    ID2D1GeometrySink* sink = nullptr;
    if (SUCCEEDED(triangle->Open(&sink))) {
        sink->BeginFigure(D2D1::Point2F(tipX, 0.0f), D2D1_FIGURE_BEGIN_FILLED);
        sink->AddLine(D2D1::Point2F(tipX + halfBase, baseY));
        sink->AddLine(D2D1::Point2F(tipX - halfBase, baseY));
        sink->EndFigure(D2D1_FIGURE_END_CLOSED);
        sink->Close();
        sink->Release();
    }

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
        hr = target_->BindDC(memory, &rect);
        if (SUCCEEDED(hr)) {
            target_->BeginDraw();
            target_->Clear(D2D1::ColorF(0, 0, 0, 0));
            brush_->SetColor(D2D1::ColorF(color.r, color.g, color.b, color.a));
            target_->FillGeometry(triangle, brush_);
            hr = target_->EndDraw();
        }
        if (hr == D2DERR_RECREATE_TARGET) {
            // 次の表示で作り直す
            ReleaseTarget();
        }
    }
    triangle->Release();

    bool updated = false;
    if (SUCCEEDED(hr)) {
        const int x = static_cast<int>(caret.left) - halfWidth - kPad;
        const int y = static_cast<int>(caret.bottom);
        // 作った後にオーナーを付け替える公式の方法は無いため作り直す
        if (hwnd_ != nullptr && owner != owner_) {
            Hide();
        }
        if (hwnd_ == nullptr) {
            hwnd_ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW |
                                        WS_EX_LAYERED | WS_EX_TRANSPARENT,
                                    kWindowClassName, L"", WS_POPUP, x, y, windowWidth,
                                    windowHeight, owner, nullptr, globals::dllInstance, nullptr);
            if (hwnd_ != nullptr) {
                owner_ = owner;
                SetWindowLongPtrW(hwnd_, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(this));
            }
        }
        if (hwnd_ != nullptr) {
            POINT destination = {x, y};
            SIZE size = {windowWidth, windowHeight};
            POINT source = {0, 0};
            BLENDFUNCTION blend = {AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
            updated = UpdateLayeredWindow(hwnd_, screen, &destination, &size, memory, &source, 0,
                                          &blend, ULW_ALPHA) != FALSE;
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

    if (!updated) {
        Hide();
        return false;
    }
    return true;
}

void CaretMarker::Hide()
{
    if (hwnd_ != nullptr) {
        DestroyWindow(hwnd_);
        hwnd_ = nullptr;
    }
}

LRESULT CALLBACK CaretMarker::WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    if (msg == WM_MOUSEACTIVATE) {
        return MA_NOACTIVATE;
    }
    if (msg == WM_DESTROY) {
        // オーナーと一緒に OS が破棄したときも、無効になったハンドルを使わず次の表示で作り直す
        auto* marker = reinterpret_cast<CaretMarker*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        if (marker != nullptr && marker->hwnd_ == hwnd) {
            marker->hwnd_ = nullptr;
        }
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}
