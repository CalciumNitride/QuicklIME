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

// 前文脈 (直前に確定した文節の読みと表記)。文脈補正 (CONVCTX) に使う
struct ConversionContext {
    std::wstring reading;
    std::wstring surface;
    bool Empty() const { return reading.empty() || surface.empty(); }
    void Clear()
    {
        reading.clear();
        surface.clear();
    }
};

// 確定学習の1文節。context は直前文節の表記 (空なら文脈なしの学習になる)
struct LearnEntry {
    std::wstring reading;
    std::wstring surface;
    std::wstring context;
};

// 変換エンジン (quicklime-engine.exe) への named pipe クライアント。
// プロトコルの詳細は docs/protocol.md を参照。
class EngineClient {
public:
    EngineClient() = default;
    ~EngineClient();

    EngineClient(const EngineClient&) = delete;
    EngineClient& operator=(const EngineClient&) = delete;

    // かなを文節列に変換する (CONVSEG / 前文脈があれば CONVCTX)。
    // エンジンに接続できない・応答が不正な場合は false を返す (呼び出し側でフォールバック)
    bool ConvertSegments(const std::wstring& kana, const ConversionContext& context,
                         std::vector<ConversionSegment>* segments);

    // 文節境界 (各文節の文字数) を固定して再変換する (Shift+←→ の文節伸縮用)
    bool ConvertSegmentsFixed(const std::wstring& kana, const std::vector<size_t>& lengths,
                              const ConversionContext& context,
                              std::vector<ConversionSegment>* segments);

    // かなを文節列に変換する (CONVSEG / CONVCTX)。毎打鍵のライブ変換用に、エンジンの
    // 自動起動や接続待ちはせず、未接続なら即 false を返す (Predict と同じ方式)
    bool ConvertSegmentsLive(const std::wstring& kana, const ConversionContext& context,
                             std::vector<ConversionSegment>* segments);

    // 読みに対する記号候補のみを取得する (CONVSYM、F4 の記号変換用)。
    // 通信に成功すれば true (記号が1つも無い場合も true で candidates は空)
    bool ConvertSymbols(const std::wstring& kana, std::vector<std::wstring>* candidates);

    // 読みに対する短縮よみ (ユーザ辞書) の候補のみを取得する (CONVUSER、F5 用)。
    // 通信に成功すれば true (候補が1つも無い場合も true で candidates は空)
    bool ConvertShortcuts(const std::wstring& kana, std::vector<std::wstring>* candidates);

    // 文節ごとの確定結果をエンジンに記録する (LEARN / 文脈があれば LEARN2)。
    // 失敗しても確定処理には影響させない
    bool Learn(const std::vector<LearnEntry>& entries);

    // 読みの前方一致で予測候補を取得する (PREDICT)。
    // 毎打鍵で呼ばれるため、エンジンの自動起動や接続待ちはせず、
    // 未接続なら即 false を返す (候補ゼロは true で candidates が空)
    bool Predict(const std::wstring& kana, std::vector<PredictionCandidate>* candidates);

    // 同梱 exe (エンジン・単語登録ツール) のパスを探す。
    // DLL と同じディレクトリ → 開発レイアウト (engine/target) の順。見つからなければ空
    static std::wstring FindExePath(const wchar_t* exeName);

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
    // CONVSEG / CONVCTX の要求行を作る (context が空か旧エンジン判定済みなら CONVSEG)
    std::string BuildSegmentsRequest(const std::wstring& kana, const ConversionContext& context,
                                     const std::string& lengthsField) const;
    // 要求を送り、旧エンジンに新コマンドを拒否されたら旧コマンド fallback で再送する。
    // live = true なら接続確立 (エンジン自動起動) をしない
    bool TransactWithFallback(const std::string& request, const std::string& fallback,
                              bool live, std::string* response);
    // CONVSEG 系の要求 (+旧コマンド fallback) を送って応答を segments にパースする
    bool RequestSegments(const std::string& request, const std::string& fallback, bool live,
                         std::vector<ConversionSegment>* segments);
    // CONVSEG 系の応答1行を segments にパースする
    bool ParseSegmentsResponse(std::string response, std::vector<ConversionSegment>* segments);

    HANDLE pipe_ = INVALID_HANDLE_VALUE;
    ULONGLONG lastLaunchTick_ = 0; // 最後にエンジン起動を試みた時刻 (連続起動の抑止)
    // 旧エンジン (CONVCTX/LEARN2 未対応) と判定したら以後は旧コマンドのみ送る
    bool legacyEngine_ = false;
};
