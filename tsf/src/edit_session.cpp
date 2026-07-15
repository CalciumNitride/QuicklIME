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
                                                         ITfComposition** compositionOut)
    : EditSessionBase(context), sink_(sink), compositionOut_(compositionOut)
{
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

// ---- UndoCommitEditSession ----

UndoCommitEditSession::UndoCommitEditSession(ITfContext* context, std::wstring expectedText,
                                             bool* succeededOut)
    : EditSessionBase(context), expectedText_(std::move(expectedText)), succeededOut_(succeededOut)
{
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
        // 内容が確定文字列と一致する場合のみ削除する
        // (確定後にキャレット移動や他の編集があった場合は何もしない)
        std::vector<WCHAR> buffer(expectedText_.size());
        ULONG read = 0;
        hr = range->GetText(ec, 0, buffer.data(), static_cast<ULONG>(buffer.size()), &read);
        if (SUCCEEDED(hr) && read == expectedText_.size() &&
            expectedText_.compare(0, expectedText_.size(), buffer.data(), read) == 0) {
            hr = range->SetText(ec, 0, L"", 0);
            if (SUCCEEDED(hr)) {
                TF_SELECTION caret = {};
                caret.range = range;
                caret.style.ase = TF_AE_NONE;
                caret.style.fInterimChar = FALSE;
                context_->SetSelection(ec, 1, &caret);
                *succeededOut_ = true;
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
