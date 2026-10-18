#include "lang_bar.h"

#include <d2d1.h>
#include <dwrite.h>

#include <vector>

#include "globals.h"
#include "icon_font.h"
#include "text_service.h"

namespace {

// AdviseSink が1本しか付かない前提の固定 cookie (CorvusSKK と同じ方式)
constexpr DWORD kSinkCookie = 0x714c4d45; // 'qLME'

// 右クリックメニューの項目ID
constexpr UINT kMenuIdConfig = 1;
constexpr UINT kMenuIdRegisterWord = 2;

constexpr wchar_t kOpenChar = L'あ';
constexpr wchar_t kClosedChar = L'A';
constexpr wchar_t kDisabledChar = L'×';  // ×

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
}

// タスクバーのテーマがライトか。値が読めなければダーク扱い
bool IsLightTaskbar()
{
    DWORD value = 0;
    DWORD size = sizeof(value);
    const LSTATUS status = RegGetValueW(
        HKEY_CURRENT_USER, L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        L"SystemUsesLightTheme", RRF_RT_REG_DWORD, nullptr, &value, &size);
    return status == ERROR_SUCCESS && value == 1;
}

} // namespace

LangBarButton::LangBarButton(TextService* service)
    : refCount_(1),
      service_(service),
      sink_(nullptr),
      d2dFactory_(nullptr),
      target_(nullptr),
      brush_(nullptr)
{
    globals::DllAddRef();
    service_->AddRef();
}

LangBarButton::~LangBarButton()
{
    if (sink_ != nullptr) {
        sink_->Release();
        sink_ = nullptr;
    }
    ReleaseTarget();
    SafeRelease(&d2dFactory_);
    service_->Release();
    globals::DllRelease();
}

bool LangBarButton::EnsureTarget()
{
    if (target_ != nullptr) {
        return true;
    }
    if (d2dFactory_ == nullptr &&
        FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2dFactory_))) {
        d2dFactory_ = nullptr;
        return false;
    }
    // 候補パネルと同じく、ホストのプロセスに D3D のデバイスを作らないようソフトウェアで描き、
    // DPI は 96 に固定して DIP を px として扱う
    const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_PREMULTIPLIED), 96.0f,
        96.0f);
    if (FAILED(d2dFactory_->CreateDCRenderTarget(&properties, &target_))) {
        target_ = nullptr;
        return false;
    }
    // アイコンは描いた後でタスクバーの背景に合成されるので、ClearType の色のにじみを出さない
    target_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    if (FAILED(target_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0, 1), &brush_))) {
        ReleaseTarget();
        return false;
    }
    return true;
}

void LangBarButton::ReleaseTarget()
{
    SafeRelease(&brush_);
    SafeRelease(&target_);
}

