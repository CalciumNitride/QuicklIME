#include "text_service.h"

#include <algorithm>
#include <cwctype>
#include <new>

#include "debug_log.h"
#include "display_attribute.h"
#include "edit_session.h"
#include "globals.h"
#include "kana_forms.h"
#include "lang_bar.h"

using namespace key_util;

namespace key_util {

// 記号キー (仮想キーコード) → かな/打鍵文字
// 日本語キーボード配列の想定 (フェーズ5で配列設定に対応する)
const SymbolKey* FindSymbolKey(WPARAM wparam, bool shifted)
{
    static const SymbolKey plain[] = {
        {VK_OEM_COMMA,  L"、", L","},
        {VK_OEM_PERIOD, L"。", L"."},
        {VK_OEM_MINUS,  L"ー", L"-"},
        {VK_OEM_2,      L"・", L"/"},
        {VK_OEM_4,      L"「", L"["},
        {VK_OEM_6,      L"」", L"]"},
        {VK_OEM_3,      L"＠", L"@"},
        {VK_OEM_PLUS,   L"；", L";"},
        {VK_OEM_1,      L"：", L":"},
        {VK_OEM_7,      L"＾", L"^"},
        {VK_OEM_5,      L"￥", L"\\"}, // ¥ キー
        {VK_OEM_102,    L"＼", L"\\"}, // ろ キー
    };
    static const SymbolKey shift[] = {
        {'1',           L"！", L"!"},
        {'2',           L"”", L"\""},
        {'3',           L"＃", L"#"},
        {'4',           L"＄", L"$"},
        {'5',           L"％", L"%"},
        {'6',           L"＆", L"&"},
        {'7',           L"’", L"'"},
        {'8',           L"（", L"("},
        {'9',           L"）", L")"},
        {VK_OEM_MINUS,  L"＝", L"="},
        {VK_OEM_2,      L"？", L"?"},
        {VK_OEM_4,      L"｛", L"{"},
        {VK_OEM_6,      L"｝", L"}"},
        {VK_OEM_3,      L"｀", L"`"},
        {VK_OEM_PLUS,   L"＋", L"+"},
        {VK_OEM_1,      L"＊", L"*"},
        {VK_OEM_COMMA,  L"＜", L"<"},
        {VK_OEM_PERIOD, L"＞", L">"},
        {VK_OEM_7,      L"～", L"~"},
        {VK_OEM_5,      L"｜", L"|"},
        {VK_OEM_102,    L"＿", L"_"},
    };
    const SymbolKey* keys = shifted ? shift : plain;
    const size_t count = shifted ? ARRAYSIZE(shift) : ARRAYSIZE(plain);
    for (size_t i = 0; i < count; ++i) {
        if (keys[i].vk == wparam) {
            return &keys[i];
        }
    }
    return nullptr;
}

bool IsLetterKey(WPARAM wparam)
{
    return wparam >= 'A' && wparam <= 'Z';
}

bool IsDigitKey(WPARAM wparam)
{
    return wparam >= '0' && wparam <= '9';
}

// テンキーは数値入力用なので、全角形にせず半角のまま未確定文字列へ入れる
wchar_t NumpadChar(WPARAM wparam)
{
    if (wparam >= VK_NUMPAD0 && wparam <= VK_NUMPAD9) {
        return static_cast<wchar_t>(L'0' + (wparam - VK_NUMPAD0));
    }
    switch (wparam) {
    case VK_MULTIPLY:
        return L'*';
    case VK_ADD:
        return L'+';
    case VK_SUBTRACT:
        return L'-';
    case VK_DECIMAL:
        return L'.';
    case VK_DIVIDE:
        return L'/';
    default:
        return 0;
    }
}

bool IsShiftPressed()
{
    return (GetKeyState(VK_SHIFT) & 0x8000) != 0;
}

bool ContainsAsciiLetter(const std::wstring& text)
{
    for (wchar_t c : text) {
        if ((c >= L'a' && c <= L'z') || (c >= L'A' && c <= L'Z')) {
            return true;
        }
    }
    return false;
}

} // namespace key_util

namespace {

// 対で使う記号 (開き, 閉じ)。かっこを変換したとき両側を同期させるために使う
struct SymbolPair {
    const wchar_t* open;
    const wchar_t* close;
};

const SymbolPair kSymbolPairs[] = {
    {L"（", L"）"}, {L"〔", L"〕"}, {L"［", L"］"}, {L"〘", L"〙"}, {L"〚", L"〛"},
    {L"｛", L"｝"}, {L"〈", L"〉"}, {L"‹", L"›"},  {L"《", L"》"}, {L"«", L"»"},
    {L"「", L"」"}, {L"『", L"』"}, {L"【", L"】"}, {L"〝", L"〟"}, {L"⁽", L"⁾"},
    {L"₍", L"₎"},  {L"(", L")"},  {L"[", L"]"},  {L"{", L"}"},  {L"“", L"”"},
    {L"‘", L"’"},
};

// text の対になる形を返す (wantClose: 閉じ形が欲しいか)。
// クオートなど左右同形の記号はそのまま返し、対記号でなければ nullptr
const wchar_t* PartnerSymbolText(const std::wstring& text, bool wantClose)
{
    for (const SymbolPair& pair : kSymbolPairs) {
        if (wantClose && text == pair.open) {
            return pair.close;
        }
        if (!wantClose && text == pair.close) {
            return pair.open;
        }
    }
    static const wchar_t* kSymmetric[] = {L"”", L"’", L"″", L"′", L"\"", L"'", L"＂"};
    for (const wchar_t* s : kSymmetric) {
        if (text == s) {
            return s;
        }
    }
    return nullptr;
}

// 打鍵で未確定文字列に入る対記号の読みの対応 (開き→閉じ / 閉じ→開き)。
// 記号キーから入るのは （）｛｝「」 とクオート (”’ は左右同形) のみ
const wchar_t* CloseReadingForOpen(const std::wstring& reading)
{
    if (reading == L"（") return L"）";
    if (reading == L"｛") return L"｝";
    if (reading == L"「") return L"」";
    return nullptr;
}

const wchar_t* OpenReadingForClose(const std::wstring& reading)
{
    if (reading == L"）") return L"（";
    if (reading == L"｝") return L"｛";
    if (reading == L"」") return L"「";
    return nullptr;
}

bool IsSymmetricQuoteReading(const std::wstring& reading)
{
    return reading == L"”" || reading == L"’";
}

// F9/F10 の連打で循環させる英字の変種列を作る。
// 元の打鍵のまま → 先頭のみ大文字 → 全部大文字 → 全部小文字 の順。
// 重複する形 (元が全部小文字なら「全部小文字」は元と同じ、など) は取り除く
std::vector<std::wstring> CaseCycleVariants(const std::wstring& raw)
{
    std::wstring lower = raw;
    for (wchar_t& c : lower) {
        c = towlower(c);
    }
    std::wstring capitalized = lower;
    for (wchar_t& c : capitalized) {
        if (c >= L'a' && c <= L'z') {
            c = towupper(c);
            break;
        }
    }
    std::wstring upper = raw;
    for (wchar_t& c : upper) {
        c = towupper(c);
    }

    std::vector<std::wstring> variants;
    for (const auto& variant : {raw, capitalized, upper, lower}) {
        if (std::find(variants.begin(), variants.end(), variant) == variants.end()) {
            variants.push_back(variant);
        }
    }
    return variants;
}

// F7/F8 の連打で循環させるカタカナの変種列を作る。
// 全てカタカナ → 末尾1文字だけひらがな → 末尾2文字だけひらがな → ... と
// 後ろから1文字ずつひらがなに戻していく (全体を一周すると先頭に戻る)。
// 重複する形 (「ー」などカタカナに変換されない文字による) は取り除く
std::vector<std::wstring> KatakanaCycleVariants(const std::wstring& reading, bool halfwidth)
{
    std::vector<std::wstring> variants;
    for (size_t hiraganaCount = 0; hiraganaCount < reading.size(); ++hiraganaCount) {
        const std::wstring prefix = reading.substr(0, reading.size() - hiraganaCount);
        const std::wstring variant =
            (halfwidth ? kana_forms::ToHalfwidth(prefix) : kana_forms::ToKatakana(prefix)) +
            reading.substr(reading.size() - hiraganaCount);
        if (std::find(variants.begin(), variants.end(), variant) == variants.end()) {
            variants.push_back(variant);
        }
    }
    return variants;
}

// 打鍵したローマ字そのものと大文字小文字の変種を候補に挿入する (英単語入力用)。
// position は挿入開始位置。既にある候補 (学習済みなど) は動かさず挿入しない
void InsertRawCandidates(ConversionSegment* segment, const std::wstring& raw, size_t position)
{
    if (raw.empty() || !ContainsAsciiLetter(raw)) {
        return;
    }
    auto& list = segment->candidates;
    position = (std::min)(position, list.size());
    for (const auto& candidate : CaseCycleVariants(raw)) {
        if (std::find(list.begin(), list.end(), candidate) == list.end()) {
            list.insert(list.begin() + position, candidate);
            ++position;
        }
    }
}

// 末尾のスペースを除いた全体が ASCII 英数字か (空文字列は false)。
// 英単語を確定した直後のスペースを半角にする判定に使う
bool IsAsciiAlnumText(const std::wstring& text)
{
    size_t end = text.size();
    while (end > 0 && text[end - 1] == L' ') {
        --end;
    }
    if (end == 0) {
        return false;
    }
    for (size_t i = 0; i < end; ++i) {
        const wchar_t c = text[i];
        if (!((c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'z') ||
              (c >= L'A' && c <= L'Z'))) {
            return false;
        }
    }
    return true;
}

// IMEオン/オフ専用キー (新しめの日本語キーボードが送出する)。古い SDK には無い
#ifndef VK_IME_ON
#define VK_IME_ON 0x16
#endif
#ifndef VK_IME_OFF
#define VK_IME_OFF 0x1A
#endif

// IMEオン/オフをトグルする preserved key。半角/全角キーは修飾キーや IME 状態に
// よって VK_KANJI / VK_OEM_AUTO / VK_OEM_ENLW のいずれかで届くため全て登録する
const TF_PRESERVEDKEY kToggleKeys[] = {
    {VK_KANJI, TF_MOD_IGNORE_ALL_MODIFIER},    // Alt+半角/全角 (漢字キー)
    {VK_OEM_AUTO, TF_MOD_IGNORE_ALL_MODIFIER}, // 半角/全角
    {VK_OEM_ENLW, TF_MOD_IGNORE_ALL_MODIFIER}, // 全角/半角
};
const TF_PRESERVEDKEY kImeOnKey = {VK_IME_ON, TF_MOD_IGNORE_ALL_MODIFIER};
const TF_PRESERVEDKEY kImeOffKey = {VK_IME_OFF, TF_MOD_IGNORE_ALL_MODIFIER};

// F10 (修飾キーなし) は WM_SYSKEYDOWN で届く唯一のファンクションキーで、
// 非 TSF アプリ (WezTerm 等) では OnKeyDown まで届かないことがある。
// Mozc と同様に preserved key として登録し、メッセージ配送前に受け取る
const TF_PRESERVEDKEY kF10Key = {VK_F10, 0};

const wchar_t kToggleKeyDesc[] = L"IMEオン/オフ";
const wchar_t kImeOnKeyDesc[] = L"IMEオン";
const wchar_t kImeOffKeyDesc[] = L"IMEオフ";
const wchar_t kF10KeyDesc[] = L"半角英字変換 (F10)";

} // namespace

