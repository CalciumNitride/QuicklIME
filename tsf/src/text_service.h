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
// composition (下線付き未確定文字列) を管理し、スペースで変換候補を
// 候補ウィンドウに表示する。候補は暫定で「カタカナ / ひらがな」のみ
// (フェーズ4で Rust エンジンによるかな漢字変換候補に差し替える)。
class TextService : public ITfTextInputProcessorEx,
                    public ITfThreadMgrEventSink,
                    public ITfKeyEventSink,
                    public ITfCompositionSink,
                    public ITfCompartmentEventSink,
                    public ITfDisplayAttributeProvider {
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

private:
    // 言語バー項目からオン/オフ切替・ツール起動を呼ぶ
    friend class LangBarButton;

    ~TextService();

    // このキー入力を IME が処理する (アプリに渡さない) かどうか
    // (context は direct 方式の後置再変換の判定で選択テキストを読むのに使う)
    bool IsKeyEaten(ITfContext* context, WPARAM wparam) const;
    // 押されているキーが設定の変換キー (key.convert: 無修飾 VK_CONVERT または
    // Ctrl+Space) か。Shift の併用は問わない (Shift+変換キー = 前候補)
    bool IsConvertKey(WPARAM wparam) const;

    // 設定ファイルの変更を確認し、変わっていれば反映する
    // (フォーカス切替・IMEオンなどの軽いタイミングで呼ぶ)
    void RefreshConfig();

    // ---- IMEオン/オフ (OPENCLOSE compartment) ----
    // オン/オフ状態を保持する compartment (呼び出し側で Release する。失敗時 nullptr)
    ITfCompartment* OpenCloseCompartment() const;
    bool IsKeyboardOpen() const;
    void SetKeyboardOpen(bool open);

    // 食べたキーを状態機械に従って処理する (方式で振り分け、処理後に昇格した
    // composition の降格を判定する)
    HRESULT HandleKey(ITfContext* context, WPARAM wparam);
    // composition 方式 (および昇格した composition) のキー処理
    HRESULT HandleKeyComposition(ITfContext* context, WPARAM wparam);

    // ファンクションキー変換 (F4-F10 に割り当てた機能) を実行する。割当の無い機能は何もしない
    HRESULT ApplyFunctionKey(ITfContext* context, KeyFunc func);

    // 同期 edit session の実行 (session の所有権を受け取り、実行後に解放する)
    HRESULT RequestSync(ITfContext* context, ITfEditSession* session, DWORD flags) const;

    HRESULT StartComposition(ITfContext* context);
    // composition のテキストを任意の文字列に差し替える
    HRESULT UpdateCompositionText(ITfContext* context, const std::wstring& text);
    // 確定 (commitText が空なら取消)。変換状態と候補ウィンドウも後始末する
    HRESULT EndComposition(ITfContext* context, const std::wstring& commitText);
    // composition を使わない直接挿入 (composition が無いときの記号入力用)
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

    // ---- 予測入力 (サジェスト) ----
    // 現在の読みで予測候補を引き直して表示する (条件を満たさなければ消す)
    HRESULT UpdatePrediction(ITfContext* context);
    // composition を Display() で更新し、続けて予測を引き直す (かな入力中の共通処理)
    HRESULT UpdateCompositionAndPredict(ITfContext* context);
    // 予測状態を破棄する (非変換中なら候補ウィンドウも隠す)
    void ClearPrediction();
    // サジェストの選択を動かす (+1: 未選択→先頭→...→末尾→先頭、-1: 先頭でさらに↑は解除)
    HRESULT MovePredictionSelection(ITfContext* context, int delta);
    // 数字キー 1〜9: サジェスト選択中に表示中ページ内の番号 (0始まり) で
    // 候補を直接選択する (対応する候補が無ければ何もしない)
    HRESULT SelectPredictionByNumber(ITfContext* context, size_t number);
    // サジェストの選択を解除してかな表示に戻す (候補ウィンドウは表示のまま)
    HRESULT DeselectPrediction(ITfContext* context);
    // 選択中のサジェスト候補で確定する (候補の完全な読みで学習も送る)
    HRESULT CommitPrediction(ITfContext* context);

    // ---- ライブ変換 (設定 live_conversion) ----
    // ライブ変換が設定で有効か
    bool LiveConversionEnabled() const { return config_.Get().liveConversion; }
    // 今の打鍵でライブ変換すべきか (Esc で止めた composition・英字モードは除く)
    bool LiveConversionActive() const {
        return LiveConversionEnabled() && !liveSuspended_ && !composer_.AsciiMode();
    }
    // かな全体を変換して composition に表示する (毎打鍵の本体)。
    // 変換できない場合はかな表示にフォールバックする
    HRESULT UpdateLiveConversion(ITfContext* context);
    // かな全体をライブ変換して liveSegments_ を更新し、表示文字列 (変換結果 + 末尾の
    // 未変換ローマ字) を返す。変換できない場合は liveSegments_ を空にしてかな表示を返す
    std::wstring LiveDisplayText();
    // ライブ変換の表示文字列 (各文節の先頭候補の連結)
    std::wstring LiveText() const;
    // ライブ変換の状態を破棄する (composition の終了時)
    void ClearLiveConversion();

    // ---- モードレス入力 (設定 modeless。判定の本体は RomajiComposer) ----
    // 無変換のまま確定する直前に、自動英字判定の判定ルール3 (末尾に残った
    // 子音1文字で英字と判定する) を適用する。表示が既に変換結果になっている経路
    // (候補選択中・サジェスト選択中・ライブ表示中) では読みを英字へ作り直せないため
    // 何もしない。読みが英字へ変わると表示も変わるため、文書 (direct) や composition の
    // 表示を同時に直せる経路からのみ呼ぶ (フォーカス移動などの run 終了では呼ばない)
    void ApplyModelessCommitRule();
    // composition / run が無いときに挿入するスペース。モードレス有効時のみ、直前の確定が
    // ASCII 英数字だけなら英文の途中とみなして半角にし、それ以外は設定 space に従う
    std::wstring StandaloneSpaceText() const;

    // 入力途中の内容を現在の状態のまま確定する (Enter と同じ処理)
    HRESULT CommitComposition(ITfContext* context);
    // 変換結果を確定する (エンジンへの学習送信 + composition 終了)
    HRESULT CommitConversion(ITfContext* context);
    // 変換確定の前half: エンジンへの学習送信のみ行い、確定文字列を返す
    // (edit session は発行せず、状態も変えない)
    std::wstring PrepareConversionCommit();
    // 文節伸縮で分割を直したまま確定したときの学習をエンジンへ送る
    void LearnResizedSegments();
    // 変換確定の状態後始末 (EndComposition の状態管理部分と同じ):
    // 確定アンドゥ情報を記憶し、変換・予測状態と composer_ を破棄する
    void FinishConversionState(const std::wstring& commitText);
    // 「確定 + 新しい composition の開始」を1つの edit session で行い、
    // composition_ を新しいものに差し替える。未確定文字列の表示は
    // 呼び出し側が UpdateCompositionAndPredict で別途行う。
    // 変換中に印字キーが来たときは必ずこれを使う (分割すると Word や
    // CUAS 経由のアプリで新しい composition が生き残らない)
    HRESULT RestartComposition(ITfContext* context, const std::wstring& commitText);
    // 確定アンドゥ (Ctrl+Backspace): 直前の確定文字列を削除して読みの
    // composition に戻す。キャレット直前が確定文字列と一致するときのみ働く
    HRESULT UndoCommit(ITfContext* context);
    // 単語登録 (既定 Ctrl+F7): 選択テキストを初期値にして登録ツールを起動する
    HRESULT LaunchWordRegister(ITfContext* context);
    // 設定ツール (quicklime-config.exe) の起動 (既定 Ctrl+F12)
    HRESULT LaunchConfigTool();
    // 変換中の表示 (選択候補の連結 + 現在文節の強調) を composition に反映する。
    // direct 方式では surface_ を置き換えて現在文節を選択状態にする
    HRESULT UpdateConvertingDisplay(ITfContext* context);
    // 未確定文字列の表示を text にする (composition のテキスト、direct 方式では surface_ の置換)
    HRESULT UpdateDisplayText(ITfContext* context, const std::wstring& text);
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

    // ---- 直接入力方式 (設定 input_style=direct。実装は text_service_direct.cpp) ----
    // 打鍵した文字を composition ではなく文書に直接入れ、IME が「自分が入れた文字列
    // (surface_) とその読み (composer_)」= run を覚えておいて毎打鍵で置き換える。
    // 文書の読み取り・置換ができないアプリでは文書単位で composition 方式に戻す
    // (docs/design/direct-input.md)
    enum class DirectCapability {
        Unknown,      // 未判定 (run を始めるとき、周辺テキストが読めるかを確かめる)
        Provisional,  // 直接挿入はできたが、別 session からの置換はまだ成功していない
        Capable,
        Incapable,    // この文書では composition 方式で動く
    };
    // direct 方式の1打鍵が composer_ に加える内容
    struct DirectKey {
        wchar_t romaji = 0;       // 0 以外ならローマ字として Push する (英字モード中は raw)
        std::wstring kana;        // romaji が 0 のとき PushKana するかな (英字モード中は raw)
        std::wstring raw;         // 打鍵文字そのもの
        bool enterAscii = false;  // Shift+英字: 英字モードに入る
    };
    // この打鍵を direct 方式で扱うか (設定が direct で、この文書が不可判定でなく、
    // composition が生きていない)
    bool UsingDirectStyle() const;
    bool InRun() const { return !surface_.empty(); }
    bool IsKeyEatenDirect(ITfContext* context, WPARAM wparam) const;
    HRESULT HandleKeyDirect(ITfContext* context, WPARAM wparam);
    // 食べずにアプリへ渡すキーのうち run を終えるもの (Enter・矢印・Ctrl 併用など) の
    // 状態処理。OnTestKeyDown / OnKeyDown の両方から呼ぶ (2回目以降は何もしない)
    void EndRunIfPassthroughKey(ITfContext* context, WPARAM wparam);
    // 打鍵を分類する。印字キー (英字・記号・数字・テンキー) でなければ false
    bool ClassifyDirectKey(WPARAM wparam, bool shifted, DirectKey* key) const;
    void PushDirectKey(const DirectKey& key);
    // 印字キー: run の開始または surface_ の置換 (不一致なら run を捨てて新規 run、
    // 非対応なら composition 方式へフォールバック)
    HRESULT TypeDirect(ITfContext* context, const DirectKey& key);
    HRESULT BackspaceDirect(ITfContext* context);
    HRESULT SpaceDirect(ITfContext* context, bool shifted);
    // 変換キー: run 中は変換開始 / 次候補 (Shift で前候補)、run が無ければ後置再変換
    HRESULT ConvertKeyDirect(ITfContext* context, bool shifted);
    // 後置再変換: 選択テキスト (ひらがな・カタカナ・ー のみ) を読みとして run を作り変換開始
    HRESULT ReconvertSelectionDirect(ITfContext* context);
    // 現在の選択テキストが後置再変換の対象なら true (対象なら textOut に入れる)
    bool ReadReconvertibleSelection(ITfContext* context, std::wstring* textOut) const;
    // 候補選択中・サジェスト選択中の確定: 選択を末尾に潰して run を終える
    HRESULT CommitRunDirect(ITfContext* context);
    // run の表示をかな表示 (ライブ変換有効ならライブ表示) にしてサジェストを引き直す
    // (composition 方式の UpdateCompositionAndPredict に相当)
    HRESULT UpdateRunAndPredict(ITfContext* context);
    // run の表示文字列: ライブ変換が働くならライブ表示、そうでなければ composer_.Display()
    // (liveSegments_ も更新する)
    std::wstring RunDisplayText();
    // run を終えるときの確定文字列 (候補選択・サジェスト選択中は surface_ そのもの、
    // ライブ表示中は変換結果 + 救済した未変換ローマ字、それ以外は composer_.Commit())
    std::wstring RunCommitText() const;
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
    // 置換が成功したときのフォールバック判定の更新。新規挿入 (newInsertion) は仮判定に
    // 留め、別 session からの置換が通ってはじめて可にする
    void NoteDirectSuccess(bool newInsertion);
    // run を終えて忘れる (既存の確定処理と同じ学習送信・文脈更新・確定アンドゥ用の記憶)
    void EndRun();
    // run の状態を学習せずに捨てる (文書と食い違った run の後始末)
    void DropRun();
    // 確定アンドゥの direct 版: 直前の run の surface を読みのかな表示に戻して run を再開する
    HRESULT UndoCommitDirect(ITfContext* context);
    // run を composition に昇格した結果
    enum class PromoteResult {
        Promoted,  // composition_ に移った (run の文書上の状態は捨てた)
        Dropped,   // 照合が文書と食い違った等で run を捨てた (文書は触らない)
        Refused,   // StartComposition が失敗・拒否された (run はそのまま。従来の direct 方式で変換する)
    };
    // 変換状態に入る直前に、run の範囲に composition を張る (文字列は変えない)。
    // 候補選択中の表示 (下線・現在文節の強調) を composition 方式の経路で行うため
    PromoteResult PromoteRun(ITfContext* context);
    // 昇格した composition が変換状態でなくなっていれば、その表示文字列のまま
    // composition を終えて direct 方式の run に戻す (学習はしない)
    void DemoteIfLeftConversion(ITfContext* context);

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
    CandidateWindow candidateWindow_;
    EngineClient engine_;                      // 変換エンジンへの named pipe クライアント
    ConfigLoader config_;                      // ユーザ設定 (config.tsv) のローダ

    // 予測入力 (かな入力中のサジェスト)。候補ウィンドウは変換中と排他で共用する
    std::vector<PredictionCandidate> predictions_;  // 予測候補 (空 = サジェスト非表示)
    int predictionIndex_;                           // 選択中の候補 index (-1 = 未選択)

    // ライブ変換。liveSegments_ が非空 ⇔ composition にライブ変換結果を表示中
    // (かな表示へのフォールバック時・変換中 (converting_)・英字モード中は必ず空)
    std::vector<ConversionSegment> liveSegments_;
    bool liveSuspended_;  // Esc でこの composition 中はライブ変換を止めた

    // direct 方式の run: 現在文書に入っている、この run 由来の文字列 (空 = run なし)
    std::wstring surface_;
    // 文書の選択開始が surface_ の何文字目にあるか (候補選択中は現在文節の先頭、
    // それ以外は末尾)。置換 session の expected 照合の基点に使う
    size_t surfaceCaret_;
    // 文書の選択の長さ (候補選択中は現在文節の長さ、それ以外は 0)
    size_t surfaceSelectLength_;
    DirectCapability directCapable_;  // フォーカス中の文書で direct 方式が使えるか
    // composition_ が run から昇格したもの (変換状態を抜けたら run に戻す) か。
    // 候補選択中の印字キーで確定して始まった次の composition にも引き継ぐ
    bool promoted_;

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