HICON LangBarButton::CreateModeIcon(wchar_t ch)
{
    IDWriteFontFace* face = font_.Face();
    if (face == nullptr || !EnsureTarget()) {
        return nullptr;
    }
    HDC screen = GetDC(nullptr);
    if (screen == nullptr) {
        return nullptr;
    }
    const int size = MulDiv(16, GetDeviceCaps(screen, LOGPIXELSY), 96);
    IconGlyph glyph;
    if (!PlaceIconGlyph(face, ch, static_cast<float>(size - 2), static_cast<float>(size),
                        &glyph)) {
        ReleaseDC(nullptr, screen);
        return nullptr;
    }

    HDC memory = CreateCompatibleDC(screen);
    BITMAPINFO info = {};
    info.bmiHeader.biSize = sizeof(info.bmiHeader);
    info.bmiHeader.biWidth = size;
    info.bmiHeader.biHeight = -size;  // 上から下へ並ぶ DIB
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    void* bits = nullptr;
    HBITMAP color = memory != nullptr
                        ? CreateDIBSection(memory, &info, DIB_RGB_COLORS, &bits, nullptr, 0)
                        : nullptr;
    HGDIOBJ oldBitmap = color != nullptr ? SelectObject(memory, color) : nullptr;

    const bool light = IsLightTaskbar();
    HRESULT hr = E_FAIL;
    if (color != nullptr) {
        const RECT rect = {0, 0, size, size};
        hr = target_->BindDC(memory, &rect);
        if (SUCCEEDED(hr)) {
            const float c = light ? 0.0f : 1.0f;
            const FLOAT advance = glyph.advance;
            DWRITE_GLYPH_RUN run = {};
            run.fontFace = face;
            run.fontEmSize = glyph.emSize;
            run.glyphCount = 1;
            run.glyphIndices = &glyph.index;
            run.glyphAdvances = &advance;
            target_->BeginDraw();
            target_->Clear(D2D1::ColorF(0, 0, 0, 0));
            brush_->SetColor(D2D1::ColorF(c, c, c, 1.0f));
            target_->DrawGlyphRun(D2D1::Point2F(glyph.originX, glyph.originY), &run, brush_);
            hr = target_->EndDraw();
        }
        if (hr == D2DERR_RECREATE_TARGET) {
            // 次のアイコンで作り直す
            ReleaseTarget();
        }
    }
    if (oldBitmap != nullptr) {
        SelectObject(memory, oldBitmap);
    }

    HICON icon = nullptr;
    if (SUCCEEDED(hr)) {
        GdiFlush();
        // アイコンの color は straight alpha。文字色は単色なので、RGB を文字色に揃えて被覆率だけを残す
        const DWORD rgb = light ? 0x000000 : 0xFFFFFF;
        auto* pixels = static_cast<DWORD*>(bits);
        for (int i = 0; i < size * size; ++i) {
            const DWORD alpha = pixels[i] >> 24;
            pixels[i] = alpha != 0 ? (alpha << 24) | rgb : 0;
        }
        // 32bpp の alpha があれば mask は使われないが、未初期化の内容は渡さない
        const int maskStride = ((size + 15) / 16) * 2;
        std::vector<BYTE> maskBits(static_cast<size_t>(maskStride) * size, 0);
        HBITMAP mask = CreateBitmap(size, size, 1, 1, maskBits.data());
        if (mask != nullptr) {
            ICONINFO iconInfo = {};
            iconInfo.fIcon = TRUE;
            iconInfo.hbmMask = mask;
            iconInfo.hbmColor = color;
            icon = CreateIconIndirect(&iconInfo);
            DeleteObject(mask);
        }
    }
    if (color != nullptr) {
        DeleteObject(color);
    }
    if (memory != nullptr) {
        DeleteDC(memory);
    }
    ReleaseDC(nullptr, screen);
    return icon;
}

// ---- IUnknown ----

STDMETHODIMP LangBarButton::QueryInterface(REFIID riid, void** ppv)
{
    if (ppv == nullptr) {
        return E_INVALIDARG;
    }
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfLangBarItem) ||
        IsEqualIID(riid, IID_ITfLangBarItemButton)) {
        *ppv = static_cast<ITfLangBarItemButton*>(this);
    } else if (IsEqualIID(riid, IID_ITfSource)) {
        *ppv = static_cast<ITfSource*>(this);
    } else {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
}

STDMETHODIMP_(ULONG) LangBarButton::AddRef()
{
    return InterlockedIncrement(&refCount_);
}

STDMETHODIMP_(ULONG) LangBarButton::Release()
{
    LONG count = InterlockedDecrement(&refCount_);
    if (count == 0) {
        delete this;
    }
    return count;
}

// ---- ITfLangBarItem ----

STDMETHODIMP LangBarButton::GetInfo(TF_LANGBARITEMINFO* info)
{
    if (info == nullptr) {
        return E_INVALIDARG;
    }
    info->clsidService = globals::kClsid;
    // GUID_LBI_INPUTMODE の項目はタスクバーの入力インジケータに対応する (Win8 以降)
    info->guidItem = GUID_LBI_INPUTMODE;
    info->dwStyle = TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY;
    info->ulSort = 0;
    wcsncpy_s(info->szDescription, globals::kDescription, _TRUNCATE);
    return S_OK;
}

STDMETHODIMP LangBarButton::GetStatus(DWORD* status)
{
    if (status == nullptr) {
        return E_INVALIDARG;
    }
    *status = 0;
    return S_OK;
}

STDMETHODIMP LangBarButton::Show(BOOL show)
{
    UNREFERENCED_PARAMETER(show);
    if (sink_ == nullptr) {
        return E_FAIL;
    }
    return sink_->OnUpdate(TF_LBI_STATUS);
}

STDMETHODIMP LangBarButton::GetTooltipString(BSTR* tooltip)
{
    if (tooltip == nullptr) {
        return E_INVALIDARG;
    }
    *tooltip = SysAllocString(globals::kDescription);
    return *tooltip != nullptr ? S_OK : E_OUTOFMEMORY;
}

// ---- ITfLangBarItemButton ----

