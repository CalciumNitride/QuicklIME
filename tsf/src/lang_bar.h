#pragma once

#include <windows.h>
#include <msctf.h>
#include <ctffunc.h> // GUID_LBI_INPUTMODE
#include <olectl.h>  // CONNECT_E_*

class TextService;

// タスクバーの IME アイコンに対応する言語バー項目 (GUID_LBI_INPUTMODE)。
// Windows 11 では入力インジケータのアイコンとして表示され、
// 左クリックで IME オン/オフのトグル、右クリックでメニュー (設定・単語登録)
// を表示する。CorvusSKK の CLangBarItemButton を参考にした実装
class LangBarButton : public ITfLangBarItemButton, public ITfSource {
public:
    // service は AddRef して保持する。ActivateEx で ITfLangBarItemMgr::AddItem し、
    // Deactivate で RemoveItem + Release して循環参照を断つ
    explicit LangBarButton(TextService* service);

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    // ITfLangBarItem
    STDMETHODIMP GetInfo(TF_LANGBARITEMINFO* info) override;
    STDMETHODIMP GetStatus(DWORD* status) override;
    STDMETHODIMP Show(BOOL show) override;
    STDMETHODIMP GetTooltipString(BSTR* tooltip) override;

    // ITfLangBarItemButton
    STDMETHODIMP OnClick(TfLBIClick click, POINT pt, const RECT* area) override;
    STDMETHODIMP InitMenu(ITfMenu* menu) override;
    STDMETHODIMP OnMenuSelect(UINT id) override;
    STDMETHODIMP GetIcon(HICON* icon) override;
    STDMETHODIMP GetText(BSTR* text) override;

    // ITfSource
    STDMETHODIMP AdviseSink(REFIID riid, IUnknown* unknown, DWORD* cookie) override;
    STDMETHODIMP UnadviseSink(DWORD cookie) override;

private:
    ~LangBarButton();

    LONG refCount_;
    TextService* service_;      // 所有元のテキストサービス (AddRef して保持)
    ITfLangBarItemSink* sink_;  // 言語バーからの更新通知先 (AdviseSink で設定)
};
