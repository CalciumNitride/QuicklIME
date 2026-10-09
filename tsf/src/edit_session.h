#pragma once

#include <windows.h>
#include <msctf.h>

#include <string>

// edit session 共通の IUnknown 実装。
// 派生クラスは DoEditSession だけを実装する。
class EditSessionBase : public ITfEditSession {
public:
    explicit EditSessionBase(ITfContext* context);

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

protected:
    virtual ~EditSessionBase();

    ITfContext* context_;

private:
    LONG refCount_;
};

// カーソル位置へ文字列を直接挿入する (composition を使わない確定入力用)
class InsertTextEditSession : public EditSessionBase {
public:
    InsertTextEditSession(ITfContext* context, std::wstring text);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring text_;
};

// カーソル位置で composition を開始する。
// precedingLength > 0 のときは、開始前にキャレット直前の precedingLength 文字を
// 読み取って precedingTextOut に返す (文脈補正のハイブリッド照合用。
// ShiftStart(負方向) + GetText で読み取るだけで文書は変えない)。
// 読み取れた場合のみ precedingReadOkOut を true にする
// (GetText 非対応のアプリでは false のままになり、呼び出し側は内部履歴を信頼する)
class StartCompositionEditSession : public EditSessionBase {
public:
    StartCompositionEditSession(ITfContext* context, ITfCompositionSink* sink,
                                ITfComposition** compositionOut, ULONG precedingLength = 0,
                                std::wstring* precedingTextOut = nullptr,
                                bool* precedingReadOkOut = nullptr);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ITfCompositionSink* sink_;         // 呼び出し元 (TextService) が所有
    ITfComposition** compositionOut_;  // 開始した composition の受け取り先
    ULONG precedingLength_;
    std::wstring* precedingTextOut_;
    bool* precedingReadOkOut_;
};

// composition のテキストを差し替え、表示属性を適用し、キャレットを末尾へ移動する。
// targetLength > 0 のときは [targetStart, targetStart+targetLength) の部分範囲へ
// targetAttribute (変換対象文節の強調) を上書き適用する
class UpdateCompositionEditSession : public EditSessionBase {
public:
    UpdateCompositionEditSession(ITfContext* context, ITfComposition* composition,
                                 std::wstring text, TfGuidAtom displayAttribute,
                                 TfGuidAtom targetAttribute = TF_INVALID_GUIDATOM,
                                 LONG targetStart = 0, LONG targetLength = 0);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ~UpdateCompositionEditSession() override;

    ITfComposition* composition_;
    std::wstring text_;
    TfGuidAtom displayAttribute_;
    TfGuidAtom targetAttribute_;
    LONG targetStart_;
    LONG targetLength_;
};

// composition の画面上の矩形 (スクリーン座標) を取得する (候補ウィンドウの位置決め用)
class GetTextExtentEditSession : public EditSessionBase {
public:
    GetTextExtentEditSession(ITfContext* context, ITfComposition* composition, RECT* rectOut,
                             bool* succeededOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ~GetTextExtentEditSession() override;

    ITfComposition* composition_;
    RECT* rectOut_;
    bool* succeededOut_;
};

// 現在の選択テキストを取得する (単語登録ダイアログの初期値用)。
// 選択が無い・長すぎる場合は textOut を空のままにする
class GetSelectionTextEditSession : public EditSessionBase {
public:
    GetSelectionTextEditSession(ITfContext* context, std::wstring* textOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring* textOut_;
};

// composition を確定文字列で置き換えて終了する (空文字列なら取消)
class EndCompositionEditSession : public EditSessionBase {
public:
    EndCompositionEditSession(ITfContext* context, ITfComposition* composition,
                              std::wstring commitText);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ~EndCompositionEditSession() override;

