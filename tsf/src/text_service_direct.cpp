// 直接入力方式 (設定 input_style=direct) のキー処理。
// TextService のメンバ関数のうち direct 方式に固有のものをこの翻訳単位に置く
// (設計は docs/design/direct-input.md)
#include "text_service.h"

#include <new>

#include "edit_session.h"

using namespace key_util;

namespace {

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
constexpr size_t kMaxReconvertLength = 127;

} // namespace

bool TextService::UsingDirectStyle() const
{
    // 生きている composition (フォールバック後や方式切替前のもの) は
    // 必ず composition 方式の経路で後始末する
    return config_.Get().inputStyle == InputStyle::Direct &&
           directCapable_ != DirectCapability::Incapable && !Composing();
}

bool TextService::IsKeyEatenDirect(ITfContext* context, WPARAM wparam) const
{
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    if (ctrl || alt) {
        if (!ctrl || alt) {
            return false;
        }
        if (IsConvertKey(wparam)) {
            // Ctrl+Space 割当の変換キー。run が無ければ後置再変換できる選択があるときだけ
            std::wstring selection;
            return InRun() || ReadReconvertibleSelection(context, &selection);
        }
        if (!InRun()) {
            switch (config_.Get().FindCtrlFunc(wparam)) {
            case KeyFunc::UndoCommit:
                return !lastCommitText_.empty();
            case KeyFunc::RegisterWord:
            case KeyFunc::OpenConfig:
                return true;
            default:
                break;
            }
            return false;
        }
        // Ctrl+H は Backspace の読み替え。Ctrl+M (Enter) は候補選択中・サジェスト選択中の
        // 確定にだけ使い、それ以外は run を終えてアプリへ渡す
        return wparam == 'H' || (wparam == 'M' && (converting_ || predictionIndex_ >= 0));
    }
    const bool shifted = IsShiftPressed();

    if (IsConvertKey(wparam)) {
        std::wstring selection;
        return InRun() || ReadReconvertibleSelection(context, &selection);
    }
    // 印字キーは run の有無によらず IME が入れる (run が無ければ新しい run を始める)
    if (IsPrintableKey(wparam, shifted)) {
        return !PassesDigitKeyThrough(wparam, shifted, InRun(),
                                      config_.Get().digitsFullwidth);
    }
    if (InRun()) {
        switch (wparam) {
        case VK_SPACE:
        case VK_ESCAPE:
        case VK_BACK:
            // Space は run を終えてからスペースを入れる。Esc は run を忘れるだけ
            return true;
        case VK_RETURN:
            // 候補選択中・サジェスト選択中の確定のみ。それ以外はアプリで改行
            // (run は EndRunIfPassthroughKey で終わる)
            return converting_ || predictionIndex_ >= 0;
        case VK_UP:
        case VK_DOWN:
            return converting_ || !predictions_.empty();
        case VK_TAB:
            return !converting_ && !predictions_.empty();
        case VK_LEFT:
        case VK_RIGHT:
        case VK_PRIOR:
        case VK_NEXT:
            return converting_;
        default:
            break;
        }
        // ファンクションキー変換 (F4-F10。割当は変更可)。割当のあるキーだけ食べる
        if (wparam >= VK_F1 && wparam <= VK_F12) {
            return config_.Get().FindPlainFunc(wparam) != KeyFunc::None;
        }
        return false;
    }
    // run が無い Space は全角スペースの直接挿入 (composition 方式と同じ条件)
    return wparam == VK_SPACE && !shifted && config_.Get().spaceFullwidth;
}

