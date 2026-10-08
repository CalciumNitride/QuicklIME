#include "debug_log.h"

#include <windows.h>

#include <cwchar>

void WriteDebugLog(const std::wstring& path, const std::wstring& message)
{
    if (path.empty()) {
        return;
    }
    SYSTEMTIME now = {};
    GetLocalTime(&now);
    wchar_t prefix[96] = {};
    swprintf_s(prefix, L"%04u-%02u-%02u %02u:%02u:%02u.%03u pid=%lu tid=%lu ", now.wYear,
               now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond, now.wMilliseconds,
               GetCurrentProcessId(), GetCurrentThreadId());
    const std::wstring line = prefix + message + L"\r\n";

    const int size = WideCharToMultiByte(CP_UTF8, 0, line.data(), static_cast<int>(line.size()),
                                         nullptr, 0, nullptr, nullptr);
    if (size <= 0) {
        return;
    }
    std::string utf8(static_cast<size_t>(size), '\0');
    WideCharToMultiByte(CP_UTF8, 0, line.data(), static_cast<int>(line.size()), utf8.data(), size,
                        nullptr, nullptr);

    HANDLE file = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                              nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) {
        return;
    }
    DWORD written = 0;
    WriteFile(file, utf8.data(), static_cast<DWORD>(utf8.size()), &written, nullptr);
    CloseHandle(file);
}
