// initguid.h を先に include することで、この翻訳単位内で msctf.h の
// GUID 群 (GUID_TFCAT_* など) の実体が定義される。
// 他の翻訳単位からは extern 宣言として参照され、リンク時にここへ解決される。
#include <windows.h>
#include <initguid.h>

#include "globals.h"

namespace globals {

const CLSID kClsid = {
    0xd8fa8028, 0x4371, 0x40e9, {0x8f, 0x49, 0x4e, 0x46, 0x5e, 0xce, 0x9a, 0x41}};

const GUID kProfileGuid = {
    0xc0730986, 0xa430, 0x4595, {0x8d, 0x18, 0xa4, 0x10, 0x37, 0x18, 0xc6, 0xc6}};

HINSTANCE dllInstance = nullptr;

static LONG s_dllRefCount = 0;

void DllAddRef()
{
    InterlockedIncrement(&s_dllRefCount);
}

void DllRelease()
{
    InterlockedDecrement(&s_dllRefCount);
}

LONG DllRefCount()
{
    return s_dllRefCount;
}

} // namespace globals
