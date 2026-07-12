#pragma once

#include <windows.h>
#include <msctf.h>

// 入力中 (未確定) テキスト用の表示属性の GUID
// {4129DAA2-56F7-4A6F-8047-8BB4BD59931C}
extern const GUID kInputDisplayAttributeGuid;

// 入力中テキストの表示属性 (点線下線) を表すオブジェクト
class InputDisplayAttributeInfo : public ITfDisplayAttributeInfo {
public:
    InputDisplayAttributeInfo();

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
    STDMETHODIMP_(ULONG) AddRef() override;
    STDMETHODIMP_(ULONG) Release() override;

    // ITfDisplayAttributeInfo
    STDMETHODIMP GetGUID(GUID* guid) override;
    STDMETHODIMP GetDescription(BSTR* description) override;
    STDMETHODIMP GetAttributeInfo(TF_DISPLAYATTRIBUTE* attribute) override;
    STDMETHODIMP SetAttributeInfo(const TF_DISPLAYATTRIBUTE* attribute) override;
    STDMETHODIMP Reset() override;

private:
    virtual ~InputDisplayAttributeInfo();

    LONG refCount_;
};

// InputDisplayAttributeInfo 1件だけを列挙する enumerator
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
    ULONG index_; // 次に返す要素 (0 または 1)
};