TextService::TextService()
    : refCount_(1),
      threadMgr_(nullptr),
      clientId_(TF_CLIENTID_NULL),
      composition_(nullptr),
      inputAttribute_(TF_INVALID_GUIDATOM),
      targetAttribute_(TF_INVALID_GUIDATOM),
      converting_(false),
      segmentIndex_(0),
      segmentsResized_(false),
      barIndex_(-1),
      barX_(0),
      barXFixed_(false),
      barCaret_{},
      rerankId_(0),
      rerankTick_(0),
      rerankDone_(false),
      surfaceCaret_(0),
      surfaceSelectLength_(0),
      promoted_(false),
      appendDocument_(AppendDocument::Unknown),
      awaitingMarker_(false),
      appendAction_(PseudoKeyAction::None),
      appendActionFunc_(KeyFunc::None),
      appendActionEndRun_(false),
      appendActionKeep_(0),
      appendFollowKeyPending_(false),
      appendResendVk_(0),
      appendResendCtrl_(false),
      appendResendShift_(false),
      mouseHook_(nullptr),
      backspaceTested_(false),
      keyEditExpected_(false),
      ownEditDepth_(0),
      textEditSinkContext_(nullptr),
      textEditSinkCookie_(TF_INVALID_COOKIE),
      openCloseCookie_(TF_INVALID_COOKIE),
      threadMgrEventCookie_(TF_INVALID_COOKIE),
      langBarButton_(nullptr)
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
    } else if (IsEqualIID(riid, IID_ITfThreadMgrEventSink)) {
        *ppv = static_cast<ITfThreadMgrEventSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfKeyEventSink)) {
        *ppv = static_cast<ITfKeyEventSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfCompositionSink)) {
        *ppv = static_cast<ITfCompositionSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfCompartmentEventSink)) {
        *ppv = static_cast<ITfCompartmentEventSink*>(this);
    } else if (IsEqualIID(riid, IID_ITfDisplayAttributeProvider)) {
        *ppv = static_cast<ITfDisplayAttributeProvider*>(this);
    } else if (IsEqualIID(riid, IID_ITfTextEditSink)) {
        *ppv = static_cast<ITfTextEditSink*>(this);
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
    if (FAILED(hr)) {
        keystrokeMgr->Release();
        Deactivate();
        return hr;
    }

    // IMEオン/オフの切替キーを preserved key として登録する (失敗しても続行)
    for (const TF_PRESERVEDKEY& key : kToggleKeys) {
        keystrokeMgr->PreserveKey(clientId_, globals::kPreservedKeyToggleGuid, &key,
                                  kToggleKeyDesc, ARRAYSIZE(kToggleKeyDesc) - 1);
    }
    keystrokeMgr->PreserveKey(clientId_, globals::kPreservedKeyImeOnGuid, &kImeOnKey,
                              kImeOnKeyDesc, ARRAYSIZE(kImeOnKeyDesc) - 1);
    keystrokeMgr->PreserveKey(clientId_, globals::kPreservedKeyImeOffGuid, &kImeOffKey,
                              kImeOffKeyDesc, ARRAYSIZE(kImeOffKeyDesc) - 1);
    keystrokeMgr->PreserveKey(clientId_, globals::kPreservedKeyF10Guid, &kF10Key,
                              kF10KeyDesc, ARRAYSIZE(kF10KeyDesc) - 1);
    keystrokeMgr->Release();

    // IMEオン/オフ状態 (OPENCLOSE compartment) の変更監視。
    // 未設定 (VT_I4 以外) なら初期状態はオンにする (従来の常時オン挙動の維持)
    ITfCompartment* compartment = OpenCloseCompartment();
    if (compartment != nullptr) {
        ITfSource* source = nullptr;
        if (SUCCEEDED(compartment->QueryInterface(IID_ITfSource,
                                                  reinterpret_cast<void**>(&source)))) {
            source->AdviseSink(IID_ITfCompartmentEventSink,
                               static_cast<ITfCompartmentEventSink*>(this), &openCloseCookie_);
            source->Release();
        }
        VARIANT value;
        VariantInit(&value);
        const bool unset = FAILED(compartment->GetValue(&value)) || value.vt != VT_I4;
        VariantClear(&value);
        compartment->Release();
        if (unset) {
            SetKeyboardOpen(true);
        }
    }

    // ドキュメントフォーカスの変更通知 (ITfThreadMgrEventSink) の購読 (失敗しても続行)。
    // 設定ファイルの変更をフォーカス切替時に確実に拾うために使う
    // (ITfKeyEventSink::OnSetFocus はアプリ切替で呼ばれないことがある)
    {
        ITfSource* source = nullptr;
        if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfSource,
                                                 reinterpret_cast<void**>(&source)))) {
            source->AdviseSink(IID_ITfThreadMgrEventSink,
                               static_cast<ITfThreadMgrEventSink*>(this),
                               &threadMgrEventCookie_);
            source->Release();
        }
    }

    // 言語バー項目 (タスクバーの IME アイコン) の登録 (失敗しても続行)
    ITfLangBarItemMgr* langBarMgr = nullptr;
    if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfLangBarItemMgr,
                                             reinterpret_cast<void**>(&langBarMgr)))) {
        langBarButton_ = new (std::nothrow) LangBarButton(this);
        if (langBarButton_ != nullptr && FAILED(langBarMgr->AddItem(langBarButton_))) {
            langBarButton_->Release();
            langBarButton_ = nullptr;
        }
        langBarMgr->Release();
    }

    // ユーザ設定の初回読み込み (以後はフォーカス切替・IMEオン時に変更を確認する)
    RefreshConfig();
    ITfDocumentMgr* focus = nullptr;
    if (SUCCEEDED(threadMgr_->GetFocus(&focus)) && focus != nullptr) {
        UpdateTextEditSink(focus);
        focus->Release();
    }
    return S_OK;
}

STDMETHODIMP TextService::Deactivate()
{
    pendingWindow_.Hide();
    CancelAppendAction();
    UpdateTextEditSink(nullptr);
    ClearConversion();
    ClearContext();
    composer_.Clear();
    surface_.clear();
    lastCommitText_.clear();
    promoted_ = false;
    if (composition_ != nullptr) {
        composition_->Release();
        composition_ = nullptr;
    }
    UpdateMouseHook();

    if (threadMgr_ != nullptr) {
        if (threadMgrEventCookie_ != TF_INVALID_COOKIE) {
            ITfSource* source = nullptr;
            if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfSource,
                                                     reinterpret_cast<void**>(&source)))) {
                source->UnadviseSink(threadMgrEventCookie_);
                source->Release();
            }
            threadMgrEventCookie_ = TF_INVALID_COOKIE;
        }
        if (langBarButton_ != nullptr) {
            ITfLangBarItemMgr* langBarMgr = nullptr;
            if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfLangBarItemMgr,
                                                     reinterpret_cast<void**>(&langBarMgr)))) {
                langBarMgr->RemoveItem(langBarButton_);
                langBarMgr->Release();
            }
            langBarButton_->Release();
            langBarButton_ = nullptr;
        }
        if (openCloseCookie_ != TF_INVALID_COOKIE) {
            ITfCompartment* compartment = OpenCloseCompartment();
            if (compartment != nullptr) {
                ITfSource* source = nullptr;
                if (SUCCEEDED(compartment->QueryInterface(
                        IID_ITfSource, reinterpret_cast<void**>(&source)))) {
                    source->UnadviseSink(openCloseCookie_);
                    source->Release();
                }
                compartment->Release();
            }
            openCloseCookie_ = TF_INVALID_COOKIE;
        }
        ITfKeystrokeMgr* keystrokeMgr = nullptr;
        if (SUCCEEDED(threadMgr_->QueryInterface(IID_ITfKeystrokeMgr,
                                                 reinterpret_cast<void**>(&keystrokeMgr)))) {
            for (const TF_PRESERVEDKEY& key : kToggleKeys) {
                keystrokeMgr->UnpreserveKey(globals::kPreservedKeyToggleGuid, &key);
            }
            keystrokeMgr->UnpreserveKey(globals::kPreservedKeyImeOnGuid, &kImeOnKey);
            keystrokeMgr->UnpreserveKey(globals::kPreservedKeyImeOffGuid, &kImeOffKey);
            keystrokeMgr->UnpreserveKey(globals::kPreservedKeyF10Guid, &kF10Key);
            keystrokeMgr->UnadviseKeyEventSink(clientId_);
            keystrokeMgr->Release();
        }
        threadMgr_->Release();
        threadMgr_ = nullptr;
    }
    clientId_ = TF_CLIENTID_NULL;
    return S_OK;
}

// ---- ITfThreadMgrEventSink ----

STDMETHODIMP TextService::OnInitDocumentMgr(ITfDocumentMgr* docMgr)
{
    UNREFERENCED_PARAMETER(docMgr);
    return S_OK;
}

STDMETHODIMP TextService::OnUninitDocumentMgr(ITfDocumentMgr* docMgr)
{
    UNREFERENCED_PARAMETER(docMgr);
    return S_OK;
}

STDMETHODIMP TextService::OnSetFocus(ITfDocumentMgr* focus, ITfDocumentMgr* prevFocus)
{
    UNREFERENCED_PARAMETER(prevFocus);
    // 擬似 Backspace の目印待ちと未完成ローマ字の表示は離れる文書のもの
    CancelAppendAction();
    pendingWindow_.Hide();
    // run は文書を離れた時点で忘れる
    if (InRun()) {
        DiscardPendingRomaji();
        EndRun();
    }
    // 確定アンドゥの記憶も離れる文書のもの。追記のみの文書は照合せずに擬似 Backspace で
    // 消すため、別の文書へ持ち越すと関係ない文字を消してしまう
    lastCommitText_.clear();
    // フォーカス切替は設定ファイルの変更を拾う機会にする (通常は更新時刻の比較のみ)。
    // 設定ツールで保存してアプリに戻る操作自体がフォーカス切替なので、実質すぐ反映される
    RefreshConfig();
    // 読めるかどうかは文書 (アプリ・編集コントロール) の性質で決まるため、文書ごとに判定し直す
    if (appendDocument_ != AppendDocument::Unknown) {
        SetAppendDocument(AppendDocument::Unknown, L"フォーカス移動");
    }
    UpdateTextEditSink(focus);
    // 別のドキュメント (アプリ・編集コントロール) に移った可能性があるため、
    // 直前の確定文脈はここでは信用しない
    ClearContext();
    UpdateMouseHook();
    return S_OK;
}

STDMETHODIMP TextService::OnPushContext(ITfContext* context)
{
    UNREFERENCED_PARAMETER(context);
    return S_OK;
}

STDMETHODIMP TextService::OnPopContext(ITfContext* context)
{
    UNREFERENCED_PARAMETER(context);
    return S_OK;
}

// ---- ITfKeyEventSink ----

STDMETHODIMP TextService::OnSetFocus(BOOL foreground)
{
    UNREFERENCED_PARAMETER(foreground);
    return S_OK;
}

void TextService::RefreshConfig()
{
    if (config_.Refresh()) {
        candidateWindow_.SetFont(config_.Get().candidateFont, config_.Get().candidateFontSize);
        pendingWindow_.SetFont(config_.Get().candidateFont, config_.Get().candidateFontSize);
    }
    // 自動英字判定の有効/無効はコンポーザが持つ。Clear() でも維持されるフラグなので
    // 設定を読む機会ごとに渡し直すだけでよい
    composer_.SetModeless(config_.Get().modeless);
}

