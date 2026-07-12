#include "display_attribute.h"

#include <new>

const GUID kInputDisplayAttributeGuid = {
    0x4129daa2, 0x56f7, 0x4a6f, {0x80, 0x47, 0x8b, 0xb4, 0xbd, 0x59, 0x93, 0x1c}};

// ---- InputDisplayAttributeInfo ----

InputDisplayAttributeInfo::InputDisplayAttributeInfo() : refCount_(1)
{
}

InputDisplayAttributeInfo::~InputDisplayAttributeInfo()
{
}

STDMETHODIMP InputDisplayAttributeInfo::QueryInterface(REFIID riid, void** ppv)
{
    if (ppv == nullptr) {
        return E_INVALIDARG;
    }
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_ITfDisplayAttributeInfo)) {
        *ppv = static_cast<ITfDisplayAttributeInfo*>(this);
        AddRef();
        return S_OK;
    }
    *ppv = nullptr;
    return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) InputDisplayAttributeInfo::AddRef()
{
    return InterlockedIncrement(&refCount_);
}

STDMETHODIMP_(ULONG) InputDisplayAttributeInfo::Release()
{
    LONG count = InterlockedDecrement(&refCount_);
    if (count == 0) {
        delete this;
    }
    return count;
}

STDMETHODIMP InputDisplayAttributeInfo::GetGUID(GUID* guid)
{
    if (guid == nullptr) {
        return E_INVALIDARG;
    }
    *guid = kInputDisplayAttributeGuid;
    return S_OK;
}

STDMETHODIMP InputDisplayAttributeInfo::GetDescription(BSTR* description)
{
    if (description == nullptr) {
        return E_INVALIDARG;
    }
    *description = SysAllocString(L"QuicklIME Input Text");
    return *description != nullptr ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP InputDisplayAttributeInfo::GetAttributeInfo(TF_DISPLAYATTRIBUTE* attribute)
{
    if (attribute == nullptr) {
        return E_INVALIDARG;
    }
    // 文字色・背景色はアプリ既定のまま、点線下線のみ付ける
    attribute->crText.type = TF_CT_NONE;
    attribute->crText.nIndex = 0;
    attribute->crBk.type = TF_CT_NONE;
    attribute->crBk.nIndex = 0;
    attribute->lsStyle = TF_LS_DOT;
    attribute->fBoldLine = FALSE;
    attribute->crLine.type = TF_CT_NONE;
    attribute->crLine.nIndex = 0;
    attribute->bAttr = TF_ATTR_INPUT;
    return S_OK;
}

STDMETHODIMP InputDisplayAttributeInfo::SetAttributeInfo(const TF_DISPLAYATTRIBUTE* attribute)
{
    UNREFERENCED_PARAMETER(attribute);
    return E_NOTIMPL;
}

STDMETHODIMP InputDisplayAttributeInfo::Reset()
{
    return S_OK;
}

// ---- EnumDisplayAttributeInfo ----

EnumDisplayAttributeInfo::EnumDisplayAttributeInfo() : refCount_(1), index_(0)
{
}

EnumDisplayAttributeInfo::~EnumDisplayAttributeInfo()
{
}

STDMETHODIMP EnumDisplayAttributeInfo::QueryInterface(REFIID riid, void** ppv)
{
    if (ppv == nullptr) {
        return E_INVALIDARG;
    }
    if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IEnumTfDisplayAttributeInfo)) {
        *ppv = static_cast<IEnumTfDisplayAttributeInfo*>(this);
        AddRef();
        return S_OK;
    }
    *ppv = nullptr;
    return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) EnumDisplayAttributeInfo::AddRef()
{
    return InterlockedIncrement(&refCount_);
}

STDMETHODIMP_(ULONG) EnumDisplayAttributeInfo::Release()
{
    LONG count = InterlockedDecrement(&refCount_);
    if (count == 0) {
        delete this;
    }
    return count;
}

STDMETHODIMP EnumDisplayAttributeInfo::Clone(IEnumTfDisplayAttributeInfo** enumInfo)
{
    if (enumInfo == nullptr) {
        return E_INVALIDARG;
    }
    auto* clone = new (std::nothrow) EnumDisplayAttributeInfo();
    if (clone == nullptr) {
        return E_OUTOFMEMORY;
    }
    clone->index_ = index_;
    *enumInfo = clone;
    return S_OK;
}

STDMETHODIMP EnumDisplayAttributeInfo::Next(ULONG count, ITfDisplayAttributeInfo** info,
                                            ULONG* fetched)
{
    if (info == nullptr) {
        return E_INVALIDARG;
    }

    ULONG taken = 0;
    if (count > 0 && index_ == 0) {
        auto* attribute = new (std::nothrow) InputDisplayAttributeInfo();
        if (attribute == nullptr) {
            return E_OUTOFMEMORY;
        }
        info[0] = attribute;
        taken = 1;
        index_ = 1;
    }

    if (fetched != nullptr) {
        *fetched = taken;
    }
    return taken == count ? S_OK : S_FALSE;
}

STDMETHODIMP EnumDisplayAttributeInfo::Reset()
{
    index_ = 0;
    return S_OK;
}

STDMETHODIMP EnumDisplayAttributeInfo::Skip(ULONG count)
{
    if (count > 0 && index_ == 0) {
        index_ = 1;
        return count == 1 ? S_OK : S_FALSE;
    }
    return count == 0 ? S_OK : S_FALSE;
}
