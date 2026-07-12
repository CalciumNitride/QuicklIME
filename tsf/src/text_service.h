#pragma once

#include <windows.h>
#include <msctf.h>

#include <string>

#include "romaji.h"

// TSF テキストサービス本体。
// フェーズ2a: composition (下線付き未確定文字列) を管理し、
// Enter で確定 / Esc で取消 / Backspace で編集できる。
// かな漢字変換 (スペース) と候補ウィンドウはフェーズ2bで追加する。
class TextService : public ITfTextInputProcessorEx,
                    public ITfKeyEventSink,
                    public ITfCompositionSink,
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

    // ITfDisplayAttributeProvider
    STDMETHODIMP EnumDisplayAttributeInfo(IEnumTfDisplayAttributeInfo** enumInfo) override;
    STDMETHODIMP GetDisplayAttributeInfo(REFGUID guid, ITfDisplayAttributeInfo** info) override;

private:
    ~TextService();

    // このキー入力を IME が処理する (アプリに渡さない) かどうか
    bool IsKeyEaten(WPARAM wparam) const;

    // 食べたキーを状態機械に従って処理する
    HRESULT HandleKey(ITfContext* context, WPARAM wparam);

    // 同期 edit session の実行 (session の所有権を受け取り、実行後に解放する)
    HRESULT RequestSync(ITfContext* context, ITfEditSession* session);

    HRESULT StartComposition(ITfContext* context);
    HRESULT UpdateComposition(ITfContext* context);
    // 確定 (commitText が空なら取消)
    HRESULT EndComposition(ITfContext* context, const std::wstring& commitText);
    // composition を使わない直接挿入 (composition が無いときの記号入力用)
    HRESULT InsertText(ITfContext* context, const std::wstring& text);

    bool Composing() const { return composition_ != nullptr; }

    LONG refCount_;
    ITfThreadMgr* threadMgr_;
    TfClientId clientId_;
    ITfComposition* composition_;   // 進行中の composition (無ければ nullptr)
    RomajiComposer composer_;
    TfGuidAtom inputAttribute_;     // 未確定文字列に付ける表示属性の atom
};