void TextService::EndRunIfPassthroughKey(ITfContext* context, WPARAM wparam)
{
    if (!UsingDirectStyle() || !InRun() || IsModifierKey(wparam)) {
        return;
    }
    // IME が食べるキーは HandleKeyDirect が状態を進める
    if (IsKeyEatenDirect(context, wparam)) {
        return;
    }
    const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
    // Ctrl/Alt 併用はアプリのショートカット (Undo・全選択など文書を変えうる)
    if (ctrl || alt || IsEditingKey(wparam)) {
        // 候補選択中は現在文節が選択されたままなので、アプリにキーを渡す前に
        // 確定して選択を末尾に潰す (Tab や Delete が文節を置き換えないように)
        if (converting_) {
            CommitRunDirect(context);
        } else {
            // 無変換のまま終える run には自動英字判定ルール3を適用する。文書の表示も
            // 英字へ直せるのはこの経路 (context を持つ) だけで、置換できなかったときは
            // ルール3 を捨てて文書に合わせる (学習・確定アンドゥを文書と食い違わせない)
            const RomajiComposer before = composer_;
            // Ctrl/Alt 併用 (Undo など) は確定ではないため、ルール3 で文書を
            // 書き換えずに終える (直後の Undo が書き換えの方を取り消してしまう)
            if (!ctrl && !alt) {
                ApplyModelessCommitRule();
            }
            const std::wstring text = composer_.Display();
            if (text != before.Display()) {
                const ReplaceRunResult result = ReplaceRunRange(
                    context, surface_, surfaceCaret_, surfaceSelectLength_, text, 0, 0);
                if (result == ReplaceRunResult::Succeeded) {
                    surface_ = text;
                    surfaceCaret_ = surface_.size();
                    surfaceSelectLength_ = 0;
                    NoteDirectSuccess(false);
                } else {
                    if (result == ReplaceRunResult::Unsupported ||
                        result == ReplaceRunResult::Unreadable) {
                        directCapable_ = DirectCapability::Incapable;
                    }
                    composer_ = before;
                }
            }
            EndRun();
        }
    }
}

HRESULT TextService::HandleKeyDirect(ITfContext* context, WPARAM wparam)
{
    const bool shifted = IsShiftPressed();
    if ((GetKeyState(VK_CONTROL) & 0x8000) != 0) {
        if (IsConvertKey(wparam)) {
            return ConvertKeyDirect(context, shifted);
        }
        if (!InRun()) {
            switch (config_.Get().FindCtrlFunc(wparam)) {
            case KeyFunc::UndoCommit:
                return UndoCommit(context);
            case KeyFunc::RegisterWord:
                return LaunchWordRegister(context);
            case KeyFunc::OpenConfig:
                return LaunchConfigTool();
            default:
                break;
            }
        }
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
    } else if (IsConvertKey(wparam)) {
        return ConvertKeyDirect(context, shifted);
    }

    DirectKey key;
    if (ClassifyDirectKey(wparam, shifted, &key)) {
        // 候補選択中・サジェスト選択中の 1〜9 は候補番号による直接選択
        if (!shifted && wparam >= '1' && wparam <= '9') {
            if (converting_) {
                return SelectCandidateByNumber(context, wparam - '1');
            }
            if (predictionIndex_ >= 0) {
                return SelectPredictionByNumber(context, wparam - '1');
            }
        }
        // 候補選択中・サジェスト選択中の印字キーは選択を確定して新しい run を始める
        if (converting_ || predictionIndex_ >= 0) {
            CommitRunDirect(context);
        }
        return TypeDirect(context, key);
    }
    switch (wparam) {
    case VK_RETURN:
        return CommitRunDirect(context);
    case VK_ESCAPE:
        if (converting_) {
            return CancelConversion(context); // 変換前の表示 (かな / ライブ表示) に戻す
        }
        if (predictionIndex_ >= 0) {
            return DeselectPrediction(context);
        }
        // 文字は文書に残したまま run だけ忘れる
        if (InRun()) {
            EndRun();
        }
        return S_OK;
    case VK_BACK:
        if (converting_) {
            return CancelConversion(context);
        }
        return BackspaceDirect(context);
    case VK_SPACE:
        return SpaceDirect(context, shifted);
    case VK_TAB:
        return MovePredictionSelection(context, shifted ? -1 : +1);
    case VK_DOWN:
        return converting_ ? CycleCandidate(context, +1)
                           : MovePredictionSelection(context, +1);
    case VK_UP:
        return converting_ ? CycleCandidate(context, -1)
                           : MovePredictionSelection(context, -1);
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
        if (InRun() && wparam >= VK_F1 && wparam <= VK_F12) {
            // 変換状態に入るキーなので composition に昇格してから適用する。
            // 昇格できない文書 (Refused) と既に候補選択中のフォールバックは direct のまま
            if (!converting_ && PromoteRun(context) == PromoteResult::Dropped) {
                return S_OK;
            }
            return ApplyFunctionKey(context, config_.Get().FindPlainFunc(wparam));
        }
        return S_OK;
    }
}

