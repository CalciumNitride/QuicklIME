#include "lang_bar.h"

#include "globals.h"
#include "text_service.h"

namespace {

// AdviseSink が1本しか付かない前提の固定 cookie (CorvusSKK と同じ方式)
constexpr DWORD kSinkCookie = 0x714c4d45; // 'qLME'

// 右クリックメニューの項目ID
constexpr UINT kMenuIdConfig = 1;
constexpr UINT kMenuIdRegisterWord = 2;

// 入力モードの文字アイコンを動的に作る (IME ON → 「あ」、OFF → 「A」)。
// 返した HICON は呼び出し側 (TSF) が破棄する
HICON CreateModeIcon(bool open)
{
    HDC screen = GetDC(nullptr);
    if (screen == nullptr) {
        return nullptr;
    }
    const int size = MulDiv(16, GetDeviceCaps(screen, LOGPIXELSY), 96);

    HICON icon = nullptr;
    HDC memDc = CreateCompatibleDC(screen);
    HBITMAP color = CreateCompatibleBitmap(screen, size, size);
    HBITMAP mask = CreateBitmap(size, size, 1, 1, nullptr);
    if (memDc != nullptr && color != nullptr && mask != nullptr) {
        HGDIOBJ oldBitmap = SelectObject(memDc, color);
        RECT rect = {0, 0, size, size};
        FillRect(memDc, &rect, static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));

        const wchar_t* text = open ? L"あ" : L"A";
        BYTE charset = open ? SHIFTJIS_CHARSET : DEFAULT_CHARSET;
        HFONT font = CreateFontW(-(size - 2), 0, 0, 0, FW_BOLD, FALSE, FALSE, FALSE,
                                 charset, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                 CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE, L"Yu Gothic UI");
        HGDIOBJ oldFont = SelectObject(memDc, font);
        SetBkMode(memDc, TRANSPARENT);
        SetTextColor(memDc, RGB(255, 255, 255));
        DrawTextW(memDc, text, 1, &rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(memDc, oldFont);
        if (font != nullptr) {
            DeleteObject(font);
        }
        SelectObject(memDc, oldBitmap);

        ICONINFO info = {};
        info.fIcon = TRUE;
        info.hbmMask = mask;
        info.hbmColor = color;
        icon = CreateIconIndirect(&info);
    }
    if (mask != nullptr) {
        DeleteObject(mask);
    }
    if (color != nullptr) {
        DeleteObject(color);
    }
    if (memDc != nullptr) {
        DeleteDC(memDc);
    }
    ReleaseDC(nullptr, screen);
    return icon;
}

} // namespace

LangBarButton::LangBarButton(TextService* service)
    : refCount_(1), service_(service), sink_(nullptr)
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
    service_->Release();
    globals::DllRelease();
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
    *icon = CreateModeIcon(service_->IsKeyboardOpen());
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