KeyState TextService::CurrentKeyState() const
{
    if (converting_) {
        return KeyState::Candidate;
    }
    if (barIndex_ >= 0) {
        return KeyState::Suggest;
    }
    return (InRun() || Composing()) ? KeyState::Run : KeyState::Idle;
}

KeyMatch TextService::MatchKeyFunc(WPARAM wparam) const
{
    if ((GetKeyState(VK_LWIN) & 0x8000) != 0 || (GetKeyState(VK_RWIN) & 0x8000) != 0) {
        return {};
    }
    KeyBinding pressed;
    pressed.ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    pressed.alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    pressed.shift = IsShiftPressed();
    pressed.vk = static_cast<UINT>(wparam);
    const KeyState state = CurrentKeyState();
    // モードレスの英字モード中の Space は、候補選択中でも英文の語の区切りとして
    // 確定して半角スペースを入れる (割当より優先する)
    if (state == KeyState::Candidate && wparam == VK_SPACE && !pressed.ctrl && !pressed.alt &&
        config_.Get().modeless && composer_.AsciiMode()) {
        return {};
    }
    return config_.Get().FindFunc(state, pressed);
}

bool TextService::CanRunKeyFunc(ITfContext* context, KeyFunc func) const
{
    if (CurrentKeyState() != KeyState::Idle) {
        return true;
    }
    switch (func) {
    case KeyFunc::Convert: {
        std::wstring selection;
        return ReadReconvertibleSelection(context, &selection);
    }
    case KeyFunc::UndoCommit:
        return !lastCommitText_.empty();
    default:
        return true;
    }
}

bool TextService::IsKeyEaten(ITfContext* context, WPARAM wparam) const
{
    // IMEオフ中は何も食べない (半角/全角キーなどは preserved key として
    // key event sink より先に処理されるため、ここには来ない)
    if (!IsKeyboardOpen()) {
        return false;
    }
    const KeyMatch match = MatchKeyFunc(wparam);
    if (match.func != KeyFunc::None) {
        return CanRunKeyFunc(context, match.func);
    }
    if (!Composing()) {
        return IsKeyEatenDirect(wparam);
    }

    // 候補選択中 (run を昇格した composition)。Ctrl / Alt 併用は原則アプリの
    // ショートカットなので確定してから渡すが、Ctrl+H (変換の取消) だけは IME が処理する。
    // Enter・Ctrl+M は、読める文書では食べずに EndRunIfPassthroughKey で確定してから
    // アプリへ渡し、そうでなければ食べて確定してから送り直す (EnterNeedsResend)
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    if (ctrl || alt) {
        return ctrl && !alt && (wparam == 'H' || (wparam == 'M' && EnterNeedsResend()));
    }
    const bool shifted = IsShiftPressed();

    // 候補選択中は編集キーも IME が処理する
    switch (wparam) {
    case VK_RETURN:
        return EnterNeedsResend();
    case VK_ESCAPE:
    case VK_BACK:
    case VK_SPACE:
    case VK_TAB:
    case VK_UP:
    case VK_DOWN:
        // Tab (変換を取り消してバーの先頭を選ぶ)・↑↓ は常に食べる
        // (アプリにキャレットやフォーカスを動かさせない)。用途が無い状態では
        // 食べた上で何もしない
        return true;
    case VK_LEFT:
    case VK_RIGHT:
    case VK_PRIOR:
    case VK_NEXT:
        // ←→・PgUp/PgDn も文節 UI では同じく常に食べる。入力全体の候補選択には文節の
        // 操作が無いので、Home / End と同じく確定してアプリへ渡す (EndRunIfPassthroughKey)
        return !converting_ || config_.Get().segmentUi;
    default:
        break;
    }
    // 英字 (Shift併用含む)・数字・記号・テンキーは確定して新しい run を始める
    return IsLetterKey(wparam) || IsDigitKey(wparam) ||
           FindSymbolKey(wparam, shifted) != nullptr || NumpadChar(wparam) != 0;
}

STDMETHODIMP TextService::OnTestKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                        BOOL* eaten)
{
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    if (!HandlePseudoKey(context, wparam, false, eaten)) {
        // 食べないキーはホストが OnKeyDown を呼ばずに処理することがあるため、
        // run の終了はここでも行う
        EndRunIfPassthroughKey(context, wparam);
        *eaten = IsKeyEaten(context, wparam) ? TRUE : FALSE;
        NoteKeyForAppend(context, wparam, *eaten != FALSE, true);
    }
    UpdateMouseHook();
    return S_OK;
}

STDMETHODIMP TextService::OnKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                    BOOL* eaten)
{
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    if (HandlePseudoKey(context, wparam, true, eaten)) {
        UpdateMouseHook();
        return S_OK;
    }
    EndRunIfPassthroughKey(context, wparam);
    *eaten = IsKeyEaten(context, wparam) ? TRUE : FALSE;
    NoteKeyForAppend(context, wparam, *eaten != FALSE, false);
    if (*eaten == FALSE) {
        UpdateMouseHook();
        return S_OK;
    }
    if (wparam < pendingKeyUps_.size()) {
        // 対応する key-up も食べる (端末エミュレータは key-up もアプリへ転送し、
        // 確定の Enter などが素通りすると入力の誤送信につながるため)
        pendingKeyUps_.set(wparam);
    }
    // eaten=TRUE を立てた以上、HandleKey の失敗はホストへ返さない
    // (失敗 HRESULT を返すと「未処理」とみなして打鍵文字をそのまま挿入する
    //  ホストがあるため。内部の失敗はその打鍵が効かないだけに留める)
    HandleKey(context, wparam);
    UpdateMouseHook();
    return S_OK;
}

STDMETHODIMP TextService::OnTestKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam,
                                      BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    *eaten = (wparam < pendingKeyUps_.size() && pendingKeyUps_.test(wparam)) ? TRUE : FALSE;
    return S_OK;
}

STDMETHODIMP TextService::OnKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(lparam);

    if (eaten == nullptr) {
        return E_INVALIDARG;
    }
    if (wparam < pendingKeyUps_.size() && pendingKeyUps_.test(wparam)) {
        pendingKeyUps_.reset(wparam);
        *eaten = TRUE;
    } else {
        *eaten = FALSE;
    }
    return S_OK;
}

STDMETHODIMP TextService::OnPreservedKey(ITfContext* context, REFGUID rguid, BOOL* eaten)
{
    if (eaten == nullptr) {
        return E_INVALIDARG;
    }

    if (IsEqualGUID(rguid, globals::kPreservedKeyF10Guid)) {
        // 現在の状態で F10 に一致する割当が無いとき (割当変更で外したとき、入力なしの
        // 状態など) はアプリへ再送する
        const KeyFunc func = (context != nullptr && IsKeyboardOpen())
                                 ? MatchKeyFunc(VK_F10).func
                                 : KeyFunc::None;
        if (func != KeyFunc::None && CanRunKeyFunc(context, func)) {
            *eaten = TRUE;
            const HRESULT hr = HandleKey(context, VK_F10);
            UpdateMouseHook();
            return hr;
        }
        // IME が使わないときは eaten=FALSE だけではアプリに F10 が届かないため、
        // Mozc と同様に WM_SYSKEYDOWN を合成してアプリ本来の F10 動作を再現する
        *eaten = FALSE;
        HWND focus = GetFocus();
        if (focus != nullptr) {
            const LPARAM lparam =
                (static_cast<LPARAM>(MapVirtualKeyW(VK_F10, MAPVK_VK_TO_VSC)) << 16) | 1;
            PostMessageW(focus, WM_SYSKEYDOWN, VK_F10, lparam);
        }
        return S_OK;
    }

    bool open;
    if (IsEqualGUID(rguid, globals::kPreservedKeyToggleGuid)) {
        open = !IsKeyboardOpen();
    } else if (IsEqualGUID(rguid, globals::kPreservedKeyImeOnGuid)) {
        open = true;
    } else if (IsEqualGUID(rguid, globals::kPreservedKeyImeOffGuid)) {
        open = false;
    } else {
        *eaten = FALSE;
        return S_OK;
    }
    *eaten = TRUE;

    if (!open && Composing()) {
        // オフにする前に候補選択中の文字列を確定する
        CommitComposition(context);
    }
    SetKeyboardOpen(open);
    UpdateMouseHook();
    return S_OK;
}

// ---- IMEオン/オフ (OPENCLOSE compartment) ----

ITfCompartment* TextService::OpenCloseCompartment() const
{
    if (threadMgr_ == nullptr) {
        return nullptr;
    }
    ITfCompartmentMgr* compartmentMgr = nullptr;
    if (FAILED(threadMgr_->QueryInterface(IID_ITfCompartmentMgr,
                                          reinterpret_cast<void**>(&compartmentMgr)))) {
        return nullptr;
    }
    ITfCompartment* compartment = nullptr;
    compartmentMgr->GetCompartment(GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, &compartment);
    compartmentMgr->Release();
    return compartment;
}

bool TextService::IsKeyboardOpen() const
{
    ITfCompartment* compartment = OpenCloseCompartment();
    if (compartment == nullptr) {
        // 取得できない環境ではオン扱い (従来の常時オン挙動)
        return true;
    }
    bool open = true;
    VARIANT value;
    VariantInit(&value);
    if (SUCCEEDED(compartment->GetValue(&value)) && value.vt == VT_I4) {
        open = value.lVal != 0;
    }
    VariantClear(&value);
    compartment->Release();
    return open;
}

void TextService::SetKeyboardOpen(bool open)
{
    ITfCompartment* compartment = OpenCloseCompartment();
    if (compartment == nullptr) {
        return;
    }
    VARIANT value;
    VariantInit(&value);
    value.vt = VT_I4;
    value.lVal = open ? 1 : 0;
    compartment->SetValue(clientId_, &value);
    compartment->Release();
}

// ---- ITfCompartmentEventSink ----

STDMETHODIMP TextService::OnChange(REFGUID rguid)
{
    if (!IsEqualGUID(rguid, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)) {
        return S_OK;
    }
    // IMEオン/オフの切替も設定ファイルの変更を拾う機会にする
    RefreshConfig();
    // オフの間にキャレットが動かされる可能性があるため、次にオンに戻ったときの
    // 誤った文脈補正を避けるためオフになった時点で文脈を破棄する
    // (run もオフの時点で忘れる)
    if (!IsKeyboardOpen()) {
        CancelAppendAction();
        pendingWindow_.Hide();
        if (InRun()) {
            DiscardPendingRomaji();
            EndRun();
        }
        ClearContext();
    }
    if (langBarButton_ != nullptr) {
        langBarButton_->NotifyUpdate();
    }
    // 外部要因 (言語バーのクリックやアプリからの切替) でオフになったら、
    // 候補選択中の文字列を確定して後始末する。自前の preserved key 経由では
    // 確定済みなのでここでは何も起きない
    if (!IsKeyboardOpen() && Composing()) {
        ITfDocumentMgr* docMgr = nullptr;
        if (threadMgr_ != nullptr && SUCCEEDED(threadMgr_->GetFocus(&docMgr)) &&
            docMgr != nullptr) {
            ITfContext* context = nullptr;
            if (SUCCEEDED(docMgr->GetTop(&context)) && context != nullptr) {
                CommitComposition(context);
                context->Release();
            }
            docMgr->Release();
        }
    }
    UpdateMouseHook();
    return S_OK;
}

// ---- ITfCompositionSink ----

