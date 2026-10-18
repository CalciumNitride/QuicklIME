#include "lang_bar.h"

#include <d2d1.h>
#include <dwrite.h>

#include <algorithm>
#include <cmath>
#include <new>
#include <vector>

#include "globals.h"
#include "resource.h"
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
constexpr wchar_t kFallbackFont[] = L"Yu Gothic UI";

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
}

// DLL のリソースに入れたフォント (RCDATA) をそのまま返す stream。
// IDWriteFactory5::CreateInMemoryFontFileLoader は 1703 以降にしか無く、対応 OS の下限 (1607) で
// 使えないため自前で持つ。リソースのメモリはモジュールが解放されるまで有効
class ResourceFontStream : public IDWriteFontFileStream {
public:
    ResourceFontStream(const void* data, UINT64 size) : refCount_(1), data_(data), size_(size) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, __uuidof(IUnknown)) ||
            IsEqualIID(riid, __uuidof(IDWriteFontFileStream))) {
            *ppv = static_cast<IDWriteFontFileStream*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refCount_); }
    STDMETHODIMP_(ULONG) Release() override
    {
        const LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    STDMETHODIMP ReadFileFragment(const void** fragmentStart, UINT64 fileOffset,
                                  UINT64 fragmentSize, void** fragmentContext) override
    {
        if (fragmentStart == nullptr || fragmentContext == nullptr) {
            return E_INVALIDARG;
        }
        *fragmentContext = nullptr;
        if (fileOffset > size_ || fragmentSize > size_ - fileOffset) {
            *fragmentStart = nullptr;
            return E_FAIL;
        }
        *fragmentStart = static_cast<const BYTE*>(data_) + fileOffset;
        return S_OK;
    }
    STDMETHODIMP_(void) ReleaseFileFragment(void* fragmentContext) override
    {
        UNREFERENCED_PARAMETER(fragmentContext);
    }
    STDMETHODIMP GetFileSize(UINT64* fileSize) override
    {
        if (fileSize == nullptr) {
            return E_INVALIDARG;
        }
        *fileSize = size_;
        return S_OK;
    }
    STDMETHODIMP GetLastWriteTime(UINT64* lastWriteTime) override
    {
        // メモリ上のフォントには更新時刻が無い (DirectWrite は E_NOTIMPL を許す)
        if (lastWriteTime != nullptr) {
            *lastWriteTime = 0;
        }
        return E_NOTIMPL;
    }

private:
    ~ResourceFontStream() = default;

    LONG refCount_;
    const void* data_;
    UINT64 size_;
};

// アイコン用のフォントのリソースを stream として返す loader。key は使わない
// (このフォント 1 つしか読まない)
class ResourceFontLoader : public IDWriteFontFileLoader {
public:
    ResourceFontLoader() : refCount_(1) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, __uuidof(IUnknown)) ||
            IsEqualIID(riid, __uuidof(IDWriteFontFileLoader))) {
            *ppv = static_cast<IDWriteFontFileLoader*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refCount_); }
    STDMETHODIMP_(ULONG) Release() override
    {
        const LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    STDMETHODIMP CreateStreamFromKey(const void* key, UINT32 keySize,
                                     IDWriteFontFileStream** stream) override
    {
        UNREFERENCED_PARAMETER(key);
        UNREFERENCED_PARAMETER(keySize);
        if (stream == nullptr) {
            return E_INVALIDARG;
        }
        *stream = nullptr;
        HRSRC info =
            FindResourceW(globals::dllInstance, MAKEINTRESOURCEW(IDR_ICON_FONT), RT_RCDATA);
        if (info == nullptr) {
            return E_FAIL;
        }
        HGLOBAL resource = LoadResource(globals::dllInstance, info);
        const void* data = resource != nullptr ? LockResource(resource) : nullptr;
        const DWORD size = SizeofResource(globals::dllInstance, info);
        if (data == nullptr || size == 0) {
            return E_FAIL;
        }
        *stream = new (std::nothrow) ResourceFontStream(data, size);
        return *stream != nullptr ? S_OK : E_OUTOFMEMORY;
    }

private:
    ~ResourceFontLoader() = default;

    LONG refCount_;
};

