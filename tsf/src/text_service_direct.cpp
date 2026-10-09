// run (打鍵したかなを未確定文字列を使わずに文書へ追記する入力) のキー処理。
// TextService のメンバ関数のうち run に固有のものをこの翻訳単位に置く
// (設計は docs/design/direct-input.md、docs/design/append-input.md)
#include "text_service.h"

#include <new>
#include <vector>

#include "debug_log.h"
#include "edit_session.h"
#include "globals.h"

using namespace key_util;

namespace {

// 擬似打鍵 (SendInput) の dwExtraInfo。ログで擬似打鍵かどうかを確かめるための目印
constexpr ULONG_PTR kPseudoKeyExtraInfo = 0x514C4D45;
// 擬似 Backspace の後ろに送る目印の打鍵 (どのアプリも使わない仮想キー)
constexpr WPARAM kMarkerVk = VK_F24;
// Alt・Win を離す前に挟む打鍵。Alt・Win を単独で押して離すとアプリのメニュー・
// スタートメニューが開くため、間に他の打鍵があったことにして起動を打ち消す。
// 0xE8 は割り当ての無い仮想キーで、押してもアプリ側で何も起きない
constexpr WORD kMenuMaskVk = 0xE8;

// 擬似 Backspace の前に離す修飾キー。押したままだとアプリには Ctrl+Backspace
// (単語削除) や Alt+Backspace (元に戻す) として届く
constexpr WORD kReleasedModifierVks[] = {VK_LCONTROL, VK_RCONTROL, VK_LSHIFT, VK_RSHIFT,
                                         VK_LMENU,    VK_RMENU,    VK_LWIN,   VK_RWIN};

// 追記のみの文書でマウスフックが run を終えるメッセージ (ボタンを押す操作とホイール)
bool IsCaretMovingMouseMessage(WPARAM message)
{
    switch (message) {
    case WM_LBUTTONDOWN:
    case WM_LBUTTONDBLCLK:
    case WM_RBUTTONDOWN:
    case WM_RBUTTONDBLCLK:
    case WM_MBUTTONDOWN:
    case WM_MBUTTONDBLCLK:
    case WM_XBUTTONDOWN:
    case WM_XBUTTONDBLCLK:
    case WM_NCLBUTTONDOWN:
    case WM_NCLBUTTONDBLCLK:
    case WM_NCRBUTTONDOWN:
    case WM_NCRBUTTONDBLCLK:
    case WM_NCMBUTTONDOWN:
    case WM_NCMBUTTONDBLCLK:
    case WM_NCXBUTTONDOWN:
    case WM_NCXBUTTONDBLCLK:
    case WM_MOUSEWHEEL:
    case WM_MOUSEHWHEEL:
        return true;
    default:
        return false;
    }
}

// マウスフックを仕掛けた TextService。WH_MOUSE のスレッド限定フックはそのスレッドで
// 呼ばれ、TextService はスレッドごとに1つなので、スレッドローカルで持てば足りる
thread_local TextService* t_mouseHookOwner = nullptr;

bool StartsWith(const std::wstring& text, const std::wstring& prefix)
{
    return text.size() >= prefix.size() && text.compare(0, prefix.size(), prefix) == 0;
}

// 末尾の表示上の1文字の UTF-16 単位数 (サロゲートペアなら 2)
size_t LastCharLength(const std::wstring& text)
{
    if (text.empty()) {
        return 0;
    }
    if (text.size() >= 2 && IS_LOW_SURROGATE(text.back()) &&
        IS_HIGH_SURROGATE(text[text.size() - 2])) {
        return 2;
    }
    return 1;
}

// 表示上の文字数 (サロゲートペアを1文字と数える)。擬似 Backspace の回数に使う
size_t DisplayCharCount(const std::wstring& text)
{
    size_t count = 0;
    for (size_t i = 0; i < text.size(); ++i) {
        if (i + 1 < text.size() && IS_HIGH_SURROGATE(text[i]) &&
            IS_LOW_SURROGATE(text[i + 1])) {
            ++i;
        }
        ++count;
    }
    return count;
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

// count 個の Backspace と目印の打鍵を送る。送信時点で押されている修飾キーは
// Backspace の前に離し、後で押し直す。全部送れたら true
bool SendPseudoBackspaces(size_t count)
{
    std::vector<INPUT> inputs;
    const auto push = [&inputs](WORD vk, DWORD flags) {
        INPUT input = {};
        input.type = INPUT_KEYBOARD;
        input.ki.wVk = vk;
        input.ki.wScan = static_cast<WORD>(MapVirtualKeyW(vk, MAPVK_VK_TO_VSC));
        // 右側の Ctrl・Alt と Win は拡張キーとして送らないと左側のキーと区別されない
        if (vk == VK_RCONTROL || vk == VK_RMENU || vk == VK_LWIN || vk == VK_RWIN) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        input.ki.dwFlags = flags;
        input.ki.dwExtraInfo = kPseudoKeyExtraInfo;
        inputs.push_back(input);
    };
    std::vector<WORD> held;
    bool menuKeyHeld = false;
    for (WORD vk : kReleasedModifierVks) {
        if ((GetAsyncKeyState(vk) & 0x8000) != 0) {
            held.push_back(vk);
            if (vk == VK_LMENU || vk == VK_RMENU || vk == VK_LWIN || vk == VK_RWIN) {
                menuKeyHeld = true;
            }
        }
    }
    if (menuKeyHeld) {
        push(kMenuMaskVk, 0);
        push(kMenuMaskVk, KEYEVENTF_KEYUP);
    }
    for (WORD vk : held) {
        push(vk, KEYEVENTF_KEYUP);
    }
    for (size_t i = 0; i < count; ++i) {
        push(VK_BACK, 0);
        push(VK_BACK, KEYEVENTF_KEYUP);
    }
    for (WORD vk : held) {
        push(vk, 0);
    }
    if (menuKeyHeld) {
        // 押し直した Alt・Win をユーザが離したときにも、メニューが開かないようにする
        // (目印の打鍵は IME が食べるのでアプリには届かない)
        push(kMenuMaskVk, 0);
        push(kMenuMaskVk, KEYEVENTF_KEYUP);
    }
    push(static_cast<WORD>(kMarkerVk), 0);
    push(static_cast<WORD>(kMarkerVk), KEYEVENTF_KEYUP);
    const UINT sent =
        SendInput(static_cast<UINT>(inputs.size()), inputs.data(), sizeof(INPUT));
    return sent == inputs.size();
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

// 擬似 Backspace と一緒に送る打鍵 (修飾キーの離し・押し直しとメニュー打ち消し)
bool IsPseudoCompanionKey(WPARAM wparam)
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
    case kMenuMaskVk:
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
constexpr size_t kMaxReconvertLength = 127;

} // namespace

bool TextService::DocumentReadable() const
{
    return appendDocument_ == AppendDocument::Readable;
}

bool TextService::IsKeyEatenDirect(WPARAM wparam) const
{
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    // Backspace は、未完成のローマ字が無ければアプリへ渡して文書の文字を消させる
    // (run は NoteKeyForAppend で追従させる)
    const bool backspaceEaten = converting_ || !AppendPendingText().empty();
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
    if (Composing()) {
        // 昇格した候補選択中も、昇格できなかった run と同じく確定してからアプリへ渡す
        CommitComposition(context);
        return;
    }
    const bool enter = !alt && (ctrl ? wparam == 'M' : wparam == VK_RETURN);
    if (enter && (converting_ || barIndex_ >= 0)) {
        // 候補選択中は確定、バー選択中は採用して run を終えてから渡す
        // (EnterNeedsResend のときは食べているのでここには来ない)
        CommitRunDirect(context);
        return;
    }
    EndAppendRunForPassthroughKey(context, ctrl, alt);
}

bool TextService::AdoptionNeedsPseudoBackspace() const
{
    return !converting_ && barIndex_ >= 0 && appendDocument_ != AppendDocument::Readable &&
           !surface_.empty();
}

bool TextService::EnterNeedsResend() const
{
    // 読める文書は確定が TSF の文書へ同期的に入るので、食べずに渡しても改行より先に入る。
    // CUAS 経由のアプリ (WezTerm など) は composition の確定結果を IME メッセージで後から
    // 受け取るため、食べずに渡すと確定結果より先に Enter が処理される。擬似 Backspace が
    // 要る採用は、書き換えが終わる前に Enter が届かないようにする
    return appendDocument_ != AppendDocument::Readable &&
           (converting_ || AdoptionNeedsPseudoBackspace());
}

HRESULT TextService::CommitAndResendEnter(ITfContext* context, WPARAM vk, bool ctrl, bool shifted)
{
    // 送り直した打鍵は入力キューの後ろに並ぶので、確定結果のメッセージより後に処理される。
    // 擬似 Backspace を送ったときは、書き換えが終わってから (目印の打鍵を受け取った後に) 送る
    appendResendVk_ = vk;
    appendResendCtrl_ = ctrl;
    appendResendShift_ = shifted;
    const HRESULT hr = Composing() ? CommitComposition(context) : CommitRunDirect(context);
    if (!awaitingMarker_) {
        SendResendKey();
    }
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
        if (!converting_ && barIndex_ >= 0) {
            return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::Continue, L"",
                                &key);
        }
        // 候補選択中の印字キーは選択を確定して新しい run を始める
        if (converting_) {
            CommitRunDirect(context);
            if (awaitingMarker_) {
                appendFollowKey_ = key;
                appendFollowKeyPending_ = true;
                return S_OK;
            }
        }
        return TypeAppend(context, key);
    }
    switch (wparam) {
    case VK_RETURN:
        // 食べるのは EnterNeedsResend のときだけ
        return CommitAndResendEnter(context, originalVk, ctrl, shifted);
    case VK_ESCAPE:
        if (converting_) {
            return CancelConversion(context); // 変換前のかな表示に戻す
        }
        if (barIndex_ >= 0) {
            return DeselectBar();
        }
        // 文字は文書に残したまま run だけ忘れる (未完成のローマ字も文書に残す)
        if (InRun()) {
            const bool mismatch = FlushAppendPending(context) == AppendResult::Mismatch;
            EndRun();
            if (mismatch) {
                ClearContext();
            }
        }
        return S_OK;
    case VK_BACK:
        if (converting_) {
            return CancelConversion(context);
        }
        return BackspaceAppend(context);
    case VK_SPACE:
        return SpaceDirect(context, shifted);
    case VK_TAB:
        if (converting_) {
            // 昇格した候補選択中と同じく、変換を取り消してバーの先頭を選ぶ
            // (バーが無ければかな表示に戻るだけ)
            const HRESULT hr = CancelConversion(context);
            if (FAILED(hr)) {
                return hr;
            }
            return MoveBarSelection(+1);
        }
        return MoveBarSelection(shifted ? -1 : +1);
    case VK_DOWN:
        return converting_ ? CycleCandidate(context, +1) : MoveBarSelection(+1);
    case VK_UP:
        return converting_ ? CycleCandidate(context, -1) : MoveBarSelection(-1);
    case VK_LEFT:
        if (!converting_) {
            return S_OK;
        }
        return shifted ? ResizeSegment(context, -1) : MoveSegment(context, -1);
    case VK_RIGHT:
        if (!converting_) {
            return S_OK;
        }
        return shifted ? ResizeSegment(context, +1) : MoveSegment(context, +1);
    case VK_PRIOR:
        return converting_ ? MoveSegmentTo(context, 0) : S_OK;
    case VK_NEXT:
        return converting_ ? MoveSegmentTo(context, segments_.size() - 1) : S_OK;
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
        composer_.PushKana(key.raw, key.raw);
    } else if (key.romaji != 0) {
        composer_.Push(key.romaji);
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

HRESULT TextService::ReplaceRunDisplay(ITfContext* context, const std::wstring& text,
                                       size_t selectOffset, size_t selectLength)
{
    // 置き換えた後の表示に未完成のローマ字が要るなら、呼び出し側が小窓を出し直す
    pendingWindow_.Hide();
    bool selected = false;
    const ReplaceRunResult result = ReplaceRunRange(context, surface_, surfaceCaret_,
                                                    surfaceSelectLength_, text, selectOffset,
                                                    selectLength, &selected);
    if (result == ReplaceRunResult::Succeeded) {
        surface_ = text;
        // 選択に失敗したときは置換 session が末尾に潰したままになっている
        surfaceCaret_ = selected ? selectOffset : surface_.size();
        surfaceSelectLength_ = selected ? selectLength : 0;
        return S_OK;
    }
    // 文書と食い違った run は文書を触らずに捨てる。候補選択中なら文書には変換結果が
    // 残るが、アプリに composition を終了されたときと同じく学習しない
    DropRun();
    ClearContext();
    return E_FAIL;
}

HRESULT TextService::SpaceDirect(ITfContext* context, bool shifted)
{
    if (!InRun()) {
        // 食べているのは !shifted && space=full のときだけ
        const std::wstring space = StandaloneSpaceText();
        if (appendDocument_ != AppendDocument::Readable && !lastCommitText_.empty()) {
            // 確定文字列の後ろに文字が入ったので、照合できない文書では確定アンドゥを捨てる
            DebugLog(L"run の外の Space で確定アンドゥの記憶を捨てる");
            lastCommitText_.clear();
        }
        return InsertText(context, space);
    }
    if (!converting_ && barIndex_ < 0) {
        return SpaceAppend(context, shifted);
    }
    // run 中の Space は幅によらず IME が入れる (run の終了と1つの edit session で行う)。
    // モードレスが有効なときだけ、英字モード中は英文の語の区切りとして半角にする
    const bool asciiWord = config_.Get().modeless && composer_.AsciiMode();
    const std::wstring space =
        (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
    if (!converting_) {
        return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::End, space,
                            nullptr);
    }
    // 文書の run の文字列は既に候補選択の結果になっている
    const std::wstring text = surface_ + space;
    const ReplaceRunResult result =
        ReplaceRunRange(context, surface_, surfaceCaret_, surfaceSelectLength_, text, 0, 0);
    if (result == ReplaceRunResult::Succeeded) {
        // スペースまで含めて確定アンドゥの対象にする (Ctrl+Backspace でスペースごと
        // 読みに戻る)。学習・文脈は EndRun が変換状態と composer_ から求める
        surface_ = text;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        EndRun();
        return S_OK;
    }
    EndRun();
    ClearContext();
    return InsertText(context, space);
}

HRESULT TextService::ConvertKeyDirect(ITfContext* context, bool previous)
{
    if (!InRun()) {
        return ReconvertSelectionDirect(context);
    }
    if (converting_) {
        return CycleCandidate(context, previous ? -1 : +1);
    }
    return BeginAppendConversion(context, KeyFunc::Convert);
}

bool TextService::ReadReconvertibleSelection(ITfContext* context, std::wstring* textOut) const
{
    textOut->clear();
    if (context == nullptr) {
        return false;
    }
    std::wstring selection;
    RequestSync(context, new (std::nothrow) GetSelectionTextEditSession(context, &selection),
                TF_ES_SYNC | TF_ES_READ);
    if (selection.size() > kMaxReconvertLength || !IsKanaOnly(selection)) {
        return false;
    }
    *textOut = selection;
    return true;
}

HRESULT TextService::ReconvertSelectionDirect(ITfContext* context)
{
    std::wstring selection;
    if (!ReadReconvertibleSelection(context, &selection)) {
        return S_OK;
    }
    // 選択文字列を読みとして run を作る。エンジンに渡す読みはひらがなに正規化し、
    // 打鍵列は元の文字 (F7-F10 の文字種変換で元の形へ戻せるように) にする
    composer_.Clear();
    for (wchar_t c : selection) {
        composer_.PushKana(std::wstring(1, KatakanaToHiragana(c)), std::wstring(1, c));
    }
    surface_ = selection;
    // ユーザの選択 = run 全体。最初の置換の前に選択を末尾へ潰す
    surfaceCaret_ = 0;
    surfaceSelectLength_ = surface_.size();
    // 選択した文字列と直前の確定は無関係。run を始める前の時点が無いので、LLM の左文脈も無しにする
    ClearContext();
    ResetRerank();
    llmRunContext_.clear();
    if (PromoteRun(context) == PromoteResult::Dropped) {
        return S_OK;
    }
    return StartConversion(context);
}

HRESULT TextService::CommitRunDirect(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    if (converting_) {
        // 現在文節の選択を末尾に潰す (文書の文字列は既に変換結果)
        const ReplaceRunResult result =
            SelectRunRange(context, surface_, surfaceCaret_, surface_.size(), 0);
        if (result != ReplaceRunResult::Succeeded) {
            DropRun();
            ClearContext();
            return S_OK;
        }
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
    } else if (barIndex_ >= 0) {
        return AdoptBarItem(context, static_cast<size_t>(barIndex_), BarAdopt::End, L"",
                            nullptr);
    }
    EndRun();
    return S_OK;
}

HRESULT TextService::CommitRunKey(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    // ルール3 (EndAppendRunForPassthroughKey と同じく読める文書だけ) で英字になる入力は、
    // 全体変換を採用せずに英字で終える
    RomajiComposer probe = composer_;
    probe.FinishForCommit();
    const bool becomesAscii =
        appendDocument_ == AppendDocument::Readable && probe.AsciiMode() != composer_.AsciiMode();
    if (!becomesAscii) {
        for (size_t i = 0; i < barItems_.size(); ++i) {
            if (barItems_[i].kind == BarKind::Whole) {
                return AdoptBarItem(context, i, BarAdopt::End, L"", nullptr);
            }
        }
    }
    // 全体変換が無ければ、アプリへ渡すキーで run を終えるときと同じ救済を通す
    // (読める文書ではモードレスのルール3、未完成のローマ字は文書に残す)
    EndAppendRunForPassthroughKey(context, false, false);
    return S_OK;
}

HRESULT TextService::AdoptBarItem(ITfContext* context, size_t index, BarAdopt mode,
                                  const std::wstring& suffix, const DirectKey* followKey)
{
    // 採用できないときは選択を解除する (Enter を送り直す経路が、選択中のまま再び
    // 採用を試みて送り直しを繰り返さないように)
    const std::wstring& kana = composer_.ConfirmedKana();
    if (index >= barItems_.size() || kana != barKana_) {
        DeselectBar();
        return E_UNEXPECTED;
    }
    const BarCandidate item = barItems_[index];
    const bool head = item.kind == BarKind::Head;
    const size_t readingLength = head ? item.reading.size() : kana.size();
    // 文書の run の文字列は確定済みかなで始まる (Backspace で末尾の英字が未完成の
    // ローマ字に戻ったときだけ、その英字まで文書に入っている)
    if (surface_.compare(0, readingLength, kana, 0, readingLength) != 0) {
        DeselectBar();
        return E_UNEXPECTED;
    }

    BarAdoption adoption;
    switch (item.kind) {
    case BarKind::Whole:
        adoption.learn = SentenceLearnEntries(item.segments);
        if (!item.segments.empty()) {
            adoption.contextReading = item.segments.back().first;
            adoption.contextSurface = item.segments.back().second;
        }
        break;
    case BarKind::Prediction:
    case BarKind::Head:
        adoption.learn.push_back({item.reading, item.surface, contextSurface_});
        adoption.contextReading = item.reading;
        adoption.contextSurface = item.surface;
        break;
    }
    adoption.readingLength = readingLength;
    adoption.adoptedLength = item.surface.size();
    adoption.endRun = mode == BarAdopt::End || (mode == BarAdopt::CommitKey && !head);
    adoption.written = item.surface + surface_.substr(readingLength);
    if (adoption.endRun) {
        // run を終えるので、未完成のローマ字も文書に残す (確定キー・Enter の救済と同じ)
        adoption.written += AppendPendingText() + suffix;
    }
    adoption.before = composer_;
    adoption_ = std::move(adoption);
    DebugLog(L"バーの候補を採用: " + adoption_.written);

    pendingWindow_.Hide();
    ClassifyAppendDocument(context);
    if (appendDocument_ == AppendDocument::Readable) {
        if (ReplaceRunText(context, surface_, adoption_.written) != ReplaceRunResult::Succeeded) {
            DebugLog(L"採用の置換に失敗: run を破棄");
            DropRun();
            ClearContext();
            return E_FAIL;
        }
        FinishBarAdoption(context, followKey == nullptr);
        return followKey != nullptr ? TypeAppend(context, *followKey) : S_OK;
    }
    // 擬似 Backspace より先に打鍵を追記すると、その文字まで消されるため目印の後に回す
    if (followKey != nullptr) {
        appendFollowKey_ = *followKey;
        appendFollowKeyPending_ = true;
    }
    const HRESULT hr = ScheduleAppendAction(context, PseudoKeyAction::Adopt, KeyFunc::None,
                                            adoption_.written, false);
    if (FAILED(hr)) {
        appendFollowKeyPending_ = false;
    }
    return hr;
}

void TextService::FinishBarAdoption(ITfContext* context, bool updateBar)
{
    engine_.Learn(adoption_.learn);
    SetCommitContext(adoption_.contextReading, adoption_.contextSurface);
    // 採用した部分は run から外す (以後 IME はその文字列を書き換えない)
    composer_.RemoveFront(adoption_.readingLength);
    const std::wstring adopted = adoption_.written.substr(0, adoption_.adoptedLength);
    const std::wstring rest = adoption_.written.substr(adoption_.adoptedLength);
    AppendLlmHistory(adopted);
    ClearBar();
    // run の先頭が変わったので、次にバーを出す位置はその時点で取り直す
    barXFixed_ = false;
    if (adoption_.endRun && !composer_.ConfirmedKana().empty()) {
        // 先頭文節を採用して run を終える: 残りのかなは採用せずに終えた run と同じ扱い
        surface_ = rest;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        EndRun();
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
    surface_ = rest;
    surfaceCaret_ = surface_.size();
    surfaceSelectLength_ = 0;
    // 続く run の手前は採用した文字列なので、それより前の確定アンドゥの記憶は使えない
    lastCommitText_.clear();
    UpdateAppendPending(context, AppendPendingText());
    if (updateBar) {
        UpdateBar(context);
    }
}

HRESULT TextService::UpdateRunAndBar(ITfContext* context)
{
    // 文書には確定したかなだけを入れ、未完成のローマ字は小窓に表示する
    HRESULT hr = ReplaceRunDisplay(context, composer_.ConfirmedKana());
    if (FAILED(hr)) {
        return hr;
    }
    UpdateAppendPending(context, AppendPendingText());
    UpdateBar(context);
    return hr;
}

void TextService::EndRun()
{
    if (!surface_.empty()) {
        if (converting_) {
            // 候補選択中の確定: 文節ごとの学習と文脈更新 (composition の確定と同じ)
            PrepareConversionCommit();
        } else {
            // 無変換の確定: 英字を含む入力 (英単語など) は読み=表記で学習し、
            // 確定したかなは次の変換の文脈にする (かなのみの学習はしない。学習は変換候補の
            // 並び替えにも使われ、「きょう→きょう」が入るとひらがな候補が先頭へ来るため)
            const std::wstring kana = composer_.Commit();
            if (ContainsAsciiLetter(kana)) {
                engine_.Learn({{kana, kana, contextSurface_}});
            }
            SetCommitContext(kana, kana);
        }
        lastCommitText_ = surface_;
        lastComposer_ = composer_;
        AppendLlmHistory(surface_);
    }
    DropRun();
}

void TextService::DropRun()
{
    pendingWindow_.Hide();
    ClearConversion();
    ClearBar();
    ResetRerank();
    llmRunContext_.clear();
    barXFixed_ = false;
    composer_.Clear();
    surface_.clear();
    surfaceCaret_ = 0;
    surfaceSelectLength_ = 0;
}

HRESULT TextService::UndoCommit(ITfContext* context)
{
    // 一度きりの操作として、成否に関わらず記憶を消す (内容が一致しない = 確定後に
    // 別の編集があった場合に、以降の Ctrl+Backspace を奪い続けないようにする)
    const std::wstring commitText = lastCommitText_;
    lastCommitText_.clear();
    if (commitText.empty() || InRun() || Composing() || context == nullptr) {
        return S_OK;
    }

    // 直前の確定文字列を run の文字列とみなして、読みのかなに戻した run を再開する
    // (composition は開始しない)。未完成のローマ字は文書に戻さず小窓に出す
    composer_ = lastComposer_;
    surface_ = commitText;
    surfaceCaret_ = surface_.size();
    surfaceSelectLength_ = 0;
    // 確定を取り消したので、その確定を前提にした文脈補正はもう使えない。
    // run を始める前の時点が無いので、LLM の左文脈も無しにする
    ClearContext();
    ResetRerank();
    llmRunContext_.clear();
    const std::wstring text = composer_.ConfirmedKana();
    ClassifyAppendDocument(context);
    if (appendDocument_ == AppendDocument::Readable) {
        // キャレット直前が直前の確定文字列と一致する場合のみ置き換える
        if (ReplaceRunText(context, commitText, text) != ReplaceRunResult::Succeeded) {
            DropRun();
            return S_OK;
        }
        surface_ = text;
        surfaceCaret_ = surface_.size();
        UpdateAppendPending(context, AppendPendingText());
        return S_OK;
    }
    // 追記のみの文書は照合できない。確定後のキャレット移動は、記憶を捨てる契機
    // (アプリへ渡したキー・マウス・フォーカス移動) で除いてある
    DebugLog(L"追記のみの文書で確定アンドゥ: " + commitText);
    return ScheduleAppendAction(context, PseudoKeyAction::Append, KeyFunc::None, text, false);
}

TextService::PromoteResult TextService::PromoteRun(ITfContext* context)
{
    ReplaceRunResult match = ReplaceRunResult::Unsupported;
    ITfComposition* composition = nullptr;
    RequestSync(context,
                new (std::nothrow) PromoteRunEditSession(context, surface_, surfaceCaret_,
                                                         static_cast<ITfCompositionSink*>(this),
                                                         inputAttribute_, &composition, &match),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (match == ReplaceRunResult::Unreadable && DocumentReadable()) {
        // ReplaceRunRange と同じく、読める文書で範囲が作れないのはキャレットが動いたため
        match = ReplaceRunResult::Mismatch;
    }
    if (match != ReplaceRunResult::Succeeded) {
        // 置換の不一致と同じく、文書と食い違った run は文書を触らずに捨てる
        DropRun();
        ClearContext();
        return PromoteResult::Dropped;
    }
    if (composition == nullptr) {
        return PromoteResult::Refused;
    }
    composition_ = composition;
    promoted_ = true;
    // 文書上の位置は以後 composition が持つ。composer_・文脈はそのまま候補選択へ
    // 引き継ぐ (確定ではないので学習・確定アンドゥの記憶はしない)
    surface_.clear();
    surfaceCaret_ = 0;
    surfaceSelectLength_ = 0;
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
    // 確定したかなだけを文書に残して run に戻し、未完成のローマ字は小窓に戻す
    const std::wstring text = composer_.ConfirmedKana();
    HRESULT hr = E_UNEXPECTED;
    if (context != nullptr) {
        hr = RequestSync(context,
                         new (std::nothrow) EndCompositionEditSession(context, composition_, text),
                         TF_ES_SYNC | TF_ES_READWRITE);
    }
    composition_->Release();
    composition_ = nullptr;
    promoted_ = false;
    if (FAILED(hr) || composer_.Empty()) {
        DropRun();
        ClearContext();
        return;
    }
    // composer_・バーの状態は維持し、run として続ける
    surface_ = text;
    surfaceCaret_ = surface_.size();
    surfaceSelectLength_ = 0;
    UpdateAppendPending(context, AppendPendingText());
}

// ---- 追記型入力 ----

void TextService::DebugLog(const std::wstring& message) const
{
    WriteDebugLog(config_.Get().debugLog, message);
}

const wchar_t* TextService::AppendDocumentName(AppendDocument document)
{
    switch (document) {
    case AppendDocument::Readable:
        return L"読める";
    case AppendDocument::AppendOnly:
        return L"追記のみ";
    default:
        return L"未判定";
    }
}

void TextService::SetAppendDocument(AppendDocument document, const wchar_t* reason)
{
    if (document != appendDocument_) {
        DebugLog(std::wstring(L"文書の分類: ") + AppendDocumentName(appendDocument_) + L" -> " +
                 AppendDocumentName(document) + L" (" + reason + L")");
    }
    appendDocument_ = document;
}

std::wstring TextService::AppendPendingText() const
{
    const std::wstring display = composer_.Display();
    const std::wstring& confirmed = composer_.ConfirmedKana();
    if (StartsWith(confirmed, surface_)) {
        return display.substr(confirmed.size());
    }
    // Backspace で末尾の素通しの英字が未変換ローマ字に戻ったときは、文書に入っている
    // 部分を除く
    if (StartsWith(display, surface_)) {
        return display.substr(surface_.size());
    }
    return display.substr(confirmed.size());
}

HRESULT TextService::TypeAppend(ITfContext* context, const DirectKey& key)
{
    // 不一致で run を捨てるときに、この打鍵を含まない状態へ戻すための控え
    const RomajiComposer before = composer_;
    if (!InRun()) {
        // run の最初の文字を入れる前に左文脈を取る (run のかなは文書に入るため)
        CaptureLlmContext(context);
    }
    PushDirectKey(key);
    AppendResult result = SyncAppendRun(context);
    if (result == AppendResult::Mismatch) {
        // 読める文書で run が文書と食い違った (キャレット移動・アプリ側の編集)。
        // 古い run は忘れ、この打鍵を新しい run の先頭として追記し直す
        composer_ = before;
        pendingWindow_.Hide();
        DiscardPendingRomaji();
        EndRun();
        ClearContext();
        CaptureLlmContext(context);
        PushDirectKey(key);
        result = SyncAppendRun(context);
    }
    if (result == AppendResult::Failed) {
        return E_FAIL;
    }
    UpdateBar(context);
    return S_OK;
}

TextService::AppendResult TextService::SyncAppendRun(ITfContext* context)
{
    const std::wstring display = composer_.Display();
    const std::wstring confirmed = composer_.ConfirmedKana();
    std::wstring delta;
    std::wstring pending;
    std::wstring newSurface;
    if (StartsWith(confirmed, surface_)) {
        delta = confirmed.substr(surface_.size());
        pending = display.substr(confirmed.size());
        newSurface = confirmed;
    } else if (StartsWith(display, surface_)) {
        pending = display.substr(surface_.size());
        newSurface = surface_;
    } else {
        // モードレスの英字切替 (「あっp」→「appl」) などで、入れたかなが書き換わった
        return RewriteAppendRun(context, confirmed, false);
    }
    if (!delta.empty()) {
        const AppendResult result = AppendRunText(context, delta);
        if (result != AppendResult::Done) {
            return result;
        }
        surface_ = newSurface;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
    }
    UpdateAppendPending(context, pending);
    return AppendResult::Done;
}

TextService::AppendResult TextService::AppendRunText(ITfContext* context,
                                                     const std::wstring& text)
{
    if (context == nullptr) {
        return AppendResult::Failed;
    }
    const bool verify = !surface_.empty() && appendDocument_ != AppendDocument::AppendOnly;
    const bool readable = appendDocument_ == AppendDocument::Readable;
    ReplaceRunResult match = ReplaceRunResult::Unsupported;
    bool written = false;
    RequestSync(context,
                new (std::nothrow) AppendRunEditSession(context, surface_, verify, readable, text,
                                                        &match, &written),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (verify) {
        if (appendDocument_ == AppendDocument::Unknown) {
            // キャレット移動による食い違いと読めない文書を区別できないので、
            // 一致しなければ追記のみ側に倒す
            SetAppendDocument(match == ReplaceRunResult::Succeeded ? AppendDocument::Readable
                                                                   : AppendDocument::AppendOnly,
                              MatchReason(match));
        } else if (match != ReplaceRunResult::Succeeded) {
            DebugLog(std::wstring(L"読める文書で追記前の照合が不一致 (") + MatchReason(match) +
                     L"): run を捨てる");
            return AppendResult::Mismatch;
        }
    }
    if (!written) {
        DebugLog(L"追記に失敗: run を破棄");
        DropRun();
        ClearContext();
        return AppendResult::Failed;
    }
    return AppendResult::Done;
}

TextService::AppendResult TextService::RewriteAppendRun(ITfContext* context,
                                                        const std::wstring& newText,
                                                        bool endRunAfter)
{
    pendingWindow_.Hide();
    ClassifyAppendDocument(context);
    if (appendDocument_ == AppendDocument::Readable) {
        const ReplaceRunResult result = ReplaceRunText(context, surface_, newText);
        if (result != ReplaceRunResult::Succeeded) {
            DebugLog(L"作り直しの置換に失敗: run を破棄");
            DropRun();
            ClearContext();
            return AppendResult::Failed;
        }
        surface_ = newText;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        if (endRunAfter) {
            EndRun();
        } else {
            UpdateAppendPending(context, AppendPendingText());
        }
        return AppendResult::Done;
    }
    const HRESULT hr =
        ScheduleAppendAction(context, PseudoKeyAction::Append, KeyFunc::None, newText, endRunAfter);
    return SUCCEEDED(hr) ? AppendResult::Done : AppendResult::Failed;
}

void TextService::UpdateAppendPending(ITfContext* context, const std::wstring& pending)
{
    if (pending.empty() || context == nullptr) {
        pendingWindow_.Hide();
        return;
    }
    RECT rect = {};
    if (!CaretRect(context, &rect)) {
        pendingWindow_.Hide();
        DebugLog(L"キャレットの矩形が取れないため小窓を出さない");
        return;
    }
    pendingWindow_.ShowInline(rect, pending);
}

bool TextService::CaretRect(ITfContext* context, RECT* rect)
{
    bool succeeded = false;
    if (context != nullptr) {
        RequestSync(context,
                    new (std::nothrow) GetSelectionExtentEditSession(context, rect, &succeeded),
                    TF_ES_SYNC | TF_ES_READ);
    }
    if (!succeeded) {
        // 選択範囲の矩形を返さない文書でも、システムキャレットがあればその位置に出す
        GUITHREADINFO info = {};
        info.cbSize = sizeof(info);
        if (GetGUIThreadInfo(0, &info) && info.hwndCaret != nullptr) {
            *rect = info.rcCaret;
            MapWindowPoints(info.hwndCaret, HWND_DESKTOP, reinterpret_cast<POINT*>(rect), 2);
            succeeded = true;
        }
    }
    return succeeded;
}

void TextService::DiscardPendingRomaji()
{
    while (!composer_.Empty()) {
        const size_t size = composer_.Display().size();
        if (size <= composer_.ConfirmedKana().size() || size <= surface_.size()) {
            break;
        }
        composer_.Backspace();
    }
}

TextService::AppendResult TextService::FlushAppendPending(ITfContext* context)
{
    const std::wstring tail = AppendPendingText();
    pendingWindow_.Hide();
    if (tail.empty()) {
        return AppendResult::Done;
    }
    const AppendResult result = AppendRunText(context, tail);
    if (result == AppendResult::Done) {
        surface_ += tail;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
    }
    return result;
}

void TextService::ClassifyAppendDocument(ITfContext* context)
{
    if (appendDocument_ != AppendDocument::Unknown || surface_.empty() || context == nullptr) {
        return;
    }
    ReplaceRunResult match = ReplaceRunResult::Unsupported;
    RequestSync(context,
                new (std::nothrow) MatchRunEditSession(context, surface_, surface_.size(), &match),
                TF_ES_SYNC | TF_ES_READ);
    SetAppendDocument(match == ReplaceRunResult::Succeeded ? AppendDocument::Readable
                                                           : AppendDocument::AppendOnly,
                      MatchReason(match));
}

HRESULT TextService::BackspaceAppend(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    // 未完成のローマ字だけを削る (文書の文字はアプリへ渡した Backspace で消える)
    if (AppendPendingText().empty()) {
        return S_OK;
    }
    composer_.Backspace();
    if (surface_.empty() && composer_.Empty()) {
        UpdateAppendPending(context, L"");
        DropRun();
        return S_OK;
    }
    UpdateAppendPending(context, AppendPendingText());
    UpdateBar(context);
    return S_OK;
}

HRESULT TextService::SpaceAppend(ITfContext* context, bool shifted)
{
    // 確定文字列を作る前に自動英字判定ルール3を適用する (「わんt」→「want」)
    ApplyModelessCommitRule();
    const bool asciiWord = config_.Get().modeless && composer_.AsciiMode();
    const std::wstring space =
        (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
    const std::wstring text = composer_.Commit() + space;
    if (StartsWith(text, surface_)) {
        const AppendResult result = AppendRunText(context, text.substr(surface_.size()));
        pendingWindow_.Hide();
        if (result == AppendResult::Done) {
            surface_ = text;
            surfaceCaret_ = surface_.size();
            surfaceSelectLength_ = 0;
            EndRun();
            return S_OK;
        }
        if (result == AppendResult::Mismatch) {
            EndRun();
            ClearContext();
            return InsertText(context, space);
        }
        return E_FAIL;
    }
    // ルール3 で英字に切り替わり、入れたかなが書き換わる (Space は食べているので、
    // 追記のみの文書でも擬似 Backspace の後に入れ直せる)
    if (RewriteAppendRun(context, text, true) == AppendResult::Failed) {
        return InsertText(context, space);
    }
    return S_OK;
}

void TextService::EndAppendRunForPassthroughKey(ITfContext* context, bool ctrl, bool alt)
{
    if (converting_) {
        // 候補選択中は現在文節が選択されたままなので、アプリにキーを渡す前に
        // 確定して選択を末尾に潰す (Tab や Delete が文節を置き換えないように)
        CommitRunDirect(context);
        return;
    }
    // バーの選択は採用せず、かなのまま run を終える (P1。追記のみの文書では、このキーが
    // 擬似 Backspace より先にアプリへ届くので、作り直すと別の文字を消してしまう)
    barIndex_ = -1;
    const RomajiComposer before = composer_;
    // 追記のみの文書ではルール3 を適用しない (理由は上の採用と同じ)。Ctrl/Alt 併用
    // (Undo など) は確定ではないため、ルール3 で文書を書き換えずに終える
    // (直後の Undo が書き換えの方を取り消してしまう)
    if (!ctrl && !alt && appendDocument_ == AppendDocument::Readable) {
        ApplyModelessCommitRule();
    }
    const std::wstring display = composer_.Display();
    if (display != before.Display()) {
        pendingWindow_.Hide();
        if (ReplaceRunText(context, surface_, display) == ReplaceRunResult::Succeeded) {
            surface_ = display;
            surfaceCaret_ = surface_.size();
            surfaceSelectLength_ = 0;
            EndRun();
        } else {
            // 置換できなかったときはルール3 を捨てて文書に合わせる (学習・確定アンドゥを
            // 文書と食い違わせない)
            composer_ = before;
            EndRun();
            ClearContext();
        }
        return;
    }
    // 未完成のローマ字は、そのまま文書に残して run を終える
    if (FlushAppendPending(context) == AppendResult::Mismatch) {
        EndRun();
        ClearContext();
        return;
    }
    EndRun();
}

HRESULT TextService::BeginAppendConversion(ITfContext* context, KeyFunc func)
{
    pendingWindow_.Hide();
    if (surface_.empty()) {
        // 文書にはまだ何も入っていない (未完成のローマ字だけ) ので、消さずに composition を張る
        return ScheduleAppendAction(context, PseudoKeyAction::Compose, func, L"", false);
    }
    ClassifyAppendDocument(context);
    if (appendDocument_ == AppendDocument::Readable) {
        const PromoteResult promoted = PromoteRun(context);
        if (promoted == PromoteResult::Dropped) {
            return S_OK;
        }
        // 昇格できなかった run は、現在文節の選択による強調で候補選択する
        return func == KeyFunc::Convert ? StartConversion(context)
                                        : ApplyFunctionKey(context, func);
    }
    return ScheduleAppendAction(context, PseudoKeyAction::Compose, func, L"", false);
}

HRESULT TextService::ScheduleAppendAction(ITfContext* context, PseudoKeyAction action,
                                          KeyFunc func, const std::wstring& text,
                                          bool endRunAfter)
{
    appendAction_ = action;
    appendActionFunc_ = func;
    appendActionText_ = text;
    appendActionEndRun_ = endRunAfter;
    const size_t count = DisplayCharCount(surface_);
    if (count == 0) {
        RunAppendAction(context);
        return S_OK;
    }
    if (!SendPseudoBackspaces(count)) {
        DebugLog(L"擬似 Backspace の送信に失敗 (SendInput)");
        appendAction_ = PseudoKeyAction::None;
        if (action == PseudoKeyAction::Compose) {
            // 文書は触っていないので run はそのまま続ける
            UpdateAppendPending(context, AppendPendingText());
        } else {
            DropRun();
            ClearContext();
        }
        return E_FAIL;
    }
    awaitingMarker_ = true;
    DebugLog(L"擬似 Backspace を送信: " + std::to_wstring(count) + L" 個 (surface=" + surface_ +
             L")");
    return S_OK;
}

void TextService::RunAppendAction(ITfContext* context)
{
    const PseudoKeyAction action = appendAction_;
    appendAction_ = PseudoKeyAction::None;
    const bool followKey = appendFollowKeyPending_;
    appendFollowKeyPending_ = false;
    if (action == PseudoKeyAction::None || context == nullptr) {
        return;
    }
    // 擬似 Backspace で run の文字列は文書から消えている
    const std::wstring deleted = surface_;
    surface_.clear();
    surfaceCaret_ = 0;
    surfaceSelectLength_ = 0;

    if (action == PseudoKeyAction::Compose) {
        HRESULT hr = StartComposition(context);
        if (FAILED(hr) || !Composing()) {
            // composition を張れない文書では、消した文字列を入れ直して run を続ける
            DebugLog(L"変換の composition を開始できない: 消した文字列を戻す");
            if (!deleted.empty() && AppendRunText(context, deleted) == AppendResult::Done) {
                surface_ = deleted;
                surfaceCaret_ = surface_.size();
            }
            UpdateAppendPending(context, AppendPendingText());
            return;
        }
        // 読める文書の昇格と同じく、composition の候補選択に入る
        promoted_ = true;
        ClearBar();
        if (appendActionFunc_ == KeyFunc::Convert) {
            StartConversion(context);
        } else {
            ApplyFunctionKey(context, appendActionFunc_);
        }
        DemoteIfLeftConversion(context);
        return;
    }
    if (action == PseudoKeyAction::Adopt) {
        if (AppendRunText(context, appendActionText_) != AppendResult::Done) {
            return;
        }
        FinishBarAdoption(context, !followKey);
        if (followKey) {
            TypeAppend(context, appendFollowKey_);
        }
        return;
    }

    const std::wstring text = appendActionText_;
    if (AppendRunText(context, text) != AppendResult::Done) {
        return;
    }
    surface_ = text;
    surfaceCaret_ = surface_.size();
    if (appendActionEndRun_) {
        EndRun();
    } else {
        UpdateAppendPending(context, AppendPendingText());
    }
    if (followKey) {
        TypeAppend(context, appendFollowKey_);
    }
}

void TextService::CancelAppendAction()
{
    awaitingMarker_ = false;
    appendAction_ = PseudoKeyAction::None;
    appendFollowKeyPending_ = false;
    appendResendVk_ = 0;
}

bool TextService::HandlePseudoKey(ITfContext* context, WPARAM wparam, bool keyDown, BOOL* eaten)
{
    if (!awaitingMarker_) {
        return false;
    }
    const bool injected =
        static_cast<ULONG_PTR>(GetMessageExtraInfo()) == kPseudoKeyExtraInfo;
    const std::wstring where = std::wstring(keyDown ? L"OnKeyDown" : L"OnTestKeyDown") +
                               L" injected=" + (injected ? L"1" : L"0");
    if (wparam == VK_BACK || IsPseudoCompanionKey(wparam)) {
        // IME が送った Backspace と修飾キーの離し・押し直しは処理せずにアプリへ渡す
        // (目印が届くまでのこれらの打鍵はすべて IME のものとみなす)
        *eaten = FALSE;
        if (wparam == VK_BACK) {
            keyEditExpected_ = true;
            DebugLog(L"擬似 Backspace をアプリへ渡す (" + where + L")");
        }
        return true;
    }
    if (wparam == kMarkerVk) {
        *eaten = TRUE;
        if (keyDown) {
            pendingKeyUps_.set(wparam);
            awaitingMarker_ = false;
            DebugLog(L"目印の打鍵を受信 (" + where + L")");
            RunAppendAction(context);
            SendResendKey();
        }
        return true;
    }
    DebugLog(L"目印待ちの間の打鍵 vk=" + std::to_wstring(wparam) + L" (" + where + L")");
    return false;
}

void TextService::NoteKeyForAppend(ITfContext* context, WPARAM wparam, bool eaten,
                                   bool fromTest)
{
    const bool tested = backspaceTested_;
    backspaceTested_ = false;
    keyEditExpected_ = false;
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    if (!eaten && (wparam == VK_RETURN || (ctrl && wparam == 'M'))) {
        // 改行・送信をアプリへ渡したら、IME が入れた文字列はもう同じ行の前方ではない
        llmHistory_.clear();
    }
    if (eaten || Composing() || IsModifierKey(wparam)) {
        return;
    }
    const bool ctrlOrAlt = ctrl || (GetKeyState(VK_MENU) & 0x8000) != 0;
    if (wparam == VK_BACK && !ctrlOrAlt) {
        keyEditExpected_ = true;
        // 同じ打鍵で OnTestKeyDown と OnKeyDown の両方が呼ばれるホストで二重に削らない
        if (!fromTest && tested) {
            return;
        }
        backspaceTested_ = fromTest;
        if (!surface_.empty() && !converting_) {
            // アプリが文書の末尾の1文字を消すので、run の表示・読みの末尾も1文字削る
            // (置換はしない。読みの打鍵列 Raw() とは対応しなくなることがある)
            const size_t length = LastCharLength(surface_);
            surface_.erase(surface_.size() - length);
            surfaceCaret_ = surface_.size();
            surfaceSelectLength_ = 0;
            for (size_t i = 0; i < length && !composer_.Empty(); ++i) {
                composer_.Backspace();
            }
            DebugLog(L"Backspace をアプリへ渡し run を追従: surface=" + surface_);
            if (surface_.empty()) {
                DropRun();
                return;
            }
            UpdateBar(context);
            return;
        }
    }
    // アプリへ渡したキーで文書やキャレットが変わりうる。照合できない文書では、
    // 確定アンドゥの擬似 Backspace が別の文字を消さないよう記憶を捨てる
    if (appendDocument_ != AppendDocument::Readable && !lastCommitText_.empty()) {
        DebugLog(L"アプリへ渡したキーで確定アンドゥの記憶を捨てる vk=" + std::to_wstring(wparam));
        lastCommitText_.clear();
    }
}

// ---- マウスフック ----

void TextService::UpdateMouseHook()
{
    const bool wanted = appendDocument_ != AppendDocument::Readable &&
                        (InRun() || !lastCommitText_.empty());
    if (wanted == (mouseHook_ != nullptr)) {
        return;
    }
    if (wanted) {
        mouseHook_ = SetWindowsHookExW(WH_MOUSE, MouseHookProc, globals::dllInstance,
                                       GetCurrentThreadId());
        if (mouseHook_ != nullptr) {
            t_mouseHookOwner = this;
        }
        DebugLog(std::wstring(L"マウスフックを仕掛ける: ") +
                 (mouseHook_ != nullptr ? L"成功" : L"失敗"));
        return;
    }
    UnhookWindowsHookEx(mouseHook_);
    mouseHook_ = nullptr;
    t_mouseHookOwner = nullptr;
    DebugLog(L"マウスフックを外す");
}

void TextService::OnMouseInput()
{
    // 擬似 Backspace の目印待ちの間は、消している途中の run を目印の後の処理に任せる
    if (awaitingMarker_) {
        return;
    }
    if (appendDocument_ != AppendDocument::Readable) {
        if (InRun()) {
            DebugLog(L"追記のみの文書で run を終了: マウス操作");
            pendingWindow_.Hide();
            DiscardPendingRomaji();
            EndRun();
            ClearContext();
        }
        if (!lastCommitText_.empty()) {
            DebugLog(L"マウス操作で確定アンドゥの記憶を捨てる");
            lastCommitText_.clear();
        }
    }
    UpdateMouseHook();
}

LRESULT CALLBACK TextService::MouseHookProc(int code, WPARAM wparam, LPARAM lparam)
{
    if (code >= 0 && IsCaretMovingMouseMessage(wparam) && t_mouseHookOwner != nullptr) {
        t_mouseHookOwner->OnMouseInput();
    }
    // マウスメッセージは止めない (フックの第1引数は無視されるので、外した後でも渡せる)
    return CallNextHookEx(nullptr, code, wparam, lparam);
}