bool TextService::ClassifyDirectKey(WPARAM wparam, bool shifted, DirectKey* key) const
{
    *key = DirectKey{};
    if (PassesDigitKeyThrough(wparam, shifted, InRun(), config_.Get().digitsFullwidth)) {
        return false; // IsKeyEatenDirect が食べないキー (ここには来ない想定)
    }
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
    // 未判定の文書での新規挿入だけ、挿入前に周辺テキストが読めるかを確かめる
    const bool probe = expected.empty() && directCapable_ == DirectCapability::Unknown;
    result = ReplaceRunResult::Unsupported;
    RequestSync(context,
                new (std::nothrow)
                    ReplaceRunEditSession(context, expected, caretOffset, newText, probe, &result),
                TF_ES_SYNC | TF_ES_READWRITE);
    if (result == ReplaceRunResult::Unreadable && directCapable_ == DirectCapability::Capable) {
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
    if (result == ReplaceRunResult::Unreadable && directCapable_ == DirectCapability::Capable) {
        // ReplaceRunRange と同じく、読める文書で範囲が作れないのはキャレットが動いたため
        result = ReplaceRunResult::Mismatch;
    }
    return result;
}

HRESULT TextService::ReplaceRunDisplay(ITfContext* context, const std::wstring& text,
                                       size_t selectOffset, size_t selectLength)
{
    bool selected = false;
    const ReplaceRunResult result = ReplaceRunRange(context, surface_, surfaceCaret_,
                                                    surfaceSelectLength_, text, selectOffset,
                                                    selectLength, &selected);
    if (result == ReplaceRunResult::Succeeded) {
        surface_ = text;
        // 選択に失敗したときは置換 session が末尾に潰したままになっている
        surfaceCaret_ = selected ? selectOffset : surface_.size();
        surfaceSelectLength_ = selected ? selectLength : 0;
        NoteDirectSuccess(false);
        return S_OK;
    }
    if (result == ReplaceRunResult::Unsupported || result == ReplaceRunResult::Unreadable) {
        directCapable_ = DirectCapability::Incapable;
    }
    // 文書と食い違った run は文書を触らずに捨てる。候補選択中なら文書には変換結果が
    // 残るが、composition 方式でアプリに composition を終了されたときと同じく学習しない
    DropRun();
    ClearContext();
    return E_FAIL;
}

void TextService::NoteDirectSuccess(bool newInsertion)
{
    if (!newInsertion) {
        // 別 session からの置換が通った = 前の打鍵で入れた文字列を読み直せた
        directCapable_ = DirectCapability::Capable;
    } else if (directCapable_ == DirectCapability::Unknown) {
        // CUAS 経由の文書は挿入直後の読み戻しだけは通るため、まだ可にはしない
        directCapable_ = DirectCapability::Provisional;
    }
}

std::wstring TextService::RunDisplayText()
{
    if (LiveConversionActive()) {
        return LiveDisplayText();
    }
    liveSegments_.clear(); // 非アクティブ時 (停止中・英字モード・設定OFF) は必ず空
    return composer_.Display();
}

std::wstring TextService::RunCommitText() const
{
    if (converting_ || predictionIndex_ >= 0) {
        return surface_;
    }
    if (!liveSegments_.empty()) {
        // 末尾の未変換ローマ字は Commit() の救済 ("n" のみ「ん」) を通した形で
        // 変換結果の後ろに付ける (CommitComposition のライブ分岐と同じ)
        return LiveText() + composer_.Commit().substr(composer_.ConfirmedKana().size());
    }
    return composer_.Commit();
}

HRESULT TextService::TypeDirect(ITfContext* context, const DirectKey& key)
{
    // 不一致・非対応で run を捨てるときに、この打鍵を含まない状態へ戻すための控え
    const RomajiComposer before = composer_;
    const std::vector<ConversionSegment> liveBefore = liveSegments_;
    bool newInsertion = !InRun();

    PushDirectKey(key);
    std::wstring text = RunDisplayText();
    ReplaceRunResult result = ReplaceRunText(context, surface_, text);
    if (result == ReplaceRunResult::Mismatch) {
        // キャレット移動やアプリ側の編集で run が文書と食い違った。古い run は
        // 忘れ (キャレットが動いた以上、直前の確定文脈も使えない)、
        // この打鍵を新しい run の先頭として挿入し直す
        composer_ = before;
        liveSegments_ = liveBefore;
        EndRun();
        ClearContext();
        PushDirectKey(key);
        newInsertion = true;
        text = RunDisplayText();
        result = ReplaceRunText(context, surface_, text);
    }
    if (result == ReplaceRunResult::Succeeded) {
        surface_ = text;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        NoteDirectSuccess(newInsertion);
        UpdatePrediction(context);
        return S_OK;
    }
    if (result == ReplaceRunResult::NoSurroundingText) {
        // 周辺テキストが読めるか判定できない (空の入力欄、または CUAS 経由の文書)。
        // 文書には何も入っていないので、この run だけ composition 方式で入力する。
        // 判定は未判定のまま据え置き、次の run で改めて確かめる
        HRESULT hr = StartComposition(context);
        if (FAILED(hr)) {
            composer_.Clear();
            liveSegments_.clear();
            return hr;
        }
        return UpdateCompositionAndPredict(context);
    }

    // 非対応・周辺テキストが読めない: この文書では以後 composition 方式で動く
    directCapable_ = DirectCapability::Incapable;
    if (newInsertion) {
        // 1打鍵目の読み戻し失敗。挿入した文字は文書に残り、run は持たない
        // (次の打鍵から composition 方式)
        composer_.Clear();
        liveSegments_.clear();
        surface_.clear();
        return S_OK;
    }
    // 2打鍵目以降: それまでの run の文字列は文書に残し、この打鍵から composition を始める
    composer_ = before;
    liveSegments_ = liveBefore;
    EndRun();
    PushDirectKey(key);
    HRESULT hr = StartComposition(context);
    if (FAILED(hr)) {
        composer_.Clear();
        return hr;
    }
    return UpdateCompositionAndPredict(context);
}

HRESULT TextService::BackspaceDirect(ITfContext* context)
{
    if (!InRun()) {
        return S_OK;
    }
    const RomajiComposer before = composer_;
    const std::vector<ConversionSegment> liveBefore = liveSegments_;
    composer_.Backspace();
    const std::wstring text = RunDisplayText();
    const ReplaceRunResult result = ReplaceRunText(context, surface_, text);
    if (result == ReplaceRunResult::Succeeded) {
        surface_ = text;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        NoteDirectSuccess(false);
        if (surface_.empty()) {
            // 消し切った run に確定に相当するものは無い (学習・アンドゥ記憶なし)
            DropRun();
            return S_OK;
        }
        UpdatePrediction(context);
        return S_OK;
    }
    // 文書と食い違った run は文書を触らずに忘れる (この Backspace は効かない)
    composer_ = before;
    liveSegments_ = liveBefore;
    if (result == ReplaceRunResult::Unsupported || result == ReplaceRunResult::Unreadable) {
        directCapable_ = DirectCapability::Incapable;
        EndRun();
    } else {
        EndRun();
        ClearContext();
    }
    return S_OK;
}

HRESULT TextService::SpaceDirect(ITfContext* context, bool shifted)
{
    if (!InRun()) {
        // 食べているのは !shifted && space=full のときだけ (composition 方式と同じ)
        return InsertText(context, StandaloneSpaceText());
    }
    // 確定文字列を作る前に自動英字判定ルール3を適用する (「わんt」→「want」)
    ApplyModelessCommitRule();
    // run 中の Space は幅によらず IME が入れる (run の終了と1つの edit session で行う)。
    // モードレスが有効なときだけ、英字モード中は英文の語の区切りとして半角にする
    const bool asciiWord = config_.Get().modeless && composer_.AsciiMode();
    const std::wstring space =
        (!shifted && config_.Get().spaceFullwidth && !asciiWord) ? L"　" : L" ";
    const std::wstring text = RunCommitText() + space;
    const ReplaceRunResult result =
        ReplaceRunRange(context, surface_, surfaceCaret_, surfaceSelectLength_, text, 0, 0);
    if (result == ReplaceRunResult::Succeeded) {
        // スペースまで含めて確定アンドゥの対象にする (Ctrl+Backspace でスペースごと
        // 読みに戻る)。学習・文脈は EndRun が変換状態と composer_ から求める
        surface_ = text;
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        NoteDirectSuccess(false);
        EndRun();
        return S_OK;
    }
    if (result == ReplaceRunResult::Unsupported || result == ReplaceRunResult::Unreadable) {
        directCapable_ = DirectCapability::Incapable;
        EndRun();
    } else {
        EndRun();
        ClearContext();
    }
    return InsertText(context, space);
}

HRESULT TextService::ConvertKeyDirect(ITfContext* context, bool shifted)
{
    if (!InRun()) {
        return ReconvertSelectionDirect(context);
    }
    if (converting_) {
        return CycleCandidate(context, shifted ? -1 : +1);
    }
    if (PromoteRun(context) == PromoteResult::Dropped) {
        return S_OK;
    }
    return StartConversion(context);
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
    // 選択した文字列と直前の確定は無関係
    ClearContext();
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
            if (result == ReplaceRunResult::Unsupported ||
                result == ReplaceRunResult::Unreadable) {
                directCapable_ = DirectCapability::Incapable;
            }
            DropRun();
            ClearContext();
            return S_OK;
        }
        surfaceCaret_ = surface_.size();
        surfaceSelectLength_ = 0;
        NoteDirectSuccess(false);
    }
    EndRun();
    return S_OK;
}