    ITfComposition* composition_;
    std::wstring commitText_;
};

// 「確定 + 新しい composition の開始」を1つの edit session (1つのドキュメント
// ロック) 内で行う。変換中に印字キーが来たときの遷移用。
// 旧 composition を commitText で置き換えて終了し、続けて確定文字列の直後で
// 新しい composition を開始する。未確定文字列の表示は行わない (呼び出し側が
// 別の edit session で UpdateCompositionText を呼んで設定する)。
// EndComposition と StartComposition を1つの edit session にまとめるのは、
// 別々の edit session に分けるとロックの合間にホストが確定処理を進めてしまい、
// 2つ目の composition が生き残らないため (Word や CUAS 経由のアプリ)。
// 一方、テキストの設定まで同じ session で行うと、CUAS がテキスト設定の
// WM_IME_COMPOSITION を生成しないアプリ (WezTerm 等) で未確定文字列が
// 表示されない問題が起きるため、テキスト設定は別の session に分離する
class RestartCompositionEditSession : public EditSessionBase {
public:
    RestartCompositionEditSession(ITfContext* context, ITfComposition* oldComposition,
                                  std::wstring commitText, ITfCompositionSink* sink,
                                  ITfComposition** compositionOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ~RestartCompositionEditSession() override;

    ITfComposition* oldComposition_;
    std::wstring commitText_;
    ITfCompositionSink* sink_;         // 呼び出し元 (TextService) が所有
    ITfComposition** compositionOut_;  // 開始した composition の受け取り先
};

// direct 方式の置換結果
enum class ReplaceRunResult {
    Succeeded,
    Mismatch,     // キャレット直前のテキストが expected と一致しない (run は捨てる)
    Unsupported,  // 文書の読み取り・置換ができない
    // 置換しようとしたが run の範囲すら作れない (選択位置の前に文字が無い)。
    // 周辺テキストが読めない文書 (CUAS 経由など) とみなす
    Unreadable,
};

// direct 方式の run の挿入・置換を1つの edit session で行う。
// expected が空なら選択位置へ newText を挿入する (選択があればそれを置き換える)。
// expected が非空なら、選択開始から caretOffset 文字前〜
// (expected.size() - caretOffset) 文字後の範囲を expected と比較し、一致したときだけ
// newText に置き換える (範囲が全く作れなければ Unreadable、内容が違えば Mismatch、
// GetText 自体が失敗すれば Unsupported)。置換後は常に選択を末尾に潰す。
// Chromium 系は、潰れていない選択と異なる範囲の置換を正しく反映せず、文字列を変えた
// session 内での選択設定も捨てるため、選択の設定は SelectRunRangeEditSession で別に行う
class ReplaceRunEditSession : public EditSessionBase {
public:
    ReplaceRunEditSession(ITfContext* context, std::wstring expected, size_t caretOffset,
                          std::wstring newText, ReplaceRunResult* resultOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expected_;
    size_t caretOffset_;
    std::wstring newText_;
    ReplaceRunResult* resultOut_;
};

// direct 方式の run の文字列は変えずに選択だけを設定する。照合は ReplaceRunEditSession と
// 同じ (expected が空なら照合せず選択開始を run の先頭とみなす)。一致したら run 内の
// [selectOffset, selectOffset + selectLength) を選択する (長さ 0 なら selectOffset の
// 位置に潰す)。SetSelection が失敗したら Unsupported を返す
class SelectRunRangeEditSession : public EditSessionBase {
public:
    SelectRunRangeEditSession(ITfContext* context, std::wstring expected, size_t caretOffset,
                              size_t selectOffset, size_t selectLength,
                              ReplaceRunResult* resultOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expected_;
    size_t caretOffset_;
    size_t selectOffset_;
    size_t selectLength_;
    ReplaceRunResult* resultOut_;
};

// direct 方式の run を composition に昇格する。照合は ReplaceRunEditSession と同じで、
// 一致した run の範囲でそのまま composition を開始し (文字列は変えない)、入力中の
// 表示属性を付けて選択を末尾に潰す。照合結果を *matchOut に、開始した composition を
// *compositionOut に返す (照合が一致しても StartComposition が失敗・拒否されれば nullptr)
class PromoteRunEditSession : public EditSessionBase {
public:
    PromoteRunEditSession(ITfContext* context, std::wstring expected, size_t caretOffset,
                          ITfCompositionSink* sink, TfGuidAtom displayAttribute,
                          ITfComposition** compositionOut, ReplaceRunResult* matchOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expected_;
    size_t caretOffset_;
    ITfCompositionSink* sink_;         // 呼び出し元 (TextService) が所有
    TfGuidAtom displayAttribute_;
    ITfComposition** compositionOut_;  // 開始した composition の受け取り先
    ReplaceRunResult* matchOut_;
};

// 追記型入力の追記を1つの edit session で行う。書き込み位置は選択範囲。
// verify を立てると、書き込み位置の直前が expected (run の文字列) と一致するかを
// MatchRunRange と同じ方法で確かめて *matchOut に返す (立てなければ Succeeded)。
// 一致せず abortOnMismatch なら何も書かない。書くときは text をその位置に入れる
class AppendRunEditSession : public EditSessionBase {
public:
    AppendRunEditSession(ITfContext* context, std::wstring expected, bool verify,
                         bool abortOnMismatch, std::wstring text, ReplaceRunResult* matchOut,
                         bool* writtenOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expected_;
    bool verify_;
    bool abortOnMismatch_;
    std::wstring text_;
    ReplaceRunResult* matchOut_;
    bool* writtenOut_;
};

// 選択開始の直前が expected (選択開始が caretOffset 文字目) と一致するかを
// 読み取り専用で確かめる (追記型入力で文書を分類するための読み戻し)
class MatchRunEditSession : public EditSessionBase {
public:
    MatchRunEditSession(ITfContext* context, std::wstring expected, size_t caretOffset,
                        ReplaceRunResult* resultOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expected_;
    size_t caretOffset_;
    ReplaceRunResult* resultOut_;
};

// 選択開始より前の最大 maxLength 文字 (UTF-16 単位) を読み取り専用で読む (LLM の左文脈用)。
// 文書の先頭に近ければ短くなる。読めなければ textOut は空のまま
class GetPrecedingTextEditSession : public EditSessionBase {
public:
    GetPrecedingTextEditSession(ITfContext* context, ULONG maxLength, std::wstring* textOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ULONG maxLength_;
    std::wstring* textOut_;
};

// 現在の選択範囲の画面上の矩形 (スクリーン座標) を取得する (direct 方式の候補
// ウィンドウ・未完成ローマ字の小窓の位置決め用。composition が無いため選択範囲を基準にする)。
// SetText 直後は同じロック内でレイアウトが更新されていないアプリがあるため、
// GetTextExtentEditSession と同様に置換とは別の session で呼ぶ
class GetSelectionExtentEditSession : public EditSessionBase {
public:
    GetSelectionExtentEditSession(ITfContext* context, RECT* rectOut, bool* succeededOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    RECT* rectOut_;
    bool* succeededOut_;
};
