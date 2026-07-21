#include "edit_session.h"

#include <vector>

namespace {

// キャレットを range の末尾へ移動する
void CollapseSelectionToEnd(TfEditCookie ec, ITfContext* context, ITfRange* range)
{
    ITfRange* end = nullptr;
    if (FAILED(range->Clone(&end))) {
        return;
    }
    end->Collapse(ec, TF_ANCHOR_END);

    TF_SELECTION selection = {};
    selection.range = end;
    selection.style.ase = TF_AE_NONE;
    selection.style.fInterimChar = FALSE;
    context->SetSelection(ec, 1, &selection);
    end->Release();
}

// composition の全範囲に表示属性 (下線) を適用する。atom が 0 なら削除する
void ApplyDisplayAttribute(TfEditCookie ec, ITfContext* context, ITfRange* range,
                           TfGuidAtom atom)
{
    ITfProperty* property = nullptr;
    if (FAILED(context->GetProperty(GUID_PROP_ATTRIBUTE, &property))) {
        return;
    }
    if (atom != TF_INVALID_GUIDATOM) {
        VARIANT var;
        VariantInit(&var);
        var.vt = VT_I4;
        var.lVal = static_cast<LONG>(atom);
        property->SetValue(ec, range, &var);
    } else {
        property->Clear(ec, range);
    }
    property->Release();
}

} // namespace

// ---- EditSessionBase ----

EditSessionBase::EditSessionBase(ITfContext* context) : context_(context), refCount_(1)
{
    context_->AddRef();
}

EditSessionBase::~EditSessionBase()
{
    context_->Release();
}

STDMETHODIMP EditSessionBase::QueryInterface(REFIID riid, void** ppv)
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

STDMETHODIMP_(ULONG) EditSessionBase::AddRef()
{
    return InterlockedIncrement(&refCount_);
}

STDMETHODIMP_(ULONG) EditSessionBase::Release()
{
    LONG count = InterlockedDecrement(&refCount_);
    if (count == 0) {
        delete this;
    }
    return count;
}

// ---- InsertTextEditSession ----

InsertTextEditSession::InsertTextEditSession(ITfContext* context, std::wstring text)
    : EditSessionBase(context), text_(std::move(text))
{
}

STDMETHODIMP InsertTextEditSession::DoEditSession(TfEditCookie ec)
{
    ITfInsertAtSelection* insertAtSelection = nullptr;
    HRESULT hr = context_->QueryInterface(IID_ITfInsertAtSelection,
                                          reinterpret_cast<void**>(&insertAtSelection));
    if (FAILED(hr)) {
        return hr;
    }

    ITfRange* range = nullptr;
    hr = insertAtSelection->InsertTextAtSelection(ec, 0, text_.c_str(),
                                                  static_cast<LONG>(text_.size()), &range);
    insertAtSelection->Release();
    if (FAILED(hr)) {
        return hr;
    }

    CollapseSelectionToEnd(ec, context_, range);
    range->Release();
    return S_OK;
}

// ---- StartCompositionEditSession ----

StartCompositionEditSession::StartCompositionEditSession(ITfContext* context,
                                                         ITfCompositionSink* sink,
                                                         ITfComposition** compositionOut,
                                                         ULONG precedingLength,
                                                         std::wstring* precedingTextOut,
                                                         bool* precedingReadOkOut)
    : EditSessionBase(context),
      sink_(sink),
      compositionOut_(compositionOut),
      precedingLength_(precedingLength),
      precedingTextOut_(precedingTextOut),
      precedingReadOkOut_(precedingReadOkOut)
{
    if (precedingTextOut_ != nullptr) {
        precedingTextOut_->clear();
    }
    if (precedingReadOkOut_ != nullptr) {
        *precedingReadOkOut_ = false;
    }
}

