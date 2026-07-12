#pragma once

#include <windows.h>
#include <msctf.h>

#include <string>

#include "romaji.h"

// TSF テキストサービス本体。
// フェーズ1では composition (未確定文字列の下線表示) を持たず、
// ローマ字からかなが確定した時点でカーソル位置へ直接挿入する最小実装。
class TextService : public ITfTextInputProcessorEx, public ITfKeyEventSink {
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

private:
    ~TextService();

    // このキー入力を IME が処理する (アプリに渡さない) かどうか
    bool IsKeyEaten(WPARAM wparam) const;

    // 確定文字列をカーソル位置へ挿入する
    HRESULT InsertText(ITfContext* context, const std::wstring& text);

    LONG refCount_;
    ITfThreadMgr* threadMgr_;
    TfClientId clientId_;
    RomajiBuffer romaji_;
};
