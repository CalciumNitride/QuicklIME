// run (打鍵した読みを IME が持ち、採用・確定で文書へ入れる入力) のキー処理。
// TextService のメンバ関数のうち run に固有のものをこの翻訳単位に置く
// (設計は docs/design/input-buffer-modes.md)
#include "text_service.h"

#include <algorithm>
#include <new>
#include <vector>

#include "debug_log.h"
#include "edit_session.h"

using namespace key_util;

namespace {

// 擬似打鍵 (SendInput) の dwExtraInfo。ログで擬似打鍵かどうかを確かめるための目印
constexpr ULONG_PTR kPseudoKeyExtraInfo = 0x514C4D45;
// Alt・Win を離す前に挟む打鍵。Alt・Win を単独で押して離すとアプリのメニュー・
// スタートメニューが開くため、間に他の打鍵があったことにして起動を打ち消す。
// 0xE8 は割り当ての無い仮想キーで、押してもアプリ側で何も起きない
constexpr WORD kMenuMaskVk = 0xE8;

// 送り直す打鍵の修飾として押されているかを見て、要らないものは送る間だけ離す修飾キー
constexpr WORD kReleasedModifierVks[] = {VK_LCONTROL, VK_RCONTROL, VK_LSHIFT, VK_RSHIFT,
                                         VK_LMENU,    VK_RMENU,    VK_LWIN,   VK_RWIN};

bool StartsWith(const std::wstring& text, const std::wstring& prefix)
{
    return text.size() >= prefix.size() && text.compare(0, prefix.size(), prefix) == 0;
}

const wchar_t* MatchReason(ReplaceRunResult match)
{
    switch (match) {
    case ReplaceRunResult::Succeeded:
        return L"読み戻し成功";
    case ReplaceRunResult::Unreadable:
        return L"周辺テキストなし";
    case ReplaceRunResult::Mismatch:
        return L"内容不一致";
    default:
        return L"読み戻し失敗";
    }
}

// 食べた打鍵 vk を ctrl・shift の修飾付きで送り直す。送信時点で押されている修飾キーの
// うち要らないものは間だけ離して押し直し、要るのに押されていないものは間だけ押す
bool SendKeyWithModifiers(WORD vk, bool ctrl, bool shift)
{
    std::vector<INPUT> inputs;
    const auto push = [&inputs](WORD key, DWORD flags) {
        INPUT input = {};
        input.type = INPUT_KEYBOARD;
        input.ki.wVk = key;
        input.ki.wScan = static_cast<WORD>(MapVirtualKeyW(key, MAPVK_VK_TO_VSC));
        if (key == VK_RCONTROL || key == VK_RMENU || key == VK_LWIN || key == VK_RWIN) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        input.ki.dwFlags = flags;
        input.ki.dwExtraInfo = kPseudoKeyExtraInfo;
        inputs.push_back(input);
    };
    std::vector<WORD> released;
    bool menuKeyReleased = false;
    bool ctrlHeld = false;
    bool shiftHeld = false;
    for (WORD key : kReleasedModifierVks) {
        if ((GetAsyncKeyState(key) & 0x8000) == 0) {
            continue;
        }
        const bool isCtrl = key == VK_LCONTROL || key == VK_RCONTROL;
        const bool isShift = key == VK_LSHIFT || key == VK_RSHIFT;
        if (isCtrl && ctrl) {
            ctrlHeld = true;
        } else if (isShift && shift) {
            shiftHeld = true;
        } else {
            released.push_back(key);
            if (!isCtrl && !isShift) {
                menuKeyReleased = true;
            }
        }
    }
    std::vector<WORD> pressed;
    if (ctrl && !ctrlHeld) {
        pressed.push_back(VK_LCONTROL);
    }
    if (shift && !shiftHeld) {
        pressed.push_back(VK_LSHIFT);
    }
    if (menuKeyReleased) {
        push(kMenuMaskVk, 0);
        push(kMenuMaskVk, KEYEVENTF_KEYUP);
    }
    for (WORD key : released) {
        push(key, KEYEVENTF_KEYUP);
    }
    for (WORD key : pressed) {
        push(key, 0);
    }
    push(vk, 0);
    push(vk, KEYEVENTF_KEYUP);
    for (WORD key : pressed) {
        push(key, KEYEVENTF_KEYUP);
    }
    for (WORD key : released) {
        push(key, 0);
    }
    if (menuKeyReleased) {
        push(kMenuMaskVk, 0);
        push(kMenuMaskVk, KEYEVENTF_KEYUP);
    }
    const UINT sent =
        SendInput(static_cast<UINT>(inputs.size()), inputs.data(), sizeof(INPUT));
    return sent == inputs.size();
}

// 修飾キー自体の押下 (Ctrl や Shift の押し始め)。run を終える契機にしない
bool IsModifierKey(WPARAM wparam)
{
    switch (wparam) {
    case VK_SHIFT:
    case VK_CONTROL:
    case VK_MENU:
    case VK_LSHIFT:
    case VK_RSHIFT:
    case VK_LCONTROL:
    case VK_RCONTROL:
    case VK_LMENU:
    case VK_RMENU:
    case VK_LWIN:
    case VK_RWIN:
    case VK_CAPITAL:
    case VK_NUMLOCK:
    case VK_SCROLL:
        return true;
    default:
        return false;
    }
}

// アプリへ渡すとキャレットや文書の内容が変わる編集キー
bool IsEditingKey(WPARAM wparam)
{
    switch (wparam) {
    case VK_RETURN:
    case VK_TAB:
    case VK_UP:
    case VK_DOWN:
    case VK_LEFT:
    case VK_RIGHT:
    case VK_PRIOR:
    case VK_NEXT:
    case VK_HOME:
    case VK_END:
    case VK_DELETE:
    case VK_INSERT:
        return true;
    default:
        return false;
    }
}

bool IsPrintableKey(WPARAM wparam, bool shifted)
{
    return IsLetterKey(wparam) || FindSymbolKey(wparam, shifted) != nullptr ||
           IsDigitKey(wparam) || NumpadChar(wparam) != 0;
}

// run の外の半角数字は IME が食べずにアプリへ素のキーを渡す。電話番号・郵便番号など
// 自動で次欄へ移動するフォームは keydown/keyup/input をそのまま受け取る必要があるため
// (Shift+1 の「！」など記号になる打鍵は数字ではないので対象外)
bool PassesDigitKeyThrough(WPARAM wparam, bool shifted, bool inRun, bool digitsFullwidth)
{
    if (inRun || digitsFullwidth || IsLetterKey(wparam) ||
        FindSymbolKey(wparam, shifted) != nullptr) {
        return false;
    }
    return IsDigitKey(wparam) || NumpadChar(wparam) != 0;
}

// 後置再変換の対象: ひらがな・カタカナ・ー のみで構成された空でない文字列
bool IsKanaOnly(const std::wstring& text)
{
    if (text.empty()) {
        return false;
    }
    for (wchar_t c : text) {
        const bool hiragana = c >= 0x3041 && c <= 0x309F;
        const bool katakana = c >= 0x30A1 && c <= 0x30FA;
        if (!hiragana && !katakana && c != 0x30FC) {
            return false;
        }
    }
    return true;
}

// カタカナ1文字をひらがなにする (対応するひらがなが無い ヷ〜ヺ とそれ以外はそのまま)
wchar_t KatakanaToHiragana(wchar_t c)
{
    if (c >= 0x30A1 && c <= 0x30F6) {
        return static_cast<wchar_t>(c - 0x60);
    }
    return c;
}

// GetSelectionTextEditSession が読める上限。これ以上の選択は末尾が切れて
// 選択全体を置き換えられないため、後置再変換の対象にしない
// (キャレット直前のかなの変換と、範囲を広げる上限もこれに揃える)
constexpr size_t kMaxReconvertLength = 127;

// 変換の範囲の前を読む長さ (UTF-16 単位)。範囲を広げる余地 (kMaxReconvertLength まで) と、
// 前文脈の照合・LLM の左文脈 (80) に足りる長さ
constexpr size_t kRangeBeforeRead = 128;

bool EndsWith(const std::wstring& text, const std::wstring& suffix)
{
    return text.size() >= suffix.size() &&
           text.compare(text.size() - suffix.size(), suffix.size(), suffix) == 0;
}

// キャレット直前のかなの変換と範囲を広げる対象の文字。カタカナは意図して入れたものが多いので
// 区切りにする (カタカナを含めたいときは選択して変換する)
bool IsHiraganaOrChoon(wchar_t c)
{
    return (c >= 0x3041 && c <= 0x309F) || c == 0x30FC;
}

// reading を読みとする読みの状態。source の確定済みかなと共通する先頭はその打鍵列を引き継ぎ
// (F6〜F10 で打鍵どおりに戻せるように)、残り (予測候補で延びた読みなど) は文字そのものを打鍵列にする
RomajiComposer ComposerForReading(const RomajiComposer& source, const std::wstring& reading)
{
    const std::wstring& kana = source.ConfirmedKana();
    size_t common = 0;
    while (common < kana.size() && common < reading.size() && kana[common] == reading[common]) {
        ++common;
    }
    RomajiComposer result;
    for (size_t i = 0; i < reading.size(); ++i) {
        const std::wstring c(1, reading[i]);
        result.PushKana(c, i < common ? source.RawRange(i, 1) : c);
    }
    return result;
}

} // namespace