HRESULT TextService::UpdateRunAndPredict(ITfContext* context)
{
    const std::wstring text = RunDisplayText();
    HRESULT hr = ReplaceRunDisplay(context, text);
    if (FAILED(hr)) {
        return hr;
    }
    UpdatePrediction(context);
    return hr;
}

void TextService::EndRun()
{
    if (!surface_.empty()) {
        if (converting_) {
            // 候補選択中の確定: 文節ごとの学習と文脈更新 (composition 方式の確定と同じ)
            PrepareConversionCommit();
        } else if (predictionIndex_ >= 0 &&
                   static_cast<size_t>(predictionIndex_) < predictions_.size()) {
            // サジェスト選択中の確定: 候補の完全な読みで学習する
            const PredictionCandidate& candidate =
                predictions_[static_cast<size_t>(predictionIndex_)];
            engine_.Learn({{candidate.reading, candidate.surface, contextSurface_}});
            SetCommitContext(candidate.reading, candidate.surface);
        } else if (!liveSegments_.empty()) {
            // ライブ表示中の確定: 文節ごとに学習する (CommitComposition のライブ分岐と同じ)
            std::vector<LearnEntry> entries;
            std::wstring prevSurface = contextSurface_;
            for (const ConversionSegment& segment : liveSegments_) {
                entries.push_back({segment.reading, segment.candidates[0], prevSurface});
                prevSurface = segment.candidates[0];
            }
            engine_.Learn(entries);
            const std::wstring suffix =
                composer_.Commit().substr(composer_.ConfirmedKana().size());
            if (suffix.empty()) {
                SetCommitContext(liveSegments_.back().reading,
                                 liveSegments_.back().candidates[0]);
            } else {
                // 生ローマ字が末尾に付くと確定文字列と文節表記が一致しないため、
                // 誤った文脈を引きずらないようクリアする
                ClearContext();
            }
        } else {
            // 無変換の確定と同じ: 英字を含む入力 (英単語など) は読み=表記で学習し、
            // 確定したかなは次の変換の文脈にする (かなのみの学習はしない。
            // CommitComposition と同じ理由)
            const std::wstring kana = composer_.Commit();
            if (ContainsAsciiLetter(kana)) {
                engine_.Learn({{kana, kana, contextSurface_}});
            }
            SetCommitContext(kana, kana);
        }
        lastCommitText_ = surface_;
        lastComposer_ = composer_;
    }
    DropRun();
}

