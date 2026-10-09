#pragma once

#include <windows.h>
#include <msctf.h>

#include <bitset>
#include <string>
#include <vector>

#include "candidate_window.h"
#include "config.h"
#include "engine_client.h"
#include "romaji.h"

class LangBarButton;
enum class ReplaceRunResult;

// キーの分類 (text_service.cpp と text_service_direct.cpp で共用。定義は text_service.cpp)
namespace key_util {

// 記号キー1つぶんの定義: 未確定文字列に入れるかな (全角形) と打鍵文字そのもの
struct SymbolKey {
    WPARAM vk;
    const wchar_t* kana; // 未確定文字列へ入れる全角形
    const wchar_t* raw;  // 打鍵文字 (生ローマ字候補・英字モード用)
};

// 記号キー (仮想キーコード) → かな/打鍵文字。該当しなければ nullptr
const SymbolKey* FindSymbolKey(WPARAM wparam, bool shifted);
bool IsLetterKey(WPARAM wparam);
bool IsDigitKey(WPARAM wparam);
// テンキーの打鍵文字 (NumLock オンの数字と演算記号)。対応しないキーは 0
wchar_t NumpadChar(WPARAM wparam);
bool IsShiftPressed();
bool ContainsAsciiLetter(const std::wstring& text);

} // namespace key_util

// TSF テキストサービス本体。
// 打鍵したかなを未確定文字列を使わずに文書へ追記し (run)、変換キーで run を
// composition に昇格して候補ウィンドウから選ぶ (docs/design/direct-input.md、
// docs/design/append-input.md)。
class TextService : public ITfTextInputProcessorEx,
                    public ITfThreadMgrEventSink,
                    public ITfKeyEventSink,
                    public ITfCompositionSink,
                    public ITfCompartmentEventSink,
                    public ITfDisplayAttributeProvider,
                    public ITfTextEditSink {
public:
    TextService();

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    // ITfTextInputProcessor
    STDMETHODIMP Activate(ITfThreadMgr* threadMgr, TfClientId clientId) override;
    STDMETHODIMP Deactivate() override;

    // ITfTextInputProcessorEx
    STDMETHODIMP ActivateEx(ITfThreadMgr* threadMgr, TfClientId clientId, DWORD flags) override;

    // ITfThreadMgrEventSink (ドキュメントフォーカスの変更通知。
    // OnSetFocus を設定ファイルの変更反映のタイミングに使う)
    STDMETHODIMP OnInitDocumentMgr(ITfDocumentMgr* docMgr) override;
    STDMETHODIMP OnUninitDocumentMgr(ITfDocumentMgr* docMgr) override;
    STDMETHODIMP OnSetFocus(ITfDocumentMgr* focus, ITfDocumentMgr* prevFocus) override;
    STDMETHODIMP OnPushContext(ITfContext* context) override;
    STDMETHODIMP OnPopContext(ITfContext* context) override;

    // ITfKeyEventSink
    STDMETHODIMP OnSetFocus(BOOL foreground) override;
    STDMETHODIMP OnTestKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten) override;
    STDMETHODIMP OnTestKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten) override;
    STDMETHODIMP OnKeyDown(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten) override;
    STDMETHODIMP OnKeyUp(ITfContext* context, WPARAM wparam, LPARAM lparam, BOOL* eaten) override;
    STDMETHODIMP OnPreservedKey(ITfContext* context, REFGUID rguid, BOOL* eaten) override;

    // ITfCompositionSink (アプリ側が composition を強制終了したときに呼ばれる)
    STDMETHODIMP OnCompositionTerminated(TfEditCookie ecWrite, ITfComposition* composition) override;

    // ITfCompartmentEventSink (IMEオン/オフ状態の変更通知)
    STDMETHODIMP OnChange(REFGUID rguid) override;

    // ITfDisplayAttributeProvider
    STDMETHODIMP EnumDisplayAttributeInfo(IEnumTfDisplayAttributeInfo** enumInfo) override;
    STDMETHODIMP GetDisplayAttributeInfo(REFGUID guid, ITfDisplayAttributeInfo** info) override;

    // ITfTextEditSink (キャレット移動による run の終了用)
    STDMETHODIMP OnEndEdit(ITfContext* context, TfEditCookie ecReadOnly,
                           ITfEditRecord* editRecord) override;