STDMETHODIMP StartCompositionEditSession::DoEditSession(TfEditCookie ec)
{
    // 現在のカーソル位置 (選択範囲) を composition の開始点にする
    ITfInsertAtSelection* insertAtSelection = nullptr;
    HRESULT hr = context_->QueryInterface(IID_ITfInsertAtSelection,
                                          reinterpret_cast<void**>(&insertAtSelection));
    if (FAILED(hr)) {
        return hr;
    }

    ITfRange* range = nullptr;
    hr = insertAtSelection->InsertTextAtSelection(ec, TF_IAS_QUERYONLY, nullptr, 0, &range);
    insertAtSelection->Release();
    if (FAILED(hr)) {
        return hr;
    }

    // 文脈補正のハイブリッド照合: composition を開始する前に、キャレット直前の
    // precedingLength_ 文字を読み取っておく (呼び出し側が内部履歴の文脈と比較する)。
    // 0 なら文脈なし・照合不要なので読み取らない。UndoCommitEditSession と同じ
    // ShiftStart (負方向) + GetText のパターンを読み取り専用で使う
    if (precedingLength_ > 0 && precedingTextOut_ != nullptr) {
        ITfRange* preceding = nullptr;
        if (SUCCEEDED(range->Clone(&preceding))) {
            preceding->Collapse(ec, TF_ANCHOR_START);
            LONG shifted = 0;
            if (SUCCEEDED(preceding->ShiftStart(ec, -static_cast<LONG>(precedingLength_),
                                                &shifted, nullptr))) {
                std::vector<WCHAR> buffer(precedingLength_);
                ULONG read = 0;
                if (SUCCEEDED(preceding->GetText(ec, 0, buffer.data(),
                                                 static_cast<ULONG>(buffer.size()), &read))) {
                    // ドキュメント先頭などで precedingLength_ 文字ぶん取れなかった場合も
                    // 読み取り自体は成功として扱う (短い文字列は文脈と一致せず、
                    // 呼び出し側で正しく不一致判定される)
                    precedingTextOut_->assign(buffer.data(), read);
                    if (precedingReadOkOut_ != nullptr) {
                        *precedingReadOkOut_ = true;
                    }
                }
            }
            preceding->Release();
        }
    }

    ITfContextComposition* contextComposition = nullptr;
    hr = context_->QueryInterface(IID_ITfContextComposition,
                                  reinterpret_cast<void**>(&contextComposition));
    if (SUCCEEDED(hr)) {
        hr = contextComposition->StartComposition(ec, range, sink_, compositionOut_);
        contextComposition->Release();
    }
    range->Release();
    return hr;
}

// ---- UpdateCompositionEditSession ----

UpdateCompositionEditSession::UpdateCompositionEditSession(
    ITfContext* context, ITfComposition* composition, std::wstring text,
    TfGuidAtom displayAttribute, TfGuidAtom targetAttribute, LONG targetStart,
    LONG targetLength)
    : EditSessionBase(context),
      composition_(composition),
      text_(std::move(text)),
      displayAttribute_(displayAttribute),
      targetAttribute_(targetAttribute),
      targetStart_(targetStart),
      targetLength_(targetLength)
{
    composition_->AddRef();
}

UpdateCompositionEditSession::~UpdateCompositionEditSession()
{
    composition_->Release();
}

STDMETHODIMP UpdateCompositionEditSession::DoEditSession(TfEditCookie ec)
{
    ITfRange* range = nullptr;
    HRESULT hr = composition_->GetRange(&range);
    if (FAILED(hr)) {
        return hr;
    }

    hr = range->SetText(ec, 0, text_.c_str(), static_cast<LONG>(text_.size()));
    if (SUCCEEDED(hr)) {
        ApplyDisplayAttribute(ec, context_, range, displayAttribute_);

        // 変換対象文節の部分範囲へ強調属性を上書きする
        if (targetLength_ > 0 && targetAttribute_ != TF_INVALID_GUIDATOM) {
            ITfRange* target = nullptr;
            if (SUCCEEDED(range->Clone(&target))) {
                LONG shifted = 0;
                target->Collapse(ec, TF_ANCHOR_START);
                target->ShiftEnd(ec, targetStart_ + targetLength_, &shifted, nullptr);
                target->ShiftStart(ec, targetStart_, &shifted, nullptr);
                ApplyDisplayAttribute(ec, context_, target, targetAttribute_);
                target->Release();
            }
        }

        CollapseSelectionToEnd(ec, context_, range);
    }
    range->Release();
    return hr;
}

// ---- GetTextExtentEditSession ----

GetTextExtentEditSession::GetTextExtentEditSession(ITfContext* context,
                                                   ITfComposition* composition, RECT* rectOut,
                                                   bool* succeededOut)
    : EditSessionBase(context),
      composition_(composition),
      rectOut_(rectOut),
      succeededOut_(succeededOut)
{
    composition_->AddRef();
    *succeededOut_ = false;
}

GetTextExtentEditSession::~GetTextExtentEditSession()
{
    composition_->Release();
}

