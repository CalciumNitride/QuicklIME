#pragma once

#include <windows.h>

#include <string>
#include <vector>

// 変換エンジン (quicklime-engine.exe) への named pipe クライアント。
// プロトコルの詳細は docs/protocol.md を参照。
class EngineClient {
public:
    EngineClient() = default;
    ~EngineClient();

    EngineClient(const EngineClient&) = delete;
    EngineClient& operator=(const EngineClient&) = delete;

    // かなを変換候補リストに変換する。
    // エンジンに接続できない・応答が不正な場合は false を返す (呼び出し側でフォールバック)
    bool Convert(const std::wstring& kana, std::vector<std::wstring>* candidates);

private:
    bool EnsureConnected();
    void Disconnect();
    // 1行の要求を送り、1行の応答を受け取る
    bool Transact(const std::string& request, std::string* response);

    HANDLE pipe_ = INVALID_HANDLE_VALUE;
};
