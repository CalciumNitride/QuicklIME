#include "text_service.h"

#include <new>

#include "display_attribute.h"
#include "edit_session.h"
#include "globals.h"

namespace {

// 記号キー (仮想キーコード) → 入力するかな
// 日本語キーボード配列の想定 (フェーズ5で配列設定に対応する)
const wchar_t* SymbolKeyToKana(WPARAM wparam, bool shifted)
{
    if (shifted) {
        switch (wparam) {
        case '1':       return L"！";
        case VK_OEM_2:  return L"？"; // Shift + /
        default:        return nullptr;
        }
    }
    switch (wparam) {
    case VK_OEM_COMMA:  return L"、";
    case VK_OEM_PERIOD: return L"。";
    case VK_OEM_MINUS:  return L"ー";
    case VK_OEM_2:      return L"・"; // /
    case VK_OEM_4:      return L"「"; // [
    case VK_OEM_6:      return L"」"; // ]
    default:            return nullptr;
    }
}

bool IsLetterKey(WPARAM wparam)
{
    return wparam >= 'A' && wparam <= 'Z';
}

bool IsDigitKey(WPARAM wparam)
{
    return wparam >= '0' && wparam <= '9';
}

bool IsShiftPressed()
{
    return (GetKeyState(VK_SHIFT) & 0x8000) != 0;
}

} // namespace

TextService::TextService()
    : refCount_(1),
      threadMgr_(nullptr),
      clientId_(TF_CLIENTID_NULL),
      composition_(nullptr),
      inputAttribute_(TF_INVALID_GUIDATOM),
      targetAttribute_(TF_INVALID_GUIDATOM),
      converting_(false),
      segmentIndex_(0)
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
        categoryMgr->RegisterGUID(kTargetDisplayAttributeGuid, &targetAttribute_);
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
    ClearConversion();
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
    const bool shifted = IsShiftPressed();

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
        case VK_LEFT:
        case VK_RIGHT:
            // 候補選択中のみ矢印キーを使う (↑↓=候補, ←→=文節移動, Shift+←→=文節伸縮)
            return converting_;
        default:
            break;
        }
        // composition 中は英字 (Shift併用含む)・数字・記号も IME が処理し、
        // 未確定文字列の外へ文字が漏れないようにする
        return IsLetterKey(wparam) || IsDigitKey(wparam) ||
               SymbolKeyToKana(wparam, shifted) != nullptr;
    }

    // composition が無いとき: Shift 併用は ！？ などの記号のみ扱う
    if (shifted) {
        return SymbolKeyToKana(wparam, true) != nullptr;
    }
    return IsLetterKey(wparam) || SymbolKeyToKana(wparam, false) != nullptr;
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
    ClearConversion();
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
    *info = CreateDisplayAttributeInfoForGuid(guid);
    return *info != nullptr ? S_OK : E_INVALIDARG;
}

// ---- 状態機械 ----

