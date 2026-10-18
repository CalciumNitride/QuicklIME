#include "icon_font.h"

#include <dwrite.h>

#include <algorithm>
#include <cmath>
#include <new>

#include "globals.h"
#include "resource.h"

namespace {

constexpr wchar_t kReferenceChar = L'あ';
constexpr wchar_t kFallbackFont[] = L"Yu Gothic UI";

template <typename T>
void SafeRelease(T** object)
{
    if (*object != nullptr) {
        (*object)->Release();
        *object = nullptr;
    }
}

// DLL のリソースに入れたフォント (RCDATA) をそのまま返す stream。
// IDWriteFactory5::CreateInMemoryFontFileLoader は 1703 以降にしか無く、対応 OS の下限 (1607) で
// 使えないため自前で持つ。リソースのメモリはモジュールが解放されるまで有効
class ResourceFontStream : public IDWriteFontFileStream {
public:
    ResourceFontStream(const void* data, UINT64 size) : refCount_(1), data_(data), size_(size) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, __uuidof(IUnknown)) ||
            IsEqualIID(riid, __uuidof(IDWriteFontFileStream))) {
            *ppv = static_cast<IDWriteFontFileStream*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refCount_); }
    STDMETHODIMP_(ULONG) Release() override
    {
        const LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    STDMETHODIMP ReadFileFragment(const void** fragmentStart, UINT64 fileOffset,
                                  UINT64 fragmentSize, void** fragmentContext) override
    {
        if (fragmentStart == nullptr || fragmentContext == nullptr) {
            return E_INVALIDARG;
        }
        *fragmentContext = nullptr;
        if (fileOffset > size_ || fragmentSize > size_ - fileOffset) {
            *fragmentStart = nullptr;
            return E_FAIL;
        }
        *fragmentStart = static_cast<const BYTE*>(data_) + fileOffset;
        return S_OK;
    }
    STDMETHODIMP_(void) ReleaseFileFragment(void* fragmentContext) override
    {
        UNREFERENCED_PARAMETER(fragmentContext);
    }
    STDMETHODIMP GetFileSize(UINT64* fileSize) override
    {
        if (fileSize == nullptr) {
            return E_INVALIDARG;
        }
        *fileSize = size_;
        return S_OK;
    }
    STDMETHODIMP GetLastWriteTime(UINT64* lastWriteTime) override
    {
        // メモリ上のフォントには更新時刻が無い (DirectWrite は E_NOTIMPL を許す)
        if (lastWriteTime != nullptr) {
            *lastWriteTime = 0;
        }
        return E_NOTIMPL;
    }

private:
    ~ResourceFontStream() = default;

    LONG refCount_;
    const void* data_;
    UINT64 size_;
};

// アイコン用のフォントのリソースを stream として返す loader。key は使わない
// (このフォント 1 つしか読まない)
class ResourceFontLoader : public IDWriteFontFileLoader {
public:
    ResourceFontLoader() : refCount_(1) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, __uuidof(IUnknown)) ||
            IsEqualIID(riid, __uuidof(IDWriteFontFileLoader))) {
            *ppv = static_cast<IDWriteFontFileLoader*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refCount_); }
    STDMETHODIMP_(ULONG) Release() override
    {
        const LONG count = InterlockedDecrement(&refCount_);
        if (count == 0) {
            delete this;
        }
        return count;
    }

    STDMETHODIMP CreateStreamFromKey(const void* key, UINT32 keySize,
                                     IDWriteFontFileStream** stream) override
    {
        UNREFERENCED_PARAMETER(key);
        UNREFERENCED_PARAMETER(keySize);
        if (stream == nullptr) {
            return E_INVALIDARG;
        }
        *stream = nullptr;
        HRSRC info =
            FindResourceW(globals::dllInstance, MAKEINTRESOURCEW(IDR_ICON_FONT), RT_RCDATA);
        if (info == nullptr) {
            return E_FAIL;
        }
        HGLOBAL resource = LoadResource(globals::dllInstance, info);
        const void* data = resource != nullptr ? LockResource(resource) : nullptr;
        const DWORD size = SizeofResource(globals::dllInstance, info);
        if (data == nullptr || size == 0) {
            return E_FAIL;
        }
        *stream = new (std::nothrow) ResourceFontStream(data, size);
        return *stream != nullptr ? S_OK : E_OUTOFMEMORY;
    }

private:
    ~ResourceFontLoader() = default;

    LONG refCount_;
};

IDWriteFontFace* CreateResourceFontFace(IDWriteFactory* factory, IDWriteFontFileLoader* loader)
{
    const UINT32 key = IDR_ICON_FONT;
    IDWriteFontFile* file = nullptr;
    if (FAILED(factory->CreateCustomFontFileReference(&key, sizeof(key), loader, &file))) {
        return nullptr;
    }
    BOOL supported = FALSE;
    DWRITE_FONT_FILE_TYPE fileType = DWRITE_FONT_FILE_TYPE_UNKNOWN;
    DWRITE_FONT_FACE_TYPE faceType = DWRITE_FONT_FACE_TYPE_UNKNOWN;
    UINT32 faceCount = 0;
    IDWriteFontFace* face = nullptr;
    if (SUCCEEDED(file->Analyze(&supported, &fileType, &faceType, &faceCount)) && supported &&
        FAILED(factory->CreateFontFace(faceType, 1, &file, 0, DWRITE_FONT_SIMULATIONS_NONE,
                                       &face))) {
        face = nullptr;
    }
    file->Release();
    return face;
}