STDMETHODIMP GetTextExtentEditSession::DoEditSession(TfEditCookie ec)
{
    ITfRange* range = nullptr;
    HRESULT hr = composition_->GetRange(&range);
    if (FAILED(hr)) {
        return hr;
    }

    ITfContextView* view = nullptr;
    hr = context_->GetActiveView(&view);
    if (SUCCEEDED(hr)) {
        BOOL clipped = FALSE;
        hr = view->GetTextExt(ec, range, rectOut_, &clipped);
        *succeededOut_ = SUCCEEDED(hr) && (rectOut_->right != 0 || rectOut_->bottom != 0);
        view->Release();
    }
    range->Release();
    return hr;
}

// ---- GetSelectionTextEditSession ----

GetSelectionTextEditSession::GetSelectionTextEditSession(ITfContext* context,
                                                         std::wstring* textOut)
    : EditSessionBase(context), textOut_(textOut)
{
    textOut_->clear();
}

STDMETHODIMP GetSelectionTextEditSession::DoEditSession(TfEditCookie ec)
{
    TF_SELECTION selection = {};
    ULONG fetched = 0;
    HRESULT hr = context_->GetSelection(ec, TF_DEFAULT_SELECTION, 1, &selection, &fetched);
    if (FAILED(hr) || fetched == 0) {
        return hr;
    }

    // 単語登録の初期値なので長い選択は先頭だけで十分
    wchar_t buffer[128] = {};
    ULONG copied = 0;
    hr = selection.range->GetText(ec, 0, buffer, ARRAYSIZE(buffer), &copied);
    if (SUCCEEDED(hr)) {
        textOut_->assign(buffer, copied);
    }
    selection.range->Release();
    return hr;
}

// ---- UndoCommitEditSession ----

UndoCommitEditSession::UndoCommitEditSession(ITfContext* context, std::wstring expectedText,
                                             ITfCompositionSink* sink, std::wstring newText,
                                             TfGuidAtom displayAttribute,
                                             ITfComposition** compositionOut, bool* succeededOut)
    : EditSessionBase(context),
      expectedText_(std::move(expectedText)),
      sink_(sink),
      newText_(std::move(newText)),
      displayAttribute_(displayAttribute),
      compositionOut_(compositionOut),
      succeededOut_(succeededOut)
{
    *compositionOut_ = nullptr;
    *succeededOut_ = false;
}

STDMETHODIMP UndoCommitEditSession::DoEditSession(TfEditCookie ec)
{
    TF_SELECTION selection = {};
    ULONG fetched = 0;
    HRESULT hr = context_->GetSelection(ec, TF_DEFAULT_SELECTION, 1, &selection, &fetched);
    if (FAILED(hr) || fetched == 0) {
        return hr;
    }
    ITfRange* range = selection.range;

    // キャレット位置に潰し、確定文字列の長さぶんだけ開始を前へ広げる
    range->Collapse(ec, TF_ANCHOR_START);
    LONG shifted = 0;
    const LONG length = static_cast<LONG>(expectedText_.size());
    hr = range->ShiftStart(ec, -length, &shifted, nullptr);
    if (SUCCEEDED(hr) && shifted == -length) {
        // 内容が確定文字列と一致する場合のみ復元する
        // (確定後にキャレット移動や他の編集があった場合は何もしない)
        std::vector<WCHAR> buffer(expectedText_.size());
        ULONG read = 0;
        hr = range->GetText(ec, 0, buffer.data(), static_cast<ULONG>(buffer.size()), &read);
        if (SUCCEEDED(hr) && read == expectedText_.size() &&
            expectedText_.compare(0, expectedText_.size(), buffer.data(), read) == 0) {
            // 確定文字列を覆う範囲で composition を開始し、確定前の読みに置き換える
            // (削除と復元を同一 session 内で済ませる)
            ITfContextComposition* contextComposition = nullptr;
            hr = context_->QueryInterface(IID_ITfContextComposition,
                                          reinterpret_cast<void**>(&contextComposition));
            if (SUCCEEDED(hr)) {
                hr = contextComposition->StartComposition(ec, range, sink_, compositionOut_);
                contextComposition->Release();
            }
            if (SUCCEEDED(hr) && *compositionOut_ != nullptr) {
                // composition が開始できた時点で成功扱いにする (以降の表示更新の
                // 失敗は、読みが composition に入らないだけで Esc 等で回復できる)
                *succeededOut_ = true;
                ITfRange* compRange = nullptr;
                if (SUCCEEDED((*compositionOut_)->GetRange(&compRange))) {
                    hr = compRange->SetText(ec, 0, newText_.c_str(),
                                            static_cast<LONG>(newText_.size()));
                    if (SUCCEEDED(hr)) {
                        ApplyDisplayAttribute(ec, context_, compRange, displayAttribute_);
                        CollapseSelectionToEnd(ec, context_, compRange);
                    }
                    compRange->Release();
                }
            }
        }
    }
    range->Release();
    return hr;
}

