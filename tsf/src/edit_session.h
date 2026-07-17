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

// カーソル位置で composition を開始する
class StartCompositionEditSession : public EditSessionBase {
public:
    StartCompositionEditSession(ITfContext* context, ITfCompositionSink* sink,
                                ITfComposition** compositionOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    ITfCompositionSink* sink_;         // 呼び出し元 (TextService) が所有
    ITfComposition** compositionOut_;  // 開始した composition の受け取り先
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

// 確定アンドゥ用: キャレット直前のテキストが expectedText と一致する場合のみ
// それを削除する (一致しなければ何もしない)。成功可否を succeededOut に返す
class UndoCommitEditSession : public EditSessionBase {
public:
    UndoCommitEditSession(ITfContext* context, std::wstring expectedText, bool* succeededOut);
    STDMETHODIMP DoEditSession(TfEditCookie ec) override;

private:
    std::wstring expectedText_;
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
