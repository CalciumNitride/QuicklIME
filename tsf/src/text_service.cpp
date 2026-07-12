#include "text_service.h"

#include <new>

#include "display_attribute.h"
#include "edit_session.h"
#include "globals.h"

namespace {

// 記号キー (仮想キーコード) → 入力するかな
// 日本語キーボード配列の想定 (フェーズ5で配列設定に対応する)
const wchar_t* SymbolKeyToKana(WPARAM wparam)
{
    switch (wparam) {
    case VK_OEM_COMMA:  return L"、";
    case VK_OEM_PERIOD: return L"。";
    case VK_OEM_MINUS:  return L"ー";
    default:            return nullptr;
    }
}

bool IsLetterKey(WPARAM wparam)
{
    return wparam >= 'A' && wparam <= 'Z';
}

} // namespace

TextService::TextService()
    : refCount_(1),
      threadMgr_(nullptr),
      clientId_(TF_CLIENTID_NULL),
      composition_(nullptr),
      inputAttribute_(TF_INVALID_GUIDATOM),
      converting_(false),
      candidateIndex_(0)
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
    } else if (IsEqualIID(riid, IID_ITfCompositionSink)) {
        *ppv = static_cast<ITfCompositionSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfDisplayAttributeProvider)) {
        *ppv = static_cast<ITfDisplayAttributeProvider*>(this);
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

    // 未確定文字列の表示属性 GUID を atom に変換しておく
    ITfCategoryMgr* categoryMgr = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr));
    if (SUCCEEDED(hr)) {
        categoryMgr->RegisterGUID(kInputDisplayAttributeGuid, &inputAttribute_);
        categoryMgr->Release();
    }

    // キーイベントを受け取るために key event sink を登録する
    ITfKeystrokeMgr* keystrokeMgr = nullptr;
    hr = threadMgr_->QueryInterface(IID_ITfKeystrokeMgr,
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
    candidateWindow_.Hide();
    converting_ = false;
    candidates_.clear();
    composer_.Clear();
    if (composition_ != nullptr) {
        composition_->Release();
        composition_ = nullptr;
    }

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

    if (Composing()) {
        // composition 中は編集キーも IME が処理する
        switch (wparam) {
        case VK_RETURN:
        case VK_ESCAPE:
        case VK_BACK:
        case VK_SPACE:
            return true;
        case VK_UP:
        case VK_DOWN:
            // 候補選択中のみ矢印キーを使う
            return converting_;
        default:
            break;
        }
    }

    // Shift 併用 (大文字入力など) は今はアプリへ素通しする
    if ((GetKeyState(VK_SHIFT) & 0x8000) != 0) {
        return false;
    }
    return IsLetterKey(wparam) || SymbolKeyToKana(wparam) != nullptr;
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
        return S_OK;
    }
    return HandleKey(context, wparam);
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

// ---- ITfCompositionSink ----

STDMETHODIMP TextService::OnCompositionTerminated(TfEditCookie ecWrite,
                                                  ITfComposition* composition)
{
    UNREFERENCED_PARAMETER(ecWrite);
    UNREFERENCED_PARAMETER(composition);

    // アプリ側の操作 (クリックなど) で composition が終了した。入力途中の状態を捨てる
    candidateWindow_.Hide();
    converting_ = false;
    candidates_.clear();
    candidateIndex_ = 0;
    composer_.Clear();
    if (composition_ != nullptr) {
        composition_->Release();
        composition_ = nullptr;
    }
    return S_OK;
}

// ---- ITfDisplayAttributeProvider ----

STDMETHODIMP TextService::EnumDisplayAttributeInfo(IEnumTfDisplayAttributeInfo** enumInfo)
{
    if (enumInfo == nullptr) {
        return E_INVALIDARG;
    }
    auto* enumerator = new (std::nothrow) ::EnumDisplayAttributeInfo();
    if (enumerator == nullptr) {
        return E_OUTOFMEMORY;
    }
    *enumInfo = enumerator;
    return S_OK;
}

STDMETHODIMP TextService::GetDisplayAttributeInfo(REFGUID guid, ITfDisplayAttributeInfo** info)
{
    if (info == nullptr) {
        return E_INVALIDARG;
    }
    if (!IsEqualGUID(guid, kInputDisplayAttributeGuid)) {
        *info = nullptr;
        return E_INVALIDARG;
    }
    auto* attribute = new (std::nothrow) InputDisplayAttributeInfo();
    if (attribute == nullptr) {
        return E_OUTOFMEMORY;
    }
    *info = attribute;
    return S_OK;
}

// ---- 状態機械 ----

HRESULT TextService::HandleKey(ITfContext* context, WPARAM wparam)
{
    // 英字: ローマ字入力を進める (候補選択中なら選択中の候補を確定してから)
    if (IsLetterKey(wparam)) {
        if (converting_) {
            HRESULT hr = EndComposition(context, candidates_[candidateIndex_]);
            if (FAILED(hr)) {
                return hr;
            }
        }
        const wchar_t c = static_cast<wchar_t>(L'a' + (wparam - 'A'));
        composer_.Push(c);
        if (!Composing()) {
            HRESULT hr = StartComposition(context);
            if (FAILED(hr)) {
                composer_.Clear();
                return hr;
            }
        }
        return UpdateCompositionText(context, composer_.Display());
    }

    // 記号: composition 中なら未確定文字列へ、そうでなければ直接挿入
    if (const wchar_t* kana = SymbolKeyToKana(wparam)) {
        if (converting_) {
            HRESULT hr = EndComposition(context, candidates_[candidateIndex_]);
            if (FAILED(hr)) {
                return hr;
            }
            return InsertText(context, kana);
        }
        if (Composing()) {
            composer_.PushKana(kana);
            return UpdateCompositionText(context, composer_.Display());
        }
        return InsertText(context, kana);
    }

    // 以下は composition 中のみ食べているキー
    switch (wparam) {
    case VK_RETURN:
        return EndComposition(context,
                              converting_ ? candidates_[candidateIndex_] : composer_.Commit());
    case VK_ESCAPE:
        // 候補選択中は変換を取り消してかな表示に戻る。それ以外は全消去
        return converting_ ? CancelConversion(context) : EndComposition(context, L"");
    case VK_BACK:
        if (converting_) {
            return CancelConversion(context);
        }
        composer_.Backspace();
        if (composer_.Empty()) {
            return EndComposition(context, L"");
        }
        return UpdateCompositionText(context, composer_.Display());
    case VK_SPACE:
        return converting_ ? CycleCandidate(context, +1) : StartConversion(context);
    case VK_DOWN:
        return converting_ ? CycleCandidate(context, +1) : S_OK;
    case VK_UP:
        return converting_ ? CycleCandidate(context, -1) : S_OK;
    default:
        return S_OK;
    }
}

