#pragma once

#include <windows.h>

#include <string>
#include <utility>
#include <vector>

// 変換結果の1文節
struct ConversionSegment {
    std::wstring reading;                  // この文節の読み
    std::vector<std::wstring> candidates;  // 候補リスト (先頭が最良)
};

// 予測入力の1候補
struct PredictionCandidate {
    std::wstring reading;  // 候補の完全な読み (採用時の LEARN に使う)
    std::wstring surface;  // 表記
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

    // 文節境界 (各文節の文字数) を固定して再変換する (Shift+←→ の文節伸縮用)
    bool ConvertSegmentsFixed(const std::wstring& kana, const std::vector<size_t>& lengths,
                              std::vector<ConversionSegment>* segments);

    // 読みに対する記号候補のみを取得する (CONVSYM、F4 の記号変換用)。
    // 通信に成功すれば true (記号が1つも無い場合も true で candidates は空)
    bool ConvertSymbols(const std::wstring& kana, std::vector<std::wstring>* candidates);

    // 文節ごとの確定結果 (読み, 表記) をエンジンに記録する (LEARN)。
    // 失敗しても確定処理には影響させない
    bool Learn(const std::vector<std::pair<std::wstring, std::wstring>>& pairs);

    // 読みの前方一致で予測候補を取得する (PREDICT)。
    // 毎打鍵で呼ばれるため、エンジンの自動起動や接続待ちはせず、
    // 未接続なら即 false を返す (候補ゼロは true で candidates が空)
    bool Predict(const std::wstring& kana, std::vector<PredictionCandidate>* candidates);

private:
    bool EnsureConnected();
    // パイプを1回だけ開いてみる (起動待ちはしない)
    bool TryOpenPipe();
    // エンジン exe を探して起動する。クールダウン中や exe が無い場合は false
    bool TryLaunchEngine();
    void Disconnect();
    // 1行の要求を送り、1行の応答を受け取る (未接続なら接続・エンジン起動も試みる)
    bool Transact(const std::string& request, std::string* response);
    // 現在の接続で1往復だけ行う。失敗時は切断して false (接続の確立はしない)
    bool SendReceive(const std::string& request, std::string* response);
    // CONVSEG 系の要求を送って応答を segments にパースする
    bool RequestSegments(const std::string& request, std::vector<ConversionSegment>* segments);

    HANDLE pipe_ = INVALID_HANDLE_VALUE;
    ULONGLONG lastLaunchTick_ = 0; // 最後にエンジン起動を試みた時刻 (連続起動の抑止)
};
