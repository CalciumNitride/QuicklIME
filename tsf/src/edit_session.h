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
// UndoCommitEditSession と同じ ShiftStart(負方向) + GetText のパターンを
// 読み取り専用で流用する)。読み取れた場合のみ precedingReadOkOut を true にする
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

// 確定アンドゥ用: キャレット直前のテキストが expectedText と一致する場合のみ、
// その範囲を覆う composition を開始して newText (確定前の読み) に置き換える
// (一致しなければ何もしない)。成功可否を succeededOut に返す。
// 削除 → composition 開始 → 表示を別々の edit session に分けると、
// RestartCompositionEditSession と同じ理由で Word 等では復元できない
class UndoCommitEditSession : public EditSessionBase {
public:
    UndoCommitEditSession(ITfContext* context, std::wstring expectedText,
                          ITfCompositionSink* sink, std::wstring newText,
                          TfGuidAtom displayAttribute, ITfComposition** compositionOut,
                          bool* succeededOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expectedText_;
    ITfCompositionSink* sink_;         // 呼び出し元 (TextService) が所有
    std::wstring newText_;
    TfGuidAtom displayAttribute_;
    ITfComposition** compositionOut_;  // 開始した composition の受け取り先
    bool* succeededOut_;
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