void TextService::DropRun()
{
    ClearConversion();
    ClearPrediction();
    ClearLiveConversion();
    composer_.Clear();
    surface_.clear();
    surfaceCaret_ = 0;
    surfaceSelectLength_ = 0;
}

HRESULT TextService::UndoCommitDirect(ITfContext* context)
{
    // 一度きりの操作として、成否に関わらず記憶を消す (UndoCommit と同じ)
    const std::wstring commitText = lastCommitText_;
    lastCommitText_.clear();
    if (commitText.empty() || InRun()) {
        return S_OK;
    }

    // キャレット直前が直前の run の surface と一致する場合のみ、読みのかな表示に
    // 置き換えて run を再開する (composition は開始しない)
    composer_ = lastComposer_;
    const std::wstring text = composer_.Display();
    if (ReplaceRunText(context, commitText, text) != ReplaceRunResult::Succeeded) {
        composer_.Clear();
        return S_OK;
    }
    surface_ = text;
    surfaceCaret_ = surface_.size();
    surfaceSelectLength_ = 0;
    // 確定を取り消したので、その確定を前提にした文脈補正はもう使えない
    ClearContext();
    // 復元した読みを即ライブ再変換すると、直したいはずの誤変換へ戻ってしまうため、
    // この run の間はライブ変換を止める
    liveSuspended_ = true;
    return S_OK;
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
    if (match == ReplaceRunResult::Unreadable && directCapable_ == DirectCapability::Capable) {
        // ReplaceRunRange と同じく、読める文書で範囲が作れないのはキャレットが動いたため
        match = ReplaceRunResult::Mismatch;
    }
    if (match != ReplaceRunResult::Succeeded) {
        if (match == ReplaceRunResult::Unsupported || match == ReplaceRunResult::Unreadable) {
            directCapable_ = DirectCapability::Incapable;
        }
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
    // 文書上の位置は以後 composition が持つ。composer_・ライブ変換の状態・文脈は
    // そのまま composition 方式へ引き継ぐ (確定ではないので学習・確定アンドゥの記憶はしない)
    surface_.clear();
    surfaceCaret_ = 0;
    surfaceSelectLength_ = 0;
    ClearPrediction();
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
    // composition に今表示している文字列 (UpdateCompositionAndPredict・サジェスト選択の
    // 表示と同じ求め方)。これを確定せずにそのまま run の surface にする
    std::wstring text;
    if (predictionIndex_ >= 0 && static_cast<size_t>(predictionIndex_) < predictions_.size()) {
        text = predictions_[static_cast<size_t>(predictionIndex_)].surface;
    } else if (!liveSegments_.empty()) {
        text = LiveText() + composer_.Display().substr(composer_.ConfirmedKana().size());
    } else {
        text = composer_.Display();
    }
    HRESULT hr = E_UNEXPECTED;
    if (context != nullptr) {
        hr = RequestSync(context,
                         new (std::nothrow) EndCompositionEditSession(context, composition_, text),
                         TF_ES_SYNC | TF_ES_READWRITE);
    }
    composition_->Release();
    composition_ = nullptr;
    promoted_ = false;
    if (FAILED(hr) || text.empty()) {
        DropRun();
        ClearContext();
        return;
    }
    // composer_・ライブ変換・サジェストの状態は維持し、direct 方式の run として続ける
    surface_ = text;
    surfaceCaret_ = surface_.size();
    surfaceSelectLength_ = 0;
}