STDMETHODIMP TextService::OnCompositionTerminated(TfEditCookie ecWrite,
                                                  ITfComposition* composition)
{
    UNREFERENCED_PARAMETER(ecWrite);
    UNREFERENCED_PARAMETER(composition);

    // アプリ側の操作 (クリックなど) で composition が終了した。候補選択中の状態を捨てる
    ClearConversion();
    ClearBar();
    barXFixed_ = false;
    composer_.Clear();
    promoted_ = false;
    if (composition_ != nullptr) {
        composition_->Release();
        composition_ = nullptr;
    }
    UpdateMouseHook();
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

// ---- ITfTextEditSink ----

STDMETHODIMP TextService::OnEndEdit(ITfContext* context, TfEditCookie ecReadOnly,
                                   ITfEditRecord* editRecord)
{
    UNREFERENCED_PARAMETER(context);
    UNREFERENCED_PARAMETER(ecReadOnly);

    if (editRecord == nullptr) {
        return S_OK;
    }
    BOOL selectionChanged = FALSE;
    if (FAILED(editRecord->GetSelectionStatus(&selectionChanged)) || !selectionChanged) {
        return S_OK;
    }
    // 自分の同期 edit session の終わりに呼ばれたもの、またはアプリへ渡した Backspace・
    // 擬似 Backspace でアプリが文書を編集したものは IME 由来とみなす
    const bool own = ownEditDepth_ > 0;
    const bool keyEdit = keyEditExpected_ || awaitingMarker_;
    DebugLog(std::wstring(L"OnEndEdit 選択変更 own=") + (own ? L"1" : L"0") +
             L" keyEdit=" + (keyEdit ? L"1" : L"0") + L" run=" + (InRun() ? L"1" : L"0") +
             L" doc=" + AppendDocumentName(appendDocument_));
    if (own || keyEdit) {
        return S_OK;
    }
    // 読める文書は次の追記の照合でキャレット移動を検出できる
    if (appendDocument_ != AppendDocument::AppendOnly || !InRun()) {
        return S_OK;
    }
    DebugLog(L"追記のみの文書で run を終了: OnEndEdit の選択変更");
    pendingWindow_.Hide();
    DiscardPendingRomaji();
    EndRun();
    ClearContext();
    UpdateMouseHook();
    return S_OK;
}

void TextService::UpdateTextEditSink(ITfDocumentMgr* docMgr)
{
    if (textEditSinkContext_ != nullptr) {
        ITfSource* source = nullptr;
        if (SUCCEEDED(textEditSinkContext_->QueryInterface(IID_ITfSource,
                                                           reinterpret_cast<void**>(&source)))) {
            source->UnadviseSink(textEditSinkCookie_);
            source->Release();
        }
        textEditSinkContext_->Release();
        textEditSinkContext_ = nullptr;
        textEditSinkCookie_ = TF_INVALID_COOKIE;
    }
    if (docMgr == nullptr) {
        return;
    }
    ITfContext* context = nullptr;
    if (FAILED(docMgr->GetTop(&context)) || context == nullptr) {
        return;
    }
    bool advised = false;
    ITfSource* source = nullptr;
    if (SUCCEEDED(context->QueryInterface(IID_ITfSource, reinterpret_cast<void**>(&source)))) {
        advised = SUCCEEDED(source->AdviseSink(IID_ITfTextEditSink,
                                               static_cast<ITfTextEditSink*>(this),
                                               &textEditSinkCookie_));
        source->Release();
    }
    if (advised) {
        textEditSinkContext_ = context;
    } else {
        textEditSinkCookie_ = TF_INVALID_COOKIE;
        context->Release();
    }
    DebugLog(std::wstring(L"ITfTextEditSink の登録: ") + (advised ? L"成功" : L"失敗"));
}

// ---- 状態機械 ----

HRESULT TextService::CommitComposition(ITfContext* context)
{
    if (!Composing()) {
        return S_OK;
    }
    if (converting_) {
        return CommitConversion(context);
    }
    // 昇格した composition は変換状態を抜けると run に戻るので、ここに来るのは
    // 後始末が間に合わなかったときだけ。表示どおりのかなで確定する
    return EndComposition(context, composer_.Commit());
}

void TextService::ApplyModelessCommitRule()
{
    if (converting_ || barIndex_ >= 0) {
        return;
    }
    composer_.FinishForCommit();
    ResolveAsciiRequest();
}

void TextService::ResolveAsciiRequest()
{
    if (!composer_.AsciiRequested()) {
        return;
    }
    const size_t element = engine_.AsciiStartLive(composer_.AsciiRequestElements(),
                                                  composer_.AsciiRequestAtCommit());
    composer_.ConfirmAscii(element);
    DebugLog(L"英字区間を始める要素: " + std::to_wstring(element) + L" (かなの位置 " +
             std::to_wstring(composer_.AsciiStart()) + L")");
}

bool TextService::SplitAsciiRun() const
{
    return config_.Get().modeless && composer_.AsciiMode() && composer_.AsciiStart() > 0;
}

std::wstring TextService::StandaloneSpaceText() const
{
    // 英字モードの run を Space で終えた直後も英文の途中なので、
    // 続く Space も半角にする (lastCommitText_ の末尾には Space 自身が入っている)。
    // モードレスが無効なら英字モードは Shift 由来だけで、Space の幅も従来どおり設定に従う
    if (config_.Get().modeless && IsAsciiAlnumText(lastCommitText_)) {
        return L" ";
    }
    return config_.Get().spaceFullwidth ? L"　" : L" ";
}

void TextService::SetCommitContext(const std::wstring& reading, const std::wstring& surface)
{
    if (reading.empty() || surface.empty()) {
        ClearContext();
        return;
    }
    contextReading_ = reading;
    contextSurface_ = surface;
}

void TextService::ClearContext()
{
    contextReading_.clear();
    contextSurface_.clear();
    // 前文脈を捨てる場面 (キャレット移動・フォーカス移動など) では、IME が入れた文字列も
    // キャレットの前にあるとは限らない
    llmHistory_.clear();
}

HRESULT TextService::HandleKey(ITfContext* context, WPARAM wparam)
{
    // 設定の反映は RefreshConfig (フォーカス切替・IMEオン切替) で行うが、
    // その通知が来ないホストでも自動英字判定が設定どおりになるよう毎打鍵で渡し直す
    composer_.SetModeless(config_.Get().modeless);

    const KeyMatch match = MatchKeyFunc(wparam);
    HRESULT hr;
    if (match.func != KeyFunc::None) {
        hr = RunKeyFunc(context, match);
    } else {
        hr = Composing() ? HandleKeyConverting(context, wparam) : HandleKeyDirect(context, wparam);
    }
    // 昇格した composition が変換状態を抜けた (変換取消・印字キーで確定して次の
    // composition が始まった・F4 で候補が無かった) なら run に戻す
    DemoteIfLeftConversion(context);
    return hr;
}

HRESULT TextService::RunKeyFunc(ITfContext* context, const KeyMatch& match)
{
    switch (match.func) {
    case KeyFunc::Convert:
        if (Composing()) {
            return converting_ ? CycleCandidate(context, match.shiftAdded ? -1 : +1)
                               : StartConversion(context);
        }
        return ConvertKeyDirect(context, match.shiftAdded);
    case KeyFunc::NextCandidate:
        return CycleCandidate(context, +1);
    case KeyFunc::PrevCandidate:
        return CycleCandidate(context, -1);
    case KeyFunc::CommitRun:
        // 打鍵はアプリへ渡さないので、確定アンドゥの記憶は残る
        if (Composing()) {
            return CommitComposition(context);
        }
        if (converting_) {
            return CommitRunDirect(context);
        }
        if (barIndex_ >= 0) {
            return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::CommitKey, L"",
                                nullptr);
        }
        return CommitRunKey(context);
    case KeyFunc::UndoCommit:
        return UndoCommit(context);
    case KeyFunc::RegisterWord:
        return LaunchWordRegister(context);
    case KeyFunc::OpenConfig:
        return LaunchConfigTool();
    case KeyFunc::None:
        return S_OK;
    default:
        // 文字種変換・記号変換・ユーザ語変換は変換状態に入るキーなので、run 中は
        // composition に昇格してから適用する。昇格できなかった run の候補選択中は、
        // 選択による強調のまま適用する
        if (Composing() || converting_) {
            return ApplyFunctionKey(context, match.func);
        }
        return BeginAppendConversion(context, match.func);
    }
}