// ---- EndCompositionEditSession ----

EndCompositionEditSession::EndCompositionEditSession(ITfContext* context,
                                                     ITfComposition* composition,
                                                     std::wstring commitText)
    : EditSessionBase(context), composition_(composition), commitText_(std::move(commitText))
{
    composition_->AddRef();
}

EndCompositionEditSession::~EndCompositionEditSession()
{
    composition_->Release();
}

STDMETHODIMP EndCompositionEditSession::DoEditSession(TfEditCookie ec)
{
    ITfRange* range = nullptr;
    HRESULT hr = composition_->GetRange(&range);
    if (SUCCEEDED(hr)) {
        // 確定文字列で置き換え、下線属性を外してから終了する
        hr = range->SetText(ec, 0, commitText_.c_str(), static_cast<LONG>(commitText_.size()));
        if (SUCCEEDED(hr)) {
            ApplyDisplayAttribute(ec, context_, range, TF_INVALID_GUIDATOM);
            CollapseSelectionToEnd(ec, context_, range);
        }
        range->Release();
    }
    composition_->EndComposition(ec);
    return hr;
}

// ---- RestartCompositionEditSession ----

RestartCompositionEditSession::RestartCompositionEditSession(
    ITfContext* context, ITfComposition* oldComposition, std::wstring commitText,
    ITfCompositionSink* sink, ITfComposition** compositionOut)
    : EditSessionBase(context),
      oldComposition_(oldComposition),
      commitText_(std::move(commitText)),
      sink_(sink),
      compositionOut_(compositionOut)
{
    oldComposition_->AddRef();
    *compositionOut_ = nullptr;
}

RestartCompositionEditSession::~RestartCompositionEditSession()
{
    oldComposition_->Release();
}

STDMETHODIMP RestartCompositionEditSession::DoEditSession(TfEditCookie ec)
{
    // 1) 旧 composition を確定文字列で置き換え、下線属性を外して終了する
    //    (EndCompositionEditSession と同じ処理)
    ITfRange* range = nullptr;
    HRESULT hr = oldComposition_->GetRange(&range);
    if (SUCCEEDED(hr)) {
        hr = range->SetText(ec, 0, commitText_.c_str(), static_cast<LONG>(commitText_.size()));
        if (SUCCEEDED(hr)) {
            ApplyDisplayAttribute(ec, context_, range, TF_INVALID_GUIDATOM);
            CollapseSelectionToEnd(ec, context_, range);
        }
        range->Release();
    }
    oldComposition_->EndComposition(ec);
    if (FAILED(hr)) {
        return hr; // 確定できていなければ新しい composition は開始しない
    }

    // 2) キャレット位置 (確定文字列の直後) で新しい composition を開始する
    ITfInsertAtSelection* insertAtSelection = nullptr;
    hr = context_->QueryInterface(IID_ITfInsertAtSelection,
                                  reinterpret_cast<void**>(&insertAtSelection));
    if (FAILED(hr)) {
        return hr;
    }
    ITfRange* startRange = nullptr;
    hr = insertAtSelection->InsertTextAtSelection(ec, TF_IAS_QUERYONLY, nullptr, 0, &startRange);
    insertAtSelection->Release();
    if (FAILED(hr)) {
        return hr;
    }
    ITfContextComposition* contextComposition = nullptr;
    hr = context_->QueryInterface(IID_ITfContextComposition,
                                  reinterpret_cast<void**>(&contextComposition));
    if (SUCCEEDED(hr)) {
        hr = contextComposition->StartComposition(ec, startRange, sink_, compositionOut_);
        contextComposition->Release();
    }
    startRange->Release();
    if (FAILED(hr) || *compositionOut_ == nullptr) {
        // StartComposition はアプリの拒否時に S_OK + nullptr を返すことがある
        return FAILED(hr) ? hr : E_FAIL;
    }

    // テキストの設定はこの session では行わない。呼び出し側が別の edit session
    // (UpdateCompositionText) で設定する。同じ session 内で EndComposition +
    // StartComposition + SetText を行うと、CUAS が SetText の
    // WM_IME_COMPOSITION を生成せず、WezTerm 等で未確定文字列が表示されない
    return hr;
}
