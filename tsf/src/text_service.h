#pragma once

#include <windows.h>
#include <msctf.h>

#include <bitset>
#include <memory>
#include <string>
#include <vector>

#include "config.h"
#include "engine_client.h"
#include "romaji.h"
#include "suggest_panel.h"

class LangBarButton;
enum class ReplaceRunResult;
struct FocusInputScope;

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
// 打鍵した読みを run として IME が持ち、文書の composition (composition 入力) か
// 候補パネルの入力行 (パネル保持入力) に出す。採用・確定で文書へ入れ、変換キーでは
// composition で候補を選ぶ (docs/design/input-buffer-modes.md)。
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

    // ITfTextEditSink (キャレット移動で後続語の予測を消す)
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
    // ホストの exe 名が改行キーの対象アプリ (設定 newline_apps) にあるか
    bool IsNewlineApp() const;
    // ホストの exe 名に対する起動時の IME の状態 (設定 ime_start_apps)。一覧にあれば true で
    // open にオンかを入れる。無ければ false (自動)
    bool FindImeStartState(bool* open) const;
    // ホストの exe 名が学習を記録しないアプリ (設定 no_learning_apps) にあるか
    bool IsNoLearningApp() const;

    // 設定ファイルの変更を確認し、変わっていれば反映する
    // (フォーカス切替・IMEオンなどの軽いタイミングで呼ぶ)
    void RefreshConfig();

    // ---- IMEオン/オフ (OPENCLOSE compartment) ----
    // オン/オフ状態を保持する compartment (呼び出し側で Release する。失敗時 nullptr)
    ITfCompartment* OpenCloseCompartment() const;
    bool IsKeyboardOpen() const;
    void SetKeyboardOpen(bool open);
    // 診断ログ用の OPENCLOSE の値 ("0" / "1" / "未設定" / "取得失敗")
    std::wstring OpenCloseLogText() const;
    // プロセスで最初に文書へフォーカスしたときだけ、起動時の IME の状態 (ime_start_apps) を
    // 適用し、前後の OPENCLOSE の値を診断ログに出す。where はログ用の呼び出し元
    void ApplyImeStartOnFirstFocus(const wchar_t* where);
    // プロセスで最初の文書フォーカスから一定時間だけ、OPENCLOSE の変更を診断ログに出す
    // (適用した後でシステムが値を書き換えるかを確かめるため)
    void LogImeStartChange();
    // フォーカス中の文書でキー入力が無効か (フォーカス中の文書・最上位 context が無い、または
    // context の KEYBOARD_DISABLED・EMPTYCONTEXT compartment が立っている。パスワード欄など)
    bool IsKeyboardDisabled() const;
    // キー入力が無効な文書でキーを受けたとき、残っている run を、学習・LLM の履歴・確定文脈に
    // 残さずに捨てる
    void DiscardRunWhileDisabled();
    // フォーカスが移った文書の無効化の状態と InputScope (inputScopeLog) を診断ログに出す
    void LogFocusDocument(ITfDocumentMgr* focus, const std::wstring& inputScopeLog);
    // フォーカスが移った文書の InputScope を読み取りの edit session で取りに行き、
    // focusInputScope_ に入れる (同期で走らなければ走った時点で入れる)。戻り値は診断ログ用
    std::wstring RequestFocusInputScope(ITfDocumentMgr* focus);
    // フォーカス中の文書が半角英数字を前提にした InputScope の欄で、キーを食べずに素通しにするか
    // (設定 input_scope_passthrough)。未取得なら context から同期で読み、読めなければ素通しにしない
    bool IsInputScopePassthrough(ITfContext* context) const;

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
    // 変換を取り消して変換前の読みに戻す (表示は直後の DemoteIfLeftConversion が run に戻して行う)
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
        // 英字候補 (今 run を終えたら英字になる形) または英字区間の大文字小文字の補正候補。
        // どちらも barAscii_ の確定文字列 (英字モード中は前者を作らないので両方は並ばない)
        Ascii,
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
    // バーの候補を作る読み (区間分割した run は日本語区間、それ以外は確定済みかな全体)
    std::wstring BarReading() const;
    // 入力全体の候補 (CONVNBEST の並び) と予測候補からバーの候補を組み立てる
    std::vector<BarCandidate> BuildBarItems(const std::wstring& kana,
                                            const std::vector<SentenceCandidate>& sentences,
                                            const std::vector<PredictionCandidate>& predictions) const;
    // items を caret の行に固定の x 位置 (設定 candidate_position center なら画面中央) で
    // 表示する。収まった候補だけを選択の対象にする。caret が null ならキャレットの矩形が
    // 取れなかったことを表す (画面中央のときだけ出せる)。1件も出せなければ false (何も変えない)
    bool ShowBarItems(const RECT* caret, std::vector<BarCandidate> items);
    // バーの候補と選択を破棄し、パネルを隠す
    void ClearBar();
    // バーの候補と選択を破棄する (パネルの表示は変えない)
    void ResetBarState();
    // バーの候補を破棄する。入力行を出している run の間は、パネルを入力行だけにして残す
    void ClearBarKeepingInput(ITfContext* context);

    // ---- サジェストパネル (docs/design/suggest-panel.md) ----
    // パネルに入力行を出す設定か (candidate_bar 1 かつ、panel_input 1 かテーマの input_row always。
    // パネル保持入力の run は設定によらず出す)
    bool PanelInputActive() const;
    // 入力行を出している状態か (入力行を出す設定で、候補選択中でない run の間)
    bool PanelInputShown() const;
    // 入力行の内容 (run の確定したかなと未完成のローマ字、run が無ければ後続語の予測の前文脈)。
    // 入力行を出さないときは false
    bool PanelInputLine(SuggestPanel::InputLine* line) const;
    // labels と入力行をパネルに出し、表示した候補の数を返す。caret が null ならキャレットの
    // 矩形が取れなかったことを表す
    size_t ShowPanel(const RECT* caret, const std::vector<SuggestPanel::Item>& labels,
                     size_t selection);
    // 表示中の候補と今の入力行でパネルを出し直す (入力行を出している run の間に呼ぶ)
    void RefreshRunPanel(ITfContext* context);
    // バーの選択を動かす (+1: 未選択→先頭→...→末尾→先頭、-1: 先頭でさらに↑は解除)。
    // 文書は書き換えない
    HRESULT MoveBarSelection(int delta);
    // 数字キー 1〜9: バー選択中に番号 (0始まり) の候補を選ぶ (無ければ何もしない)
    HRESULT SelectBarByNumber(size_t number);
    // バーの選択を解除する (バーは表示のまま)
    HRESULT DeselectBar();
    // サジェスト選択中の ForgetLearning: 選んでいる候補の学習をエンジンから消し、消えたものが
    // あれば候補を作り直して同じ番号 (無ければ末尾) を選ぶ。文書は書き換えない
    // (docs/design/forget-learning.md)
    HRESULT ForgetBarLearning(ITfContext* context);
    // バーの index の候補を採用する。採用部分 (run を終えるときは残りの読みと suffix も) を
    // 文書に入れ、残りの読みで run を続ける。followKey があれば、採用の後にその打鍵を入れる
    HRESULT AdoptBarItem(ITfContext* context, size_t index, BarAdopt mode,
                         const std::wstring& suffix, const DirectKey* followKey);
    // 英字候補の採用: 読みを英字候補の複製に置き換える。run を終えるとき (Space・確定キー・
    // Enter) は suffix を付けて文書に入れ、印字キーは英字区間に続ける
    HRESULT AdoptAsciiCandidate(ITfContext* context, BarAdopt mode, const std::wstring& suffix,
                                const DirectKey* followKey);
    // 採用を文書に入れた後の状態処理 (学習・文脈・読みの切り離し・run の終了または
    // 継続)。updateBar なら run を続けるときにバーを作り直す
    void FinishBarAdoption(ITfContext* context, bool updateBar);
    // 確定キー (バー未選択): アプリへ渡すキーと同じ救済を通して、見えているとおりに run を終える
    // (run 終了時の判定 (ルール3・6・7) も全体変換も適用しない)
    HRESULT CommitRunKey(ITfContext* context);

    // ---- 読みなしの後続語予測 (docs/design/next-word-prediction.md) ----
    // run の無いときに、今の前文脈の後続語の予測を候補バーに出す (設定・前文脈・候補・接続の
    // どれかが欠ければ何もしない)
    void ShowNextWordBar(ITfContext* context);
    // 確定キーの処理の後に呼ぶ: run が終わっていれば後続語の予測を出す
    void ShowNextWordAfterCommit(ITfContext* context);
    // 後続語の予測を出していて、その候補を選んでいるか
    bool NextWordSelected() const;
    // 打鍵の前処理: 後続語の予測を出しているとき、予測のまま扱うキー (未選択なら選択の機能、
    // 選択中なら選択・採用・Esc のキー) でなければ予測を消す (そのキーは今どおり処理する)
    void DismissNextWordForKey(WPARAM wparam);
    // 後続語の予測の選択を動かす (未選択から delta > 0 で先頭、delta < 0 で末尾を選ぶ)
    HRESULT MoveNextWordSelection(int delta);
    // 選択中の後続語の予測を、suffix を付けてキャレット位置に追記する (既存の文字列は書き換えない)。
    // 確定キー (CommitKey) なら続けて次の語の予測を出し、followKey があればその打鍵で run を始める
    HRESULT AdoptNextWord(ITfContext* context, BarAdopt mode, const std::wstring& suffix,
                          const DirectKey* followKey);
    // 候補バーの位置の基準 (run の composition があればその矩形、無ければ選択範囲の矩形、
    // 取れなければシステムキャレットの矩形)。どれも取れなければ false
    // source には位置の出所 (診断ログ用の説明) を返す
    bool CaretRect(ITfContext* context, RECT* rect, const wchar_t** source = nullptr);
    // 文字位置から得た rect の左上が文書のルートウィンドウの外なら、ルートウィンドウの中央の点
    // (幅・高さ 0) に置き換える。TSF が入力欄と無関係な位置 (モニタの隅に丸めた値など) を返す
    // アプリでも候補ウィンドウ等を見える場所に出すため。ルートウィンドウが得られなければ何もしない。
    // where は置き換えたときのデバッグログの見出し
    void FallbackIfOutsideWindow(ITfContext* context, RECT* rect, const wchar_t* where) const;

    // ---- LLM による並べ替え (docs/design/llm-rerank.md) ----
    // run を始めるときに LLM の左文脈を取る。読める文書は run の前の文字列を読み、それ以外は
    // IME が文書に入れた文字列の履歴を使う (設定 llm が無効なら何もしない)
    void CaptureLlmContext(ITfContext* context);
    // 呼び出し側が文書から読んだ、範囲の先頭より前の文字列を LLM の左文脈にする
    // (設定 llm が無効なら何もしない)
    void CaptureLlmContextFrom(const std::wstring& preceding);
    // IME が文書に入れた文字列を履歴 (読めると判定されていない文書の左文脈) に足す
    void AppendLlmHistory(const std::wstring& text);
    // 部分採用の後も run は続く。文書は読み直さず、run を始めたときの左文脈に採用した文字列を足す
    void ExtendLlmContext(const std::wstring& adopted);
    // バーを作り直した後に並べ替えを依頼し、結果の問い合わせ (タイマー) を始める
    void RequestRerank(const std::wstring& kana);
    // 結果の問い合わせを止める (依頼と受け取った結果は変換キーで使うので覚えておく)
    void StopRerankPolling();
    // 依頼と受け取った結果を捨てる
    void ResetRerank();
    // タイマー: 結果を問い合わせ、条件を満たせば並べ替えた候補でバーを組み直す
    void OnRerankTimer();
    // 変換キー: kana と前文脈が依頼したときのままで結果が出ていれば、並べ替えた候補を返す
    // (出ていなければ待たずに false)
    bool TakeRerankedCandidates(const std::wstring& kana,
                                std::vector<SentenceCandidate>* candidates);

    // ---- モードレス入力 (設定 modeless。判定の本体は RomajiComposer) ----
    // 候補バーの英字候補 (docs/design/modeless-commit-candidate.md): composer_ の複製に run 終了時の
    // 判定 (ルール3・6・7) を当て、英字になればその複製を result に入れて true。composer_ は変えない。
    // ルール3 が英単語辞書に一致しなかったとき (run 全体を英字にする扱い) は候補にしない
    bool BuildAsciiCandidate(RomajiComposer* result);
    // ルール6 の文脈: 直前の確定文字列の末尾 (スペースを除く) が ASCII 英字か
    // (数字は英語の文脈にしない)
    bool PrecedingCommitEndsWithLetter() const;
    // 自動英字判定の成立 (RomajiComposer::AsciiRequested) を、英単語辞書の問い合わせ
    // (ASCIISTART) の結果で決着させる。composer に打鍵を
    // 入れた直後・run 終了時の判定を適用した直後に呼ぶ
    void ResolveAsciiRequest(RomajiComposer* composer);
    // モードレス入力の英字区間で英小文字 c を打ったとき、英単語辞書の問い合わせ (ASCIIWORD) で
    // 英字区間を閉じられれば閉じて、c を日本語区間の打鍵として入れる (docs/design/mixed-conversion.md)。
    // 閉じなかったら false (c はまだ入れていない)
    bool CloseAsciiSection(wchar_t c);
    // 英字だけの text の大文字小文字を補正した正しい表記 (ASCIIFORM。docs/design/english-case.md)。
    // text と異なる表記があれば form に入れて true。同じ text への応答は覚えておき、
    // text が変わるまで問い合わせ直さない (未接続・エラーは覚えない)
    bool AsciiFormOf(const std::wstring& text, std::wstring* form);
    // 候補バーの補正候補: 開いている英字区間の文字列が正しい表記を持てば、composer_ の複製の
    // 英字区間をその表記に置き換えて result に入れて true。採用は英字候補と同じ経路
    bool BuildAsciiFormCandidate(RomajiComposer* result);
    // モードレス入力で、英字区間と日本語区間の両方を含む run か (変換キーで英字を含む読みを
    // 1文節に固定せずに変換する)
    bool MixedAsciiRun() const;
    // モードレス入力で、日本語区間を残して英字区間に分かれている run か
    // (モードレスが無効なら英字モードでも区間分割の扱いはしない)
    bool SplitAsciiRun() const;
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
    // 呼び出し側が別の edit session で行う。
    // 候補選択中に印字キーが来たときは必ずこれを使う (分割すると Word や
    // CUAS 経由のアプリで新しい composition が生き残らない)
    HRESULT RestartComposition(ITfContext* context, const std::wstring& commitText);
    // 確定アンドゥ (Ctrl+Backspace): 読める文書でキャレット直前が直前の確定文字列と一致するとき、
    // その文字列を消して読みの run を再開する
    HRESULT UndoCommit(ITfContext* context);
    // 確定アンドゥを今行えるか (読める文書で、キャレット直前が直前の確定文字列で終わる)
    bool CanUndoCommit(ITfContext* context) const;
    // 単語登録 (既定 Ctrl+F7): 選択テキストを初期値にして登録ツールを起動する
    HRESULT LaunchWordRegister(ITfContext* context);
    // 設定ツール (quicklime-config.exe) の起動 (既定 Ctrl+F12)
    HRESULT LaunchConfigTool();
    // 変換中の表示 (選択候補の連結 + 現在文節の強調) を composition に反映する
    HRESULT UpdateConvertingDisplay(ITfContext* context);
    // 現在の選択に基づく確定文字列 (全文節の選択候補の連結)
    std::wstring ConvertedText() const;
    // 現在文節の候補一覧と変換中の入力行をパネルに出す (docs/design/conversion-panel.md)
    void ShowCandidateWindow(ITfContext* context);
    // 候補一覧を閉じる (F7〜F10)。入力行を出す設定なら入力行だけのパネルを残す
    void HideCandidateList(ITfContext* context);
    // 候補選択中のパネルを出す。list が false なら候補一覧を出さず入力行だけにする
    void ShowConversionPanel(ITfContext* context, bool list);
    // 変換中の入力行の内容 (外したかなと文節)。入力行を出さないときは false
    bool ConversionInputLine(SuggestPanel::InputLine* line) const;
    // 候補一覧の表示位置 (composition の矩形。取れなければキャレット/マウス位置)
    RECT CandidateAnchor(ITfContext* context);
    // 変換状態を破棄する (composition もパネルも触らない。パネルは呼び出し側が
    // ClearBar か UpdateBar で描き直す)
    void ClearConversion();

    // ---- 文脈補正 (直前確定文節の読み・表記を覚えておき、次の変換に渡す) ----
    // 現在の文脈補正用コンテキストを取得する (未設定なら空)
    ConversionContext CurrentContext() const { return {contextReading_, contextSurface_}; }
    // 確定した文節の読み・表記を文脈として記憶する
    void SetCommitContext(const std::wstring& reading, const std::wstring& surface);
    // 文脈を破棄する (フォーカス移動・IMEオフ・確定アンドゥなど、直前の確定が
    // 次の変換の左文脈として使えなくなったとき)
    void ClearContext();

    // 候補選択中の composition があるか (run の composition は runComposition_ に別に持つ)
    bool Composing() const { return composition_ != nullptr; }

    // ---- run (実装は text_service_direct.cpp。docs/design/input-buffer-modes.md) ----
    // 打鍵した読み (composer_) = run を IME が持ち、採用・確定したときに文書へ入れる。
    // 読みの表示場所は run を始めた時点の設定で決め (RunMode)、変換するときは composition で
    // 候補を選ぶ
    // 1打鍵が composer_ に加える内容
    struct DirectKey {
        wchar_t romaji = 0;       // 0 以外ならローマ字として Push する (英字モード中は raw)
        std::wstring kana;        // romaji が 0 のとき PushKana するかな (英字モード中は raw)
        std::wstring raw;         // 打鍵文字そのもの
        bool enterAscii = false;  // Shift+英字: 英字モードに入る
    };
    // run の読みを出す場所 (設定 input_buffer)
    enum class RunMode {
        Composition,  // 文書の composition (runComposition_)
        Panel,        // 候補パネルの入力行だけ (文書には採用・確定まで何も入れない)
    };
    // 候補選択中の composition は run に数えない
    bool InRun() const { return !Composing() && !composer_.Empty(); }
    // 割当に一致しなかったキーを食べるか (候補選択中の composition 以外)
    bool IsKeyEatenDirect(WPARAM wparam) const;
    HRESULT HandleKeyDirect(ITfContext* context, WPARAM wparam);
    // ←→ (修飾キーなし) を ↑↓ と同じくサジェストの選択移動に使うか (横並びのテーマで
    // run のバーか後続語の予測を選択中。変換中は除く)
    bool ArrowMovesBarSelection(WPARAM wparam) const;
    // 食べずにアプリへ渡すキーのうち run を終えるもの (Enter・矢印・Ctrl 併用など) の
    // 状態処理。候補選択中・バー選択中の Enter は確定してから渡す
    // (EnterNeedsResend のときは食べるのでここでは扱わない)。
    // OnTestKeyDown / OnKeyDown の両方から呼ぶ (2回目以降は何もしない)
    void EndRunIfPassthroughKey(ITfContext* context, WPARAM wparam);
    // 今の run (昇格した composition を含む) がパネル保持入力か (run も候補選択中の
    // composition も無ければ false)
    bool PanelRunActive() const { return (InRun() || Composing()) && runMode_ == RunMode::Panel; }
    // run を終えてアプリへ渡すキーで、読めると判定されていない文書のため食べて文書に入れた後に
    // 送り直すものか (Enter・矢印・Tab (バーなし)・Home/End/Delete・PgUp/PgDn・Ctrl 併用)
    bool PanelKeyNeedsResend(ITfContext* context, WPARAM wparam) const;
    // PanelKeyNeedsResend のキー: 読み (Enter のバー選択中は選択中の候補) を文書に入れて run を
    // 終えてから、その打鍵をアプリへ送り直す
    HRESULT EndPanelRunAndResend(ITfContext* context, WPARAM wparam);
    // run の方式を今の設定で決める (run を始めるとき)
    void BeginRunMode();
    // run の読みの composition を張る (開始と文字列の設定は別の edit session)。張れなければ
    // false を返し、その run はパネル保持入力として続ける
    bool StartRunComposition(ITfContext* context);
    // run の composition を text で置き換えて終える (空なら取消)。edit session は composition の
    // 範囲から得た context で行う
    HRESULT EndRunComposition(const std::wstring& text);
    // run の composition を commitText で終え、続けて同じ edit session で新しい composition を
    // 張る (先頭文節の採用・採用に続く打鍵)。新しい composition を張れなければ、その run は
    // パネル保持入力として続ける
    HRESULT RestartRunComposition(ITfContext* context, const std::wstring& commitText);
    // run の読みの表示 (composition の文字列) を composer_ に合わせる。パネル保持入力は何もしない
    // (入力行は UpdateBar が描く)
    HRESULT UpdateRunDisplay(ITfContext* context);
    // text を run の位置に入れる (composition 入力は composition を text で終え、パネル保持入力は
    // キャレット位置に挿入する)。文書の分類に使う直前の挿入として覚える。LLM の履歴には
    // 足さない (呼び出し側の EndRun・FinishBarAdoption が足す)
    HRESULT WriteRunText(ITfContext* context, const std::wstring& text);
    // run の文字列 text を文書に入れて run を終える (学習・記憶は EndRun と同じ)。
    // 入れられなかったら run を捨てる
    HRESULT CommitRunText(ITfContext* context, const std::wstring& text);
    // 未判定の文書を、IME が直前に入れた文字列の読み戻しで分類する。キャレット直前が
    // その文字列で終わっていれば読める文書にし、そうでなければ未判定のまま残す。
    // 照合に使った文字列は忘れる (次に入れたときにまた判定する)
    void ClassifyByLastInsert(ITfContext* context);
    // 打鍵の前処理: run が無いとき、run なしで文書を読む機能 (変換キー・F6〜F10・確定アンドゥ) の
    // 打鍵なら、食べるかを決める前に文書を分類する
    void ClassifyBeforeIdleKey(ITfContext* context, WPARAM wparam);

    // Enter (Ctrl+M) を食べて確定し、確定の後に元の打鍵を送り直す (読めると判定されていない
    // 文書の候補選択中と後続語の予測の選択中)
    bool EnterNeedsResend() const;
    // 食べた Enter (Ctrl+M)・改行キーの確定を行い、確定の後で打鍵 vk をアプリへ送り直す
    HRESULT CommitAndResendEnter(ITfContext* context, WPARAM vk, bool ctrl, bool shifted);
    // 食べた打鍵を、確定の後でアプリへ送り直す
    void SendResendKey();
    // 改行キー: 入力中なら Enter と同じく確定してから、アプリへ Shift+Enter を送る
    HRESULT InsertNewline(ITfContext* context);
    // 打鍵を分類する。印字キー (英字・記号・数字・テンキー) でなければ false
    bool ClassifyDirectKey(WPARAM wparam, bool shifted, DirectKey* key) const;
    void PushDirectKey(const DirectKey& key);
    HRESULT SpaceDirect(ITfContext* context, bool shifted);
    // 変換 (KeyFunc::Convert): run 中は変換開始 / 次候補 (previous なら前候補)、
    // run が無ければ入力なしの対象 (ReadIdleTarget) の変換
    HRESULT ConvertKeyDirect(ITfContext* context, bool previous);
    // 入力なしの変換キー・F6〜F10 の対象 (docs/design/conversion-range.md、
    // docs/design/adopted-reconversion.md)
    struct IdleTarget {
        // 対象の文字列 (読みはひらがなに正規化して作る。記憶が対象のときはその表記)
        std::wstring text;
        // 対象の先頭より前の文書の文字列 (選択が無いときだけ読む)
        std::wstring before;
        bool selection = false;  // 選択範囲 (後置再変換) か、キャレット直前の文字列か
        bool adopted = false;    // 最新の記憶1件が対象か (読みの状態は記憶したものを使う)
    };
    // 選択があれば、選択開始から読み戻して照合できる、ひらがな・カタカナ・ー だけの選択
    // (後置再変換)。選択が空なら、読める文書でキャレット直前が有効な記憶で終わり、最新の記憶の
    // 表記がひらがな・ー だけでなければその1件、それ以外はキャレット直前のひらがな・ー の連続。
    // どちらも kMaxReconvertLength 文字まで。対象が無ければ false
    bool ReadIdleTarget(ITfContext* context, IdleTarget* target) const;
    // 入力なしの対象を読みとして run を作り、昇格して func (Convert か F6〜F10) の変換に入る
    HRESULT ConvertIdleTarget(ITfContext* context, KeyFunc func);
    // キャレット (選択の開始) より前の文書の文字列を読み、直前の inRange を除いた残りを before に
    // 返す。キャレットの直前が inRange で終わっていなければ false
    bool ReadTextBeforeCaret(ITfContext* context, const std::wstring& inRange,
                             std::wstring* before) const;
    // 昇格した直後に、変換の範囲の伸縮に使う状態を始める。縮めて外したかなは文書に書かず、
    // composition の先頭に残す
    void BeginRangeTracking();
    // 候補選択中の ←→ で変換の範囲を伸縮できるか (入力全体の候補選択で、英字を含まない run か
    // F6〜F10 の文字種変換中)
    bool RangeResizable() const;
    // 縮めて外し、composition の先頭に残しているかな (無ければ空)
    std::wstring RangeExcludedInComposition() const;
    // 範囲を変えた後の前文脈を決める (外したかな、または範囲を始めたときの前文脈)
    void ApplyRangeContext();
    // 変換の取消: composition の先頭に残した外したかなを読みに戻す (run 全体のかなに戻る)
    void RestoreExcludedKana();
    // 範囲の右端を固定し、左端を1文字動かして変換をやり直す (delta < 0 で広げ、delta > 0 で
    // 縮める)。広げられるのは縮めて外したかなを戻すところまで。できないときは何もしない
    HRESULT ResizeConversionRange(ITfContext* context, int delta);

    // ---- 採用した文字列の記憶 (docs/design/adopted-reconversion.md) ----
    // run が文書に残した出力1件 (候補バーの採用・候補選択の確定・かなのままの run の終了)
    struct AdoptedText {
        std::wstring surface;  // 文書に入れた文字列 (文節の表記の連結)
        std::vector<std::pair<std::wstring, std::wstring>> segments;  // 文節ごとの (読み, 表記)
        RomajiComposer composer;  // 確定前の読みの状態 (Commit() が文節の読みの連結)
    };
    // 記憶を1件足す。文節の読み・表記の連結が composer・surface と食い違えば1文節にする。
    // 読みの合計が kMaxReconvertLength を超えた分は古いものから捨てる
    void RememberAdopted(const std::wstring& surface,
                         std::vector<std::pair<std::wstring, std::wstring>> segments,
                         const RomajiComposer& composer);
    // 候補選択の確定を記憶する。範囲を追跡していれば、範囲が覆っていた記憶を、範囲の前に残った
    // 記憶・縮めて外したかな (かなの1件)・確定した範囲に置き換える
    void RememberConversion(const std::vector<std::pair<std::wstring, std::wstring>>& committed);
    // 記憶の表記の長さの合計 (照合のために文書を読む長さ)
    size_t AdoptedSurfaceLength() const;
    // preceding (キャレット直前の文書の文字列) の末尾から、新しい記憶の表記が連続して一致する数
    size_t MatchAdopted(const std::wstring& preceding) const;
    // 照合で一致しなかった記憶と、それより古い記憶を捨てる
    void DropUnmatchedAdopted(const std::wstring& preceding);
    // 照合済みの記憶のうち、範囲の先頭 (preceding の rangeStart 文字目) でちょうど終わる連続した
    // 記憶を adopted_[*from, *to) に返す (無ければ空)。*to 以降は範囲が (一部でも) 覆う記憶
    void FindAdoptedChain(const std::wstring& preceding, size_t rangeStart, size_t* from,
                          size_t* to) const;
    // BeginRangeTracking の後に、範囲の前の記憶 adopted_[from, to) を範囲の状態に写す。
    // fromAdopted なら最新の記憶から始めた変換
    void BeginRangeAdopted(size_t from, size_t to, bool fromAdopted);
    // 記憶から始めた変換の範囲が、始めたときの記憶のままか
    bool RangeAdoptedIntact() const;
    // 記憶から始めた変換の候補の先頭に記憶の表記を置く (文書を変えない)
    void PlaceAdoptedCandidate();
    // バー選択中の確定 (選択中の候補を採用して run を終える)。バーを選んでいなければ
    // 見えているとおりの読みで run を終える
    HRESULT CommitRunDirect(ITfContext* context);
    // run の読みの表示を更新してバーを作り直す
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
    // 文書に text を入れた run を終えて忘れる (既存の確定処理と同じ学習送信・文脈更新・
    // 確定アンドゥ用の記憶)。text が空なら学習せずに捨てる
    void EndRun(const std::wstring& text);
    // run の状態を学習せずに捨てる。run の composition が残っていれば空で終える
    void DropRun();
    // run を composition に昇格した結果
    enum class PromoteResult {
        Promoted,  // composition_ に移った
        Dropped,   // 照合が文書と食い違った・入れ直しに失敗した等で run を捨てた
        Refused,   // StartComposition が失敗・拒否された (消した文字列は入れ直した)
    };
    // 入力なしの対象を変換するとき、文書の対象の文字列 text (選択開始が caretOffset 文字目、
    // 選択の長さ selectLength) を照合込みで消し、キャレット位置に空の composition を張る。
    // 未確定文字列は呼び出し元の表示更新 (StartConversion / ApplyFunctionKey) が設定する
    PromoteResult PromoteRun(ITfContext* context, const std::wstring& text, size_t caretOffset,
                             size_t selectLength);
    // 昇格した composition が変換状態でなくなっていれば、run に戻す (学習はしない)。
    // composition 入力はその composition を読みの表示に戻して run の composition にし、
    // パネル保持入力は composition を空で終えて読みを入力行に戻す
    void DemoteIfLeftConversion(ITfContext* context);
    // 文書が読み戻せると分かっているか (範囲が作れないのをキャレット移動とみなせるか)
    bool DocumentReadable() const;

    // ---- 文書の分類 ----
    // IME が直前に入れた文字列をキャレット直前から読み戻して分類する (ClassifyByLastInsert)
    enum class AppendDocument {
        Unknown,   // 未判定 (まだ読み戻しを試していない、または一致しなかった)
        Readable,  // 直前に入れた文字列を読み戻せた (置換で後置再変換・確定アンドゥができる)
    };
    void DebugLog(const std::wstring& message) const;
    // キー押下の診断ログ。state・match は処理前に取ったもの
    void LogKeyDown(const wchar_t* where, WPARAM wparam, KeyState state, const KeyMatch& match,
                    BOOL eaten, bool disabled) const;
    void LogKeyUp(const wchar_t* where, WPARAM wparam, BOOL eaten) const;
    void SetAppendDocument(AppendDocument document, const wchar_t* reason);
    // 印字キー: 読みを進め、読みの表示とバーを更新する (run が無ければ新しい run を始める)
    HRESULT TypeAppend(ITfContext* context, const DirectKey& key);
    HRESULT BackspaceAppend(ITfContext* context);
    HRESULT SpaceAppend(ITfContext* context, bool shifted);
    // 食べずにアプリへ渡すキー (Enter・矢印など) の前に、見えているとおりの読みで run を終える
    void EndAppendRunForPassthroughKey(ITfContext* context);
    // 変換キー・F4〜F10: composition 入力は run の composition で、パネル保持入力は空の
    // composition を張って候補選択に入る
    HRESULT BeginAppendConversion(ITfContext* context, KeyFunc func);
    // 食べなかったキーの後処理 (改行・送信をアプリへ渡したら LLM の履歴を捨てる)
    void NoteKeyForAppend(WPARAM wparam, bool eaten);
    static const wchar_t* AppendDocumentName(AppendDocument document);
    // フォーカス中の文書の最上位 context に ITfTextEditSink を付け替える
    void UpdateTextEditSink(ITfDocumentMgr* docMgr);

    LONG refCount_;
    ITfThreadMgr* threadMgr_;
    TfClientId clientId_;
    ITfComposition* composition_;   // 候補選択中の composition (無ければ nullptr)
    // composition 入力の run の読みを出している composition (無ければ nullptr)。変換に入ると
    // composition_ へ移し、変換を取り消すとここへ戻す
    ITfComposition* runComposition_;
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
    SuggestPanel suggestPanel_;                // 候補バー・変換の候補一覧と入力行
    // 変換の候補一覧の 1 ページの件数 (作業領域に収まる行数。数字キーの対応付けに使う)
    size_t conversionPageSize_;
    EngineClient engine_;                      // 変換エンジンへの named pipe クライアント
    ConfigLoader config_;                      // ユーザ設定 (config.tsv) のローダ
    PanelThemeLoader panelTheme_;              // サジェストパネルのテーマ (設定 panel_theme)

    // 候補バー
    std::vector<BarCandidate> barItems_;  // 表示中の候補 (空 = 非表示)
    std::vector<SuggestPanel::Item> barLabels_;  // 表示中の候補の表示 (barItems_ と同じ並び)
    std::wstring barKana_;                // 候補を作った確定済みかな
    int barIndex_;                        // 選択中の候補 index (-1 = 未選択)
    // バーの x 座標。run で最初に出した時点 (部分採用の後はその時点) の位置に固定する
    int barX_;
    bool barXFixed_;
    // 表示中のバーを組み立てた候補 (作業領域で切る前) と予測候補、表示した caret の矩形。
    // 並べ替えの結果でバーを組み直すときに使う (タイマーからは文書の矩形を取らない)
    std::vector<BarCandidate> barBuilt_;
    std::vector<PredictionCandidate> barPredictions_;
    RECT barCaret_;
    bool barCaretValid_;  // barCaret_ を取れたか (画面中央のときは取れなくても出す)
    // 英字候補・補正候補を作った読みの複製 (採用でこれに置き換える) と、作ったときの composer_ の表示
    bool barHasAscii_ = false;
    RomajiComposer barAscii_;
    std::wstring barAsciiSource_;
    // 最後に ASCIIFORM で問い合わせた文字列と応答の表記 (英字区間の打鍵ごとに問い合わせ直さない)
    std::wstring asciiFormText_;
    std::wstring asciiForm_;
    // 表示中のバーが run の無いときに出した後続語の予測か (候補は barItems_ に予測候補として持つ)
    bool nextWordBar_;

    // LLM による並べ替え。左文脈は run の先頭より前の文字列で、メモリ上だけで使う
    std::wstring llmRunContext_;   // この run の左文脈 (run を始めたときに取る)
    // IME が文書に入れた文字列の末尾 (読めると判定されていない文書の左文脈)
    std::wstring llmHistory_;
    unsigned long long rerankId_;  // 最新の依頼の ID (0 = 依頼なし)
    ULONGLONG rerankTick_;         // 依頼した時刻 (問い合わせの打ち切り用)
    std::wstring rerankKana_;      // 依頼したときの読みと前文脈
    ConversionContext rerankContext_;
    bool rerankDone_;              // 結果を受け取った
    std::vector<SentenceCandidate> rerankResult_;
    // 採用を文書に入れた後に行うこと
    struct BarAdoption {
        std::vector<LearnEntry> learn;
        std::wstring contextReading;   // 次の前文脈にする文節
        std::wstring contextSurface;
        size_t readingLength = 0;      // 読みの先頭から取り除く文字数
        size_t adoptedLength = 0;      // written のうち採用部分の長さ
        std::wstring written;          // 文書に入れる文字列
        bool endRun = false;
        RomajiComposer before;         // 採用前の読み (確定アンドゥの復元用)
        // 採用部分の文節ごとの (読み, 表記) (記憶に使う)
        std::vector<std::pair<std::wstring, std::wstring>> segments;
    };
    BarAdoption adoption_;

    // composition_ が run から昇格したもの (変換状態を抜けたら run に戻す) か。
    // 候補選択中の印字キーで確定して始まった次の composition にも引き継ぐ
    bool promoted_;
    // 今の run (昇格した composition を含む) の方式。run を始めた時点の設定で決め、
    // run の途中で設定が読み直されても変えない
    RunMode runMode_;
    // IME が直前に文書へ入れた文字列 (文書の分類用。照合したら忘れる)
    std::wstring lastInserted_;

    // 変換の範囲の伸縮 (docs/design/conversion-range.md)。昇格したときに始める
    bool rangeTracked_;                   // 伸縮できる状態か
    // 縮めて外したかな (composition の先頭に残す)。末尾 rangeShift_ 文字が外したかな
    std::wstring rangeBefore_;
    // 範囲の先頭が始めたときから何文字右へ動いたか (縮めた文字数)
    int rangeShift_;
    // 縮めるたびに積む、縮める直前の読みの状態。← で戻すときに外した文字の打鍵と
    // 英字区間の開始位置ごと戻すため。積んだ順に rangeBefore_ の末尾の文字と対応する
    std::vector<RomajiComposer> rangeStates_;
    ConversionContext rangeBaseContext_;  // 範囲を始めたときの前文脈
    // 範囲を変えたときにやり直す変換 (Convert は通常の変換、それ以外は F6〜F10)
    KeyFunc rangeFunc_;
    // 範囲の前の記憶の写し (古い順)。確定で adopted_ の範囲が覆っていた記憶をこれに置き換える
    std::vector<AdoptedText> rangeAdopted_;
    // 範囲を始めたときの adopted_ のうち、確定でそのまま残す記憶の数 (rangeAdopted_ はこの後ろ)
    size_t rangeAdoptedFrom_;
    bool rangeFromAdopted_;   // 最新の記憶から始めた変換か
    AdoptedText rangeOrigin_;       // 変換を始めた記憶 (rangeFromAdopted_ のとき)

    // 採用した文字列の記憶 (文書に入れた順。末尾が最新)。メモリ上だけに置き、
    // フォーカス移動・IME オフで捨てる
    std::vector<AdoptedText> adopted_;

    AppendDocument appendDocument_;          // フォーカス中の文書の分類
    // 確定の後にアプリへ送り直す打鍵 (0 なら無し) とその修飾キー
    // (EnterNeedsResend・PanelKeyNeedsResend のときのキーと、改行キーの Shift+Enter)
    WPARAM appendResendVk_;
    bool appendResendCtrl_;
    bool appendResendShift_;
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
    // ホストプロセスの exe 名 (改行キー・起動時の IME の対象アプリの判定用。プロセスごとに不変)
    std::wstring hostExeName_;
    // フォーカス中の文書の InputScope (docs/design/input-scope.md)。後から走る取得の
    // edit session と共有するため shared_ptr で持つ
    std::shared_ptr<FocusInputScope> focusInputScope_;
};
