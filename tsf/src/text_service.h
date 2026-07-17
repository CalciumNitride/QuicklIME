#pragma once

#include <windows.h>
#include <msctf.h>

#include <bitset>
#include <string>
#include <vector>

#include "candidate_window.h"
#include "engine_client.h"
#include "romaji.h"

// TSF テキストサービス本体。
// composition (下線付き未確定文字列) を管理し、スペースで変換候補を
// 候補ウィンドウに表示する。候補は暫定で「カタカナ / ひらがな」のみ
// (フェーズ4で Rust エンジンによるかな漢字変換候補に差し替える)。
class TextService : public ITfTextInputProcessorEx,
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
    ~TextService();

    // このキー入力を IME が処理する (アプリに渡さない) かどうか
    bool IsKeyEaten(WPARAM wparam) const;

    // ---- IMEオン/オフ (OPENCLOSE compartment) ----
    // オン/オフ状態を保持する compartment (呼び出し側で Release する。失敗時 nullptr)
    ITfCompartment* OpenCloseCompartment() const;
    bool IsKeyboardOpen() const;
    void SetKeyboardOpen(bool open);

    // 食べたキーを状態機械に従って処理する
    HRESULT HandleKey(ITfContext* context, WPARAM wparam);

    // 同期 edit session の実行 (session の所有権を受け取り、実行後に解放する)
    HRESULT RequestSync(ITfContext* context, ITfEditSession* session, DWORD flags);

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
    HRESULT CycleCandidate(ITfContext* context, int delta);
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
    // サジェストの選択を解除してかな表示に戻す (候補ウィンドウは表示のまま)
    HRESULT DeselectPrediction(ITfContext* context);
    // 選択中のサジェスト候補で確定する (候補の完全な読みで学習も送る)
    HRESULT CommitPrediction(ITfContext* context);

    // 入力途中の内容を現在の状態のまま確定する (Enter と同じ処理)
    HRESULT CommitComposition(ITfContext* context);
    // 変換結果を確定する (エンジンへの学習送信 + composition 終了)
    HRESULT CommitConversion(ITfContext* context);
    // 確定アンドゥ (Ctrl+Backspace): 直前の確定文字列を削除して読みの
    // composition に戻す。キャレット直前が確定文字列と一致するときのみ働く
    HRESULT UndoCommit(ITfContext* context);
    // 単語登録 (Ctrl+F7): 選択テキストを初期値にして登録ツールを起動する
    HRESULT LaunchWordRegister(ITfContext* context);
    // 変換中の表示 (選択候補の連結 + 現在文節の強調) を composition に反映する
    HRESULT UpdateConvertingDisplay(ITfContext* context);
    // 現在の選択に基づく確定文字列 (全文節の選択候補の連結)
    std::wstring ConvertedText() const;
    // 現在文節の候補一覧で候補ウィンドウを表示する
    void ShowCandidateWindow(ITfContext* context);
    // 候補ウィンドウの表示位置 (composition の矩形。取れなければキャレット/マウス位置)
    RECT CandidateAnchor(ITfContext* context);
    // 変換状態を破棄する (composition は触らない)
    void ClearConversion();

    bool Composing() const { return composition_ != nullptr; }

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
    CandidateWindow candidateWindow_;
    EngineClient engine_;                      // 変換エンジンへの named pipe クライアント

    // 予測入力 (かな入力中のサジェスト)。候補ウィンドウは変換中と排他で共用する
    std::vector<PredictionCandidate> predictions_;  // 予測候補 (空 = サジェスト非表示)
    int predictionIndex_;                           // 選択中の候補 index (-1 = 未選択)

    std::wstring lastCommitText_;   // 直前に確定した文字列 (確定アンドゥ用。使うと消える)
    RomajiComposer lastComposer_;   // 直前の確定時点のコンポーザ (読みと打鍵列の復元用)

    DWORD openCloseCookie_;         // OPENCLOSE compartment sink の cookie
    // key-down を食べたキー。端末エミュレータは key-up もアプリへ転送するため、
    // 食べたキーの key-up も食べて確定キーなどが漏れないようにする
    std::bitset<256> pendingKeyUps_;
};