// ---- 変換 (候補選択) ----

HRESULT TextService::StartConversion(ITfContext* context)
{
    if (!Composing()) {
        return E_UNEXPECTED;
    }

    // 変換候補はエンジンに問い合わせる。
    // エンジンが起動していない場合はひらがな1候補のみで動作を継続する
    const std::wstring kana = composer_.Commit();
    if (!engine_.Convert(kana, &candidates_) || candidates_.empty()) {
        candidates_.clear();
        candidates_.push_back(kana);
    }
    candidateIndex_ = 0;
    converting_ = true;

    HRESULT hr = UpdateCompositionText(context, candidates_[candidateIndex_]);
    ShowCandidateWindow(context);
    return hr;
}

HRESULT TextService::CycleCandidate(ITfContext* context, int delta)
{
    if (!converting_ || candidates_.empty()) {
        return E_UNEXPECTED;
    }
    const size_t count = candidates_.size();
    candidateIndex_ = (candidateIndex_ + count + delta) % count;
    candidateWindow_.SetSelection(candidateIndex_);
    return UpdateCompositionText(context, candidates_[candidateIndex_]);
}

HRESULT TextService::CancelConversion(ITfContext* context)
{
    candidateWindow_.Hide();
    converting_ = false;
    candidates_.clear();
    candidateIndex_ = 0;
    return UpdateCompositionText(context, composer_.Display());
}

void TextService::ShowCandidateWindow(ITfContext* context)
{
    // composition の矩形を取得して候補ウィンドウの位置を決める
    RECT rect = {};
    bool succeeded = false;
    if (Composing()) {
        RequestSync(context,
                    new (std::nothrow) GetTextExtentEditSession(context, composition_, &rect,
                                                                &succeeded),
                    TF_ES_SYNC | TF_ES_READ);
    }

    if (!succeeded) {
        // 取得できないアプリではキャレット位置、それも無ければマウス位置へフォールバック
        GUITHREADINFO info = {};
        info.cbSize = sizeof(info);
        if (GetGUIThreadInfo(0, &info) && info.hwndCaret != nullptr) {
            rect = info.rcCaret;
            MapWindowPoints(info.hwndCaret, HWND_DESKTOP, reinterpret_cast<POINT*>(&rect), 2);
        } else {
            POINT pt = {};
            GetCursorPos(&pt);
            rect = {pt.x, pt.y, pt.x, pt.y};
        }
    }

    candidateWindow_.Show(rect, candidates_, candidateIndex_);
}

// ---- composition 操作 ----

HRESULT TextService::RequestSync(ITfContext* context, ITfEditSession* session, DWORD flags)
{
    if (session == nullptr) {
        return E_OUTOFMEMORY;
    }
    HRESULT hrSession = S_OK;
    HRESULT hr = context->RequestEditSession(clientId_, session, flags, &hrSession);
    session->Release();
    return FAILED(hr) ? hr : hrSession;
}

HRESULT TextService::StartComposition(ITfContext* context)
{
    if (Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }
    return RequestSync(context,
                       new (std::nothrow) StartCompositionEditSession(
                           context, static_cast<ITfCompositionSink*>(this), &composition_),
                       TF_ES_SYNC | TF_ES_READWRITE);
}

HRESULT TextService::UpdateCompositionText(ITfContext* context, const std::wstring& text)
{
    if (!Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }
    return RequestSync(context,
                       new (std::nothrow) UpdateCompositionEditSession(context, composition_,
                                                                       text, inputAttribute_),
                       TF_ES_SYNC | TF_ES_READWRITE);
}

HRESULT TextService::EndComposition(ITfContext* context, const std::wstring& commitText)
{
    if (!Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }

    // commitText は candidates_ の要素への参照であることがあるため、
    // 変換状態を後始末する前に必ずコピーを取る
    const std::wstring text = commitText;

    // 変換状態と候補ウィンドウの後始末
    candidateWindow_.Hide();
    converting_ = false;
    candidates_.clear();
    candidateIndex_ = 0;

    HRESULT hr = RequestSync(
        context, new (std::nothrow) EndCompositionEditSession(context, composition_, text),
        TF_ES_SYNC | TF_ES_READWRITE);
    composition_->Release();
    composition_ = nullptr;
    composer_.Clear();
    return hr;
}

HRESULT TextService::InsertText(ITfContext* context, const std::wstring& text)
{
    if (text.empty()) {
        return S_OK;
    }
    if (context == nullptr) {
        return E_INVALIDARG;
    }
    return RequestSync(context, new (std::nothrow) InsertTextEditSession(context, text),
                       TF_ES_SYNC | TF_ES_READWRITE);
}
