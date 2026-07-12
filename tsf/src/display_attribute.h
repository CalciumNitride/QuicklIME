#pragma once

#include <windows.h>
#include <msctf.h>

// 入力中 (未確定) テキスト用の表示属性の GUID
// {4129DAA2-56F7-4A6F-8047-8BB4BD59931C}
extern const GUID kInputDisplayAttributeGuid;

// 変換対象の文節用の表示属性の GUID
// {FE4A53BB-970F-4850-80DC-59CE2EC5494E}
extern const GUID kTargetDisplayAttributeGuid;

// 提供する表示属性の数 (入力中 / 変換対象文節)
constexpr ULONG kDisplayAttributeCount = 2;

// index (0 = 入力中, 1 = 変換対象文節) から表示属性オブジェクトを作る。
// 範囲外は nullptr
ITfDisplayAttributeInfo* CreateDisplayAttributeInfo(ULONG index);

// GUID から表示属性オブジェクトを作る。未知の GUID は nullptr
ITfDisplayAttributeInfo* CreateDisplayAttributeInfoForGuid(REFGUID guid);

// 表示属性1件を列挙対象ごとに返す enumerator
class EnumDisplayAttributeInfo : public IEnumTfDisplayAttributeInfo {
public:
    EnumDisplayAttributeInfo();

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    // IEnumTfDisplayAttributeInfo
    STDMETHODIMP Clone(IEnumTfDisplayAttributeInfo** enumInfo) override;
    STDMETHODIMP Next(ULONG count, ITfDisplayAttributeInfo** info, ULONG* fetched) override;
    STDMETHODIMP Reset() override;
    STDMETHODIMP Skip(ULONG count) override;

private:
    virtual ~EnumDisplayAttributeInfo();

    LONG refCount_;
    ULONG index_; // 次に返す要素
};