IDWriteFontFace* CreateResourceFontFace(IDWriteFactory* factory, IDWriteFontFileLoader* loader)
{
    const UINT32 key = IDR_ICON_FONT;
    IDWriteFontFile* file = nullptr;
    if (FAILED(factory->CreateCustomFontFileReference(&key, sizeof(key), loader, &file))) {
        return nullptr;
    }
    BOOL supported = FALSE;
    DWRITE_FONT_FILE_TYPE fileType = DWRITE_FONT_FILE_TYPE_UNKNOWN;
    DWRITE_FONT_FACE_TYPE faceType = DWRITE_FONT_FACE_TYPE_UNKNOWN;
    UINT32 faceCount = 0;
    IDWriteFontFace* face = nullptr;
    if (SUCCEEDED(file->Analyze(&supported, &fileType, &faceType, &faceCount)) && supported &&
        FAILED(factory->CreateFontFace(faceType, 1, &file, 0, DWRITE_FONT_SIMULATIONS_NONE,
                                       &face))) {
        face = nullptr;
    }
    file->Release();
    return face;
}

IDWriteFontFace* CreateSystemFontFace(IDWriteFactory* factory)
{
    IDWriteFontCollection* fonts = nullptr;
    if (FAILED(factory->GetSystemFontCollection(&fonts, FALSE))) {
        return nullptr;
    }
    IDWriteFontFace* face = nullptr;
    UINT32 index = 0;
    BOOL exists = FALSE;
    IDWriteFontFamily* family = nullptr;
    if (SUCCEEDED(fonts->FindFamilyName(kFallbackFont, &index, &exists)) && exists &&
        SUCCEEDED(fonts->GetFontFamily(index, &family))) {
        IDWriteFont* font = nullptr;
        if (SUCCEEDED(family->GetFirstMatchingFont(DWRITE_FONT_WEIGHT_REGULAR,
                                                   DWRITE_FONT_STRETCH_NORMAL,
                                                   DWRITE_FONT_STYLE_NORMAL, &font))) {
            if (FAILED(font->CreateFontFace(&face))) {
                face = nullptr;
            }
            font->Release();
        }
        family->Release();
    }
    fonts->Release();
    return face;
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

// 字形の外接矩形 (design units。y は上向き)
struct InkBounds {
    float left;
    float top;
    float right;
    float bottom;
};

InkBounds GlyphInkBounds(const DWRITE_GLYPH_METRICS& m)
{
    // verticalOriginY は topSideBearing と字形の上端 (yMax) の和
    const float top = static_cast<float>(m.verticalOriginY - m.topSideBearing);
    const float bottom =
        static_cast<float>(m.verticalOriginY) - static_cast<float>(m.advanceHeight) +
        static_cast<float>(m.bottomSideBearing);
    const float left = static_cast<float>(m.leftSideBearing);
    const float right = static_cast<float>(m.advanceWidth) - static_cast<float>(m.rightSideBearing);
    return {left, top, right, bottom};
}

bool GlyphOf(IDWriteFontFace* face, wchar_t ch, UINT16* index, DWRITE_GLYPH_METRICS* metrics)
{
    const UINT32 codePoint = ch;
    return SUCCEEDED(face->GetGlyphIndices(&codePoint, 1, index)) &&
           SUCCEEDED(face->GetDesignGlyphMetrics(index, 1, metrics, FALSE));
}

} // namespace

