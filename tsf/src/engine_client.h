#pragma once

#include <windows.h>

#include <string>
#include <vector>

// 変換結果の1文節
struct ConversionSegment {
    std::wstring reading;                  // この文節の読み
    std::vector<std::wstring> candidates;  // 候補リスト (先頭が最良)
};

// 変換エンジン (quicklime-engine.exe) への named pipe クライアント。
// プロトコルの詳細は docs/protocol.md を参照。
class EngineClient {
public:
    EngineClient() = default;
    ~EngineClient();

    EngineClient(const EngineClient&) = delete;
    EngineClient& operator=(const EngineClient&) = delete;

    // かなを文節列に変換する (CONVSEG)。
    // エンジンに接続できない・応答が不正な場合は false を返す (呼び出し側でフォールバック)
    bool ConvertSegments(const std::wstring& kana, std::vector<ConversionSegment>* segments);

private:
    bool EnsureConnected();
    void Disconnect();
    // 1行の要求を送り、1行の応答を受け取る
    bool Transact(const std::string& request, std::string* response);

    HANDLE pipe_ = INVALID_HANDLE_VALUE;
};
