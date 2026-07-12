#include "engine_client.h"

#include "globals.h"

namespace {

constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\quicklime-engine";

// 応答が長くなった場合の暴走防止
constexpr size_t kMaxResponseBytes = 64 * 1024;

// エンジン起動を再試行するまでの間隔 (起動失敗の連打を防ぐ)
constexpr ULONGLONG kLaunchCooldownMs = 10 * 1000;

// エンジン起動後、パイプ作成を待つ最大回数と間隔 (計2秒)
constexpr int kConnectRetryCount = 20;
constexpr DWORD kConnectRetryIntervalMs = 100;

// エンジン exe の探索候補を返す。
// 1. DLL と同じディレクトリ (配布時のレイアウト)
// 2. 開発レイアウト (tsf/build/Debug -> engine/target/{release,debug})
std::wstring FindEngineExe()
{
    wchar_t dllPath[MAX_PATH] = {};
    const DWORD length = GetModuleFileNameW(globals::dllInstance, dllPath, ARRAYSIZE(dllPath));
    if (length == 0 || length >= ARRAYSIZE(dllPath)) {
        return {};
    }
    std::wstring dir(dllPath);
    const size_t lastSep = dir.find_last_of(L'\\');
    if (lastSep == std::wstring::npos) {
        return {};
    }
    dir.resize(lastSep);

    const wchar_t* candidates[] = {
        L"\\quicklime-engine.exe",
        L"\\..\\..\\..\\engine\\target\\release\\quicklime-engine.exe",
        L"\\..\\..\\..\\engine\\target\\debug\\quicklime-engine.exe",
    };
    for (const wchar_t* candidate : candidates) {
        const std::wstring path = dir + candidate;
        if (GetFileAttributesW(path.c_str()) != INVALID_FILE_ATTRIBUTES) {
            return path;
        }
    }
    return {};
}

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

bool EngineClient::TryOpenPipe()
{
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

bool EngineClient::TryLaunchEngine()
{
    // 直前に起動を試みたばかりなら何もしない (起動失敗の連打防止)
    const ULONGLONG now = GetTickCount64();
    if (lastLaunchTick_ != 0 && now - lastLaunchTick_ < kLaunchCooldownMs) {
        return false;
    }
    lastLaunchTick_ = now;

    const std::wstring exePath = FindEngineExe();
    if (exePath.empty()) {
        return false;
    }

    STARTUPINFOW startupInfo = {};
    startupInfo.cb = sizeof(startupInfo);
    PROCESS_INFORMATION processInfo = {};
    // コンソールウィンドウを出さずに起動する
    if (!CreateProcessW(exePath.c_str(), nullptr, nullptr, nullptr, FALSE, CREATE_NO_WINDOW,
                        nullptr, nullptr, &startupInfo, &processInfo)) {
        return false;
    }
    CloseHandle(processInfo.hThread);
    CloseHandle(processInfo.hProcess);
    return true;
}

bool EngineClient::EnsureConnected()
{
    if (pipe_ != INVALID_HANDLE_VALUE) {
        return true;
    }
    if (TryOpenPipe()) {
        return true;
    }

    // エンジンが起動していないようなので自動起動を試みる
    if (!TryLaunchEngine()) {
        return false;
    }

    // 辞書読み込みが終わってパイプができるまで少し待つ
    for (int i = 0; i < kConnectRetryCount; ++i) {
        Sleep(kConnectRetryIntervalMs);
        if (TryOpenPipe()) {
            return true;
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

namespace {

// separator で区切られたフィールド列に分割する (空フィールドは捨てる)
std::vector<std::string> SplitFields(const std::string& text, char separator)
{
    std::vector<std::string> fields;
    size_t begin = 0;
    while (begin <= text.size()) {
        size_t end = text.find(separator, begin);
        if (end == std::string::npos) {
            end = text.size();
        }
        if (end > begin) {
            fields.push_back(text.substr(begin, end - begin));
        }
        begin = end + 1;
    }
    return fields;
}

} // namespace

bool EngineClient::ConvertSegments(const std::wstring& kana,
                                   std::vector<ConversionSegment>* segments)
{
    if (segments == nullptr || kana.empty()) {
        return false;
    }

    std::string response;
    if (!Transact("CONVSEG\t" + WideToUtf8(kana) + "\n", &response)) {
        return false;
    }

    // 応答: "OK\t読み\x1F候補1\x1F候補2...\t読み\x1F候補...\n"
    const size_t newline = response.find('\n');
    if (newline != std::string::npos) {
        response.resize(newline);
    }
    if (response.rfind("OK\t", 0) != 0) {
        return false;
    }

    segments->clear();
    for (const std::string& segmentField : SplitFields(response.substr(3), '\t')) {
        const std::vector<std::string> fields = SplitFields(segmentField, '\x1f');
        if (fields.size() < 2) {
            return false; // 読み + 候補1つ以上が必須
        }
        ConversionSegment segment;
        segment.reading = Utf8ToWide(fields[0]);
        for (size_t i = 1; i < fields.size(); ++i) {
            segment.candidates.push_back(Utf8ToWide(fields[i]));
        }
        segments->push_back(std::move(segment));
    }
    return !segments->empty();
}

bool EngineClient::Learn(const std::vector<std::pair<std::wstring, std::wstring>>& pairs)
{
    std::string request = "LEARN";
    size_t count = 0;
    for (const auto& [reading, surface] : pairs) {
        if (reading.empty() || surface.empty()) {
            continue;
        }
        request += "\t" + WideToUtf8(reading) + "\x1f" + WideToUtf8(surface);
        ++count;
    }
    if (count == 0) {
        return false;
    }
    request += "\n";

    std::string response;
    return Transact(request, &response) && response.rfind("OK", 0) == 0;
}