STDMETHODIMP LangBarButton::OnClick(TfLBIClick click, POINT pt, const RECT* area)
{
    if (click == TF_LBI_CLK_LEFT) {
        // 入力欄でない間はアイコンが「×」のままで、切り替えても状態が見えないまま変わるため何もしない
        if (service_->IsKeyboardDisabled()) {
            return S_OK;
        }
        // 左クリック: IME オン/オフのトグル。
        // オフ時の入力途中文字列の確定は OPENCLOSE compartment の OnChange 側で行われる
        service_->SetKeyboardOpen(!service_->IsKeyboardOpen());
        return S_OK;
    }
    if (click != TF_LBI_CLK_RIGHT) {
        return S_OK;
    }
    // 右クリック: 設定メニューをポップアップ表示する (CorvusSKK と同じ方式)
    HMENU menu = CreatePopupMenu();
    if (menu == nullptr) {
        return S_OK;
    }
    AppendMenuW(menu, MF_STRING, kMenuIdConfig, L"設定...");
    AppendMenuW(menu, MF_STRING, kMenuIdRegisterWord, L"単語登録...");
    TPMPARAMS tpm = {};
    TPMPARAMS* tpmPtr = nullptr;
    if (area != nullptr) {
        tpm.cbSize = sizeof(tpm);
        tpm.rcExclude = *area;
        tpmPtr = &tpm;
    }
    const BOOL id = TrackPopupMenuEx(
        menu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_NONOTIFY | TPM_RETURNCMD | TPM_LEFTBUTTON,
        pt.x, pt.y, GetFocus(), tpmPtr);
    DestroyMenu(menu);
    return OnMenuSelect(static_cast<UINT>(id));
}

STDMETHODIMP LangBarButton::InitMenu(ITfMenu* menu)
{
    // BTN_BUTTON スタイルでは通常呼ばれないが、メニュー扱いされた場合に備えて実装する
    if (menu == nullptr) {
        return E_INVALIDARG;
    }
    menu->AddMenuItem(kMenuIdConfig, 0, nullptr, nullptr, L"設定...", 6, nullptr);
    menu->AddMenuItem(kMenuIdRegisterWord, 0, nullptr, nullptr, L"単語登録...", 8, nullptr);
    return S_OK;
}

STDMETHODIMP LangBarButton::OnMenuSelect(UINT id)
{
    switch (id) {
    case kMenuIdConfig:
        return service_->LaunchConfigTool();
    case kMenuIdRegisterWord:
        return service_->LaunchWordRegister(nullptr);
    default:
        return S_OK; // メニューのキャンセル (id=0) を含む
    }
}

STDMETHODIMP LangBarButton::GetIcon(HICON* icon)
{
    if (icon == nullptr) {
        return E_INVALIDARG;
    }
    // 入力欄でないときは、オン/オフより「×」を優先する
    const wchar_t ch = service_->IsKeyboardDisabled() ? kDisabledChar
                       : service_->IsKeyboardOpen()   ? kOpenChar
                                                      : kClosedChar;
    *icon = CreateModeIcon(ch);
    return *icon != nullptr ? S_OK : E_FAIL;
}

STDMETHODIMP LangBarButton::GetText(BSTR* text)
{
    if (text == nullptr) {
        return E_INVALIDARG;
    }
    *text = SysAllocString(globals::kDescription);
    return *text != nullptr ? S_OK : E_OUTOFMEMORY;
}

// ---- ITfSource ----

STDMETHODIMP LangBarButton::AdviseSink(REFIID riid, IUnknown* unknown, DWORD* cookie)
{
    if (unknown == nullptr || cookie == nullptr) {
        return E_INVALIDARG;
    }
    if (!IsEqualIID(riid, IID_ITfLangBarItemSink)) {
        return CONNECT_E_CANNOTCONNECT;
    }
    if (sink_ != nullptr) {
        return CONNECT_E_ADVISELIMIT;
    }
    if (FAILED(unknown->QueryInterface(IID_ITfLangBarItemSink,
                                       reinterpret_cast<void**>(&sink_)))) {
        sink_ = nullptr;
        return E_NOINTERFACE;
    }
    *cookie = kSinkCookie;
    return S_OK;
}

STDMETHODIMP LangBarButton::UnadviseSink(DWORD cookie)
{
    if (cookie != kSinkCookie || sink_ == nullptr) {
        return CONNECT_E_NOCONNECTION;
    }
    sink_->Release();
    sink_ = nullptr;
    return S_OK;
}

void LangBarButton::NotifyUpdate()
{
    if (sink_ != nullptr) {
        sink_->OnUpdate(TF_LBI_ICON);
    }
}
