#include <windows.h>
#include <msctf.h>
#include <olectl.h>

#include <new>

STDAPI DllUnregisterServer();

#include "globals.h"
#include "registry.h"
#include "text_service.h"

namespace {

// TextService を生成する class factory (プロセスに1つの静的インスタンス)
class ClassFactory : public IClassFactory {
public:
    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IClassFactory)) {
            *ppv = static_cast<IClassFactory*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        return E_NOINTERFACE;
    }

    // 静的インスタンスなので参照カウントは DLL 全体のカウントに委譲する
    STDMETHODIMP_(ULONG) AddRef() override
    {
        globals::DllAddRef();
        return 2;
    }

    STDMETHODIMP_(ULONG) Release() override
    {
        globals::DllRelease();
        return 1;
    }

    // IClassFactory
    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override
    {
        if (ppv == nullptr) {
            return E_INVALIDARG;
        }
        *ppv = nullptr;
        if (outer != nullptr) {
            return CLASS_E_NOAGGREGATION;
        }

        auto* service = new (std::nothrow) TextService();
        if (service == nullptr) {
            return E_OUTOFMEMORY;
        }
        HRESULT hr = service->QueryInterface(riid, ppv);
        service->Release();
        return hr;
    }

    STDMETHODIMP LockServer(BOOL lock) override
    {
        if (lock) {
            globals::DllAddRef();
        } else {
            globals::DllRelease();
        }
        return S_OK;
    }
};

ClassFactory g_classFactory;

} // namespace

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID reserved)
{
    UNREFERENCED_PARAMETER(reserved);

    switch (reason) {
    case DLL_PROCESS_ATTACH:
        globals::dllInstance = instance;
        // スレッドごとの通知は不要なので無効化する (パフォーマンス向上)
        DisableThreadLibraryCalls(instance);
        break;
    default:
        break;
    }
    return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv)
{
    if (ppv == nullptr) {
        return E_INVALIDARG;
    }
    if (!IsEqualCLSID(rclsid, globals::kClsid)) {
        *ppv = nullptr;
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    return g_classFactory.QueryInterface(riid, ppv);
}

STDAPI DllCanUnloadNow()
{
    return globals::DllRefCount() == 0 ? S_OK : S_FALSE;
}

STDAPI DllRegisterServer()
{
    // regsvr32 は COM を初期化してから呼ぶが、他の呼び出し元に備えて自前でも初期化する
    HRESULT hrInit = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

    HRESULT hr = S_OK;
    if (!RegisterComServer() || !RegisterProfile() || !RegisterCategories()) {
        DllUnregisterServer();
        hr = SELFREG_E_CLASS;
    }

    if (SUCCEEDED(hrInit)) {
        CoUninitialize();
    }
    return hr;
}

STDAPI DllUnregisterServer()
{
    HRESULT hrInit = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

    UnregisterCategories();
    UnregisterProfile();
    UnregisterComServer();

    if (SUCCEEDED(hrInit)) {
        CoUninitialize();
    }
    return S_OK;
}