HRESULT TextService::HandleKey(ITfContext* context, WPARAM wparam)
{
    const bool shifted = IsShiftPressed();

    // Shift+英字: ローマ字変換に回さず、大文字の半角英字をそのまま確定済みかな列へ入れる
    // (数字キーと同様、未変換ローマ字があれば先に確定してから追加される)
    if (shifted && IsLetterKey(wparam)) {
        const std::wstring upper(1, static_cast<wchar_t>(wparam));
        if (converting_) {
            HRESULT hr = CommitConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
            return InsertText(context, upper);
        }
        if (Composing()) {
            composer_.PushKana(upper);
            return UpdateCompositionText(context, composer_.Display());
        }
        return InsertText(context, upper);
    }

    // 英字: ローマ字入力を進める (候補選択中なら選択中の候補を確定してから)
    if (IsLetterKey(wparam)) {
        if (converting_) {
            HRESULT hr = CommitConversion(context);
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
    // (数字キーと重なる Shift+1 (！) などがあるため、数字判定より先に見る)
    if (const wchar_t* kana = SymbolKeyToKana(wparam, shifted)) {
        if (converting_) {
            HRESULT hr = CommitConversion(context);
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

    // 数字: composition 中のみここへ来る (composition が無ければ食べていない)
    if (IsDigitKey(wparam)) {
        const std::wstring digit(1, static_cast<wchar_t>(wparam));
        if (converting_) {
            HRESULT hr = CommitConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
            return InsertText(context, digit);
        }
        composer_.PushKana(digit);
        return UpdateCompositionText(context, composer_.Display());
    }

    // 以下は composition 中のみ食べているキー
    switch (wparam) {
    case VK_RETURN:
        return converting_ ? CommitConversion(context)
                           : EndComposition(context, composer_.Commit());
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
    case VK_LEFT:
        // Shift+← は現在文節を1文字縮める、← は文節移動
        if (!converting_) {
            return S_OK;
        }
        return shifted ? ResizeSegment(context, -1) : MoveSegment(context, -1);
    case VK_RIGHT:
        // Shift+→ は現在文節を1文字伸ばす、→ は文節移動
        if (!converting_) {
            return S_OK;
        }
        return shifted ? ResizeSegment(context, +1) : MoveSegment(context, +1);
    default:
        return S_OK;
    }
}

// ---- 変換 (文節と候補の選択) ----

HRESULT TextService::StartConversion(ITfContext* context)
{
    if (!Composing()) {
        return E_UNEXPECTED;
    }

    // 文節列をエンジンに問い合わせる。
    // エンジンが起動していない場合はひらがな1文節のみで動作を継続する
    const std::wstring kana = composer_.Commit();
    if (!engine_.ConvertSegments(kana, &segments_) || segments_.empty()) {
        segments_.clear();
        ConversionSegment fallback;
        fallback.reading = kana;
        fallback.candidates.push_back(kana);
        segments_.push_back(std::move(fallback));
    }
    selected_.assign(segments_.size(), 0);
    segmentIndex_ = 0;
    converting_ = true;

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

HRESULT TextService::CycleCandidate(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }
    const size_t count = segments_[segmentIndex_].candidates.size();
    selected_[segmentIndex_] = (selected_[segmentIndex_] + count + delta) % count;
    candidateWindow_.SetSelection(selected_[segmentIndex_]);
    return UpdateConvertingDisplay(context);
}

HRESULT TextService::MoveSegment(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }
    const size_t count = segments_.size();
    segmentIndex_ = (segmentIndex_ + count + delta) % count;
    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context); // 候補一覧を現在文節のものに差し替える
    return hr;
}

HRESULT TextService::ResizeSegment(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }

    // 文節 i より前はそのまま残し、文節 i を新しい長さに固定し、
    // それより後ろは境界を固定せず自然な区切りに再変換する。
    // (後ろを固定長のまま引き継ぐと、Shift+→→の直後に Shift+← で戻したときに
    //  元の区切りに戻らず不自然な文節に割れてしまうため)
    const size_t i = segmentIndex_;

    std::wstring kana;
    size_t prefixLen = 0;
    for (size_t k = 0; k < segments_.size(); ++k) {
        if (k < i) {
            prefixLen += segments_[k].reading.size();
        }
        kana += segments_[k].reading;
    }

    const size_t currentLen = segments_[i].reading.size();
    size_t newLen;
    if (delta > 0) {
        if (prefixLen + currentLen >= kana.size()) {
            return S_OK; // これ以上伸ばせる文字が残っていない
        }
        newLen = currentLen + 1;
    } else {
        if (currentLen <= 1) {
            return S_OK; // 1文字の文節はこれ以上縮められない
        }
        newLen = currentLen - 1;
    }

    // 文節 i を新しい長さで固定変換する
    const std::wstring segmentKana = kana.substr(prefixLen, newLen);
    std::vector<ConversionSegment> fixedResult;
    if (!engine_.ConvertSegmentsFixed(segmentKana, {newLen}, &fixedResult) ||
        fixedResult.empty()) {
        return S_OK; // エンジン不調時は現状維持
    }

    // 残りは境界を固定せず自由に再変換する
    std::vector<ConversionSegment> tailResult;
    const std::wstring tailKana = kana.substr(prefixLen + newLen);
    if (!tailKana.empty() &&
        (!engine_.ConvertSegments(tailKana, &tailResult) || tailResult.empty())) {
        return S_OK; // エンジン不調時は現状維持
    }

    segments_.resize(i);
    segments_.push_back(std::move(fixedResult[0]));
    for (ConversionSegment& segment : tailResult) {
        segments_.push_back(std::move(segment));
    }
    selected_.resize(i);
    selected_.resize(segments_.size(), 0);

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

HRESULT TextService::CancelConversion(ITfContext* context)
{
    ClearConversion();
    return UpdateCompositionText(context, composer_.Display());
}

HRESULT TextService::CommitConversion(ITfContext* context)
{
    // 文節ごとの確定結果をエンジンに学習させる (失敗しても確定は続行する)
    std::vector<std::pair<std::wstring, std::wstring>> pairs;
    for (size_t i = 0; i < segments_.size(); ++i) {
        pairs.emplace_back(segments_[i].reading, segments_[i].candidates[selected_[i]]);
    }
    engine_.Learn(pairs);

    return EndComposition(context, ConvertedText());
}

void TextService::ClearConversion()
{
    candidateWindow_.Hide();
    converting_ = false;
    segments_.clear();
    selected_.clear();
    segmentIndex_ = 0;
}

std::wstring TextService::ConvertedText() const
{
    std::wstring text;
    for (size_t i = 0; i < segments_.size(); ++i) {
        text += segments_[i].candidates[selected_[i]];
    }
    return text;
}

HRESULT TextService::UpdateConvertingDisplay(ITfContext* context)
{
    if (!Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }

    // 現在文節の位置 (文字数) を求めて、その範囲だけ強調属性を付ける
    LONG targetStart = 0;
    for (size_t i = 0; i < segmentIndex_; ++i) {
        targetStart += static_cast<LONG>(segments_[i].candidates[selected_[i]].size());
    }
    const LONG targetLength =
        static_cast<LONG>(segments_[segmentIndex_].candidates[selected_[segmentIndex_]].size());

    return RequestSync(context,
                       new (std::nothrow) UpdateCompositionEditSession(
                           context, composition_, ConvertedText(), inputAttribute_,
                           targetAttribute_, targetStart, targetLength),
                       TF_ES_SYNC | TF_ES_READWRITE);
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

    if (!segments_.empty()) {
        candidateWindow_.Show(rect, segments_[segmentIndex_].candidates,
                              selected_[segmentIndex_]);
    }
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

    // commitText は変換状態の要素への参照であることがあるため、
    // 変換状態を後始末する前に必ずコピーを取る
    const std::wstring text = commitText;

    // 変換状態と候補ウィンドウの後始末
    ClearConversion();

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
