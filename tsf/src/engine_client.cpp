#include "engine_client.h"

namespace {

constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\quicklime-engine";

// 応答が長くなった場合の暴走防止 (フェーズ3では候補2つなので十分大きい)
constexpr size_t kMaxResponseBytes = 64 * 1024;

std::string WideToUtf8(const std::wstring& wide)
{
    if (wide.empty()) {
        return {};
    }
    const int size = WideCharToMultiByte(CP_UTF8, 0, wide.c_str(),
                                         static_cast<int>(wide.size()), nullptr, 0, nullptr,
                                         nullptr);
    if (size <= 0) {
        return {};
    }
    std::string utf8(size, '\0');
    WideCharToMultiByte(CP_UTF8, 0, wide.c_str(), static_cast<int>(wide.size()), utf8.data(),
                        size, nullptr, nullptr);
    return utf8;
}

std::wstring Utf8ToWide(const std::string& utf8)
{
    if (utf8.empty()) {
        return {};
    }
    const int size = MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(),
                                         static_cast<int>(utf8.size()), nullptr, 0);
    if (size <= 0) {
        return {};
    }
    std::wstring wide(size, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, utf8.c_str(), static_cast<int>(utf8.size()), wide.data(),
                        size);
    return wide;
}

} // namespace

EngineClient::~EngineClient()
{
    Disconnect();
}

bool EngineClient::EnsureConnected()
{
    if (pipe_ != INVALID_HANDLE_VALUE) {
        return true;
    }

    for (int attempt = 0; attempt < 2; ++attempt) {
        pipe_ = CreateFileW(kPipeName, GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                            0, nullptr);
        if (pipe_ != INVALID_HANDLE_VALUE) {
            return true;
        }
        // 全インスタンスが使用中の場合だけ少し待って再試行する
        if (GetLastError() != ERROR_PIPE_BUSY || !WaitNamedPipeW(kPipeName, 200)) {
            return false;
        }
    }
    return false;
}

void EngineClient::Disconnect()
{
    if (pipe_ != INVALID_HANDLE_VALUE) {
        CloseHandle(pipe_);
        pipe_ = INVALID_HANDLE_VALUE;
    }
}

bool EngineClient::Transact(const std::string& request, std::string* response)
{
    // エンジンが再起動した直後などは古い接続が切れているため、1回だけ接続し直して再送する
    for (int attempt = 0; attempt < 2; ++attempt) {
        if (!EnsureConnected()) {
            return false;
        }

        DWORD written = 0;
        if (!WriteFile(pipe_, request.data(), static_cast<DWORD>(request.size()), &written,
                       nullptr) ||
            written != request.size()) {
            Disconnect();
            continue;
        }

        // 改行が来るまで読む
        response->clear();
        char buffer[1024];
        bool failed = false;
        while (response->find('\n') == std::string::npos) {
            DWORD read = 0;
            if (!ReadFile(pipe_, buffer, sizeof(buffer), &read, nullptr) || read == 0) {
                failed = true;
                break;
            }
            response->append(buffer, read);
            if (response->size() > kMaxResponseBytes) {
                failed = true;
                break;
            }
        }
        if (failed) {
            Disconnect();
            continue;
        }
        return true;
    }
    return false;
}

bool EngineClient::Convert(const std::wstring& kana, std::vector<std::wstring>* candidates)
{
    if (candidates == nullptr || kana.empty()) {
        return false;
    }

    std::string response;
    if (!Transact("CONVERT\t" + WideToUtf8(kana) + "\n", &response)) {
        return false;
    }

    // 応答: "OK\t候補1\t候補2...\n"
    const size_t newline = response.find('\n');
    if (newline != std::string::npos) {
        response.resize(newline);
    }
    if (response.rfind("OK\t", 0) != 0) {
        return false;
    }

    candidates->clear();
    size_t begin = 3; // "OK\t" の直後
    while (begin <= response.size()) {
        size_t end = response.find('\t', begin);
        if (end == std::string::npos) {
            end = response.size();
        }
        const std::string field = response.substr(begin, end - begin);
        if (!field.empty()) {
            candidates->push_back(Utf8ToWide(field));
        }
        begin = end + 1;
    }
    return !candidates->empty();
}