private:
    // 言語バー項目からオン/オフ切替・ツール起動を呼ぶ
    friend class LangBarButton;

    ~TextService();

    // このキー入力を IME が処理する (アプリに渡さない) かどうか
    // (context は direct 方式の後置再変換の判定で選択テキストを読むのに使う)
    bool IsKeyEaten(ITfContext* context, WPARAM wparam) const;

    // ---- キー割当 (docs/design/keymap.md) ----
    // 現在の入力状態 (割当の照合に使う)
    KeyState CurrentKeyState() const;
    // 押されている修飾キーと wparam を現在の状態の割当と照合する
    KeyMatch MatchKeyFunc(WPARAM wparam) const;
    // 割当に一致した機能を今実行できるか (入力なしの状態では、実行できないときは
    // 打鍵を食べずにアプリへ渡す)
    bool CanRunKeyFunc(ITfContext* context, KeyFunc func) const;
    // 割当に一致した機能を実行する
    HRESULT RunKeyFunc(ITfContext* context, const KeyMatch& match);

    // 設定ファイルの変更を確認し、変わっていれば反映する
    // (フォーカス切替・IMEオンなどの軽いタイミングで呼ぶ)
    void RefreshConfig();

    // ---- IMEオン/オフ (OPENCLOSE compartment) ----
    // オン/オフ状態を保持する compartment (呼び出し側で Release する。失敗時 nullptr)
    ITfCompartment* OpenCloseCompartment() const;
    bool IsKeyboardOpen() const;
    void SetKeyboardOpen(bool open);

    // 食べたキーを状態機械に従って処理する (候補選択中の composition とそれ以外で
    // 振り分け、処理後に昇格した composition の降格を判定する)
    HRESULT HandleKey(ITfContext* context, WPARAM wparam);
    // 候補選択中 (run を昇格した composition) のキー処理
    HRESULT HandleKeyConverting(ITfContext* context, WPARAM wparam);

    // 文字種変換・記号変換・ユーザ語変換 (既定 F4-F10) を実行する。それ以外の機能は何もしない
    HRESULT ApplyFunctionKey(ITfContext* context, KeyFunc func);

    // 同期 edit session の実行 (session の所有権を受け取り、実行後に解放する)
    HRESULT RequestSync(ITfContext* context, ITfEditSession* session, DWORD flags) const;

    HRESULT StartComposition(ITfContext* context);
    // composition のテキストを任意の文字列に差し替える
    HRESULT UpdateCompositionText(ITfContext* context, const std::wstring& text);
    // 確定 (commitText が空なら取消)。変換状態と候補ウィンドウも後始末する
    HRESULT EndComposition(ITfContext* context, const std::wstring& commitText);
    // composition を使わない直接挿入 (run が無いときのスペース入力用)
    HRESULT InsertText(ITfContext* context, const std::wstring& text);

    // ファンクションキーによる直接変換の文字種 (F6-F10)
    enum class ConversionForm {
        Hiragana,          // F6
        Katakana,          // F7
        HalfwidthKatakana, // F8
        FullwidthAscii,    // F9
        HalfwidthAscii,    // F10
    };

    // 変換の開始 / 現在文節の候補移動 / 文節の移動 / 変換の取消 (かな表示に戻す)
    HRESULT StartConversion(ITfContext* context);
    // 変換の候補生成 (エンジン問い合わせ + 生ローマ字候補 + 対記号同期) で変換状態を作る。
    // 表示は行わない (両方式で共用)
    void BuildConversionSegments();
    HRESULT CycleCandidate(ITfContext* context, int delta);
    // 数字キー 1〜9: 候補ウィンドウの表示中ページ内の番号 (0始まり) で
    // 現在文節の候補を直接選択する (対応する候補が無ければ何もしない)
    HRESULT SelectCandidateByNumber(ITfContext* context, size_t number);
    // 現在文節の選択を index に変えて表示へ反映する共通処理
    HRESULT ApplyCandidateSelection(ITfContext* context, size_t index);
    HRESULT MoveSegment(ITfContext* context, int delta);
    // 指定した文節へ直接移動する (PgUp=先頭, PgDn=末尾)
    HRESULT MoveSegmentTo(ITfContext* context, size_t index);
    // 現在文節の境界を delta 文字ぶん伸縮し、境界固定で再変換する
    HRESULT ResizeSegment(ITfContext* context, int delta);
    HRESULT CancelConversion(ITfContext* context);

    // F6-F10: 現在文節 (未変換なら全文を1文節にして) を指定の文字種へ直接変換する。
    // F7/F8 は連打で後ろから1文字ずつひらがなに戻し、F9/F10 は連打で
    // 元のまま → 先頭大文字 → 全部大文字 (→ 全部小文字) と循環する。
    // 候補ウィンドウは表示しない
    HRESULT DirectConvert(ITfContext* context, ConversionForm form);
    // F4: 現在文節 (未変換なら全文) を特殊変換 (記号辞書 + 日付・時刻) の候補のみで変換する
    HRESULT ConvertToSymbols(ITfContext* context);
    // F5: 現在文節 (未変換なら全文) を短縮よみ (ユーザ辞書) の候補のみで変換する
    HRESULT ConvertToShortcuts(ITfContext* context);
    // 未変換なら全文を1文節とした変換状態を作る (直接変換の下準備)
    void EnsureConversionState();
    // index の文節の選択候補がかっこ・クオートなどの対記号なら、
    // 対になる側の文節の選択も対応する記号に同期させる
    void SyncPairedSegment(size_t index);
    // 現在文節を form で変換した文字列 (対応する打鍵が無いなどの場合は空)
    std::wstring SegmentFormText(size_t index, ConversionForm form) const;
    // index の文節の読みに対応する打鍵列 (切り出せない場合は空)
    std::wstring SegmentRawText(size_t index) const;
    // F7-F10 連打用: 現在の選択候補から見て循環列の次の形
    std::wstring NextFormText(size_t index, ConversionForm form) const;

    struct DirectKey;

    // ---- 候補バー (docs/design/candidate-bar.md) ----
    // 候補の種類
    enum class BarKind {
        Whole,       // 全体変換 (確定済みかな全体の CONVNBEST の上位)
        Prediction,  // 予測候補
        Head,        // 先頭文節 (部分採用)
    };
    struct BarCandidate {
        BarKind kind;
        std::wstring surface;  // 採用する表記
        // 学習に送る読み (全体変換は確定済みかな、予測候補は候補の完全な読み、
        // 先頭文節はその文節の読み)
        std::wstring reading;
        // 全体変換の文節ごとの (読み, 表記) (全体変換の学習に使う)
        std::vector<std::pair<std::wstring, std::wstring>> segments;
    };
    // 採用の仕方
    enum class BarAdopt {
        End,        // run を終える (Enter・Space)
        CommitKey,  // 確定キー: 先頭文節は run を続け、それ以外は run を終える
        Continue,   // 採用した部分だけを run から外して続ける (印字キー)
    };
    // 現在の確定済みかなで候補を作り直して表示する (条件を満たさなければ消す)。選択は解除する
    HRESULT UpdateBar(ITfContext* context);
    // composition を Display() で更新し、続けてバーを作り直す (候補選択を取り消したときの表示)
    HRESULT UpdateCompositionAndBar(ITfContext* context);
    // バーの候補と選択を破棄する (非変換中なら候補ウィンドウも隠す)
    void ClearBar();
    // バーの選択を動かす (+1: 未選択→先頭→...→末尾→先頭、-1: 先頭でさらに↑は解除)。
    // 文書は書き換えない
    HRESULT MoveBarSelection(int delta);
    // 数字キー 1〜9: バー選択中に番号 (0始まり) の候補を選ぶ (無ければ何もしない)
    HRESULT SelectBarByNumber(size_t number);
    // バーの選択を解除する (バーは表示のまま)
    HRESULT DeselectBar();
    // バーの index の候補を採用する。run の文字列を「採用部分 + 残りのかな」(run を終えるときは
    // 未完成のローマ字と suffix も) に作り直す (追記のみの文書では擬似 Backspace の後に追記する)。
    // followKey があれば、採用の後にその打鍵を入れる
    HRESULT AdoptBarItem(ITfContext* context, size_t index, BarAdopt mode,
                         const std::wstring& suffix, const DirectKey* followKey);
    // 採用の文書の書き換えを終えた後の状態処理 (学習・文脈・読みの切り離し・run の終了または
    // 継続)。updateBar なら run を続けるときにバーを作り直す
    void FinishBarAdoption(ITfContext* context, bool updateBar);
    // 確定キー (バー未選択): 全体変換があれば採用して run を終え、無ければ (ルール3で英字に
    // なる場合も) アプリへ渡すキーと同じ救済を通して run を終える
    HRESULT CommitRunKey(ITfContext* context);
    // 未完成ローマ字の小窓・候補バーの位置の基準 (選択範囲の矩形、取れなければ
    // システムキャレットの矩形)。どちらも取れなければ false
    bool CaretRect(ITfContext* context, RECT* rect);

    // ---- モードレス入力 (設定 modeless。判定の本体は RomajiComposer) ----
    // 無変換のまま確定する直前に、自動英字判定の判定ルール3 (末尾に残った
    // 子音1文字で英字と判定する) を適用する。変換結果で確定する経路
    // (候補選択中・バー選択中) では読みを英字へ作り直せないため何もしない。
    // 読みが英字へ変わると表示も変わるため、文書の表示を同時に直せる経路からのみ呼ぶ
    // (フォーカス移動などの run 終了では呼ばない)
    void ApplyModelessCommitRule();
    // run が無いときに挿入するスペース。モードレス有効時のみ、直前の確定が
    // ASCII 英数字だけなら英文の途中とみなして半角にし、それ以外は設定 space に従う
    std::wstring StandaloneSpaceText() const;

    // 候補選択中の composition を現在の選択のまま確定する (IME オフ時)
    HRESULT CommitComposition(ITfContext* context);
    // 変換結果を確定する (エンジンへの学習送信 + composition 終了)
    HRESULT CommitConversion(ITfContext* context);
    // 変換確定の前half: エンジンへの学習送信のみ行い、確定文字列を返す
    // (edit session は発行せず、状態も変えない)
    std::wstring PrepareConversionCommit();
    // 確定した文節列 (読み, 表記) の学習内容: 文節ごとの (読み, 表記, 直前文節の表記) と、
    // 2文節以上なら読み全体 → 表記の連結 (文脈なし)
    std::vector<LearnEntry> SentenceLearnEntries(
        const std::vector<std::pair<std::wstring, std::wstring>>& segments) const;
    // 文節伸縮で分割を直したまま確定したときの学習をエンジンへ送る
    void LearnResizedSegments();
    // 変換確定の状態後始末 (EndComposition の状態管理部分と同じ):
    // 確定アンドゥ情報を記憶し、変換・予測状態と composer_ を破棄する
    void FinishConversionState(const std::wstring& commitText);
    // 「確定 + 新しい composition の開始」を1つの edit session で行い、
    // composition_ を新しいものに差し替える。未確定文字列の表示は
    // 呼び出し側が UpdateCompositionAndPredict で別途行う。
    // 候補選択中に印字キーが来たときは必ずこれを使う (分割すると Word や
    // CUAS 経由のアプリで新しい composition が生き残らない)
    HRESULT RestartComposition(ITfContext* context, const std::wstring& commitText);
    // 確定アンドゥ (Ctrl+Backspace): 直前の確定文字列を読みのかなに戻して run を再開する。
    // 読める文書はキャレット直前が確定文字列と一致するときだけ置き換え、
    // 追記のみの文書は擬似 Backspace で消してから追記する
    HRESULT UndoCommit(ITfContext* context);
    // 単語登録 (既定 Ctrl+F7): 選択テキストを初期値にして登録ツールを起動する
    HRESULT LaunchWordRegister(ITfContext* context);
    // 設定ツール (quicklime-config.exe) の起動 (既定 Ctrl+F12)
    HRESULT LaunchConfigTool();
    // 変換中の表示 (選択候補の連結 + 現在文節の強調) を composition に反映する。
    // 昇格できなかった run では surface_ を置き換えて現在文節を選択状態にする
    HRESULT UpdateConvertingDisplay(ITfContext* context);
    // 現在の選択に基づく確定文字列 (全文節の選択候補の連結)
    std::wstring ConvertedText() const;
    // 現在文節の候補一覧で候補ウィンドウを表示する
    void ShowCandidateWindow(ITfContext* context);
    // 候補ウィンドウの表示位置 (composition の矩形。取れなければキャレット/マウス位置)
    RECT CandidateAnchor(ITfContext* context);
    // 変換状態を破棄する (composition は触らない)
    void ClearConversion();

    // ---- 文脈補正 (直前確定文節の読み・表記を覚えておき、次の変換に渡す) ----
    // 現在の文脈補正用コンテキストを取得する (未設定なら空)
    ConversionContext CurrentContext() const { return {contextReading_, contextSurface_}; }
    // 確定した文節の読み・表記を文脈として記憶する
    void SetCommitContext(const std::wstring& reading, const std::wstring& surface);
    // 文脈を破棄する (フォーカス移動・IMEオフ・確定アンドゥなど、直前の確定が
    // 次の変換の左文脈として使えなくなったとき)
    void ClearContext();

    bool Composing() const { return composition_ != nullptr; }

    // ---- run (実装は text_service_direct.cpp) ----
    // 打鍵したかなを composition ではなく文書に追記し、IME が「自分が入れた文字列
    // (surface_) とその読み (composer_)」= run を覚えておく。未完成のローマ字は文書に
    // 入れずにキャレット付近の小窓に出し、変換するときは run を composition に昇格する
    // (docs/design/direct-input.md、docs/design/append-input.md)
    // 1打鍵が composer_ に加える内容
    struct DirectKey {
        wchar_t romaji = 0;       // 0 以外ならローマ字として Push する (英字モード中は raw)
        std::wstring kana;        // romaji が 0 のとき PushKana するかな (英字モード中は raw)
        std::wstring raw;         // 打鍵文字そのもの
        bool enterAscii = false;  // Shift+英字: 英字モードに入る
    };
    // 未完成のローマ字だけの run (文書にはまだ何も入っていない) もある。
    // 候補選択中の composition は run に数えない
    bool InRun() const { return !surface_.empty() || (!Composing() && !composer_.Empty()); }
    // 割当に一致しなかったキーを食べるか (候補選択中の composition 以外)
    bool IsKeyEatenDirect(WPARAM wparam) const;
    HRESULT HandleKeyDirect(ITfContext* context, WPARAM wparam);
    // 食べずにアプリへ渡すキーのうち run を終えるもの (Enter・矢印・Ctrl 併用など) の
    // 状態処理。候補選択中・バー選択中の Enter は確定してから渡す
    // (EnterNeedsResend のときは食べるのでここでは扱わない)。
    // OnTestKeyDown / OnKeyDown の両方から呼ぶ (2回目以降は何もしない)
    void EndRunIfPassthroughKey(ITfContext* context, WPARAM wparam);
    // バー選択中の採用に擬似 Backspace が要る (文書が読めると判定されていない)
    bool AdoptionNeedsPseudoBackspace() const;
    // Enter (Ctrl+M) を食べて確定し、確定の後に元の打鍵を送り直す (読めると判定されていない
    // 文書の候補選択中と、擬似 Backspace が要るバーの採用)。composition の有無によらない
    bool EnterNeedsResend() const;
    // 食べた Enter (Ctrl+M) の確定を行い、確定の後で元の打鍵 vk をアプリへ送り直す
    HRESULT CommitAndResendEnter(ITfContext* context, WPARAM vk, bool ctrl, bool shifted);
    // 食べた Enter (Ctrl+M) を、確定 (擬似 Backspace による書き換えを含む) の後でアプリへ送り直す
    void SendResendKey();
    // 打鍵を分類する。印字キー (英字・記号・数字・テンキー) でなければ false
    bool ClassifyDirectKey(WPARAM wparam, bool shifted, DirectKey* key) const;
    void PushDirectKey(const DirectKey& key);
    HRESULT SpaceDirect(ITfContext* context, bool shifted);
    // 変換 (KeyFunc::Convert): run 中は変換開始 / 次候補 (previous なら前候補)、
    // run が無ければ後置再変換
    HRESULT ConvertKeyDirect(ITfContext* context, bool previous);
    // 後置再変換: 選択テキスト (ひらがな・カタカナ・ー のみ) を読みとして run を作り変換開始
    HRESULT ReconvertSelectionDirect(ITfContext* context);
    // 現在の選択テキストが後置再変換の対象なら true (対象なら textOut に入れる)
    bool ReadReconvertibleSelection(ITfContext* context, std::wstring* textOut) const;
    // 候補選択中の確定 (選択を末尾に潰して run を終える)、バー選択中の確定 (選択中の候補を
    // 採用して run を終える)
    HRESULT CommitRunDirect(ITfContext* context);
    // 文書には確定したかなを、小窓には未完成のローマ字を出してバーを作り直す
    // (昇格できなかった run の変換を取り消したとき)
    HRESULT UpdateRunAndBar(ITfContext* context);
    // キャレット直前の expected を newText に置き換える (expected が空なら挿入)
    ReplaceRunResult ReplaceRunText(ITfContext* context, const std::wstring& expected,
                                    const std::wstring& newText);
    // 選択開始が expected の caretOffset 文字目、選択の長さが currentSelectLength であるとして
    // expected を newText に置き換える。選択が末尾に潰れたキャレットでなければ先に末尾へ
    // 潰してから置換する (Chromium 系は潰れていない選択と異なる範囲の置換を正しく反映
    // しないため)。selectLength > 0 なら置換後に別 session で newText 内の範囲を選択し、
    // 選択できたかを *selectedOut に返す (選択に失敗しても置換の結果は Succeeded のまま。
    // そのとき選択は末尾に潰れている)
    ReplaceRunResult ReplaceRunRange(ITfContext* context, const std::wstring& expected,
                                     size_t caretOffset, size_t currentSelectLength,
                                     const std::wstring& newText, size_t selectOffset,
                                     size_t selectLength, bool* selectedOut = nullptr);
    // 文字列は変えずに、run (expected、選択開始が caretOffset 文字目) 内の
    // [selectOffset, selectOffset + selectLength) を選択する (長さ 0 なら潰す)
    ReplaceRunResult SelectRunRange(ITfContext* context, const std::wstring& expected,
                                    size_t caretOffset, size_t selectOffset,
                                    size_t selectLength);
    // surface_ を text に置き換えて run を続ける。失敗したら run を捨てる (文書は触らない)
    HRESULT ReplaceRunDisplay(ITfContext* context, const std::wstring& text,
                              size_t selectOffset = 0, size_t selectLength = 0);
    // run を終えて忘れる (既存の確定処理と同じ学習送信・文脈更新・確定アンドゥ用の記憶)
    void EndRun();
    // run の状態を学習せずに捨てる (文書と食い違った run の後始末)
    void DropRun();
    // run を composition に昇格した結果
    enum class PromoteResult {
        Promoted,  // composition_ に移った (run の文書上の状態は捨てた)
        Dropped,   // 照合が文書と食い違った等で run を捨てた (文書は触らない)
        Refused,   // StartComposition が失敗・拒否された (run はそのまま。選択による強調で変換する)
    };
    // 変換状態に入る直前に、run の範囲に composition を張る (文字列は変えない)。
    // 候補選択中の表示 (下線・現在文節の強調) を composition の経路で行うため
    PromoteResult PromoteRun(ITfContext* context);
    // 昇格した composition が変換状態でなくなっていれば、確定したかなのまま
    // composition を終えて run に戻す (学習はしない)
    void DemoteIfLeftConversion(ITfContext* context);
    // 文書が読み戻せると分かっているか (範囲が作れないのをキャレット移動とみなせるか)
    bool DocumentReadable() const;

    // ---- 追記型入力 (実装は text_service_direct.cpp) ----
    // 文書の分類。読める文書は追記前に照合し、変換・作り直しは置換で行う。
    // 追記のみの文書は照合せずに追記し、変換・作り直しは擬似 Backspace で消してから行う
    enum class AppendDocument {
        Unknown,     // 未判定 (まだ読み戻しを試していない、または run の1文字目)
        Readable,    // run の文字列を別 session から読み戻せた (照合して追記、置換で変換)
        AppendOnly,  // 読み戻せなかった (照合せず追記、擬似 Backspace で変換)
    };
    // 擬似 Backspace の後、目印の打鍵を受け取った時点で行うこと
    enum class PseudoKeyAction {
        None,
        Compose,  // run の読みで composition を張り、変換 (appendActionFunc_) に入る
        Append,   // appendActionText_ を追記する (英字切替・ルール3・確定アンドゥ)
        Adopt,    // バーの採用 (adoption_.written) を追記して FinishBarAdoption を行う
    };
    void DebugLog(const std::wstring& message) const;
    void SetAppendDocument(AppendDocument document, const wchar_t* reason);
    // 文書に入っていない未完成のローマ字 (小窓に出す文字列)
    std::wstring AppendPendingText() const;
    // 印字キー: 読みを進め、増えたかなを追記して未完成のローマ字の表示を更新する
    HRESULT TypeAppend(ITfContext* context, const DirectKey& key);
    enum class AppendResult {
        Done,
        Mismatch,  // 読める文書で照合が一致しなかった (何も書いていない)
        Failed,
    };
    // composer_ に合わせて文書 (追記・作り直し) と未完成ローマ字の表示を更新する
    AppendResult SyncAppendRun(ITfContext* context);
    // text を追記する。未判定の文書では照合の結果で分類し、読める文書で不一致なら
    // 何も書かずに Mismatch
    AppendResult AppendRunText(ITfContext* context, const std::wstring& text);
    // run の文字列を newText に作り直す (読める文書は置換、追記のみの文書は擬似 Backspace)。
    // endRunAfter なら作り直した後に run を終える
    AppendResult RewriteAppendRun(ITfContext* context, const std::wstring& newText,
                                  bool endRunAfter);
    // 未完成ローマ字の小窓の表示を pending にする (空なら隠す)
    void UpdateAppendPending(ITfContext* context, const std::wstring& pending);
    // 文書に入っていない未完成のローマ字を読みから取り除く
    void DiscardPendingRomaji();
    // 未完成ローマ字をそのまま文書に追記する (run を終える前の後始末)
    AppendResult FlushAppendPending(ITfContext* context);
    // 未判定の文書を run の文字列の読み戻しで分類する
    void ClassifyAppendDocument(ITfContext* context);
    HRESULT BackspaceAppend(ITfContext* context);
    HRESULT SpaceAppend(ITfContext* context, bool shifted);
    // 食べずにアプリへ渡すキー (Enter・矢印など) の前に run を終える
    void EndAppendRunForPassthroughKey(ITfContext* context, bool ctrl, bool alt);
    // 変換キー・F4〜F10: 読める文書は昇格、追記のみの文書は擬似 Backspace の後に composition
    HRESULT BeginAppendConversion(ITfContext* context, KeyFunc func);
    // run の文字列 (surface_) を擬似 Backspace で文書から消してから action を行う
    // (消す文字が無ければ即座に。text は Append で入れる文字列)
    HRESULT ScheduleAppendAction(ITfContext* context, PseudoKeyAction action, KeyFunc func,
                                 const std::wstring& text, bool endRunAfter);
    void RunAppendAction(ITfContext* context);
    // 目印待ちと、目印の後に行う予定だったことを捨てる (フォーカス移動・IME オフ)
    void CancelAppendAction();
    // OnTestKeyDown / OnKeyDown の入口: 目印待ちの間の擬似打鍵を処理したら true
    bool HandlePseudoKey(ITfContext* context, WPARAM wparam, bool keyDown, BOOL* eaten);
    // 食べなかったキーの後処理 (アプリへ渡した Backspace に run を追従させる。
    // 追記のみの文書ではアプリへ渡したキーで確定アンドゥの記憶を捨てる)
    void NoteKeyForAppend(ITfContext* context, WPARAM wparam, bool eaten, bool fromTest);
    static const wchar_t* AppendDocumentName(AppendDocument document);
    // フォーカス中の文書の最上位 context に ITfTextEditSink を付け替える
    void UpdateTextEditSink(ITfDocumentMgr* docMgr);
    // 追記のみの文書 (未判定を含む) で run か確定アンドゥの記憶がある間だけ、このスレッドに
    // マウスフックを仕掛ける (文書を読み戻せないので、クリック・ホイールでのキャレット移動を
    // 見落とすと擬似 Backspace が別の文字を消す)。状態が変わりうる入口の最後で呼ぶ
    void UpdateMouseHook();
    // マウスのボタンを押す操作・ホイールを受けた: 文書に触れずに run を終え、
    // 確定アンドゥの記憶も捨てる
    void OnMouseInput();
    static LRESULT CALLBACK MouseHookProc(int code, WPARAM wparam, LPARAM lparam);

    LONG refCount_;
    ITfThreadMgr* threadMgr_;
    TfClientId clientId_;
    ITfComposition* composition_;   // 進行中の composition (無ければ nullptr)
    RomajiComposer composer_;
    TfGuidAtom inputAttribute_;     // 未確定文字列に付ける表示属性の atom
    TfGuidAtom targetAttribute_;    // 変換対象文節に付ける表示属性の atom

    bool converting_;                          // 変換中 (候補選択中) かどうか
    std::vector<ConversionSegment> segments_;  // 変換結果の文節列
    std::vector<size_t> selected_;             // 文節ごとの選択中候補 index
    size_t segmentIndex_;                      // 操作対象の文節
    // この composition で文節伸縮 (Shift+←→) を行ったか。
    // 人が直した区切りだけを境界学習に送るための判定に使う
    bool segmentsResized_;
    // 最初の文節伸縮を行う直前の文節ごとの読みの長さ。
    // 確定時に「区切り直し」と「複合語を割って入力した」を見分けるのに使う
    std::vector<size_t> preResizeLengths_;
    // 入力全体を1文節にした候補選択 (segment_ui 0) の、候補ごとの文節。確定時の学習に使う
    std::vector<SentenceCandidate> wholeCandidates_;
    CandidateWindow candidateWindow_;
    EngineClient engine_;                      // 変換エンジンへの named pipe クライアント
    ConfigLoader config_;                      // ユーザ設定 (config.tsv) のローダ

    // 候補バー。候補ウィンドウは候補選択中の縦の表示と排他で共用する
    std::vector<BarCandidate> barItems_;  // 表示中の候補 (空 = 非表示)
    std::wstring barKana_;                // 候補を作った確定済みかな
    int barIndex_;                        // 選択中の候補 index (-1 = 未選択)
    // バーの x 座標。run で最初に出した時点 (部分採用の後はその時点) の位置に固定する
    int barX_;
    bool barXFixed_;
    // 採用で文書を書き換えた後に行うこと (追記のみの文書では目印の打鍵まで持ち越す)
    struct BarAdoption {
        std::vector<LearnEntry> learn;
        std::wstring contextReading;   // 次の前文脈にする文節
        std::wstring contextSurface;
        size_t readingLength = 0;      // 読みの先頭から取り除く文字数
        size_t adoptedLength = 0;      // written のうち採用部分の長さ
        std::wstring written;          // 文書の run の文字列を置き換える文字列
        bool endRun = false;
        RomajiComposer before;         // 採用前の読み (確定アンドゥの復元用)
    };
    BarAdoption adoption_;

    // run: 現在文書に入っている、この run 由来の文字列 (未完成のローマ字は含まない)
    std::wstring surface_;
    // 文書の選択開始が surface_ の何文字目にあるか (候補選択中は現在文節の先頭、
    // それ以外は末尾)。置換 session の expected 照合の基点に使う
    size_t surfaceCaret_;
    // 文書の選択の長さ (候補選択中は現在文節の長さ、それ以外は 0)
    size_t surfaceSelectLength_;
    // composition_ が run から昇格したもの (変換状態を抜けたら run に戻す) か。
    // 候補選択中の印字キーで確定して始まった次の composition にも引き継ぐ
    bool promoted_;

    // 追記型入力の状態
    AppendDocument appendDocument_;          // フォーカス中の文書の分類
    CandidateWindow pendingWindow_;          // 未完成のローマ字の小窓
    bool awaitingMarker_;                    // 擬似 Backspace を送り、目印の打鍵を待っている
    PseudoKeyAction appendAction_;           // 目印の打鍵を受け取ったときに行うこと
    KeyFunc appendActionFunc_;               // Compose で行う変換 (Convert は通常の変換)
    std::wstring appendActionText_;          // Append で入れる文字列
    bool appendActionEndRun_;                // Append の後に run を終えるか
    // Adopt の後に入れる印字キー (バー選択中の印字キーで採用したとき)。
    // 擬似 Backspace より先に追記すると、その文字まで消されるため目印の後に回す
    DirectKey appendFollowKey_;
    bool appendFollowKeyPending_;
    // 確定の後にアプリへ送り直す打鍵 (0 なら無し) とその修飾キー
    // (EnterNeedsResend のときの Enter・Ctrl+M)
    WPARAM appendResendVk_;
    bool appendResendCtrl_;
    bool appendResendShift_;
    HHOOK mouseHook_;                        // UpdateMouseHook が仕掛けたマウスフック
    // 直前の OnTestKeyDown でアプリへ渡した Backspace に run を追従させた (同じ打鍵の
    // OnKeyDown で二重に削らないため)
    bool backspaceTested_;
    // アプリへ渡した Backspace による文書の編集が来る見込み (OnEndEdit で IME 由来とみなす)
    bool keyEditExpected_;
    mutable int ownEditDepth_;               // 自分の edit session の実行中 (OnEndEdit の判定用)
    ITfContext* textEditSinkContext_;        // ITfTextEditSink を付けた context
    DWORD textEditSinkCookie_;

    std::wstring lastCommitText_;   // 直前に確定した文字列 (確定アンドゥ用。使うと消える)
    RomajiComposer lastComposer_;   // 直前の確定時点のコンポーザ (読みと打鍵列の復元用)

    // 文脈補正用の確定履歴 (直前に確定した最終文節の読み・表記)。
    // lastCommitText_ (確定アンドゥ用、使うと消える) とは寿命が異なるため別に持つ
    std::wstring contextReading_;
    std::wstring contextSurface_;

    DWORD openCloseCookie_;         // OPENCLOSE compartment sink の cookie
    DWORD threadMgrEventCookie_;    // thread manager event sink の cookie
    LangBarButton* langBarButton_;  // 言語バー項目 (登録失敗時は nullptr)
    // key-down を食べたキー。端末エミュレータは key-up もアプリへ転送するため、
    // 食べたキーの key-up も食べて確定キーなどが漏れないようにする
    std::bitset<256> pendingKeyUps_;
};