bool TextService::DocumentReadable() const
{
    return appendDocument_ == AppendDocument::Readable;
}

bool TextService::IsKeyEatenDirect(WPARAM wparam) const
{
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    // run の読みは IME が持っているので、run 中の Backspace は常に食べて読みの末尾を削る
    const bool backspaceEaten = converting_ || InRun();
    if (NextWordSelected() && !InRun()) {
        // 後続語の予測の選択中。DismissNextWordForKey が予測を残したキーだけが来る
        if (ctrl || alt) {
            return ctrl && !alt && wparam == 'M' && EnterNeedsResend();
        }
        switch (wparam) {
        case VK_RETURN:
            return EnterNeedsResend();
        case VK_ESCAPE:
        case VK_TAB:
        case VK_UP:
        case VK_DOWN:
        case VK_SPACE:
            return true;
        default:
            // 数字は run の外でもアプリへ渡さない (1〜9 は番号の選択、それ以外は採用して run を始める)
            return IsPrintableKey(wparam, IsShiftPressed());
        }
    }
    if (ctrl || alt) {
        if (!ctrl || alt || !InRun()) {
            return false;
        }
        // Ctrl+H は Backspace の読み替え。Ctrl+M は Enter と同じく確定してからアプリへ渡す
        return (wparam == 'H' && backspaceEaten) || (wparam == 'M' && EnterNeedsResend());
    }
    const bool shifted = IsShiftPressed();

    // 印字キーは run の有無によらず IME が入れる (run が無ければ新しい run を始める)
    if (IsPrintableKey(wparam, shifted)) {
        return !PassesDigitKeyThrough(wparam, shifted, InRun(),
                                      config_.Get().digitsFullwidth);
    }
    if (InRun()) {
        switch (wparam) {
        case VK_SPACE:
        case VK_ESCAPE:
            // Space は run を終えてからスペースを入れる。Esc は run を忘れるだけ
            return true;
        case VK_BACK:
            return backspaceEaten;
        case VK_RETURN:
            // 確定 (run の終了) は EndRunIfPassthroughKey で済ませてからアプリへ渡す。
            // 確定がアプリに反映される前に Enter が届きうる場合だけ食べて送り直す
            return EnterNeedsResend();
        case VK_UP:
        case VK_DOWN:
        case VK_TAB:
            // 候補選択中の Tab は変換を取り消してバーの先頭を選ぶ。バーが無ければ
            // 他の編集キーと同じく run を終えてアプリへ渡す
            return converting_ || !barItems_.empty();
        case VK_LEFT:
        case VK_RIGHT:
            // 入力全体の候補選択の ←→ は変換の範囲の伸縮 (伸縮できなくても食べる)
            if (!shifted && RangeResizable()) {
                return true;
            }
            [[fallthrough]];
        case VK_PRIOR:
        case VK_NEXT:
            // 文節の操作は文節 UI のときだけ。入力全体の候補選択では確定してアプリへ渡す
            return converting_ && config_.Get().segmentUi;
        default:
            break;
        }
        return false;
    }
    // run が無い Space は全角スペースの直接挿入 (Shift+Space と space=half は食べずに通す)
    return wparam == VK_SPACE && !shifted && config_.Get().spaceFullwidth;
}

void TextService::EndRunIfPassthroughKey(ITfContext* context, WPARAM wparam)
{
    if (NextWordSelected() && !InRun() && !Composing()) {
        // 読める文書の Enter (Ctrl+M) は食べずに、後続語の予測を採用してから渡す
        const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        const bool enter = !alt && (ctrl ? wparam == 'M' : wparam == VK_RETURN);
        if (enter && !IsKeyEaten(context, wparam)) {
            AdoptNextWord(context, BarAdopt::End, L"", nullptr);
        }
        return;
    }
    if ((!Composing() && !InRun()) || IsModifierKey(wparam)) {
        return;
    }
    // IME が食べるキーは HandleKey が状態を進める
    if (IsKeyEaten(context, wparam)) {
        return;
    }
    // Ctrl/Alt 併用はアプリのショートカット (Undo・全選択など文書を変えうる)
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    if (!ctrl && !alt && !IsEditingKey(wparam)) {
        return;
    }
    // run は Alt 併用では終えない。Alt 併用の打鍵は key event sink に届かないアプリ
    // (WezTerm・メモ帳) があり、そこでは run が続くので、届くアプリでもキャレット移動と同じ
    // 扱いにそろえる (読みは IME が持っているので壊れない)
    if (alt && InRun()) {
        return;
    }
    if (Composing()) {
        CommitComposition(context);
        return;
    }
    const bool enter = !alt && (ctrl ? wparam == 'M' : wparam == VK_RETURN);
    if (enter && barIndex_ >= 0) {
        // バー選択中は採用して run を終えてから渡す
        CommitRunDirect(context);
        return;
    }
    EndAppendRunForPassthroughKey(context);
}

bool TextService::EnterNeedsResend() const
{
    // 読める文書は確定が TSF の文書へ同期的に入るので、食べずに渡しても改行より先に入る。
    // CUAS 経由のアプリ (WezTerm など) は composition の確定結果を IME メッセージで後から
    // 受け取るため、食べずに渡すと確定結果より先に Enter が処理される。後続語の予測の挿入も
    // composition の確定と同じく後から届きうる (run 中は PanelKeyNeedsResend が扱う)
    return appendDocument_ != AppendDocument::Readable && (converting_ || NextWordSelected());
}

HRESULT TextService::CommitAndResendEnter(ITfContext* context, WPARAM vk, bool ctrl, bool shifted)
{
    // 送り直した打鍵は入力キューの後ろに並ぶので、確定結果のメッセージより後に処理される
    appendResendVk_ = vk;
    appendResendCtrl_ = ctrl;
    appendResendShift_ = shifted;
    HRESULT hr = S_OK;
    if (Composing()) {
        hr = CommitComposition(context);
    } else if (NextWordSelected() && !InRun()) {
        hr = AdoptNextWord(context, BarAdopt::End, L"", nullptr);
    } else if (barIndex_ >= 0) {
        hr = CommitRunDirect(context);
    } else {
        // バーを選んでいない run (改行キーでだけ来る) は、食べずにアプリへ渡す Enter と同じ
        // 終え方にする (見えているとおりの読みを入れる)
        EndAppendRunForPassthroughKey(context);
    }
    SendResendKey();
    return hr;
}

void TextService::SendResendKey()
{
    const WPARAM vk = appendResendVk_;
    appendResendVk_ = 0;
    if (vk == 0) {
        return;
    }
    DebugLog(L"食べた打鍵をアプリへ送り直す vk=" + std::to_wstring(vk));
    if (!SendKeyWithModifiers(static_cast<WORD>(vk), appendResendCtrl_, appendResendShift_)) {
        DebugLog(L"打鍵の送り直しに失敗 (SendInput)");
    }
}

HRESULT TextService::InsertNewline(ITfContext* context)
{
    if (CurrentKeyState() != KeyState::Idle) {
        // 確定より先に改行が届かないよう、常に食べて確定の後に送り直す
        return CommitAndResendEnter(context, VK_RETURN, false, true);
    }
    DebugLog(L"改行キー: Shift+Enter を送る");
    if (!SendKeyWithModifiers(VK_RETURN, false, true)) {
        DebugLog(L"Shift+Enter の送信に失敗 (SendInput)");
    }
    return S_OK;
}

HRESULT TextService::HandleKeyDirect(ITfContext* context, WPARAM wparam)
{
    const bool shifted = IsShiftPressed();
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
            return S_OK; // IsKeyEatenDirect が食べる Ctrl 併用は上記のみ
        }
    }

    DirectKey key;
    if (NextWordSelected() && !InRun()) {
        // 後続語の予測の選択中 (run なし)。選択の操作は run のバー選択と同じ
        if (ClassifyDirectKey(wparam, shifted, &key)) {
            if (!shifted && wparam >= '1' && wparam <= '9') {
                return SelectBarByNumber(wparam - '1');
            }
            return AdoptNextWord(context, BarAdopt::Continue, L"", &key);
        }
        switch (wparam) {
        case VK_RETURN:
            // 食べるのは EnterNeedsResend のときだけ
            return CommitAndResendEnter(context, originalVk, ctrl, shifted);
        case VK_ESCAPE:
            ClearBar();
            return S_OK;
        case VK_SPACE: {
            // run の外の Space (StandaloneSpaceText) と同じく、モードレスでは英数字の語の後を
            // 英文の途中とみなして半角にする
            const std::wstring& surface = barItems_[static_cast<size_t>(barIndex_)].surface;
            const bool asciiWord =
                config_.Get().modeless && std::all_of(surface.begin(), surface.end(), [](wchar_t c) {
                    return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'z') ||
                           (c >= L'A' && c <= L'Z');
                });
            const std::wstring space =
                (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
            return AdoptNextWord(context, BarAdopt::End, space, nullptr);
        }
        case VK_TAB:
            return MoveBarSelection(shifted ? -1 : +1);
        case VK_DOWN:
            return MoveBarSelection(+1);
        case VK_UP:
            return MoveBarSelection(-1);
        default:
            return S_OK;
        }
    }
    if (ClassifyDirectKey(wparam, shifted, &key)) {
        // 候補選択中・バー選択中の 1〜9 は候補番号による直接選択
        if (!shifted && wparam >= '1' && wparam <= '9') {
            if (converting_) {
                return SelectCandidateByNumber(context, wparam - '1');
            }
            if (barIndex_ >= 0) {
                return SelectBarByNumber(wparam - '1');
            }
        }
        // バー選択中の印字キーは選択中の候補を採用し、その打鍵から続ける
        if (barIndex_ >= 0) {
            return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::Continue, L"",
                                &key);
        }
        return TypeAppend(context, key);
    }
    switch (wparam) {
    case VK_RETURN:
        // 食べるのは EnterNeedsResend のときだけ
        return CommitAndResendEnter(context, originalVk, ctrl, shifted);
    case VK_ESCAPE:
        if (barIndex_ >= 0) {
            return DeselectBar();
        }
        // run は文書に何も入れずに捨てる (composition 入力は composition を空で終える)
        if (InRun()) {
            DropRun();
        }
        return S_OK;
    case VK_BACK:
        return BackspaceAppend(context);
    case VK_SPACE:
        return SpaceDirect(context, shifted);
    case VK_TAB:
        return MoveBarSelection(shifted ? -1 : +1);
    case VK_DOWN:
        return MoveBarSelection(+1);
    case VK_UP:
        return MoveBarSelection(-1);
    default:
        return S_OK;
    }
}

