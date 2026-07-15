#include "text_service.h"

#include <algorithm>
#include <cwctype>
#include <new>

#include "display_attribute.h"
#include "edit_session.h"
#include "globals.h"
#include "kana_forms.h"

namespace {

// 記号キー1つぶんの定義: 未確定文字列に入れるかな (全角形) と打鍵文字そのもの
struct SymbolKey {
    WPARAM vk;
    const wchar_t* kana; // 未確定文字列へ入れる全角形
    const wchar_t* raw;  // 打鍵文字 (生ローマ字候補・英字モード用)
};

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

bool IsLetterKey(WPARAM wparam)
{
    return wparam >= 'A' && wparam <= 'Z';
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
    // Ctrl / Alt 併用時は原則アプリのショートカットなので手を出さないが、
    // composition 中の Ctrl+M (確定) と Ctrl+H (1文字削除)、
    // composition が無いときの Ctrl+Backspace (確定アンドゥ) だけは IME が処理する
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    if (ctrl || alt) {
        if (ctrl && !alt && !Composing() && wparam == VK_BACK) {
            return !lastCommitText_.empty();
        }
        return ctrl && !alt && Composing() && (wparam == 'M' || wparam == 'H');
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
        case VK_PRIOR:
        case VK_NEXT:
            // 候補選択中のみ PgUp/PgDn で先頭/末尾の文節へ移動する
            return converting_;
        case VK_F4:
        case VK_F6:
        case VK_F7:
        case VK_F8:
        case VK_F9:
        case VK_F10:
            // ファンクションキー変換 (F4=特殊変換 (記号・日付), F6-F10=文字種の直接変換)
            return true;
        default:
            break;
        }
        // composition 中は英字 (Shift併用含む)・数字・記号も IME が処理し、
        // 未確定文字列の外へ文字が漏れないようにする
        return IsLetterKey(wparam) || IsDigitKey(wparam) ||
               FindSymbolKey(wparam, shifted) != nullptr;
    }

    // composition が無いとき: 英字 (Shift併用含む) と記号キーで composition を開始する
    return IsLetterKey(wparam) || FindSymbolKey(wparam, shifted) != nullptr;
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
    // Ctrl 併用ショートカット: Ctrl+M は Enter、Ctrl+H は BackSpace として扱う。
    // (英字の打鍵と解釈されないよう、ここで読み替えてから通常の処理に流す)
    if ((GetKeyState(VK_CONTROL) & 0x8000) != 0) {
        switch (wparam) {
        case 'M':
            wparam = VK_RETURN;
            break;
        case 'H':
            wparam = VK_BACK;
            break;
        case VK_BACK:
            // Ctrl+Backspace: 確定アンドゥ (composition が無いときのみ食べている)
            return UndoCommit(context);
        default:
            return S_OK; // IsKeyEaten が食べる Ctrl 併用は上記のみ
        }
    }

    const bool shifted = IsShiftPressed();

    // Shift+英字: 大文字をそのまま未確定文字列へ入れ、英字モードに入る。
    // 以降の英字はローマ字変換せずアルファベットのまま続く (既存IMEと同様)
    if (shifted && IsLetterKey(wparam)) {
        if (converting_) {
            HRESULT hr = CommitConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
        }
        const std::wstring upper(1, static_cast<wchar_t>(wparam));
        composer_.EnterAsciiMode();
        composer_.PushKana(upper, upper);
        if (!Composing()) {
            HRESULT hr = StartComposition(context);
            if (FAILED(hr)) {
                composer_.Clear();
                return hr;
            }
        }
        return UpdateCompositionText(context, composer_.Display());
    }

    // 英字: ローマ字入力を進める (候補選択中なら選択中の候補を確定してから)。
    // 英字モード中は変換せず小文字のまま続ける
    if (IsLetterKey(wparam)) {
        if (converting_) {
            HRESULT hr = CommitConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
        }
        const wchar_t c = static_cast<wchar_t>(L'a' + (wparam - 'A'));
        if (composer_.AsciiMode()) {
            const std::wstring letter(1, c);
            composer_.PushKana(letter, letter);
        } else {
            composer_.Push(c);
        }
        if (!Composing()) {
            HRESULT hr = StartComposition(context);
            if (FAILED(hr)) {
                composer_.Clear();
                return hr;
            }
        }
        return UpdateCompositionText(context, composer_.Display());
    }