HRESULT TextService::HandleKeyConverting(ITfContext* context, WPARAM wparam)
{
    // Ctrl 併用ショートカット: Ctrl+H は BackSpace、Ctrl+M は Enter として扱う。
    // (英字の打鍵と解釈されないよう、ここで読み替えてから通常の処理に流す)
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const WPARAM originalVk = wparam;
    if (ctrl) {
        switch (wparam) {
        case 'H':
            wparam = VK_BACK;
            break;
        case 'M':
            wparam = VK_RETURN;
            break;
        default:
            return S_OK; // IsKeyEaten が食べる Ctrl 併用は上記のみ
        }
    }

    const bool shifted = IsShiftPressed();

    // 印字キー: 選択中の候補を確定して新しい run を始める。確定と新しい composition の
    // 開始を1つの edit session で行い、その composition は打鍵の処理後に run へ戻す
    // (DemoteIfLeftConversion)
    DirectKey key;
    if (ClassifyDirectKey(wparam, shifted, &key)) {
        // 数字 1〜9 は候補番号による直接選択
        if (!shifted && wparam >= '1' && wparam <= '9' && converting_) {
            return SelectCandidateByNumber(context, wparam - '1');
        }
        if (!converting_) {
            return S_OK;
        }
        const std::wstring commitText = PrepareConversionCommit();
        FinishConversionState(commitText);
        PushDirectKey(key);
        HRESULT hr = RestartComposition(context, commitText);
        if (FAILED(hr)) {
            composer_.Clear();
            return hr;
        }
        // 新しい composition はまだ空なので、キャレットの前は確定した文字列で終わっている
        CaptureLlmContext(context);
        return UpdateCompositionAndBar(context);
    }

    switch (wparam) {
    case VK_RETURN:
        // 食べるのは EnterNeedsResend のときだけ
        return CommitAndResendEnter(context, originalVk, ctrl, shifted);
    case VK_ESCAPE:
    case VK_BACK:
        // 変換を取り消して変換前のかな表示に戻る (その後 run に戻す)
        return converting_ ? CancelConversion(context) : S_OK;
    case VK_SPACE: {
        // 割当 (既定は次候補・前候補) に一致しない Space は、昇格できなかった run の
        // 候補選択中と同じく確定してスペースを入れる。英字モード中は英文の語の区切り
        // なので設定 space によらず半角
        const bool asciiWord = config_.Get().modeless && composer_.AsciiMode();
        const std::wstring space =
            (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
        HRESULT hr = CommitComposition(context);
        if (FAILED(hr)) {
            return hr;
        }
        hr = InsertText(context, space);
        // 確定アンドゥはスペースまで含めて戻す (run を Space で終えたときと同じ)。
        // 追記のみの文書は確定文字列を照合せずに擬似 Backspace で消すため、文書の
        // 末尾と記憶を揃えておく必要がある
        if (SUCCEEDED(hr) && !lastCommitText_.empty()) {
            lastCommitText_ += space;
        }
        return hr;
    }
    case VK_TAB:
        // 変換を取り消してバーの先頭を選ぶ (バーが無ければかな表示に戻るだけ)。
        // 先に run へ戻し、選択は run のバー選択として行う
        if (converting_) {
            HRESULT hr = CancelConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
            DemoteIfLeftConversion(context);
            return MoveBarSelection(+1);
        }
        return S_OK;
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
    case VK_PRIOR:
        // PgUp: 先頭の文節へ移動
        return converting_ ? MoveSegmentTo(context, 0) : S_OK;
    case VK_NEXT:
        // PgDn: 末尾の文節へ移動
        return converting_ ? MoveSegmentTo(context, segments_.size() - 1) : S_OK;
    default:
        return S_OK;
    }
}

HRESULT TextService::ApplyFunctionKey(ITfContext* context, KeyFunc func)
{
    switch (func) {
    case KeyFunc::ConvertSymbol:
        return ConvertToSymbols(context);
    case KeyFunc::ConvertUser:
        return ConvertToShortcuts(context);
    case KeyFunc::ToHiragana:
        return DirectConvert(context, ConversionForm::Hiragana);
    case KeyFunc::ToKatakana:
        return DirectConvert(context, ConversionForm::Katakana);
    case KeyFunc::ToHalfKatakana:
        return DirectConvert(context, ConversionForm::HalfwidthKatakana);
    case KeyFunc::ToFullAscii:
        return DirectConvert(context, ConversionForm::FullwidthAscii);
    case KeyFunc::ToHalfAscii:
        return DirectConvert(context, ConversionForm::HalfwidthAscii);
    default:
        return S_OK;
    }
}

// ---- 変換 (文節と候補の選択) ----

HRESULT TextService::StartConversion(ITfContext* context)
{
    if (!Composing() && !InRun()) {
        return E_UNEXPECTED;
    }
    BuildConversionSegments();
    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

void TextService::BuildConversionSegments()
{
    // 並べ替えの結果は ClearBar の後も覚えている (候補の取得で使う)
    ClearBar();

    // 文節列をエンジンに問い合わせる。
    // エンジンが起動していない場合はひらがな1文節のみで動作を継続する。
    // 英字が残っている入力 (英単語の打鍵など) は文節分割しても意味を
    // 成さないため、全体を1文節に固定して変換する
    const std::wstring kana = composer_.Commit();
    wholeCandidates_.clear();
    bool ok;
    if (ContainsAsciiLetter(kana)) {
        ok = engine_.ConvertSegmentsFixed(kana, {kana.size()}, CurrentContext(), &segments_);
    } else if (config_.Get().segmentUi) {
        ok = engine_.ConvertSegments(kana, CurrentContext(), &segments_);
    } else {
        // 入力全体を1文節として候補を選ぶ。候補ごとの文節は確定時の学習に使う。
        // この読みの並べ替えの結果が出ていればその並びを使う (待たない)
        ok = TakeRerankedCandidates(kana, &wholeCandidates_) ||
             engine_.ConvertNBest(kana, CurrentContext(), &wholeCandidates_);
        if (ok) {
            ConversionSegment segment;
            segment.reading = kana;
            for (const SentenceCandidate& candidate : wholeCandidates_) {
                segment.candidates.push_back(candidate.surface);
            }
            segments_.assign(1, std::move(segment));
        }
    }
    if (!ok || segments_.empty()) {
        segments_.clear();
        wholeCandidates_.clear();
        ConversionSegment fallback;
        fallback.reading = kana;
        fallback.candidates.push_back(kana);
        segments_.push_back(std::move(fallback));
    }
    // 全体が1文節なら、打鍵したローマ字をそのまま半角候補として加える
    // (「apple」と打って apple / Apple / APPLE を選べるようにする)。
    // 英字を含む入力ではかな漢字候補が役に立たないため先頭候補の直後へ、
    // 通常のかな入力では邪魔にならないよう末尾へ入れる
    if (segments_.size() == 1) {
        const size_t position =
            ContainsAsciiLetter(kana) ? 1 : segments_[0].candidates.size();
        InsertRawCandidates(&segments_[0], composer_.Raw(), position);
    }
    selected_.assign(segments_.size(), 0);
    segmentIndex_ = 0;
    converting_ = true;

    // 対記号の既定候補を両側で揃える (学習で片側だけ変わっている場合など)
    for (size_t i = 0; i < segments_.size(); ++i) {
        SyncPairedSegment(i);
    }
}

void TextService::SyncPairedSegment(size_t index)
{
    const std::wstring& reading = segments_[index].reading;
    size_t partner = segments_.size(); // 「見つからない」の印
    bool wantClose = false;            // 相手の文節に閉じ形を入れるか

    if (const wchar_t* closeReading = CloseReadingForOpen(reading)) {
        // 開き記号: 同じ深さの閉じ記号を前方に探す
        int depth = 0;
        for (size_t j = index + 1; j < segments_.size(); ++j) {
            if (segments_[j].reading == reading) {
                ++depth;
            } else if (segments_[j].reading == closeReading) {
                if (depth == 0) {
                    partner = j;
                    break;
                }
                --depth;
            }
        }
        wantClose = true;
    } else if (const wchar_t* openReading = OpenReadingForClose(reading)) {
        // 閉じ記号: 同じ深さの開き記号を後方に探す
        int depth = 0;
        for (size_t j = index; j-- > 0;) {
            if (segments_[j].reading == reading) {
                ++depth;
            } else if (segments_[j].reading == openReading) {
                if (depth == 0) {
                    partner = j;
                    break;
                }
                --depth;
            }
        }
        wantClose = false;
    } else if (IsSymmetricQuoteReading(reading)) {
        // 左右同形のクオート: 同じ読みの文節が交互に開き/閉じでペアになる
        size_t precedingCount = 0;
        for (size_t j = 0; j < index; ++j) {
            if (segments_[j].reading == reading) {
                ++precedingCount;
            }
        }
        if (precedingCount % 2 == 0) {
            for (size_t j = index + 1; j < segments_.size(); ++j) {
                if (segments_[j].reading == reading) {
                    partner = j;
                    break;
                }
            }
            wantClose = true;
        } else {
            for (size_t j = index; j-- > 0;) {
                if (segments_[j].reading == reading) {
                    partner = j;
                    break;
                }
            }
            wantClose = false;
        }
    } else {
        return; // 対記号の文節ではない
    }

    if (partner >= segments_.size()) {
        return; // 対の相手が入力に無い (片側だけの入力)
    }

    const std::wstring& current = segments_[index].candidates[selected_[index]];
    const wchar_t* partnerText = PartnerSymbolText(current, wantClose);
    if (partnerText == nullptr) {
        return;
    }
    auto& candidates = segments_[partner].candidates;
    auto it = std::find(candidates.begin(), candidates.end(), partnerText);
    if (it == candidates.end()) {
        candidates.push_back(partnerText);
        it = candidates.end() - 1;
    }
    selected_[partner] = static_cast<size_t>(it - candidates.begin());
}

HRESULT TextService::CycleCandidate(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }
    const size_t count = segments_[segmentIndex_].candidates.size();
    return ApplyCandidateSelection(context,
                                   (selected_[segmentIndex_] + count + delta) % count);
}

HRESULT TextService::SelectCandidateByNumber(ITfContext* context, size_t number)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }
    // 候補ウィンドウは選択位置を含むページを表示していて、行頭の番号は
    // ページ内相対。番号もそのページ内の候補に対応付ける
    const size_t page = selected_[segmentIndex_] / CandidateWindow::kPageSize;
    const size_t index = page * CandidateWindow::kPageSize + number;
    if (index >= segments_[segmentIndex_].candidates.size()) {
        return S_OK; // 表示されていない番号は無視する
    }
    return ApplyCandidateSelection(context, index);
}

HRESULT TextService::ApplyCandidateSelection(ITfContext* context, size_t index)
{
    selected_[segmentIndex_] = index;
    SyncPairedSegment(segmentIndex_);
    HRESULT hr = UpdateConvertingDisplay(context);
    if (candidateWindow_.Visible()) {
        candidateWindow_.SetSelection(selected_[segmentIndex_]);
    } else {
        ShowCandidateWindow(context); // F7-F10 で閉じた後の候補送りでは出し直す
    }
    return hr;
}

HRESULT TextService::MoveSegment(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }
    const size_t count = segments_.size();
    return MoveSegmentTo(context, (segmentIndex_ + count + delta) % count);
}

HRESULT TextService::MoveSegmentTo(ITfContext* context, size_t index)
{
    if (!converting_ || index >= segments_.size()) {
        return E_UNEXPECTED;
    }
    segmentIndex_ = index;
    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context); // 候補一覧を現在文節のものに差し替える
    return hr;
}

