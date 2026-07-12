#include "display_attribute.h"

#include <new>

const GUID kInputDisplayAttributeGuid = {
    0x4129daa2, 0x56f7, 0x4a6f, {0x80, 0x47, 0x8b, 0xb4, 0xbd, 0x59, 0x93, 0x1c}};

const GUID kTargetDisplayAttributeGuid = {
    0xfe4a53bb, 0x970f, 0x4850, {0x80, 0xdc, 0x59, 0xce, 0x2e, 0xc5, 0x49, 0x4e}};

namespace {

// 1つの表示属性 (GUID + 属性値 + 説明) を表す汎用オブジェクト
class DisplayAttributeInfo : public ITfDisplayAttributeInfo {
public:
    DisplayAttributeInfo(REFGUID guid, const wchar_t* description,
                         const TF_DISPLAYATTRIBUTE& attribute)
        : refCount_(1), guid_(guid), description_(description), attribute_(attribute)
    {
    }

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
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

    STDMETHODIMP_(ULONG) AddRef() override
    {
        return InterlockedIncrement(&refCount_);
    }

    STDMETHODIMP_(ULONG) Release() override
    {
        LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    // ITfDisplayAttributeInfo
    STDMETHODIMP GetGUID(GUID* guid) override
    {
        if (guid == nullptr) {
            return E_INVALIDARG;
        }
        *guid = guid_;
        return S_OK;
    }

    STDMETHODIMP GetDescription(BSTR* description) override
    {
        if (description == nullptr) {
            return E_INVALIDARG;
        }
        *description = SysAllocString(description_);
        return *description != nullptr ? S_OK : E_OUTOFMEMORY;
    }

    STDMETHODIMP GetAttributeInfo(TF_DISPLAYATTRIBUTE* attribute) override
    {
        if (attribute == nullptr) {
            return E_INVALIDARG;
        }
        *attribute = attribute_;
        return S_OK;
    }

    STDMETHODIMP SetAttributeInfo(const TF_DISPLAYATTRIBUTE* attribute) override
    {
        UNREFERENCED_PARAMETER(attribute);
        return E_NOTIMPL;
    }

    STDMETHODIMP Reset() override
    {
        return S_OK;
    }

private:
    virtual ~DisplayAttributeInfo() = default;

    LONG refCount_;
    GUID guid_;
    const wchar_t* description_;
    TF_DISPLAYATTRIBUTE attribute_;
};

// 色指定なしの下線だけの属性値を作る
TF_DISPLAYATTRIBUTE UnderlineAttribute(TF_DA_LINESTYLE style, BOOL bold, TF_DA_ATTR_INFO attr)
{
    TF_DISPLAYATTRIBUTE da = {};
    da.crText.type = TF_CT_NONE;
    da.crBk.type = TF_CT_NONE;
    da.lsStyle = style;
    da.fBoldLine = bold;
    da.crLine.type = TF_CT_NONE;
    da.bAttr = attr;
    return da;
}

} // namespace

ITfDisplayAttributeInfo* CreateDisplayAttributeInfo(ULONG index)
{
    switch (index) {
    case 0:
        // 入力中: 点線下線
        return new (std::nothrow) DisplayAttributeInfo(
            kInputDisplayAttributeGuid, L"QuicklIME Input Text",
            UnderlineAttribute(TF_LS_DOT, FALSE, TF_ATTR_INPUT));
    case 1:
        // 変換対象文節: 実線太下線
        return new (std::nothrow) DisplayAttributeInfo(
            kTargetDisplayAttributeGuid, L"QuicklIME Target Segment",
            UnderlineAttribute(TF_LS_SOLID, TRUE, TF_ATTR_TARGET_CONVERTED));
    default:
        return nullptr;
    }
}

ITfDisplayAttributeInfo* CreateDisplayAttributeInfoForGuid(REFGUID guid)
{
    if (IsEqualGUID(guid, kInputDisplayAttributeGuid)) {
        return CreateDisplayAttributeInfo(0);
    }
    if (IsEqualGUID(guid, kTargetDisplayAttributeGuid)) {
        return CreateDisplayAttributeInfo(1);
    }
    return nullptr;
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
    while (taken < count && index_ < kDisplayAttributeCount) {
        ITfDisplayAttributeInfo* attribute = CreateDisplayAttributeInfo(index_);
        if (attribute == nullptr) {
            return E_OUTOFMEMORY;
        }
        info[taken] = attribute;
        ++taken;
        ++index_;
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
    const ULONG remaining = kDisplayAttributeCount - index_;
    if (count <= remaining) {
        index_ += count;
        return S_OK;
    }
    index_ = kDisplayAttributeCount;
    return S_FALSE;
}