LangBarButton::LangBarButton(TextService* service)
    : refCount_(1),
      service_(service),
      sink_(nullptr),
      d2dFactory_(nullptr),
      dwriteFactory_(nullptr),
      fontLoader_(nullptr),
      fontFace_(nullptr),
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
    SafeRelease(&fontFace_);
    if (fontLoader_ != nullptr) {
        dwriteFactory_->UnregisterFontFileLoader(fontLoader_);
        SafeRelease(&fontLoader_);
    }
    SafeRelease(&dwriteFactory_);
    SafeRelease(&d2dFactory_);
    service_->Release();
    globals::DllRelease();
}

bool LangBarButton::EnsureFontFace()
{
    if (fontFace_ != nullptr) {
        return true;
    }
    // 自前の loader を登録するので、プロセスで共有の factory を汚さないよう ISOLATED にする
    if (dwriteFactory_ == nullptr &&
        FAILED(DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED, __uuidof(IDWriteFactory),
                                   reinterpret_cast<IUnknown**>(&dwriteFactory_)))) {
        dwriteFactory_ = nullptr;
        return false;
    }
    if (fontLoader_ == nullptr) {
        auto* loader = new (std::nothrow) ResourceFontLoader();
        if (loader != nullptr) {
            if (SUCCEEDED(dwriteFactory_->RegisterFontFileLoader(loader))) {
                fontLoader_ = loader;
            } else {
                loader->Release();
            }
        }
    }
    if (fontLoader_ != nullptr) {
        fontFace_ = CreateResourceFontFace(dwriteFactory_, fontLoader_);
    }
    if (fontFace_ == nullptr) {
        fontFace_ = CreateSystemFontFace(dwriteFactory_);
    }
    return fontFace_ != nullptr;
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
    if (!EnsureFontFace() || !EnsureTarget()) {
        return nullptr;
    }
    // 文字サイズは「あ」の字形で決め、「A」「×」も同じ文字サイズで描く
    UINT16 referenceIndex = 0;
    DWRITE_GLYPH_METRICS referenceMetrics = {};
    UINT16 index = 0;
    DWRITE_GLYPH_METRICS metrics = {};
    if (!GlyphOf(fontFace_, kOpenChar, &referenceIndex, &referenceMetrics) ||
        !GlyphOf(fontFace_, ch, &index, &metrics)) {
        return nullptr;
    }
    const InkBounds reference = GlyphInkBounds(referenceMetrics);
    const float referenceLong =
        (std::max)(reference.right - reference.left, reference.top - reference.bottom);
    if (referenceLong <= 0.0f) {
        return nullptr;
    }

    HDC screen = GetDC(nullptr);
    if (screen == nullptr) {
        return nullptr;
    }
    const int size = MulDiv(16, GetDeviceCaps(screen, LOGPIXELSY), 96);

    DWRITE_FONT_METRICS fontMetrics = {};
    fontFace_->GetMetrics(&fontMetrics);
    const float emSize = static_cast<float>(size - 2) *
                         static_cast<float>(fontMetrics.designUnitsPerEm) / referenceLong;
    const float scale = emSize / static_cast<float>(fontMetrics.designUnitsPerEm);
    const InkBounds ink = GlyphInkBounds(metrics);
    const float half = static_cast<float>(size) / 2.0f;
    // 線をぼかさないよう、描く原点は整数 px に丸める
    const float originX = std::round(half - (ink.left + ink.right) / 2.0f * scale);
    const float originY = std::round(half + (ink.top + ink.bottom) / 2.0f * scale);

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
            const FLOAT advance = static_cast<FLOAT>(metrics.advanceWidth) * scale;
            DWRITE_GLYPH_RUN run = {};
            run.fontFace = fontFace_;
            run.fontEmSize = emSize;
            run.glyphCount = 1;
            run.glyphIndices = &index;
            run.glyphAdvances = &advance;
            target_->BeginDraw();
            target_->Clear(D2D1::ColorF(0, 0, 0, 0));
            brush_->SetColor(D2D1::ColorF(c, c, c, 1.0f));
            target_->DrawGlyphRun(D2D1::Point2F(originX, originY), &run, brush_);
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
