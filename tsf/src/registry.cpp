#include "registry.h"

#include <msctf.h>
#include <strsafe.h>

#include "globals.h"

namespace {

// このテキストサービスが対応する TSF カテゴリ。
// - TIP_KEYBOARD: キーボード型の入力方式であること (必須)
// - TIPCAP_IMMERSIVESUPPORT: UWP / immersive アプリでの動作に対応
// - TIPCAP_SYSTRAYSUPPORT: デスクトップの入力インジケータでの表示に対応
const GUID kSupportCategories[] = {
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
};

// CLSID を "{XXXXXXXX-....}" 形式の文字列にする
BOOL ClsidToString(REFCLSID clsid, wchar_t (&buffer)[64])
{
    return StringFromGUID2(clsid, buffer, ARRAYSIZE(buffer)) != 0;
}

} // namespace

BOOL RegisterComServer()
{
    wchar_t clsidString[64] = {};
    if (!ClsidToString(globals::kClsid, clsidString)) {
        return FALSE;
    }

    wchar_t keyPath[128] = L"CLSID\\";
    if (FAILED(StringCchCatW(keyPath, ARRAYSIZE(keyPath), clsidString))) {
        return FALSE;
    }

    // HKCR\CLSID\{...} (既定値 = 説明)
    HKEY clsidKey = nullptr;
    if (RegCreateKeyExW(HKEY_CLASSES_ROOT, keyPath, 0, nullptr, REG_OPTION_NON_VOLATILE,
                        KEY_WRITE, nullptr, &clsidKey, nullptr) != ERROR_SUCCESS) {
        return FALSE;
    }

    BOOL result = FALSE;
    HKEY inprocKey = nullptr;
    do {
        if (RegSetValueExW(clsidKey, nullptr, 0, REG_SZ,
                           reinterpret_cast<const BYTE*>(globals::kDescription),
                           sizeof(globals::kDescription)) != ERROR_SUCCESS) {
            break;
        }

        // HKCR\CLSID\{...}\InprocServer32 (既定値 = DLL パス, ThreadingModel = Apartment)
        if (RegCreateKeyExW(clsidKey, L"InprocServer32", 0, nullptr, REG_OPTION_NON_VOLATILE,
                            KEY_WRITE, nullptr, &inprocKey, nullptr) != ERROR_SUCCESS) {
            break;
        }

        wchar_t dllPath[MAX_PATH] = {};
        DWORD length = GetModuleFileNameW(globals::dllInstance, dllPath, ARRAYSIZE(dllPath));
        if (length == 0 || length >= ARRAYSIZE(dllPath)) {
            break;
        }
        if (RegSetValueExW(inprocKey, nullptr, 0, REG_SZ,
                           reinterpret_cast<const BYTE*>(dllPath),
                           (length + 1) * sizeof(wchar_t)) != ERROR_SUCCESS) {
            break;
        }

        static constexpr wchar_t kThreadingModel[] = L"Apartment";
        if (RegSetValueExW(inprocKey, L"ThreadingModel", 0, REG_SZ,
                           reinterpret_cast<const BYTE*>(kThreadingModel),
                           sizeof(kThreadingModel)) != ERROR_SUCCESS) {
            break;
        }

        result = TRUE;
    } while (false);

    if (inprocKey != nullptr) {
        RegCloseKey(inprocKey);
    }
    RegCloseKey(clsidKey);
    return result;
}

void UnregisterComServer()
{
    wchar_t clsidString[64] = {};
    if (!ClsidToString(globals::kClsid, clsidString)) {
        return;
    }

    wchar_t keyPath[128] = L"CLSID\\";
    if (FAILED(StringCchCatW(keyPath, ARRAYSIZE(keyPath), clsidString))) {
        return;
    }
    RegDeleteTreeW(HKEY_CLASSES_ROOT, keyPath);
}

BOOL RegisterProfile()
{
    ITfInputProcessorProfileMgr* profileMgr = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfInputProcessorProfileMgr,
                                  reinterpret_cast<void**>(&profileMgr));
    if (FAILED(hr)) {
        return FALSE;
    }

    // アイコンは未実装のため指定しない (フェーズ5で追加予定)
    hr = profileMgr->RegisterProfile(
        globals::kClsid, globals::kLangId, globals::kProfileGuid, globals::kDescription,
        static_cast<ULONG>(wcslen(globals::kDescription)), nullptr, 0, 0, nullptr, 0, TRUE, 0);

    profileMgr->Release();
    return SUCCEEDED(hr);
}

void UnregisterProfile()
{
    ITfInputProcessorProfileMgr* profileMgr = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfInputProcessorProfileMgr,
                                  reinterpret_cast<void**>(&profileMgr));
    if (FAILED(hr)) {
        return;
    }
    profileMgr->UnregisterProfile(globals::kClsid, globals::kLangId, globals::kProfileGuid, 0);
    profileMgr->Release();
}

BOOL RegisterCategories()
{
    ITfCategoryMgr* categoryMgr = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr));
    if (FAILED(hr)) {
        return FALSE;
    }

    for (const GUID& category : kSupportCategories) {
        hr = categoryMgr->RegisterCategory(globals::kClsid, category, globals::kClsid);
        if (FAILED(hr)) {
            break;
        }
    }

    categoryMgr->Release();
    return SUCCEEDED(hr);
}

void UnregisterCategories()
{
    ITfCategoryMgr* categoryMgr = nullptr;
    HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                  IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr));
    if (FAILED(hr)) {
        return;
    }
    for (const GUID& category : kSupportCategories) {
        categoryMgr->UnregisterCategory(globals::kClsid, category, globals::kClsid);
    }
    categoryMgr->Release();
}