IDWriteFontFace* CreateSystemFontFace(IDWriteFactory* factory)
{
    IDWriteFontCollection* fonts = nullptr;
    if (FAILED(factory->GetSystemFontCollection(&fonts, FALSE))) {
        return nullptr;
    }
    IDWriteFontFace* face = nullptr;
    UINT32 index = 0;
    BOOL exists = FALSE;
    IDWriteFontFamily* family = nullptr;
    if (SUCCEEDED(fonts->FindFamilyName(kFallbackFont, &index, &exists)) && exists &&
        SUCCEEDED(fonts->GetFontFamily(index, &family))) {
        IDWriteFont* font = nullptr;
        if (SUCCEEDED(family->GetFirstMatchingFont(DWRITE_FONT_WEIGHT_REGULAR,
                                                   DWRITE_FONT_STRETCH_NORMAL,
                                                   DWRITE_FONT_STYLE_NORMAL, &font))) {
            if (FAILED(font->CreateFontFace(&face))) {
                face = nullptr;
            }
            font->Release();
        }
        family->Release();
    }
    fonts->Release();
    return face;
}

// 字形の外接矩形 (design units。y は上向き)
struct InkBounds {
    float left;
    float top;
    float right;
    float bottom;
};

InkBounds GlyphInkBounds(const DWRITE_GLYPH_METRICS& m)
{
    // verticalOriginY は topSideBearing と字形の上端 (yMax) の和
    const float top = static_cast<float>(m.verticalOriginY - m.topSideBearing);
    const float bottom =
        static_cast<float>(m.verticalOriginY) - static_cast<float>(m.advanceHeight) +
        static_cast<float>(m.bottomSideBearing);
    const float left = static_cast<float>(m.leftSideBearing);
    const float right = static_cast<float>(m.advanceWidth) - static_cast<float>(m.rightSideBearing);
    return {left, top, right, bottom};
}

bool GlyphOf(IDWriteFontFace* face, wchar_t ch, UINT16* index, DWRITE_GLYPH_METRICS* metrics)
{
    const UINT32 codePoint = ch;
    return SUCCEEDED(face->GetGlyphIndices(&codePoint, 1, index)) &&
           SUCCEEDED(face->GetDesignGlyphMetrics(index, 1, metrics, FALSE));
}

} // namespace

IconFont::IconFont() : factory_(nullptr), loader_(nullptr), face_(nullptr)
{
}

IconFont::~IconFont()
{
    SafeRelease(&face_);
    if (loader_ != nullptr) {
        factory_->UnregisterFontFileLoader(loader_);
        SafeRelease(&loader_);
    }
    SafeRelease(&factory_);
}

IDWriteFontFace* IconFont::Face()
{
    if (face_ != nullptr) {
        return face_;
    }
    // 自前の loader を登録するので、プロセスで共有の factory を汚さないよう ISOLATED にする
    if (factory_ == nullptr &&
        FAILED(DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED, __uuidof(IDWriteFactory),
                                   reinterpret_cast<IUnknown**>(&factory_)))) {
        factory_ = nullptr;
        return nullptr;
    }
    if (loader_ == nullptr) {
        auto* loader = new (std::nothrow) ResourceFontLoader();
        if (loader != nullptr) {
            if (SUCCEEDED(factory_->RegisterFontFileLoader(loader))) {
                loader_ = loader;
            } else {
                loader->Release();
            }
        }
    }
    if (loader_ != nullptr) {
        face_ = CreateResourceFontFace(factory_, loader_);
    }
    if (face_ == nullptr) {
        face_ = CreateSystemFontFace(factory_);
    }
    return face_;
}

bool PlaceIconGlyph(IDWriteFontFace* face, wchar_t ch, float inkLong, float size, IconGlyph* glyph)
{
    // 文字サイズは「あ」の字形で決め、ほかの字も同じ文字サイズで描く
    UINT16 referenceIndex = 0;
    DWRITE_GLYPH_METRICS referenceMetrics = {};
    UINT16 index = 0;
    DWRITE_GLYPH_METRICS metrics = {};
    if (!GlyphOf(face, kReferenceChar, &referenceIndex, &referenceMetrics) ||
        !GlyphOf(face, ch, &index, &metrics)) {
        return false;
    }
    const InkBounds reference = GlyphInkBounds(referenceMetrics);
    const float referenceLong =
        (std::max)(reference.right - reference.left, reference.top - reference.bottom);
    if (referenceLong <= 0.0f) {
        return false;
    }
    DWRITE_FONT_METRICS fontMetrics = {};
    face->GetMetrics(&fontMetrics);
    const float emSize = inkLong * static_cast<float>(fontMetrics.designUnitsPerEm) / referenceLong;
    const float scale = emSize / static_cast<float>(fontMetrics.designUnitsPerEm);
    const InkBounds ink = GlyphInkBounds(metrics);
    const float half = size / 2.0f;
    glyph->index = index;
    glyph->emSize = emSize;
    glyph->advance = static_cast<float>(metrics.advanceWidth) * scale;
    // 線をぼかさないよう、描く原点は整数 px に丸める
    glyph->originX = std::round(half - (ink.left + ink.right) / 2.0f * scale);
    glyph->originY = std::round(half + (ink.top + ink.bottom) / 2.0f * scale);
    return true;
}