bool TextService::ClassifyDirectKey(WPARAM wparam, bool shifted, DirectKey* key) const
{
    *key = DirectKey{};
    if (IsLetterKey(wparam)) {
        if (shifted) {
            // Shift+英字: 大文字をそのまま入れ、英字モードに入る
            const std::wstring upper(1, static_cast<wchar_t>(wparam));
            key->enterAscii = true;
            key->kana = upper;
            key->raw = upper;
        } else {
            const wchar_t c = static_cast<wchar_t>(L'a' + (wparam - 'A'));
            key->romaji = c;
            key->kana = std::wstring(1, c);
            key->raw = key->kana;
        }
        return true;
    }
    // 記号 (数字キーと重なる Shift+1 (！) などがあるため、数字判定より先に見る)
    if (const SymbolKey* symbol = FindSymbolKey(wparam, shifted)) {
        std::wstring kana = symbol->kana;
        if (!shifted && wparam == VK_OEM_COMMA) {
            kana = config_.Get().punctComma;
        } else if (!shifted && wparam == VK_OEM_PERIOD) {
            kana = config_.Get().punctPeriod;
        }
        key->kana = kana;
        key->raw = symbol->raw;
        return true;
    }
    if (IsDigitKey(wparam)) {
        wchar_t c = static_cast<wchar_t>(wparam);
        if (config_.Get().digitsFullwidth) {
            c = static_cast<wchar_t>(c - L'0' + L'０');
        }
        key->kana = std::wstring(1, c);
        key->raw = key->kana;
        return true;
    }
    if (wchar_t numpad = NumpadChar(wparam)) {
        // 設定 digits=full のときは数字のみ全角にする (演算記号は半角のまま)
        if (config_.Get().digitsFullwidth && numpad >= L'0' && numpad <= L'9') {
            numpad = static_cast<wchar_t>(numpad - L'0' + L'０');
        }
        key->kana = std::wstring(1, numpad);
        key->raw = key->kana;
        return true;
    }
    return false;
}

void TextService::PushDirectKey(const DirectKey& key)
{
    if (key.enterAscii) {
        composer_.EnterAsciiMode();
    }
    if (composer_.AsciiMode()) {
        if (key.romaji == 0 || !CloseAsciiSection(key.romaji)) {
            composer_.PushKana(key.raw, key.raw);
        }
    } else if (key.romaji != 0) {
        composer_.Push(key.romaji);
        ResolveAsciiRequest(&composer_);
    } else {
        composer_.PushKana(key.kana, key.raw);
    }
}

ReplaceRunResult TextService::ReplaceRunText(ITfContext* context, const std::wstring& expected,
                                             const std::wstring& newText)
{
    return ReplaceRunRange(context, expected, expected.size(), 0, newText, 0, 0);
}

