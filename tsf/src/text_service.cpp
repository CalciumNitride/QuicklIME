#include "text_service.h"

#include <new>

#include "globals.h"

namespace {

// 確定文字列をカーソル位置へ挿入する edit session。
// ドキュメントの編集は必ず edit session の中 (DoEditSession) で行う必要がある。
class InsertTextEditSession : public ITfEditSession {
public:
    InsertTextEditSession(ITfContext* context, std::wstring text)
        : refCount_(1), context_(context), text_(std::move(text))
    {
        context_->AddRef();
    }

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfEditSession)) {
            *ppv = static_cast<ITfEditSession*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    STDMETHODIMP_(ULONG) AddRef() override
    {
        return InterlockedIncrement(&refCount_);
    }

    STDMETHODIMP_(ULONG) Release() override
    {
        LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    // ITfEditSession
    STDMETHODIMP DoEditSession(TfEditCookie ec) override
    {
        ITfInsertAtSelection* insertAtSelection = nullptr;
        HRESULT hr = context_->QueryInterface(IID_ITfInsertAtSelection,
                                              reinterpret_cast<void**>(&insertAtSelection));
        if (FAILED(hr)) {
            return hr;
        }

        ITfRange* range = nullptr;
        hr = insertAtSelection->InsertTextAtSelection(
            ec, 0, text_.c_str(), static_cast<LONG>(text_.size()), &range);
        insertAtSelection->Release();
        if (FAILED(hr)) {
            return hr;
        }

        // カーソルを挿入した文字列の直後へ移動する
        range->Collapse(ec, TF_ANCHOR_END);
        TF_SELECTION selection = {};
        selection.range = range;
        selection.style.ase = TF_AE_NONE;
        selection.style.fInterimChar = FALSE;
        context_->SetSelection(ec, 1, &selection);
        range->Release();
        return S_OK;
    }

private:
    ~InsertTextEditSession()
    {
        context_->Release();
    }

    LONG refCount_;
    ITfContext* context_;
    std::wstring text_;
};

} // namespace

TextService::TextService()
    : refCount_(1), threadMgr_(nullptr), clientId_(TF_CLIENTID_NULL)
{
    globals::DllAddRef();
}

TextService::~TextService()
{
    globals::DllRelease();
}

// ---- IUnknown ----

STDMETHODIMP TextService::QueryInterface(REFIID riid, void** ppv)
{
    if (ppv == nullptr) {
        return E_INVALIDARG;
    }
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfTextInputProcessor) ||
        IsEqualIID(riid, IID_ITfTextInputProcessorEx)) {
        *ppv = static_cast<ITfTextInputProcessorEx*>(this);
    } else if (IsEqualIID(riid, IID_ITfKeyEventSink)) {
        *ppv = static_cast<ITfKeyEventSink*>(this);
    } else {
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
}

STDMETHODIMP_(ULONG) TextService::AddRef()
{
    return InterlockedIncrement(&refCount_);
}

STDMETHODIMP_(ULONG) TextService::Release()
{
    LONG count = InterlockedDecrement(&refCount_);
    if (count == 0) {
        delete this;
    }
    return count;
}

// ---- ITfTextInputProcessor(Ex) ----

STDMETHODIMP TextService::Activate(ITfThreadMgr* threadMgr, TfClientId clientId)
{
    return ActivateEx(threadMgr, clientId, 0);
}

STDMETHODIMP TextService::ActivateEx(ITfThreadMgr* threadMgr, TfClientId clientId, DWORD flags)
{
    UNREFERENCED_PARAMETER(flags);

    if (threadMgr == nullptr) {
        return E_INVALIDARG;
    }

    threadMgr_ = threadMgr;
    threadMgr_->AddRef();
    clientId_ = clientId;

    // キーイベントを受け取るために key event sink を登録する
    ITfKeystrokeMgr* keystrokeMgr = nullptr;
    HRESULT hr = threadMgr_->QueryInterface(IID_ITfKeystrokeMgr,
                                            reinterpret_cast<void**>(&keystrokeMgr));
    if (FAILED(hr)) {
        Deactivate();
        return hr;
    }
    hr = keystrokeMgr->AdviseKeyEventSink(clientId_, static_cast<ITfKeyEventSink*>(this), TRUE);
    keystrokeMgr->Release();
    if (FAILED(hr)) {
        Deactivate();
        return hr;
    }
    return S_OK;
}

STDMETHODIMP TextService::Deactivate()
{
    romaji_.Clear();

    if (threadMgr_ != nullptr) {
        ITfKeystrokeMgr* keystrokeMgr = nullptr;
        if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfKeystrokeMgr,
                                                 reinterpret_cast<void**>(&keystrokeMgr)))) {
            keystrokeMgr->UnadviseKeyEventSink(clientId_);
            keystrokeMgr->Release();
        }
        threadMgr_->Release();
        threadMgr_ = nullptr;
    }
    clientId_ = TF_CLIENTID_NULL;
    return S_OK;
}

// ---- ITfKeyEventSink ----

STDMETHODIMP TextService::OnSetFocus(BOOL foreground)
{
    UNREFERENCED_PARAMETER(foreground);
    return S_OK;
}

bool TextService::IsKeyEaten(WPARAM wparam) const
{
    // Ctrl / Alt 併用時はアプリのショートカットなので手を出さない
    if ((GetKeyState(VK_CONTROL) & 0x8000) != 0 || (GetKeyState(VK_MENU) & 0x8000) != 0) {
        return false;
    }
    // Shift 併用 (大文字入力など) はフェーズ2で扱う。今はアプリへ素通しする
    if ((GetKeyState(VK_SHIFT) & 0x8000) != 0) {
        return false;
    }
    return wparam >= 'A' && wparam <= 'Z';
}

STDMETHODIMP TextService::OnTestKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                        BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = IsKeyEaten(wparam) ? TRUE : FALSE;
    if (*eaten == FALSE) {
        // 英字以外のキーが押されたら入力途中の子音は破棄する
        romaji_.Clear();
    }
    return S_OK;
}

STDMETHODIMP TextService::OnKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                    BOOL* eaten)
{
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = IsKeyEaten(wparam) ? TRUE : FALSE;
    if (*eaten == FALSE) {
        romaji_.Clear();
        return S_OK;
    }

    // 仮想キーコード ('A'-'Z') を英小文字へ変換してローマ字バッファに渡す
    const wchar_t c = static_cast<wchar_t>(L'a' + (wparam - 'A'));
    return InsertText(context, romaji_.Push(c));
}

STDMETHODIMP TextService::OnTestKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                      BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(wparam);
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = FALSE;
    return S_OK;
}

STDMETHODIMP TextService::OnKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(wparam);
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = FALSE;
    return S_OK;
}

STDMETHODIMP TextService::OnPreservedKey(ITfContext* context, REFGUID rguid, BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(rguid);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = FALSE;
    return S_OK;
}

// ---- 内部処理 ----

HRESULT TextService::InsertText(ITfContext* context, const std::wstring& text)
{
    if (text.empty()) {
        return S_OK;
    }
    if (context == nullptr) {
        return E_INVALIDARG;
    }

    auto* session = new (std::nothrow) InsertTextEditSession(context, text);
    if (session == nullptr) {
        return E_OUTOFMEMORY;
    }

    // キーイベント処理中なので同期の edit session が使える
    HRESULT hrSession = S_OK;
    HRESULT hr = context->RequestEditSession(clientId_, session, TF_ES_SYNC | TF_ES_READWRITE,
                                             &hrSession);
    session->Release();
    return FAILED(hr) ? hr : hrSession;
}