HRESULT TextService::ResizeSegment(ITfContext* context, int delta)
{
    if (!converting_ || segments_.empty()) {
        return E_UNEXPECTED;
    }

    // 確定時に「区切り直し」と「複合語を割って入力した」を見分けるため、
    // 人が触る前の分割を1度だけ覚えておく
    if (!segmentsResized_) {
        preResizeLengths_.clear();
        preResizeLengths_.reserve(segments_.size());
        for (const ConversionSegment& segment : segments_) {
            preResizeLengths_.push_back(segment.reading.size());
        }
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

    // 文節 i を新しい長さで固定変換する。文脈は i が先頭文節のときだけ外部文脈を渡す
    // (2文節目以降はエンジン側で連鎖しないため、ここでは前の文節の表記を渡す)
    const ConversionContext segmentContext =
        i == 0 ? CurrentContext() : ConversionContext{segments_[i - 1].reading,
                                                       segments_[i - 1].candidates[selected_[i - 1]]};
    const std::wstring segmentKana = kana.substr(prefixLen, newLen);
    std::vector<ConversionSegment> fixedResult;
    if (!engine_.ConvertSegmentsFixed(segmentKana, {newLen}, segmentContext, &fixedResult) ||
        fixedResult.empty()) {
        return S_OK; // エンジン不調時は現状維持
    }

    // 残りは境界を固定せず自由に再変換する (文脈は新しく固定した文節 i を渡す)
    std::vector<ConversionSegment> tailResult;
    const std::wstring tailKana = kana.substr(prefixLen + newLen);
    const ConversionContext tailContext{fixedResult[0].reading, fixedResult[0].candidates[0]};
    if (!tailKana.empty() &&
        (!engine_.ConvertSegments(tailKana, tailContext, &tailResult) || tailResult.empty())) {
        return S_OK; // エンジン不調時は現状維持
    }

    segments_.resize(i);
    segments_.push_back(std::move(fixedResult[0]));
    for (ConversionSegment& segment : tailResult) {
        segments_.push_back(std::move(segment));
    }
    selected_.resize(i);
    selected_.resize(segments_.size(), 0);
    segmentsResized_ = true;

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

// ---- ファンクションキー変換 (F4, F6-F10) ----

void TextService::EnsureConversionState()
{
    if (converting_) {
        return;
    }
    ClearBar();
    ConversionSegment segment;
    segment.reading = composer_.Commit();
    segment.candidates.push_back(segment.reading);
    segments_.clear();
    segments_.push_back(std::move(segment));
    selected_.assign(1, 0);
    segmentIndex_ = 0;
    converting_ = true;
}

std::wstring TextService::SegmentRawText(size_t index) const
{
    // この文節の読みに対応する打鍵列を composer から切り出す
    // (文節読みの連結 = composer_.Commit() であることを前提にできる)
    size_t start = 0;
    for (size_t i = 0; i < index; ++i) {
        start += segments_[i].reading.size();
    }
    return composer_.RawRange(start, segments_[index].reading.size());
}

std::wstring TextService::SegmentFormText(size_t index, ConversionForm form) const
{
    const std::wstring& reading = segments_[index].reading;
    switch (form) {
    case ConversionForm::Hiragana:
        return reading;
    case ConversionForm::Katakana:
        return kana_forms::ToKatakana(reading);
    case ConversionForm::HalfwidthKatakana:
        return kana_forms::ToHalfwidth(reading);
    case ConversionForm::FullwidthAscii:
        return kana_forms::ToFullwidthAscii(SegmentRawText(index));
    case ConversionForm::HalfwidthAscii:
        return SegmentRawText(index);
    }
    return {};
}

std::wstring TextService::NextFormText(size_t index, ConversionForm form) const
{
    std::vector<std::wstring> variants;
    switch (form) {
    case ConversionForm::Hiragana:
        return SegmentFormText(index, form); // 循環しない単一の形
    case ConversionForm::Katakana:
    case ConversionForm::HalfwidthKatakana:
        variants = KatakanaCycleVariants(segments_[index].reading,
                                         form == ConversionForm::HalfwidthKatakana);
        break;
    case ConversionForm::FullwidthAscii:
    case ConversionForm::HalfwidthAscii: {
        const std::wstring raw = SegmentRawText(index);
        if (raw.empty()) {
            return {};
        }
        variants = CaseCycleVariants(raw);
        if (form == ConversionForm::FullwidthAscii) {
            for (std::wstring& variant : variants) {
                variant = kana_forms::ToFullwidthAscii(variant);
            }
        }
        break;
    }
    }
    if (variants.empty()) {
        return {};
    }

    // 現在の選択が循環列のどれかなら次の形へ、そうでなければ先頭から
    const std::wstring& current = segments_[index].candidates[selected_[index]];
    auto it = std::find(variants.begin(), variants.end(), current);
    if (it == variants.end()) {
        return variants.front();
    }
    return variants[(static_cast<size_t>(it - variants.begin()) + 1) % variants.size()];
}

HRESULT TextService::DirectConvert(ITfContext* context, ConversionForm form)
{
    if (!Composing() && !InRun()) {
        return E_UNEXPECTED;
    }
    EnsureConversionState();

    // F7-F10 は連打で形を循環させる (F7/F8: 後ろから1文字ずつひらがなへ、
    // F9/F10: 大文字小文字の切り替え)。F6 は単一の形 (ひらがな) を選ぶだけ
    const std::wstring converted = NextFormText(segmentIndex_, form);
    if (converted.empty()) {
        return S_OK; // 打鍵列を切り出せない場合などは何もしない
    }

    // 変換結果を候補に加えて (既にあればそれを) 選択状態にする
    auto& candidates = segments_[segmentIndex_].candidates;
    auto it = std::find(candidates.begin(), candidates.end(), converted);
    if (it == candidates.end()) {
        candidates.push_back(converted);
        it = candidates.end() - 1;
    }
    selected_[segmentIndex_] = static_cast<size_t>(it - candidates.begin());
    SyncPairedSegment(segmentIndex_);

    HRESULT hr = UpdateConvertingDisplay(context);
    // 直接変換は結果が一意 (または連打で循環) なので候補ウィンドウは出さない
    // (表示中なら閉じる)
    candidateWindow_.Hide();
    return hr;
}

HRESULT TextService::ConvertToSymbols(ITfContext* context)
{
    if (!Composing() && !InRun()) {
        return E_UNEXPECTED;
    }
    const std::wstring reading =
        converting_ ? segments_[segmentIndex_].reading : composer_.Commit();

    std::vector<std::wstring> symbols;
    if (!engine_.ConvertSymbols(reading, &symbols) || symbols.empty()) {
        return S_OK; // 特殊変換の候補が無い読み (またはエンジン不調) なら何もしない
    }

    EnsureConversionState();

    // 既にこの文節を特殊変換の候補のみで表示中なら、F4 の連打は ↓ と同じく次候補へ送る
    auto& candidates = segments_[segmentIndex_].candidates;
    if (candidates == symbols) {
        return CycleCandidate(context, +1);
    }
    candidates = std::move(symbols);
    selected_[segmentIndex_] = 0;
    SyncPairedSegment(segmentIndex_);

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

HRESULT TextService::ConvertToShortcuts(ITfContext* context)
{
    if (!Composing() && !InRun()) {
        return E_UNEXPECTED;
    }
    const std::wstring reading =
        converting_ ? segments_[segmentIndex_].reading : composer_.Commit();

    std::vector<std::wstring> shortcuts;
    if (!engine_.ConvertShortcuts(reading, &shortcuts) || shortcuts.empty()) {
        return S_OK; // 短縮よみが無い読み (またはエンジン不調) なら何もしない
    }

    EnsureConversionState();

    // 既にこの文節を短縮よみの候補のみで表示中なら、F5 の連打は ↓ と同じく次候補へ送る
    auto& candidates = segments_[segmentIndex_].candidates;
    if (candidates == shortcuts) {
        return CycleCandidate(context, +1);
    }
    candidates = std::move(shortcuts);
    selected_[segmentIndex_] = 0;
    SyncPairedSegment(segmentIndex_);

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
}

HRESULT TextService::CancelConversion(ITfContext* context)
{
    ClearConversion();
    // かな入力に戻るので、バーも作り直して復活させる
    if (!Composing()) {
        return UpdateRunAndBar(context);
    }
    return UpdateCompositionAndBar(context);
}

// ---- 候補バー ----

HRESULT TextService::UpdateBar(ITfContext* context)
{
    barIndex_ = -1;
    const TsfConfig& config = config_.Get();
    // 候補選択中は縦の候補ウィンドウを使う。モードレスの英字モード中は英文の入力なので出さない
    // (日本語区間が残っている区間分割の run は、日本語区間に対して出す)
    if (!config.candidateBar || (!Composing() && !InRun()) || converting_ ||
        (config.modeless && composer_.AsciiMode() && !SplitAsciiRun())) {
        ClearBar();
        return S_OK;
    }
    const std::wstring kana = BarReading();
    if (kana.size() < static_cast<size_t>(config.minSuggestChars)) {
        ClearBar();
        return S_OK;
    }

    // どちらもエンジン未接続なら即 false を返す (自動起動・接続待ちで打鍵を止めない)
    std::vector<SentenceCandidate> sentences;
    if (!engine_.ConvertNBestLive(kana, CurrentContext(), &sentences)) {
        sentences.clear();
    }
    // 区間分割した run は日本語区間の後ろに英単語が続いているので、読みを延ばす予測は合わない
    std::vector<PredictionCandidate> predictions;
    if (SplitAsciiRun() || !engine_.Predict(kana, &predictions)) {
        predictions.clear();
    }

    std::vector<BarCandidate> items = BuildBarItems(kana, sentences, predictions);
    if (items.empty()) {
        ClearBar();
        return S_OK;
    }

    RECT caret = {};
    if (!CaretRect(context, &caret)) {
        ClearBar();
        return S_OK;
    }
    if (!barXFixed_) {
        barX_ = caret.left;
        barXFixed_ = true;
    }
    if (!ShowBarItems(caret, std::move(items))) {
        ClearBar();
        return S_OK;
    }
    barKana_ = kana;
    barPredictions_ = std::move(predictions);
    RequestRerank(kana);
    return S_OK;
}

std::wstring TextService::BarReading() const
{
    // 未完成のローマ字は採用の対象外なので、候補は確定済みかなだけから作る
    const std::wstring& kana = composer_.ConfirmedKana();
    return SplitAsciiRun() ? kana.substr(0, composer_.AsciiStart()) : kana;
}

std::vector<TextService::BarCandidate> TextService::BuildBarItems(
    const std::wstring& kana, const std::vector<SentenceCandidate>& sentences,
    const std::vector<PredictionCandidate>& predictions) const
{
    std::vector<BarCandidate> items;
    const auto contains = [&items](const std::wstring& surface) {
        return std::any_of(items.begin(), items.end(),
                           [&surface](const BarCandidate& item) { return item.surface == surface; });
    };
    constexpr size_t kMaxWholes = 3;
    for (const SentenceCandidate& sentence : sentences) {
        if (items.size() >= kMaxWholes) {
            break;
        }
        if (sentence.surface != kana && !contains(sentence.surface)) {
            items.push_back({BarKind::Whole, sentence.surface, kana, sentence.segments});
        }
    }
    // 先頭文節は上位の候補から順に、2文節以上の候補の第1文節を (読み, 表記) の重複なく集める
    // (区切りの違う「今日は…」「今日…」が並ぶ)
    constexpr size_t kMaxHeads = 3;
    std::vector<std::pair<std::wstring, std::wstring>> heads;
    for (const SentenceCandidate& sentence : sentences) {
        if (heads.size() >= kMaxHeads) {
            break;
        }
        if (sentence.segments.size() < 2) {
            continue;
        }
        const auto& head = sentence.segments[0];
        if (head.first.empty() || kana.compare(0, head.first.size(), head.first) != 0 ||
            std::find(heads.begin(), heads.end(), head) != heads.end()) {
            continue;
        }
        heads.push_back(head);
    }
    const size_t predictionLimit = CandidateWindow::kPageSize - items.size() - heads.size();
    size_t predictionCount = 0;
    for (const PredictionCandidate& candidate : predictions) {
        if (predictionCount >= predictionLimit) {
            break;
        }
        if (contains(candidate.surface)) {
            continue;
        }
        items.push_back({BarKind::Prediction, candidate.surface, candidate.reading, {}});
        ++predictionCount;
    }
    for (const auto& [reading, surface] : heads) {
        if (!contains(surface)) {
            items.push_back({BarKind::Head, surface, reading, {}});
        }
    }
    return items;
}

bool TextService::ShowBarItems(const RECT& caret, std::vector<BarCandidate> items)
{
    // 区間分割した run の全体変換は、採用後の文字列が分かるよう英字区間を付けて見せる
    const std::wstring asciiTail =
        SplitAsciiRun() ? composer_.ConfirmedKana().substr(composer_.AsciiStart()) : L"";
    std::vector<CandidateWindow::BarItem> labels;
    for (const BarCandidate& item : items) {
        const bool head = item.kind == BarKind::Head;
        labels.push_back({head ? item.surface : item.surface + asciiTail, head});
    }
    // selection に範囲外を渡し、どの候補も強調しない表示にする
    const size_t shown = candidateWindow_.ShowBar(caret, barX_, labels, labels.size());
    if (shown == 0) {
        return false;
    }
    barBuilt_ = items;
    barCaret_ = caret;
    // 作業領域に収まらず表示しなかった候補は、選択の対象からも外す
    items.resize(shown);
    barItems_ = std::move(items);
    return true;
}

// ---- LLM による並べ替え ----

namespace {

// RERANKGET で結果を問い合わせる間隔と、問い合わせをやめるまでの時間
constexpr UINT kLlmPollMs = 20;
constexpr ULONGLONG kLlmPollLimitMs = 1000;
// 読める文書で run の前を読む長さ (UTF-16 単位)
constexpr ULONG kLlmContextRead = 80;
// 追記のみの文書で覚えておく、IME が入れた文字列の長さ (エンジンの LLM_CONTEXT_CHARS と同じ)
constexpr size_t kLlmContextChars = 40;

// 最後の改行より後ろだけを残し、タブはスペースにする (行をまたぐ文脈と、行プロトコルの区切りを避ける)
std::wstring TrimLlmContext(const std::wstring& text)
{
    const size_t newline = text.find_last_of(L"\r\n");
    std::wstring line = newline == std::wstring::npos ? text : text.substr(newline + 1);
    std::replace(line.begin(), line.end(), L'\t', L' ');
    return line;
}

// 末尾 length 単位を残す。先頭がサロゲートペアの後ろ半分になるなら1単位多く削る
std::wstring KeepTail(const std::wstring& text, size_t length)
{
    if (text.size() <= length) {
        return text;
    }
    size_t start = text.size() - length;
    if (IS_LOW_SURROGATE(text[start])) {
        ++start;
    }
    return text.substr(start);
}

// transitory な文脈 (コモンコントロールの編集欄など) は文書が読めないので、親の文脈を返す
// (Mozc と同じ。GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT)。戻り値は AddRef 済み
ITfContext* ContextForReading(ITfContext* context)
{
    TF_STATUS status = {};
    if (FAILED(context->GetStatus(&status)) || (status.dwStaticFlags & TS_SS_TRANSITORY) == 0) {
        context->AddRef();
        return context;
    }
    ITfContext* parent = nullptr;
    ITfDocumentMgr* docMgr = nullptr;
    if (SUCCEEDED(context->GetDocumentMgr(&docMgr)) && docMgr != nullptr) {
        ITfCompartmentMgr* compartmentMgr = nullptr;
        if (SUCCEEDED(docMgr->QueryInterface(IID_ITfCompartmentMgr,
                                             reinterpret_cast<void**>(&compartmentMgr)))) {
            ITfCompartment* compartment = nullptr;
            if (SUCCEEDED(compartmentMgr->GetCompartment(
                    GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT, &compartment)) &&
                compartment != nullptr) {
                VARIANT value;
                VariantInit(&value);
                if (SUCCEEDED(compartment->GetValue(&value)) && value.vt == VT_UNKNOWN &&
                    value.punkVal != nullptr) {
                    ITfDocumentMgr* parentDocMgr = nullptr;
                    if (SUCCEEDED(value.punkVal->QueryInterface(
                            IID_ITfDocumentMgr, reinterpret_cast<void**>(&parentDocMgr)))) {
                        parentDocMgr->GetTop(&parent);
                        parentDocMgr->Release();
                    }
                }
                VariantClear(&value);
                compartment->Release();
            }
            compartmentMgr->Release();
        }
        docMgr->Release();
    }
    if (parent == nullptr) {
        context->AddRef();
        return context;
    }
    return parent;
}

} // namespace

void TextService::CaptureLlmContext(ITfContext* context)
{
    ResetRerank();
    llmRunContext_.clear();
    if (!config_.Get().llm || context == nullptr) {
        return;
    }
    std::wstring text;
    const wchar_t* source = L"履歴";
    if (appendDocument_ == AppendDocument::Readable) {
        source = L"文書";
        ITfContext* target = ContextForReading(context);
        RequestSync(target, new (std::nothrow) GetPrecedingTextEditSession(target, kLlmContextRead, &text),
                    TF_ES_SYNC | TF_ES_READ);
        target->Release();
    } else {
        text = llmHistory_;
    }
    llmRunContext_ = TrimLlmContext(text);
    // 文脈そのものはログに書かない
    DebugLog(std::wstring(L"LLM の左文脈: ") + source + L"、長さ " +
             std::to_wstring(llmRunContext_.size()));
}

void TextService::AppendLlmHistory(const std::wstring& text)
{
    llmHistory_ = KeepTail(llmHistory_ + text, kLlmContextChars);
}

void TextService::ExtendLlmContext(const std::wstring& adopted)
{
    if (!config_.Get().llm) {
        return;
    }
    // 部分採用を続けても run を始めたときに読む長さを超えて伸ばさない (エンジンは末尾だけを使う)
    llmRunContext_ = KeepTail(TrimLlmContext(llmRunContext_ + adopted), kLlmContextRead);
}

void TextService::RequestRerank(const std::wstring& kana)
{
    ResetRerank();
    if (!config_.Get().llm) {
        return;
    }
    const unsigned long long id = engine_.Rerank(llmRunContext_, CurrentContext(), kana);
    if (id == 0) {
        return;
    }
    rerankId_ = id;
    rerankTick_ = GetTickCount64();
    rerankKana_ = kana;
    rerankContext_ = CurrentContext();
    if (!candidateWindow_.StartTimer(kLlmPollMs, [this] { OnRerankTimer(); })) {
        rerankId_ = 0;
    }
}

void TextService::StopRerankPolling()
{
    candidateWindow_.StopTimer();
}

void TextService::ResetRerank()
{
    StopRerankPolling();
    rerankId_ = 0;
    rerankDone_ = false;
    rerankResult_.clear();
    rerankKana_.clear();
    rerankContext_.Clear();
}

void TextService::OnRerankTimer()
{
    if (rerankId_ == 0 || rerankDone_ || GetTickCount64() - rerankTick_ > kLlmPollLimitMs) {
        StopRerankPolling();
        return;
    }
    EngineClient::RerankStatus status = EngineClient::RerankStatus::None;
    std::vector<SentenceCandidate> candidates;
    if (!engine_.RerankGet(rerankId_, &status, &candidates) ||
        status == EngineClient::RerankStatus::None) {
        StopRerankPolling();
        return;
    }
    if (status == EngineClient::RerankStatus::Pending) {
        return;
    }
    StopRerankPolling();
    rerankDone_ = true;
    rerankResult_ = std::move(candidates);
    DebugLog(L"並べ替えの結果を受信 id=" + std::to_wstring(rerankId_));

    // 選んでいる候補・候補選択中の並びは動かさない。依頼の後にバーを作り直していれば
    // ID が変わっているのでここには来ない
    if (barIndex_ >= 0 || converting_ || barItems_.empty() || barKana_ != rerankKana_) {
        return;
    }
    std::vector<BarCandidate> items = BuildBarItems(barKana_, rerankResult_, barPredictions_);
    const auto same = [](const BarCandidate& a, const BarCandidate& b) {
        return a.kind == b.kind && a.surface == b.surface && a.reading == b.reading &&
               a.segments == b.segments;
    };
    if (items.empty() ||
        std::equal(items.begin(), items.end(), barBuilt_.begin(), barBuilt_.end(), same)) {
        return;
    }
    ShowBarItems(barCaret_, std::move(items));
}

bool TextService::TakeRerankedCandidates(const std::wstring& kana,
                                         std::vector<SentenceCandidate>* candidates)
{
    const ConversionContext context = CurrentContext();
    if (rerankId_ == 0 || kana != rerankKana_ || context.reading != rerankContext_.reading ||
        context.surface != rerankContext_.surface) {
        return false;
    }
    if (!rerankDone_) {
        EngineClient::RerankStatus status = EngineClient::RerankStatus::None;
        std::vector<SentenceCandidate> received;
        if (!engine_.RerankGet(rerankId_, &status, &received) ||
            status != EngineClient::RerankStatus::Done) {
            return false;
        }
        rerankDone_ = true;
        rerankResult_ = std::move(received);
    }
    *candidates = rerankResult_;
    return !candidates->empty();
}

HRESULT TextService::UpdateCompositionAndBar(ITfContext* context)
{
    HRESULT hr = UpdateCompositionText(context, composer_.Display());
    if (FAILED(hr)) {
        return hr;
    }
    UpdateBar(context);
    return hr;
}

void TextService::ClearBar()
{
    barItems_.clear();
    barKana_.clear();
    barIndex_ = -1;
    barBuilt_.clear();
    barPredictions_.clear();
    StopRerankPolling();
    // 変換中は候補ウィンドウを変換側が使っているので触らない
    if (!converting_) {
        candidateWindow_.Hide();
    }
}

HRESULT TextService::MoveBarSelection(int delta)
{
    if (converting_ || barItems_.empty()) {
        return S_OK;
    }
    if (delta > 0) {
        // 未選択 (-1) からは先頭へ、末尾からは先頭へ循環する
        barIndex_ = (barIndex_ + 1) % static_cast<int>(barItems_.size());
    } else if (barIndex_ < 0) {
        return S_OK; // 未選択の↑は何もしない
    } else if (barIndex_ == 0) {
        // 先頭でさらに↑は選択解除
        return DeselectBar();
    } else {
        --barIndex_;
    }
    candidateWindow_.SetSelection(static_cast<size_t>(barIndex_));
    return S_OK;
}

HRESULT TextService::SelectBarByNumber(size_t number)
{
    if (converting_ || barItems_.empty() || barIndex_ < 0) {
        return E_UNEXPECTED; // バー選択中のみ呼ばれる
    }
    if (number >= barItems_.size()) {
        return S_OK; // 表示されていない番号は無視する
    }
    barIndex_ = static_cast<int>(number);
    candidateWindow_.SetSelection(number);
    return S_OK;
}

HRESULT TextService::DeselectBar()
{
    barIndex_ = -1;
    candidateWindow_.SetSelection(barItems_.size()); // 範囲外 = 強調なし
    return S_OK;
}

HRESULT TextService::CommitConversion(ITfContext* context)
{
    return EndComposition(context, PrepareConversionCommit());
}

std::wstring TextService::PrepareConversionCommit()
{
    // 文節ごとの確定結果をエンジンに学習させる (失敗しても確定は続行する)。
    // 入力全体の候補を選んだときは、その候補の文節を確定した文節とする
    std::vector<std::pair<std::wstring, std::wstring>> committed;
    if (segments_.size() == 1) {
        const std::wstring& surface = segments_[0].candidates[selected_[0]];
        const auto it = std::find_if(
            wholeCandidates_.begin(), wholeCandidates_.end(),
            [&surface](const SentenceCandidate& candidate) { return candidate.surface == surface; });
        if (it != wholeCandidates_.end()) {
            committed = it->segments;
        }
    }
    if (committed.empty()) {
        for (size_t i = 0; i < segments_.size(); ++i) {
            committed.push_back({segments_[i].reading, segments_[i].candidates[selected_[i]]});
        }
    }
    engine_.Learn(SentenceLearnEntries(committed));
    // 人が文節を伸縮して分割を直したときだけ、その直し方を学習させる
    if (segmentsResized_) {
        LearnResizedSegments();
    }
    if (!committed.empty()) {
        SetCommitContext(committed.back().first, committed.back().second);
    }
    return ConvertedText();
}

std::vector<LearnEntry> TextService::SentenceLearnEntries(
    const std::vector<std::pair<std::wstring, std::wstring>>& segments) const
{
    // 各文節の文脈 = 先頭文節は外部文脈、以降は1つ前の文節の表記
    std::vector<LearnEntry> entries;
    std::wstring prevSurface = contextSurface_;
    std::wstring reading;
    std::wstring surface;
    for (const auto& [segmentReading, segmentSurface] : segments) {
        entries.push_back({segmentReading, segmentSurface, prevSurface});
        prevSurface = segmentSurface;
        reading += segmentReading;
        surface += segmentSurface;
    }
    // 読み全体の学習は、次に同じ読みを打ったとき入力全体の候補の先頭に来るようにする。
    // 1文節なら文節の学習と同じなので送らない
    if (segments.size() >= 2) {
        entries.push_back({reading, surface, L""});
    }
    return entries;
}

void TextService::LearnResizedSegments()
{
    // 文節の読みの長さから、文頭からの区切り位置 (先頭と末尾は含めない) を作る
    const auto boundaries = [](const auto& lengths) {
        std::vector<size_t> offsets;
        size_t sum = 0;
        for (size_t i = 0; i + 1 < lengths.size(); ++i) {
            sum += lengths[i];
            offsets.push_back(sum);
        }
        return offsets;
    };
    std::vector<size_t> lengths;
    lengths.reserve(segments_.size());
    for (const ConversionSegment& segment : segments_) {
        lengths.push_back(segment.reading.size());
    }
    const std::vector<size_t> before = boundaries(preResizeLengths_);
    const std::vector<size_t> after = boundaries(lengths);
    // 片方の区切りがもう片方をすべて含む = 1文節を割った / 複数文節をまとめただけで、
    // 区切りの位置そのものは動いていない。辞書に無い語を入力しようとしたとみなす
    const bool divided = std::includes(after.begin(), after.end(), before.begin(), before.end());
    const bool merged = std::includes(before.begin(), before.end(), after.begin(), after.end());

    // 区切りが動いていれば「区切り直し」。直した分割を境界として学習する
    if (!divided && !merged) {
        std::vector<std::wstring> readings;
        readings.reserve(segments_.size());
        for (const ConversionSegment& segment : segments_) {
            readings.push_back(segment.reading);
        }
        engine_.LearnSegmentBoundaries(readings);
        return;
    }

    // 割った (まとめた) 範囲を1語 (連結した読みと表記) として学習する。
    // ここで境界を学習してしまうと、次回も同じ語が割れる方向に効いてしまう。
    // 範囲の区切りは、伸縮の前後で共通して残っている側 (部分集合のほう)
    std::vector<size_t> spanEnds = divided ? before : after;
    size_t total = 0;
    for (size_t length : lengths) {
        total += length;
    }
    spanEnds.push_back(total);

    std::vector<std::pair<std::wstring, std::wstring>> words;
    size_t begin = 0;
    size_t index = 0;   // 確定した文節の走査位置
    size_t origin = 0;  // 伸縮前の文節の走査位置
    for (size_t end : spanEnds) {
        std::wstring reading;
        std::wstring surface;
        size_t count = 0;
        while (index < segments_.size() && begin + reading.size() < end) {
            reading += segments_[index].reading;
            surface += segments_[index].candidates[selected_[index]];
            ++index;
            ++count;
        }
        size_t originCount = 0;
        for (size_t pos = begin; origin < preResizeLengths_.size() && pos < end; ++origin) {
            pos += preResizeLengths_[origin];
            ++originCount;
        }
        // どちらか一方でも2文節に割れている範囲だけが「人が直した1語」
        if (count >= 2 || originCount >= 2) {
            words.push_back({reading, surface});
        }
        begin = end;
    }
    if (!words.empty()) {
        engine_.LearnWords(words);
    }
}

void TextService::FinishConversionState(const std::wstring& commitText)
{
    // 確定アンドゥ (Ctrl+Backspace) 用に確定文字列と確定前のコンポーザを覚えておく
    if (!commitText.empty()) {
        lastCommitText_ = commitText;
        lastComposer_ = composer_;
    }
    ClearConversion();
    ClearBar();
    barXFixed_ = false;
    composer_.Clear();
}

HRESULT TextService::RestartComposition(ITfContext* context, const std::wstring& commitText)
{
    if (!Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }
    // 確定と新 composition の開始を1つの edit session (1つのロック) で行う。
    // EndComposition → StartComposition と別々の同期 session に分けると、
    // ロックの合間にホストが確定処理を進めてしまい、Word では2つ目の session が
    // 失敗して打鍵が素通りし、CUAS 経由のアプリ (WezTerm 等) では新しい
    // composition の未確定文字列がそのまま確定されてしまう。
    // テキストの設定は別の edit session で行う。同じ session 内で
    // EndComposition + StartComposition + SetText を行うと、CUAS が
    // SetText の WM_IME_COMPOSITION を生成せず、WezTerm 等で
    // 未確定文字列が表示されない問題が発生するため
    ITfComposition* newComposition = nullptr;
    HRESULT hr = RequestSync(context,
                             new (std::nothrow) RestartCompositionEditSession(
                                 context, composition_, commitText,
                                 static_cast<ITfCompositionSink*>(this), &newComposition),
                             TF_ES_SYNC | TF_ES_READWRITE);
    composition_->Release();
    composition_ = newComposition;
    if (composition_ == nullptr) {
        promoted_ = false;
    }
    if (SUCCEEDED(hr) && composition_ == nullptr) {
        hr = E_FAIL;
    }
    if (SUCCEEDED(hr)) {
        AppendLlmHistory(commitText);
    }
    return hr;
}

HRESULT TextService::LaunchWordRegister(ITfContext* context)
{
    // 選択テキストを単語欄の初期値として渡す (変換できなかった語を選択して登録する用途)
    std::wstring selection;
    if (context != nullptr) {
        RequestSync(context, new (std::nothrow) GetSelectionTextEditSession(context, &selection),
                    TF_ES_SYNC | TF_ES_READ);
    }
    // 複数行の選択は単語ではないので先頭行だけにし、引数に渡せない引用符は除く
    const size_t newline = selection.find_first_of(L"\r\n");
    if (newline != std::wstring::npos) {
        selection.resize(newline);
    }
    selection.erase(std::remove(selection.begin(), selection.end(), L'"'), selection.end());

    const std::wstring exePath = EngineClient::FindExePath(L"quicklime-regword.exe");
    if (exePath.empty()) {
        return S_OK; // ツールが見つからない場合は何もしない
    }
    std::wstring commandLine = L"\"" + exePath + L"\"";
    if (!selection.empty()) {
        commandLine += L" \"" + selection + L"\"";
    }

    STARTUPINFOW startupInfo = {};
    startupInfo.cb = sizeof(startupInfo);
    PROCESS_INFORMATION processInfo = {};
    if (CreateProcessW(exePath.c_str(), commandLine.data(), nullptr, nullptr, FALSE, 0, nullptr,
                       nullptr, &startupInfo, &processInfo)) {
        CloseHandle(processInfo.hThread);
        CloseHandle(processInfo.hProcess);
    }
    return S_OK;
}

HRESULT TextService::LaunchConfigTool()
{
    const std::wstring exePath = EngineClient::FindExePath(L"quicklime-config.exe");
    if (exePath.empty()) {
        return S_OK; // ツールが見つからない場合は何もしない
    }
    // IME を持つプロセスから起動する設定ウィンドウが前面に出られるようにする
    AllowSetForegroundWindow(ASFW_ANY);
    std::wstring commandLine = L"\"" + exePath + L"\"";
    STARTUPINFOW startupInfo = {};
    startupInfo.cb = sizeof(startupInfo);
    PROCESS_INFORMATION processInfo = {};
    if (CreateProcessW(exePath.c_str(), commandLine.data(), nullptr, nullptr, FALSE, 0, nullptr,
                       nullptr, &startupInfo, &processInfo)) {
        CloseHandle(processInfo.hThread);
        CloseHandle(processInfo.hProcess);
    }
    return S_OK;
}

void TextService::ClearConversion()
{
    candidateWindow_.Hide();
    converting_ = false;
    segments_.clear();
    selected_.clear();
    segmentIndex_ = 0;
    segmentsResized_ = false;
    preResizeLengths_.clear();
    wholeCandidates_.clear();
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
    if (context == nullptr || (!Composing() && !InRun())) {
        return E_UNEXPECTED;
    }

    // 現在文節の位置 (文字数) を求めて、その範囲だけ強調する
    LONG targetStart = 0;
    for (size_t i = 0; i < segmentIndex_; ++i) {
        targetStart += static_cast<LONG>(segments_[i].candidates[selected_[i]].size());
    }
    const LONG targetLength =
        static_cast<LONG>(segments_[segmentIndex_].candidates[selected_[segmentIndex_]].size());

    if (!Composing()) {
        // 昇格できなかった run は composition の表示属性が使えないため、現在文節を
        // 選択状態にして強調する
        return ReplaceRunDisplay(context, ConvertedText(), static_cast<size_t>(targetStart),
                                 static_cast<size_t>(targetLength));
    }
    return RequestSync(context,
                       new (std::nothrow) UpdateCompositionEditSession(
                           context, composition_, ConvertedText(), inputAttribute_,
                           targetAttribute_, targetStart, targetLength),
                       TF_ES_SYNC | TF_ES_READWRITE);
}

RECT TextService::CandidateAnchor(ITfContext* context)
{
    // composition の矩形を取得して候補ウィンドウの位置を決める
    // (run では選択範囲 = 昇格できなかった run の現在文節、それ以外は末尾に潰した選択)
    RECT rect = {};
    bool succeeded = false;
    if (Composing()) {
        RequestSync(context,
                    new (std::nothrow) GetTextExtentEditSession(context, composition_, &rect,
                                                                &succeeded),
                    TF_ES_SYNC | TF_ES_READ);
    } else if (InRun()) {
        RequestSync(context,
                    new (std::nothrow) GetSelectionExtentEditSession(context, &rect, &succeeded),
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
    // 未完成ローマ字の小窓はキャレットに重ねて出すので、候補ウィンドウはその下に出す
    RECT pendingRect = {};
    if (pendingWindow_.WindowRect(&pendingRect) && pendingRect.bottom > rect.bottom) {
        rect.bottom = pendingRect.bottom;
    }
    return rect;
}

void TextService::ShowCandidateWindow(ITfContext* context)
{
    if (!segments_.empty()) {
        candidateWindow_.Show(CandidateAnchor(context), segments_[segmentIndex_].candidates,
                              selected_[segmentIndex_]);
    }
}

// ---- composition 操作 ----

HRESULT TextService::RequestSync(ITfContext* context, ITfEditSession* session,
                                 DWORD flags) const
{
    if (session == nullptr) {
        return E_OUTOFMEMORY;
    }
    HRESULT hrSession = S_OK;
    ++ownEditDepth_;
    HRESULT hr = context->RequestEditSession(clientId_, session, flags, &hrSession);
    --ownEditDepth_;
    session->Release();
    return FAILED(hr) ? hr : hrSession;
}

HRESULT TextService::StartComposition(ITfContext* context)
{
    if (Composing() || context == nullptr) {
        return E_UNEXPECTED;
    }
    // 文脈補正のハイブリッド照合: 内部履歴 (contextSurface_) があれば、
    // composition 開始と同じ edit session でキャレット直前の実テキストを読み、
    // 履歴と食い違っていれば (クリック等でキャレットが動いていれば) 文脈を破棄する。
    // 読み取れないアプリでは precedingReadOk が false のままになり、内部履歴を信頼する
    const ULONG precedingLength = static_cast<ULONG>(contextSurface_.size());
    std::wstring precedingText;
    bool precedingReadOk = false;
    const HRESULT hr = RequestSync(
        context,
        new (std::nothrow) StartCompositionEditSession(
            context, static_cast<ITfCompositionSink*>(this), &composition_, precedingLength,
            &precedingText, &precedingReadOk),
        TF_ES_SYNC | TF_ES_READWRITE);
    if (precedingLength > 0 && precedingReadOk && precedingText != contextSurface_) {
        ClearContext();
    }
    return hr;
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

    // 確定アンドゥ (Ctrl+Backspace) 用に確定文字列と確定前のコンポーザを覚えておく
    // (取消 = 空文字列の確定はアンドゥの対象にしない)
    if (!text.empty()) {
        lastCommitText_ = text;
        lastComposer_ = composer_;
    }

    // 変換状態・バーと候補ウィンドウの後始末
    ClearConversion();
    ClearBar();
    barXFixed_ = false;

    HRESULT hr = RequestSync(
        context, new (std::nothrow) EndCompositionEditSession(context, composition_, text),
        TF_ES_SYNC | TF_ES_READWRITE);
    if (SUCCEEDED(hr)) {
        AppendLlmHistory(text);
    }
    composition_->Release();
    composition_ = nullptr;
    promoted_ = false;
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
    const HRESULT hr = RequestSync(context, new (std::nothrow) InsertTextEditSession(context, text),
                                   TF_ES_SYNC | TF_ES_READWRITE);
    if (SUCCEEDED(hr)) {
        AppendLlmHistory(text);
    }
    return hr;
}