ReplaceRunResult TextService::ReplaceRunRange(ITfContext* context, const std::wstring& expected,
                                              size_t caretOffset, size_t currentSelectLength,
                                              const std::wstring& newText, size_t selectOffset,
                                              size_t selectLength, bool* selectedOut)
{
    if (selectedOut != nullptr) {
        *selectedOut = false;
    }
    ReplaceRunResult result = ReplaceRunResult::Unsupported;
    if (context == nullptr) {
        return result;
    }
    if (caretOffset != expected.size() || currentSelectLength > 0) {
        result = SelectRunRange(context, expected, caretOffset, expected.size(), 0);
        if (result != ReplaceRunResult::Succeeded) {
            return result;
        }
        caretOffset = expected.size();
    }
    result = ReplaceRunResult::Unsupported;
    RequestSync(context,
                new (std::nothrow)
                    ReplaceRunEditSession(context, expected, caretOffset, newText, &result),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (result == ReplaceRunResult::Unreadable && DocumentReadable()) {
        // 読める文書と分かっている以上、範囲が作れないのはキャレットが動いたため
        result = ReplaceRunResult::Mismatch;
    }
    if (result == ReplaceRunResult::Succeeded && selectLength > 0) {
        const ReplaceRunResult selected =
            SelectRunRange(context, newText, newText.size(), selectOffset, selectLength);
        if (selectedOut != nullptr) {
            *selectedOut = selected == ReplaceRunResult::Succeeded;
        }
    }
    return result;
}

ReplaceRunResult TextService::SelectRunRange(ITfContext* context, const std::wstring& expected,
                                             size_t caretOffset, size_t selectOffset,
                                             size_t selectLength)
{
    ReplaceRunResult result = ReplaceRunResult::Unsupported;
    if (context == nullptr) {
        return result;
    }
    RequestSync(context,
                new (std::nothrow) SelectRunRangeEditSession(context, expected, caretOffset,
                                                             selectOffset, selectLength, &result),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (result == ReplaceRunResult::Unreadable && DocumentReadable()) {
        // ReplaceRunRange と同じく、読める文書で範囲が作れないのはキャレットが動いたため
        result = ReplaceRunResult::Mismatch;
    }
    return result;
}

HRESULT TextService::SpaceDirect(ITfContext* context, bool shifted)
{
    if (!InRun()) {
        // 食べているのは !shifted && space=full のときだけ
        return InsertText(context, StandaloneSpaceText());
    }
    if (barIndex_ < 0) {
        return SpaceAppend(context, shifted);
    }
    // バー選択中の Space は幅によらず IME が入れる (採用と1つの文字列で入れる)。
    // モードレスが有効なときだけ、英字モード中と英字候補の採用は英文の語の区切りとして半角にする
    const bool asciiCandidate = static_cast<size_t>(barIndex_) < barItems_.size() &&
                                barItems_[static_cast<size_t>(barIndex_)].kind == BarKind::Ascii;
    const bool asciiWord = config_.Get().modeless && (composer_.AsciiMode() || asciiCandidate);
    const std::wstring space =
        (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
    return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::End, space, nullptr);
}

HRESULT TextService::ConvertKeyDirect(ITfContext* context, bool previous)
{
    if (!InRun()) {
        return ConvertIdleTarget(context, KeyFunc::Convert);
    }
    if (converting_) {
        return CycleCandidate(context, previous ? -1 : +1);
    }
    return BeginAppendConversion(context, KeyFunc::Convert);
}

bool TextService::ReadIdleTarget(ITfContext* context, IdleTarget* target) const
{
    *target = IdleTarget{};
    if (context == nullptr) {
        return false;
    }
    std::wstring selection;
    RequestSync(context, new (std::nothrow) GetSelectionTextEditSession(context, &selection),
                TF_ES_SYNC | TF_ES_READ);
    if (!selection.empty()) {
        if (selection.size() > kMaxReconvertLength || !IsKanaOnly(selection)) {
            return false;
        }
        // 選択開始から読み戻して照合できない文書では、選択を消して composition に昇格できない
        // ので、変換キーをアプリへ渡す
        ReplaceRunResult match = ReplaceRunResult::Unsupported;
        RequestSync(context, new (std::nothrow) MatchRunEditSession(context, selection, 0, &match),
                    TF_ES_SYNC | TF_ES_READ);
        if (match != ReplaceRunResult::Succeeded) {
            DebugLog(std::wstring(L"選択の読み戻しが一致しないため変換しない (") +
                     MatchReason(match) + L")");
            return false;
        }
        target->text = selection;
        target->selection = true;
        return true;
    }
    // 未判定の文書は読めるかどうかが分からない
    if (!DocumentReadable()) {
        return false;
    }
    std::wstring preceding;
    RequestSync(context,
                new (std::nothrow) GetPrecedingTextEditSession(
                    context,
                    static_cast<ULONG>(kMaxReconvertLength + kRangeBeforeRead +
                                       AdoptedSurfaceLength()),
                    &preceding),
                TF_ES_SYNC | TF_ES_READ);
    // 最新の記憶がひらがな・ー だけなら、下のキャレット直前のかなが同じ範囲 (またはそれより左まで) を
    // 対象にするので、記憶は漢字などを含むときだけ使う
    if (MatchAdopted(preceding) > 0) {
        const std::wstring& surface = adopted_.back().surface;
        if (!std::all_of(surface.begin(), surface.end(), IsHiraganaOrChoon)) {
            target->text = surface;
            target->before = preceding.substr(0, preceding.size() - surface.size());
            target->adopted = true;
            return true;
        }
    }
    size_t start = preceding.size();
    while (start > 0 && preceding.size() - start < kMaxReconvertLength &&
           IsHiraganaOrChoon(preceding[start - 1])) {
        --start;
    }
    if (start == preceding.size()) {
        return false;
    }
    target->text = preceding.substr(start);
    target->before = preceding.substr(0, start);
    return true;
}

HRESULT TextService::ConvertIdleTarget(ITfContext* context, KeyFunc func)
{
    IdleTarget target;
    if (!ReadIdleTarget(context, &target)) {
        return S_OK;
    }
    // 取消で降格するときの run の方式
    BeginRunMode();
    size_t chainFrom = adopted_.size();
    size_t chainTo = adopted_.size();
    if (!target.selection) {
        const std::wstring preceding = target.before + target.text;
        DropUnmatchedAdopted(preceding);
        FindAdoptedChain(preceding, target.before.size(), &chainFrom, &chainTo);
    }
    if (target.adopted) {
        composer_ = adopted_.back().composer;
    } else {
        // 対象を読みとして run を作る。エンジンに渡す読みはひらがなに正規化し、
        // 打鍵列は元の文字 (F7-F10 の文字種変換で元の形へ戻せるように) にする
        composer_.Clear();
        for (wchar_t c : target.text) {
            composer_.PushKana(std::wstring(1, KatakanaToHiragana(c)), std::wstring(1, c));
        }
    }
    if (target.selection) {
        // 選択した文字列と直前の確定は無関係。run を始める前の時点が無いので、LLM の左文脈も無しにする
        ClearContext();
        ResetRerank();
        llmRunContext_.clear();
    } else {
        // 範囲の直前が記憶なら、その最後の文節を前文脈にする。それ以外は文書の漢字の読みが
        // 分からないので、範囲の直前が直前の確定の表記で終わるときだけその確定を前文脈にする
        if (chainTo > chainFrom) {
            const auto& last = adopted_[chainTo - 1].segments.back();
            SetCommitContext(last.first, last.second);
        } else if (contextSurface_.empty() || !EndsWith(target.before, contextSurface_)) {
            ClearContext();
        }
        CaptureLlmContextFrom(target.before);
    }
    const size_t length = target.text.size();
    const PromoteResult promoted =
        PromoteRun(context, target.text, target.selection ? 0 : length,
                   target.selection ? length : 0);
    if (promoted == PromoteResult::Dropped) {
        return S_OK;
    }
    if (promoted == PromoteResult::Refused) {
        DebugLog(L"composition を張れないため変換しない");
        DropRun();
        return S_OK;
    }
    // 伸縮は composition の中だけで行う (範囲の前の文書の文字列へは広げない)
    BeginRangeTracking();
    if (!target.selection) {
        BeginRangeAdopted(chainFrom, chainTo, target.adopted);
    }
    return func == KeyFunc::Convert ? StartConversion(context) : ApplyFunctionKey(context, func);
}

bool TextService::ReadTextBeforeCaret(ITfContext* context, const std::wstring& inRange,
                                      std::wstring* before) const
{
    before->clear();
    if (context == nullptr) {
        return false;
    }
    std::wstring preceding;
    RequestSync(context,
                new (std::nothrow) GetPrecedingTextEditSession(
                    context,
                    static_cast<ULONG>(inRange.size() + kRangeBeforeRead + AdoptedSurfaceLength()),
                    &preceding),
                TF_ES_SYNC | TF_ES_READ);
    if (!EndsWith(preceding, inRange)) {
        return false;
    }
    *before = preceding.substr(0, preceding.size() - inRange.size());
    return true;
}

void TextService::BeginRangeTracking()
{
    rangeTracked_ = true;
    rangeBefore_.clear();
    rangeShift_ = 0;
    rangeStates_.clear();
    // 昇格 (StartComposition) で前文脈が捨てられていればそれを反映した後の値
    rangeBaseContext_ = CurrentContext();
    rangeAdopted_.clear();
    rangeAdoptedFrom_ = adopted_.size();
    rangeFromAdopted_ = false;
    rangeOrigin_ = AdoptedText{};
}

bool TextService::RangeResizable() const
{
    // 英字を含む run の変換キーでの変換は、打鍵したローマ字の候補 (apple / Apple / APPLE) が
    // 範囲を変えると成り立たない。F6〜F10 の文字種変換は、かな1文字ごとの打鍵から残りを
    // 作り直せるので英字を含んでいても伸縮する
    if (!converting_ || config_.Get().segmentUi || !rangeTracked_ || !Composing()) {
        return false;
    }
    const bool formConversion =
        rangeFunc_ >= KeyFunc::ToHiragana && rangeFunc_ <= KeyFunc::ToHalfAscii;
    return formConversion || !ContainsAsciiLetter(composer_.Commit());
}

std::wstring TextService::RangeExcludedInComposition() const
{
    if (!rangeTracked_ || rangeShift_ <= 0) {
        return L"";
    }
    return rangeBefore_.substr(rangeBefore_.size() - static_cast<size_t>(rangeShift_));
}

void TextService::ApplyRangeContext()
{
    // 縮めて外したかなは読み = 表記の前文脈にする。外したかなが無ければ範囲を始めたときの前文脈
    const std::wstring excluded = RangeExcludedInComposition();
    if (excluded.empty()) {
        contextReading_ = rangeBaseContext_.reading;
        contextSurface_ = rangeBaseContext_.surface;
        return;
    }
    contextReading_ = excluded;
    contextSurface_ = excluded;
}

void TextService::RestoreExcludedKana()
{
    const std::wstring excluded = RangeExcludedInComposition();
    if (excluded.empty()) {
        return;
    }
    // rangeBefore_ は外したかなだけを持つので、積んだ状態は外した文字と同じ数だけあり、
    // 最初に積んだものが縮める前の run 全体の読み
    if (rangeStates_.empty()) {
        composer_.PrependKana(excluded);
    } else {
        composer_ = rangeStates_.front();
    }
    rangeStates_.clear();
    rangeBefore_.erase(rangeBefore_.size() - excluded.size());
    rangeShift_ = 0;
    contextReading_ = rangeBaseContext_.reading;
    contextSurface_ = rangeBaseContext_.surface;
}

HRESULT TextService::ResizeConversionRange(ITfContext* context, int delta)
{
    if (delta < 0) {
        // 外したかなを文書に書かないので、run より前の文書の文字列へは広げない
        if (rangeStates_.empty()) {
            return S_OK;
        }
    } else if (composer_.ConfirmedKana().size() < 2) {
        // 未完成のローマ字 (n) は数えない。縮めた後も範囲に文字を残す
        return S_OK;
    }
    const KeyFunc func = rangeFunc_;
    // 外したかなは composition の先頭に残し (UpdateConvertingDisplay)、文書は確定まで変えない
    ClearConversion();
    if (delta < 0) {
        rangeBefore_.pop_back();
        composer_ = std::move(rangeStates_.back());
        rangeStates_.pop_back();
        --rangeShift_;
    } else {
        rangeStates_.push_back(composer_);
        rangeBefore_ += composer_.ConfirmedKana()[0];
        composer_.RemoveFront(1);
        ++rangeShift_;
    }
    ApplyRangeContext();
    DebugLog(std::wstring(delta < 0 ? L"変換の範囲を広げる (composition 内): "
                                    : L"変換の範囲を縮める (composition 内): ") +
             composer_.Commit());
    return func == KeyFunc::Convert ? StartConversion(context) : ApplyFunctionKey(context, func);
}

void TextService::RememberAdopted(const std::wstring& surface,
                                  std::vector<std::pair<std::wstring, std::wstring>> segments,
                                  const RomajiComposer& composer)
{
    // 未判定の文書でも記憶し、使う時点で読める文書と判定されていて照合で一致したものだけを使う
    if (surface.empty()) {
        return;
    }
    const std::wstring reading = composer.Commit();
    std::wstring readings;
    std::wstring surfaces;
    bool emptyReading = false;
    for (const auto& [segmentReading, segmentSurface] : segments) {
        readings += segmentReading;
        surfaces += segmentSurface;
        emptyReading = emptyReading || segmentReading.empty();
    }
    if (segments.empty() || emptyReading || readings != reading || surfaces != surface) {
        segments.assign(1, {reading, surface});
    }
    if (reading.empty()) {
        return;
    }
    adopted_.push_back({surface, std::move(segments), composer});
    size_t total = 0;
    for (const AdoptedText& memory : adopted_) {
        total += memory.composer.Commit().size();
    }
    while (!adopted_.empty() && total > kMaxReconvertLength) {
        total -= adopted_.front().composer.Commit().size();
        adopted_.erase(adopted_.begin());
    }
}

void TextService::RememberConversion(
    const std::vector<std::pair<std::wstring, std::wstring>>& committed)
{
    if (rangeTracked_) {
        // 範囲が覆っていた記憶 (一部だけ入ったものを含む) を置き換える
        adopted_.resize((std::min)(rangeAdoptedFrom_, adopted_.size()));
        adopted_.insert(adopted_.end(), rangeAdopted_.begin(), rangeAdopted_.end());
        // composition の先頭に残して一緒に確定する外したかなは、かなの1件にする
        const std::wstring kanaBefore = RangeExcludedInComposition();
        if (!kanaBefore.empty()) {
            RomajiComposer kana;
            for (wchar_t c : kanaBefore) {
                kana.PushKana(std::wstring(1, c), std::wstring(1, c));
            }
            RememberAdopted(kanaBefore, {{kanaBefore, kanaBefore}}, kana);
        }
    }
    RememberAdopted(ConvertedText(), committed, composer_);
}

size_t TextService::AdoptedSurfaceLength() const
{
    size_t length = 0;
    for (const AdoptedText& memory : adopted_) {
        length += memory.surface.size();
    }
    return length;
}

size_t TextService::MatchAdopted(const std::wstring& preceding) const
{
    size_t end = preceding.size();
    size_t count = 0;
    for (size_t i = adopted_.size(); i-- > 0;) {
        const std::wstring& surface = adopted_[i].surface;
        if (surface.size() > end || preceding.compare(end - surface.size(), surface.size(),
                                                      surface) != 0) {
            break;
        }
        end -= surface.size();
        ++count;
    }
    return count;
}

void TextService::DropUnmatchedAdopted(const std::wstring& preceding)
{
    const size_t matched = MatchAdopted(preceding);
    if (matched == adopted_.size()) {
        return;
    }
    DebugLog(L"文書と一致しない記憶を捨てる: " + std::to_wstring(adopted_.size() - matched) +
             L" 件");
    adopted_.erase(adopted_.begin(),
                   adopted_.begin() + static_cast<std::ptrdiff_t>(adopted_.size() - matched));
}

void TextService::FindAdoptedChain(const std::wstring& preceding, size_t rangeStart, size_t* from,
                                   size_t* to) const
{
    // 新しい記憶から順に、範囲の先頭より右にかかる記憶を範囲が覆う記憶とする
    size_t end = preceding.size();
    size_t covered = adopted_.size();
    bool chain = false;
    for (size_t i = adopted_.size(); i-- > 0;) {
        if (end <= rangeStart) {
            chain = end == rangeStart;
            break;
        }
        covered = i;
        end -= (std::min)(end, adopted_[i].surface.size());
    }
    *to = covered;
    *from = chain ? 0 : covered;
}

void TextService::BeginRangeAdopted(size_t from, size_t to, bool fromAdopted)
{
    from = (std::min)(from, adopted_.size());
    to = (std::min)((std::max)(from, to), adopted_.size());
    rangeAdoptedFrom_ = from;
    rangeAdopted_.assign(adopted_.begin() + static_cast<std::ptrdiff_t>(from),
                         adopted_.begin() + static_cast<std::ptrdiff_t>(to));
    rangeFromAdopted_ = fromAdopted && !adopted_.empty();
    if (rangeFromAdopted_) {
        rangeOrigin_ = adopted_.back();
    }
}

bool TextService::RangeAdoptedIntact() const
{
    return rangeTracked_ && rangeFromAdopted_ && rangeShift_ == 0;
}

void TextService::PlaceAdoptedCandidate()
{
    // 文節 UI の複数文節には記憶の表記を1候補として置けないので、入力全体の候補選択だけ
    if (!RangeAdoptedIntact() || segments_.size() != 1) {
        return;
    }
    const std::wstring& surface = rangeOrigin_.surface;
    auto& candidates = segments_[0].candidates;
    candidates.erase(std::remove(candidates.begin(), candidates.end(), surface), candidates.end());
    candidates.insert(candidates.begin(), surface);
    selected_[0] = 0;
    // そのまま確定したときは、記憶の文節で学習する (PrepareConversionCommit)
    const bool known = std::any_of(
        wholeCandidates_.begin(), wholeCandidates_.end(),
        [&surface](const SentenceCandidate& candidate) { return candidate.surface == surface; });
    if (!known) {
        wholeCandidates_.push_back({rangeOrigin_.segments, surface});
    }
}

HRESULT TextService::CommitRunDirect(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    if (barIndex_ >= 0) {
        return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::End, L"",
                            nullptr);
    }
    EndAppendRunForPassthroughKey(context);
    return S_OK;
}

HRESULT TextService::CommitRunKey(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    // run 終了時の判定 (ルール3・6・7) も全体変換も適用せず、見えているとおりに終える
    // (Enter で送信するアプリで、見えている文字と送られる文字を食い違わせないため)
    EndAppendRunForPassthroughKey(context);
    return S_OK;
}

HRESULT TextService::AdoptBarItem(ITfContext* context, size_t index, BarAdopt mode,
                                  const std::wstring& suffix, const DirectKey* followKey)
{
    // 採用できないときは選択を解除する (Enter を送り直す経路が、選択中のまま再び
    // 採用を試みて送り直しを繰り返さないように)
    const std::wstring kana = BarReading();
    if (index >= barItems_.size() || kana != barKana_) {
        DeselectBar();
        return E_UNEXPECTED;
    }
    const BarCandidate item = barItems_[index];
    if (item.kind == BarKind::Ascii) {
        return AdoptAsciiCandidate(context, mode, suffix, followKey);
    }
    const bool head = item.kind == BarKind::Head;
    const size_t readingLength = head ? item.reading.size() : kana.size();

    BarAdoption adoption;
    switch (item.kind) {
    case BarKind::Whole:
        adoption.learn = SentenceLearnEntries(item.segments);
        if (!item.segments.empty()) {
            adoption.contextReading = item.segments.back().first;
            adoption.contextSurface = item.segments.back().second;
        }
        adoption.segments = item.segments;
        if (adoption.segments.empty()) {
            adoption.segments.push_back({kana.substr(0, readingLength), item.surface});
        }
        break;
    case BarKind::Prediction:
    case BarKind::Head:
        adoption.learn.push_back({item.reading, item.surface, contextSurface_});
        adoption.contextReading = item.reading;
        adoption.contextSurface = item.surface;
        adoption.segments.push_back({item.reading, item.surface});
        break;
    }
    adoption.readingLength = readingLength;
    adoption.adoptedLength = item.surface.size();
    adoption.endRun = mode == BarAdopt::End || (mode == BarAdopt::CommitKey && !head);
    // 文書に入れるのは採用部分だけ。run を終えるときは残りの読み (未完成のローマ字を含む) と
    // suffix も続けて入れる
    adoption.written = item.surface;
    if (adoption.endRun) {
        adoption.written += composer_.Display().substr(readingLength) + suffix;
    }
    adoption.before = composer_;
    adoption_ = std::move(adoption);
    DebugLog(L"バーの候補を採用: " + adoption_.written);

    // 採用の後も run が続くとき (先頭文節の採用・採用に続く打鍵) は、composition の終了と
    // 次の composition の開始を1つの edit session で行う (分割すると Word や CUAS 経由の
    // アプリで新しい composition が生き残らない)
    RomajiComposer rest = composer_;
    rest.RemoveFront(readingLength);
    const bool restart = runComposition_ != nullptr && !adoption_.endRun &&
                         (!rest.Empty() || followKey != nullptr);
    const HRESULT hr = restart ? RestartRunComposition(context, adoption_.written)
                               : WriteRunText(context, adoption_.written);
    if (FAILED(hr)) {
        DebugLog(L"採用を文書に入れられない: run を破棄");
        DropRun();
        ClearContext();
        return E_FAIL;
    }
    if (restart && rest.Empty()) {
        // 採用で run は終わるが、続く打鍵の run は張り直した composition で始める
        // (FinishBarAdoption の DropRun に composition を終えさせない)
        ITfComposition* next = runComposition_;
        runComposition_ = nullptr;
        FinishBarAdoption(context, false);
        runComposition_ = next;
        return TypeAppend(context, *followKey);
    }
    FinishBarAdoption(context, followKey == nullptr);
    return followKey != nullptr ? TypeAppend(context, *followKey) : S_OK;
}

HRESULT TextService::AdoptAsciiCandidate(ITfContext* context, BarAdopt mode,
                                         const std::wstring& suffix, const DirectKey* followKey)
{
    if (!barHasAscii_ || barAsciiSource_ != composer_.Display()) {
        DeselectBar();
        return E_UNEXPECTED;
    }
    // 読みを英字候補を作った複製に置き換える。採用した部分は英字区間になり、学習は変換せずに
    // 確定したときと同じ (EndRun が英字を含む読みを 読み = 表記 で送る)
    const bool endRun = mode != BarAdopt::Continue;
    composer_ = barAscii_;
    ClearBar();
    std::wstring text = composer_.Commit();
    if (endRun) {
        text += suffix;
    }
    DebugLog(L"英字候補を採用: " + text);
    // 読みを置き換えるだけで、run を終えるときに初めて文書に入れる
    if (endRun) {
        return CommitRunText(context, text);
    }
    if (followKey != nullptr) {
        return TypeAppend(context, *followKey);
    }
    return UpdateRunAndBar(context);
}

void TextService::FinishBarAdoption(ITfContext* context, bool updateBar)
{
    if (!IsNoLearningApp()) {
        engine_.Learn(adoption_.learn);
    }
    SetCommitContext(adoption_.contextReading, adoption_.contextSurface);
    // 採用した部分は run から外す
    composer_.RemoveFront(adoption_.readingLength);
    const std::wstring adopted = adoption_.written.substr(0, adoption_.adoptedLength);
    const std::wstring rest = adoption_.written.substr(adoption_.adoptedLength);
    // 採用した部分を1件として記憶する (残りのかなは run の終わりで別の1件になる)
    std::wstring adoptedReading;
    for (const auto& segment : adoption_.segments) {
        adoptedReading += segment.first;
    }
    RememberAdopted(adopted, adoption_.segments,
                    ComposerForReading(adoption_.before, adoptedReading));
    AppendLlmHistory(adopted);
    ClearBar();
    // run の先頭が変わったので、次にバーを出す位置はその時点で取り直す
    barXFixed_ = false;
    if (adoption_.endRun && !composer_.ConfirmedKana().empty()) {
        // 先頭文節を採用して run を終える: 残りのかなは採用せずに終えた run と同じ扱い
        EndRun(rest);
        return;
    }
    if (adoption_.endRun || composer_.Empty()) {
        // 採用した部分で run が終わる。確定アンドゥは採用前の読みに戻す
        AppendLlmHistory(rest);
        lastCommitText_ = adoption_.written;
        lastComposer_ = adoption_.before;
        DropRun();
        return;
    }
    ExtendLlmContext(adopted);
    // 続く run の手前は採用した文字列なので、それより前の確定アンドゥの記憶は使えない
    lastCommitText_.clear();
    UpdateRunDisplay(context);
    if (updateBar) {
        UpdateBar(context);
    }
}

void TextService::ShowNextWordBar(ITfContext* context)
{
    const TsfConfig& config = config_.Get();
    if (!config.candidateBar || !config.nextWord || context == nullptr || InRun() ||
        Composing() || converting_) {
        return;
    }
    // エンジン未接続・前文脈が空・NEXTWORD を知らない旧エンジンでは false が返る
    std::vector<PredictionCandidate> candidates;
    if (!engine_.NextWord(CurrentContext(), &candidates) || candidates.empty()) {
        return;
    }
    std::vector<BarCandidate> items;
    for (const PredictionCandidate& candidate : candidates) {
        if (items.size() >= SuggestPanel::kMaxItems) {
            break;
        }
        items.push_back({BarKind::Prediction, candidate.surface, candidate.reading, {}});
    }
    RECT caret = {};
    const bool gotCaret = CaretRect(context, &caret);
    if (!gotCaret && !config.candidateCenter) {
        return;
    }
    ClearBar();
    // run のバーと違って読みが伸びないので、出すたびにその時点の位置に出す。次の run の
    // バーは run で最初に出した時点の位置に固定し直す (barXFixed_ は立てない)
    if (gotCaret) {
        barX_ = caret.left;
    }
    // 候補の縁と入力行の前文脈は、後続語の予測であることで決まる
    nextWordBar_ = true;
    if (!ShowBarItems(gotCaret ? &caret : nullptr, std::move(items))) {
        ClearBar();
        return;
    }
    DebugLog(L"後続語の予測を表示: " + std::to_wstring(barItems_.size()) + L" 件 (前文脈 " +
             contextSurface_ + L")");
}

void TextService::ShowNextWordAfterCommit(ITfContext* context)
{
    ShowNextWordBar(context);
}

bool TextService::NextWordSelected() const
{
    return nextWordBar_ && barIndex_ >= 0 && static_cast<size_t>(barIndex_) < barItems_.size();
}

void TextService::DismissNextWordForKey(WPARAM wparam)
{
    if (!nextWordBar_ || IsModifierKey(wparam)) {
        return;
    }
    const KeyFunc func = MatchKeyFunc(wparam).func;
    bool keep = func == KeyFunc::NextWordNext || func == KeyFunc::NextWordPrev;
    if (!keep && barIndex_ >= 0) {
        const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        if (func != KeyFunc::None) {
            // 変換キーなどは予測を消してから入力なしの機能として処理する
            keep = func == KeyFunc::CommitRun || func == KeyFunc::ForgetLearning;
        } else if (ctrl || alt) {
            keep = ctrl && !alt && wparam == 'M';
        } else {
            switch (wparam) {
            case VK_RETURN:
            case VK_ESCAPE:
            case VK_TAB:
            case VK_UP:
            case VK_DOWN:
            case VK_SPACE:
                keep = true;
                break;
            default:
                keep = IsPrintableKey(wparam, IsShiftPressed());
                break;
            }
        }
    }
    if (!keep) {
        DebugLog(L"打鍵で後続語の予測を消す vk=" + std::to_wstring(wparam));
        ClearBar();
    }
}

HRESULT TextService::MoveNextWordSelection(int delta)
{
    if (!nextWordBar_ || barItems_.empty()) {
        return S_OK;
    }
    if (delta < 0 && barIndex_ < 0) {
        barIndex_ = static_cast<int>(barItems_.size()) - 1;
        suggestPanel_.SetSelection(static_cast<size_t>(barIndex_));
        return S_OK;
    }
    return MoveBarSelection(delta);
}

HRESULT TextService::AdoptNextWord(ITfContext* context, BarAdopt mode,
                                   const std::wstring& suffix, const DirectKey* followKey)
{
    if (!NextWordSelected()) {
        ClearBar();
        return E_UNEXPECTED;
    }
    const BarCandidate item = barItems_[static_cast<size_t>(barIndex_)];
    ClearBar();
    const std::wstring text = item.surface + suffix;
    DebugLog(L"後続語の予測を採用: " + text);
    if (FAILED(InsertText(context, text))) {
        DebugLog(L"後続語の予測の追記に失敗");
        ClearContext();
        return E_FAIL;
    }
    if (!IsNoLearningApp()) {
        engine_.Learn({{item.reading, item.surface, contextSurface_}});
    }
    SetCommitContext(item.reading, item.surface);
    const RomajiComposer composer = ComposerForReading(RomajiComposer(), item.reading);
    RememberAdopted(item.surface, {{item.reading, item.surface}}, composer);
    // 確定アンドゥは suffix (スペース) まで含めて読みのかなに戻す (run を Space で終えたときと同じ)
    lastCommitText_ = text;
    lastComposer_ = composer;
    if (followKey != nullptr) {
        return TypeAppend(context, *followKey);
    }
    if (mode == BarAdopt::CommitKey) {
        ShowNextWordBar(context);
    }
    return S_OK;
}

HRESULT TextService::UpdateRunAndBar(ITfContext* context)
{
    const HRESULT hr = UpdateRunDisplay(context);
    UpdateBar(context);
    return hr;
}

void TextService::EndRun(const std::wstring& text)
{
    if (!text.empty()) {
        // 無変換の確定: 英字を含む入力 (英単語など) は読み=表記で学習し、
        // 確定したかなは次の変換の文脈にする (かなのみの学習はしない。学習は変換候補の
        // 並び替えにも使われ、「きょう→きょう」が入るとひらがな候補が先頭へ来るため)
        const std::wstring kana = composer_.Commit();
        if (ContainsAsciiLetter(kana) && !IsNoLearningApp()) {
            engine_.Learn({{kana, kana, contextSurface_}});
        }
        SetCommitContext(kana, kana);
        // Space で終えた run は IME が入れたスペースで終わる。スペースは記憶に含めず、
        // 記憶の照合をそこで途切れさせる
        std::wstring trimmed = text;
        while (!trimmed.empty() && (trimmed.back() == L' ' || trimmed.back() == L'　')) {
            trimmed.pop_back();
        }
        RememberAdopted(trimmed, {{kana, trimmed}}, composer_);
        lastCommitText_ = text;
        lastComposer_ = composer_;
        AppendLlmHistory(text);
    }
    DropRun();
}

void TextService::DropRun()
{
    if (runComposition_ != nullptr) {
        EndRunComposition(L"");
    }
    ClearConversion();
    ClearBar();
    ResetRerank();
    llmRunContext_.clear();
    barXFixed_ = false;
    composer_.Clear();
    rangeTracked_ = false;
    rangeStates_.clear();
}

bool TextService::CanUndoCommit(ITfContext* context) const
{
    if (lastCommitText_.empty() || !DocumentReadable()) {
        return false;
    }
    std::wstring before;
    return ReadTextBeforeCaret(context, lastCommitText_, &before);
}

HRESULT TextService::UndoCommit(ITfContext* context)
{
    // 一度きりの操作として、成否に関わらず記憶を消す (内容が一致しない = 確定後に
    // 別の編集があった場合に、以降の Ctrl+Backspace を奪い続けないようにする)
    const std::wstring commitText = lastCommitText_;
    lastCommitText_.clear();
    if (commitText.empty() || lastComposer_.Empty() || InRun() || Composing() ||
        context == nullptr) {
        return S_OK;
    }
    ClassifyByLastInsert(context);
    // 読み戻せない文書では、消す文字列がキャレット直前にあるかを確かめられない
    if (!DocumentReadable()) {
        DebugLog(L"読めると判定されていない文書では確定アンドゥしない: " + commitText);
        return S_OK;
    }
    // キャレット直前が直前の確定文字列と一致する場合のみ消す
    if (ReplaceRunText(context, commitText, L"") != ReplaceRunResult::Succeeded) {
        return S_OK;
    }
    // 読みに戻した確定の記憶は捨てる (確定文字列は記憶の表記の後ろにスペースなどが付きうる)
    if (!adopted_.empty() && StartsWith(commitText, adopted_.back().surface)) {
        adopted_.pop_back();
    }
    composer_ = lastComposer_;
    // 確定を取り消したので、その確定を前提にした文脈補正はもう使えない。
    // run を始める前の時点が無いので、LLM の左文脈も無しにする
    ClearContext();
    ResetRerank();
    llmRunContext_.clear();
    BeginRunMode();
    if (runMode_ == RunMode::Composition) {
        StartRunComposition(context);
    }
    return UpdateRunAndBar(context);
}

TextService::PromoteResult TextService::PromoteRun(ITfContext* context, const std::wstring& text,
                                                   size_t caretOffset, size_t selectLength)
{
    // 既存の文字列の範囲に composition を張ると、composition を独自に解釈するエディタ
    // (Discord の入力欄) が確定済みの文字列を内部モデルから消さず、確定時に元のかなが
    // 残る。そのため対象の文字列を消してから空の composition を張る
    const ReplaceRunResult match =
        ReplaceRunRange(context, text, caretOffset, selectLength, L"", 0, 0);
    if (match != ReplaceRunResult::Succeeded) {
        DebugLog(std::wstring(L"変換の対象を消せない (") + MatchReason(match) + L"): run を破棄");
        DropRun();
        ClearContext();
        return PromoteResult::Dropped;
    }
    const HRESULT hr = StartComposition(context);
    if (FAILED(hr) || !Composing()) {
        DebugLog(L"昇格の composition を開始できない: 消した文字列を戻す");
        const HRESULT restored =
            RequestSync(context, new (std::nothrow) InsertTextEditSession(context, text),
                        TF_ES_SYNC | TF_ES_READWRITE);
        if (FAILED(restored)) {
            DropRun();
            ClearContext();
            return PromoteResult::Dropped;
        }
        return PromoteResult::Refused;
    }
    promoted_ = true;
    // composer_・文脈はそのまま候補選択へ引き継ぐ (確定ではないので学習・確定アンドゥの記憶は
    // しない)
    ClearBar();
    return PromoteResult::Promoted;
}

void TextService::DemoteIfLeftConversion(ITfContext* context)
{
    if (!promoted_) {
        return;
    }
    if (!Composing()) {
        promoted_ = false;
        return;
    }
    if (converting_) {
        return;
    }
    promoted_ = false;
    if (runMode_ == RunMode::Composition && !composer_.Empty()) {
        // composition は終えずに読みの表示に戻し、run の composition として続ける
        runComposition_ = composition_;
        composition_ = nullptr;
        UpdateRunAndBar(context);
        return;
    }
    // パネル保持入力は composition を空の文字列で終え、読みを入力行に戻す
    HRESULT hr = E_UNEXPECTED;
    if (context != nullptr) {
        hr = RequestSync(context,
                         new (std::nothrow) EndCompositionEditSession(context, composition_, L""),
                         TF_ES_SYNC | TF_ES_READWRITE);
    }
    composition_->Release();
    composition_ = nullptr;
    if (FAILED(hr) || composer_.Empty()) {
        DropRun();
        ClearContext();
        return;
    }
    // composer_ は維持し、run として続ける
    UpdateBar(context);
}

// ---- 文書の分類と run の表示 ----

void TextService::DebugLog(const std::wstring& message) const
{
    WriteDebugLog(config_.Get().debugLog, message);
}

const wchar_t* TextService::AppendDocumentName(AppendDocument document)
{
    return document == AppendDocument::Readable ? L"読める" : L"未判定";
}

void TextService::SetAppendDocument(AppendDocument document, const wchar_t* reason)
{
    if (document != appendDocument_) {
        DebugLog(std::wstring(L"文書の分類: ") + AppendDocumentName(appendDocument_) + L" -> " +
                 AppendDocumentName(document) + L" (" + reason + L")");
    }
    appendDocument_ = document;
}

HRESULT TextService::TypeAppend(ITfContext* context, const DirectKey& key)
{
    if (!InRun()) {
        BeginRunMode();
        ClassifyByLastInsert(context);
        // run の最初の打鍵の前に左文脈を取る (composition 入力は読みが文書に入るため)
        CaptureLlmContext(context);
    }
    PushDirectKey(key);
    if (composer_.Empty()) {
        DropRun();
        return S_OK;
    }
    if (runMode_ == RunMode::Composition && runComposition_ == nullptr) {
        StartRunComposition(context);
    }
    return UpdateRunAndBar(context);
}

void TextService::BeginRunMode()
{
    runMode_ = config_.Get().panelBuffer ? RunMode::Panel : RunMode::Composition;
    if (runMode_ == RunMode::Panel && runComposition_ != nullptr) {
        // 採用に続く打鍵のために張り直した composition は、方式が変わったら使わない
        EndRunComposition(L"");
    }
}

bool TextService::StartRunComposition(ITfContext* context)
{
    // 開始と同じ session で文字列を設定すると CUAS が WM_IME_COMPOSITION を生成しないため、
    // 文字列は呼び出し側が UpdateRunDisplay で別の session として設定する
    StartComposition(context);
    if (!Composing()) {
        DebugLog(L"run の composition を張れない: パネル保持入力として続ける");
        runMode_ = RunMode::Panel;
        return false;
    }
    runComposition_ = composition_;
    composition_ = nullptr;
    return true;
}

HRESULT TextService::EndRunComposition(const std::wstring& text)
{
    if (runComposition_ == nullptr) {
        return S_OK;
    }
    ITfComposition* composition = runComposition_;
    runComposition_ = nullptr;
    // フォーカス移動や IME オフの通知など、呼び出し側が context を持たない場面でも終えられる
    // ように、composition の範囲から context を得る
    ITfContext* context = nullptr;
    ITfRange* range = nullptr;
    if (SUCCEEDED(composition->GetRange(&range)) && range != nullptr) {
        range->GetContext(&context);
        range->Release();
    }
    HRESULT hr = E_FAIL;
    if (context != nullptr) {
        hr = RequestSync(context,
                         new (std::nothrow) EndCompositionEditSession(context, composition, text),
                         TF_ES_SYNC | TF_ES_READWRITE);
        context->Release();
    }
    composition->Release();
    if (SUCCEEDED(hr) && !text.empty()) {
        lastInserted_ = text;
    }
    if (FAILED(hr)) {
        DebugLog(L"run の composition を終えられない hr=" + DebugHResultText(hr));
    }
    return hr;
}

HRESULT TextService::RestartRunComposition(ITfContext* context, const std::wstring& commitText)
{
    if (runComposition_ == nullptr || context == nullptr) {
        return E_UNEXPECTED;
    }
    ITfComposition* next = nullptr;
    const HRESULT hr = RequestSync(context,
                                   new (std::nothrow) RestartCompositionEditSession(
                                       context, runComposition_, commitText,
                                       static_cast<ITfCompositionSink*>(this), &next),
                                   TF_ES_SYNC | TF_ES_READWRITE);
    runComposition_->Release();
    runComposition_ = next;
    if (next == nullptr) {
        DebugLog(L"run の composition を張り直せない hr=" + DebugHResultText(hr));
        runMode_ = RunMode::Panel;
        return FAILED(hr) ? hr : E_FAIL;
    }
    if (!commitText.empty()) {
        lastInserted_ = commitText;
    }
    return S_OK;
}

HRESULT TextService::UpdateRunDisplay(ITfContext* context)
{
    if (runComposition_ == nullptr) {
        return S_OK;
    }
    if (context == nullptr) {
        return E_INVALIDARG;
    }
    return RequestSync(context,
                       new (std::nothrow) UpdateCompositionEditSession(
                           context, runComposition_, composer_.Display(), inputAttribute_),
                       TF_ES_SYNC | TF_ES_READWRITE);
}

bool TextService::CaretRect(ITfContext* context, RECT* rect, const wchar_t** source)
{
    bool succeeded = false;
    if (context != nullptr && runComposition_ != nullptr) {
        // composition 入力のバーは composition の位置に合わせる (選択は composition の末尾にある)
        RequestSync(context,
                    new (std::nothrow)
                        GetTextExtentEditSession(context, runComposition_, rect, &succeeded),
                    TF_ES_SYNC | TF_ES_READ);
        if (succeeded && source != nullptr) {
            *source = L"run の composition の矩形";
        }
    }
    if (!succeeded && context != nullptr) {
        RequestSync(context,
                    new (std::nothrow) GetSelectionExtentEditSession(context, rect, &succeeded),
                    TF_ES_SYNC | TF_ES_READ);
        if (succeeded && source != nullptr) {
            *source = L"選択範囲の矩形";
        }
    }
    if (!succeeded) {
        // 選択範囲の矩形を返さない文書でも、システムキャレットがあればその位置に出す
        GUITHREADINFO info = {};
        info.cbSize = sizeof(info);
        if (GetGUIThreadInfo(0, &info) && info.hwndCaret != nullptr) {
            *rect = info.rcCaret;
            MapWindowPoints(info.hwndCaret, HWND_DESKTOP, reinterpret_cast<POINT*>(rect), 2);
            succeeded = true;
            if (source != nullptr) {
                *source = L"キャレット (GetGUIThreadInfo、選択範囲の矩形は取得失敗)";
            }
        }
    }
    if (succeeded) {
        FallbackIfOutsideWindow(context, rect, L"候補バーの位置");
    }
    return succeeded;
}

void TextService::FallbackIfOutsideWindow(ITfContext* context, RECT* rect,
                                          const wchar_t* where) const
{
    HWND root = nullptr;
    if (context != nullptr) {
        ITfContextView* view = nullptr;
        if (SUCCEEDED(context->GetActiveView(&view)) && view != nullptr) {
            HWND hwnd = nullptr;
            if (SUCCEEDED(view->GetWnd(&hwnd)) && hwnd != nullptr) {
                root = GetAncestor(hwnd, GA_ROOT);
            }
            view->Release();
        }
    }
    if (root == nullptr) {
        // CUAS 経由などビューがウィンドウを返さない文書では、入力を受けているスレッドの
        // フォーカス (無ければアクティブ) のウィンドウを文書のウィンドウとみなす
        GUITHREADINFO info = {};
        info.cbSize = sizeof(info);
        if (GetGUIThreadInfo(GetCurrentThreadId(), &info)) {
            HWND target = info.hwndFocus != nullptr ? info.hwndFocus : info.hwndActive;
            if (target != nullptr) {
                root = GetAncestor(target, GA_ROOT);
            }
        }
    }
    RECT rootRect = {};
    if (root == nullptr || !GetWindowRect(root, &rootRect)) {
        return;
    }
    // 幅・高さ 0 の矩形も来るため、矩形の重なりではなく左上の点で判定する
    const POINT origin = {rect->left, rect->top};
    if (PtInRect(&rootRect, origin)) {
        return;
    }
    const RECT original = *rect;
    const LONG x = rootRect.left + (rootRect.right - rootRect.left) / 2;
    const LONG y = rootRect.top + (rootRect.bottom - rootRect.top) / 2;
    *rect = {x, y, x, y};
    DebugLog(std::wstring(where) + L": 文書のウィンドウの外のため代わりの位置を使う 元=" +
             DebugRectText(original) + L" ウィンドウ=" + DebugRectText(rootRect) + L" 代わり=" +
             DebugRectText(*rect));
}

HRESULT TextService::BackspaceAppend(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    // 読みは文書に無いので、Backspace は常に食べて読みの末尾を削る。空になれば run を終える
    // (composition 入力は composition を空で終える)
    composer_.Backspace();
    ResolveAsciiRequest(&composer_);
    if (composer_.Empty()) {
        DropRun();
        return S_OK;
    }
    return UpdateRunAndBar(context);
}

HRESULT TextService::SpaceAppend(ITfContext* context, bool shifted)
{
    const bool asciiWord = config_.Get().modeless && composer_.AsciiMode();
    const std::wstring space =
        (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
    return CommitRunText(context, composer_.Commit() + space);
}

void TextService::EndAppendRunForPassthroughKey(ITfContext* context)
{
    // バーの選択は採用せず、見えているとおりの読み (未完成のローマ字を含む) で run を終える。
    // run 終了時の判定 (ルール3・6・7) も適用しない。Enter で送信するアプリで、見えている
    // 文字と送られる文字を食い違わせないため
    barIndex_ = -1;
    CommitRunText(context, composer_.Display());
}

HRESULT TextService::BeginAppendConversion(ITfContext* context, KeyFunc func)
{
    rangeTracked_ = false;
    rangeStates_.clear();
    if (runComposition_ != nullptr) {
        // composition 入力は run の composition をそのまま候補選択に使う
        composition_ = runComposition_;
        runComposition_ = nullptr;
    } else {
        // パネル保持入力は文書に読みが無いので、キャレット位置へ空の composition を張る
        StartComposition(context);
        if (!Composing()) {
            DebugLog(L"変換の composition を開始できない: run を続ける");
            return S_OK;
        }
    }
    promoted_ = true;
    ClearBar();
    // 外したかなは composition の先頭に残し、文書には書かない
    BeginRangeTracking();
    return func == KeyFunc::Convert ? StartConversion(context) : ApplyFunctionKey(context, func);
}

void TextService::NoteKeyForAppend(WPARAM wparam, bool eaten)
{
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    if (!eaten && (wparam == VK_RETURN || (ctrl && wparam == 'M'))) {
        // 改行・送信をアプリへ渡したら、IME が入れた文字列はもう同じ行の前方ではない
        llmHistory_.clear();
    }
}

// ---- run の終了と文書への書き込み ----

HRESULT TextService::WriteRunText(ITfContext* context, const std::wstring& text)
{
    if (runComposition_ != nullptr) {
        const HRESULT hr = EndRunComposition(text);
        if (SUCCEEDED(hr)) {
            DebugLog(L"run の composition を確定: " + text);
        }
        return hr;
    }
    if (text.empty()) {
        return S_OK;
    }
    if (context == nullptr) {
        return E_INVALIDARG;
    }
    const HRESULT hr = RequestSync(context, new (std::nothrow) InsertTextEditSession(context, text),
                                   TF_ES_SYNC | TF_ES_READWRITE);
    if (SUCCEEDED(hr)) {
        lastInserted_ = text;
        DebugLog(L"パネルの読みを挿入: " + text);
    }
    return hr;
}

HRESULT TextService::CommitRunText(ITfContext* context, const std::wstring& text)
{
    const HRESULT hr = WriteRunText(context, text);
    if (FAILED(hr)) {
        DebugLog(L"run の文字列を文書に入れられない: run を破棄");
        DropRun();
        ClearContext();
        return hr;
    }
    EndRun(text);
    return S_OK;
}

void TextService::ClassifyBeforeIdleKey(ITfContext* context, WPARAM wparam)
{
    if (InRun() || Composing() || lastInserted_.empty() ||
        appendDocument_ != AppendDocument::Unknown || IsModifierKey(wparam) ||
        !IsKeyboardOpen()) {
        return;
    }
    // 入力なしの変換と確定アンドゥはキャレット直前を読めると分かっている文書でしか働かず、
    // 食べるかどうかをその分類で決める (CanRunKeyFunc) ので、その前に分類しておく
    const KeyFunc func = MatchKeyFunc(wparam).func;
    if (func == KeyFunc::Convert || func == KeyFunc::UndoCommit ||
        (func >= KeyFunc::ToHiragana && func <= KeyFunc::ToHalfAscii)) {
        ClassifyByLastInsert(context);
    }
}

bool TextService::PanelKeyNeedsResend(ITfContext* context, WPARAM wparam) const
{
    // 読める文書では文書への書き込みが TSF の文書へ同期的に入るので、食べずに渡しても後に届く
    if (!InRun() || DocumentReadable() || IsModifierKey(wparam)) {
        return false;
    }
    const KeyMatch match = MatchKeyFunc(wparam);
    if (match.func != KeyFunc::None && CanRunKeyFunc(context, match.func)) {
        return false;
    }
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    // Alt 併用は run を終えずにアプリへ渡す (EndRunIfPassthroughKey) ので送り直さない
    if (alt) {
        return false;
    }
    if (ctrl) {
        // Ctrl+H は Backspace の読み替えで、IME が読みを削る
        return wparam != 'H';
    }
    switch (wparam) {
    case VK_TAB:
    case VK_UP:
    case VK_DOWN:
        // バーが出ていればバーの選択に使う
        return barItems_.empty();
    default:
        return IsEditingKey(wparam);
    }
}

HRESULT TextService::EndPanelRunAndResend(ITfContext* context, WPARAM wparam)
{
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool shifted = IsShiftPressed();
    appendResendVk_ = wparam;
    appendResendCtrl_ = ctrl;
    appendResendShift_ = shifted;
    const bool enter = ctrl ? wparam == 'M' : wparam == VK_RETURN;
    HRESULT hr = S_OK;
    if (enter && barIndex_ >= 0) {
        hr = CommitRunDirect(context);
    } else {
        EndAppendRunForPassthroughKey(context);
    }
    SendResendKey();
    return hr;
}

void TextService::ClassifyByLastInsert(ITfContext* context)
{
    if (context == nullptr) {
        return;
    }
    const std::wstring inserted = lastInserted_;
    lastInserted_.clear();
    if (appendDocument_ != AppendDocument::Unknown || inserted.empty()) {
        return;
    }
    ReplaceRunResult match = ReplaceRunResult::Unsupported;
    RequestSync(context,
                new (std::nothrow) MatchRunEditSession(context, inserted, inserted.size(), &match),
                TF_ES_SYNC | TF_ES_READ);
    if (match == ReplaceRunResult::Succeeded) {
        SetAppendDocument(AppendDocument::Readable, L"直前の挿入を読み戻し成功");
        return;
    }
    // 挿入の後にアプリへ渡したキーで文書が変わったのか、読めない文書なのかを区別できないので、
    // 未判定のまま次の挿入でまた判定する
    DebugLog(std::wstring(L"直前の挿入の読み戻しが一致しない (") + MatchReason(match) +
             L"): 未判定のまま");
}