    // 記号: 全角形を未確定文字列へ入れる (composition が無ければ開始する)。
    // 英字モード中は打鍵文字そのまま。
    // (数字キーと重なる Shift+1 (！) などがあるため、数字判定より先に見る)
    if (const SymbolKey* symbol = FindSymbolKey(wparam, shifted)) {
        if (converting_) {
            HRESULT hr = CommitConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
        }
        if (composer_.AsciiMode()) {
            composer_.PushKana(symbol->raw, symbol->raw);
        } else {
            composer_.PushKana(symbol->kana, symbol->raw);
        }
        if (!Composing()) {
            HRESULT hr = StartComposition(context);
            if (FAILED(hr)) {
                composer_.Clear();
                return hr;
            }
        }
        return UpdateCompositionText(context, composer_.Display());
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
        composer_.PushKana(digit, digit);
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
    case VK_PRIOR:
        // PgUp: 先頭の文節へ移動
        return converting_ ? MoveSegmentTo(context, 0) : S_OK;
    case VK_NEXT:
        // PgDn: 末尾の文節へ移動
        return converting_ ? MoveSegmentTo(context, segments_.size() - 1) : S_OK;
    case VK_F4:
        return ConvertToSymbols(context);
    case VK_F6:
        return DirectConvert(context, ConversionForm::Hiragana);
    case VK_F7:
        return DirectConvert(context, ConversionForm::Katakana);
    case VK_F8:
        return DirectConvert(context, ConversionForm::HalfwidthKatakana);
    case VK_F9:
        return DirectConvert(context, ConversionForm::FullwidthAscii);
    case VK_F10:
        return DirectConvert(context, ConversionForm::HalfwidthAscii);
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
    // エンジンが起動していない場合はひらがな1文節のみで動作を継続する。
    // 英字が残っている入力 (英単語の打鍵など) は文節分割しても意味を
    // 成さないため、全体を1文節に固定して変換する
    const std::wstring kana = composer_.Commit();
    const bool ok = ContainsAsciiLetter(kana)
        ? engine_.ConvertSegmentsFixed(kana, {kana.size()}, &segments_)
        : engine_.ConvertSegments(kana, &segments_);
    if (!ok || segments_.empty()) {
        segments_.clear();
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

    HRESULT hr = UpdateConvertingDisplay(context);
    ShowCandidateWindow(context);
    return hr;
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
    selected_[segmentIndex_] = (selected_[segmentIndex_] + count + delta) % count;
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

// ---- ファンクションキー変換 (F4, F6-F10) ----

void TextService::EnsureConversionState()
{
    if (converting_) {
        return;
    }
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
    if (!Composing()) {
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
    if (!Composing()) {
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

HRESULT TextService::UndoCommit(ITfContext* context)
{
    // 一度きりの操作として、成否に関わらず記憶を消す
    // (内容が一致しない = 確定後に別の編集があった場合に、以降の
    //  Ctrl+Backspace を奪い続けないようにする)
    const std::wstring commitText = lastCommitText_;
    lastCommitText_.clear();
    if (commitText.empty() || Composing()) {
        return S_OK;
    }

    // キャレット直前が確定文字列と一致する場合のみ削除する
    bool succeeded = false;
    RequestSync(context,
                new (std::nothrow) UndoCommitEditSession(context, commitText, &succeeded),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (!succeeded) {
        return S_OK;
    }

    // 確定前の読みを composition として復元する (そのまま再変換や F6-F10 が使える)
    composer_ = lastComposer_;
    HRESULT hr = StartComposition(context);
    if (FAILED(hr)) {
        composer_.Clear();
        return hr;
    }
    return UpdateCompositionText(context, composer_.Display());
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

    // 確定アンドゥ (Ctrl+Backspace) 用に確定文字列と確定前のコンポーザを覚えておく
    // (取消 = 空文字列の確定はアンドゥの対象にしない)
    if (!text.empty()) {
        lastCommitText_ = text;
        lastComposer_ = composer_;
    }

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
