#pragma once

#include <windows.h>

#include <string>
#include <utility>
#include <vector>

// 変換結果の1文節
struct ConversionSegment {
    std::wstring reading;                  // この文節の読み
    std::vector<std::wstring> candidates;  // 候補リスト (先頭が最良)
    // 候補のうち補正の表記 (docs/design/typo-segment-ui.md)。候補の並びは後から入れ替わるので
    // 位置ではなく表記で持つ。選んで確定しても学習を送らない
    std::vector<std::wstring> typoSurfaces;
};

// 入力全体の候補 (CONVNBEST の1件)
struct SentenceCandidate {
    // 文節ごとの (読み, 表記)。確定・採用時の学習に使う
    std::vector<std::pair<std::wstring, std::wstring>> segments;
    std::wstring surface;  // 文節の表記の連結
    // 1 打の誤りを戻した読みで変換した補正の候補 (docs/design/typo-conversion.md)。
    // 採用・確定しても学習を送らない
    bool typo = false;
};

// SUGGEST の1候補 (docs/design/unified-suggest.md の E)
struct SuggestCandidate {
    enum class Kind {
        Whole,        // 全体変換 (whole)
        Typo,         // 全体変換の補正の候補 (typo)
        Predict,      // 予測 (predict)
        PredictTypo,  // ずれを 1 つ以上使った予測 (predict_typo)
    };
    Kind kind = Kind::Whole;
    // 全体変換は文節ごとの (読み, 表記)、予測は 1 つの (候補の完全な読み, 表記)
    std::vector<std::pair<std::wstring, std::wstring>> segments;
    std::wstring surface;  // 文節の表記の連結
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

    // かなを文節列に変換する (CONVSEG / 前文脈があれば CONVCTX)。kinds を付けて、補正の表記を
    // typoSurfaces に受け取る。kinds を知らない旧エンジンには付けずに送り直す。
    // エンジンに接続できない・応答が不正な場合は false を返す (呼び出し側でフォールバック)
    bool ConvertSegments(const std::wstring& kana, const ConversionContext& context,
                         std::vector<ConversionSegment>* segments);

    // 文節境界 (各文節の文字数) を固定して再変換する (Shift+←→ の文節伸縮用)
    bool ConvertSegmentsFixed(const std::wstring& kana, const std::vector<size_t>& lengths,
                              const ConversionContext& context,
                              std::vector<ConversionSegment>* segments);

    // かなに対する入力全体の候補を取得する (CONVNBEST)。keys はかなの打鍵
    // (docs/design/unified-suggest.md の A の書式。空なら送らない)。CONVNBEST を知らない旧エンジンには
    // CONVCTX / CONVSEG を送り、各文節の先頭候補を連結した1件だけを返す
    bool ConvertNBest(const std::wstring& kana, const ConversionContext& context,
                      const std::wstring& keys, std::vector<SentenceCandidate>* candidates);

    // ConvertNBest の毎打鍵の候補バー用 (SUGGEST を知らない旧エンジンでバーを作る。打鍵は送らない)。
    // 接続待ちはせず、未接続ならエンジンを起動だけして即 false を返す (Predict と同じ方式)
    bool ConvertNBestLive(const std::wstring& kana, const ConversionContext& context,
                          std::vector<SentenceCandidate>* candidates);

    // 全体変換と予測を 1 つの並びで取得する (SUGGEST。毎打鍵の候補バー用)。keys はかなの打鍵
    // (docs/design/unified-suggest.md の A の書式)。接続待ちはせず、未接続ならエンジンを起動だけして
    // 即 false を返す。SUGGEST を知らない旧エンジンのときも false で、以後 SuggestUnsupported が
    // true になる (候補ゼロは true で candidates が空)
    bool SuggestLive(const std::wstring& kana, const ConversionContext& context,
                     const std::wstring& keys, std::vector<SuggestCandidate>* candidates);

    // 今の接続の相手が SUGGEST を知らないエンジンか (CONVNBEST と PREDICT で代用する)
    bool SuggestUnsupported() const { return suggestUnsupported_ || legacyEngine_; }

    // ASCIISTART の照合方法
    enum class AsciiMatch {
        Prefix,    // 前方一致 (完全一致を含む)
        Exact,     // 完全一致
        ExactAny,  // 長さの制限なしの完全一致
    };

    // かなのかたまりごとの打鍵列から、英字区間を始める要素の位置を問い合わせる (ASCIISTART)。
    // 一致すれば true で、位置を element に入れる。打鍵中に呼ぶため、接続待ちはしない
    // (未接続ならエンジンを起動だけする)。未接続・エラー・ASCIISTART を知らない旧エンジンの
    // ときは一致なし (false) と同じに扱う
    bool AsciiStartLive(const std::vector<std::wstring>& elements, AsciiMatch match,
                        size_t* element);

    // 英字区間の文字列 + 打った文字が英単語の前方一致か (prefix) と、先頭部分のうち英単語と
    // 完全一致するものの長さ (lengths、昇順) を問い合わせる (ASCIIWORD)。打鍵中に呼ぶため、
    // 接続待ちはしない (未接続ならエンジンを起動だけする)。未接続・エラー・ASCIIWORD を
    // 知らない旧エンジンのときは false (呼び出し側は英字区間を閉じない)
    bool AsciiWordLive(const std::wstring& text, bool* prefix, std::vector<size_t>* lengths);

    // 英字だけの文字列の大文字小文字を補正した正しい表記を問い合わせる (ASCIIFORM)。応答を
    // 得られれば true で、表記を form に入れる (表記が無い・text と同じなら空)。打鍵中に呼ぶため、
    // 接続待ちはしない (未接続ならエンジンを起動だけする)。未接続・エラー・ASCIIFORM を
    // 知らない旧エンジンのときは false (呼び出し側は補正の候補を出さない)
    bool AsciiFormLive(const std::wstring& text, std::wstring* form);

    // RERANKGET の結果
    enum class RerankStatus {
        Pending,  // 推論中・順番待ち
        Done,     // 並べ替えた候補がある
        None,     // ID が古い・不明、推論に失敗、LLM が無効
    };

    // かなに対する入力全体の候補の、LLM による並べ替えを依頼する (RERANK)。keys はかなの打鍵
    // (ConvertNBest と同じ。空なら送らない)。毎打鍵の候補バー用に、接続待ちはしない (未接続なら
    // エンジンを起動だけする)。受け付けた ID を返す (0 = 並べ替えをしない。未接続・LLM が無効・
    // RERANK を知らない旧エンジンも 0)
    unsigned long long Rerank(const std::wstring& llmContext, const ConversionContext& context,
                              const std::wstring& kana, const std::wstring& keys);

    // 並べ替えの結果を問い合わせる (RERANKGET)。Done のときは candidates に CONVNBEST と同じ形の
    // 候補を入れる。通信に失敗したら false
    bool RerankGet(unsigned long long id, RerankStatus* status,
                   std::vector<SentenceCandidate>* candidates);

    // 読みに対する記号候補のみを取得する (CONVSYM、F4 の記号変換用)。
    // 通信に成功すれば true (記号が1つも無い場合も true で candidates は空)
    bool ConvertSymbols(const std::wstring& kana, std::vector<std::wstring>* candidates);

    // 読みに対する短縮よみ (ユーザ辞書) の候補のみを取得する (CONVUSER、F5 用)。
    // 通信に成功すれば true (候補が1つも無い場合も true で candidates は空)
    bool ConvertShortcuts(const std::wstring& kana, std::vector<std::wstring>* candidates);

    // 文節ごとの確定結果をエンジンに記録する (LEARN / 文脈があれば LEARN2)。
    // 失敗しても確定処理には影響させない
    bool Learn(const std::vector<LearnEntry>& entries);

    // 人が文節伸縮で直した分割の文節読みをエンジンに記録する (LEARNSEG)。
    // 既定の分割のまま確定したものは送らない (誤った区切りまで境界として強化しないため)
    bool LearnSegmentBoundaries(const std::vector<std::wstring>& readings);

    // 辞書に無い複合語を文節伸縮で割って (まとめて) 入力したとき、その範囲の
    // (連結した読み, 連結した表記) を1語としてエンジンに記録する (LEARNWORD)
    bool LearnWords(const std::vector<std::pair<std::wstring, std::wstring>>& words);

    // 読みの前方一致で予測候補を取得する (PREDICT)。前文脈があれば送り、その後に確定した語を
    // 上位に寄せさせる。毎打鍵で呼ばれるため、接続待ちはせず、未接続ならエンジンを起動だけして
    // 即 false を返す (候補ゼロは true で candidates が空)
    bool Predict(const std::wstring& kana, const ConversionContext& context,
                 std::vector<PredictionCandidate>* candidates);

    // 前文脈の後に確定したことのある語を取得する (NEXTWORD。読みなしの後続語予測)。
    // Predict と同じく接続待ちはしない。未接続・前文脈が空・NEXTWORD を
    // 知らない旧エンジンのときは false (候補ゼロは true で candidates が空)
    bool NextWord(const ConversionContext& context, std::vector<PredictionCandidate>* candidates);

    // (読み, 表記) の組の学習をエンジンから消す (FORGET)。消した件数を count に入れる
    // (0 なら学習でない候補だった)。キー操作の応答を止めないよう、エンジンの自動起動・
    // 接続待ちはしない。未接続・エラー・FORGET を知らない旧エンジンのときは false
    bool Forget(const std::vector<std::pair<std::wstring, std::wstring>>& pairs, size_t* count);

    // 表記 text から読みを推測し、語ごとの (読み, 表記) を表記の順に words に入れる (REVERSE。
    // docs/design/selection-reconversion.md)。変換キーを食べるかの判定で呼ぶため、エンジンの
    // 自動起動・接続待ちはしない。未接続・ERR (読めない文字・逆引きの索引を作っている途中)・
    // REVERSE を知らない旧エンジン・表記の連結が text と食い違う応答のときは false
    bool Reverse(const std::wstring& text,
                 std::vector<std::pair<std::wstring, std::wstring>>* words);

    // パイプに接続してみて、開けなければエンジンを起動する。起動の完了は待たない
    // (最初の変換より前にエンジンを立ち上げておくため、アクティブ化のときに呼ぶ)
    void StartEngine();

    // 同梱 exe (エンジン・単語登録ツール) のパスを探す。
    // DLL と同じディレクトリ → 開発レイアウト (engine/target) の順。見つからなければ空
    static std::wstring FindExePath(const wchar_t* exeName);

private:
    bool EnsureConnected();
    // パイプを1回だけ開いてみる (起動待ちはしない)
    bool TryOpenPipe();
    // 打鍵ごとの問い合わせ用。未接続ならパイプを1回だけ開いてみて、開けなければ
    // エンジンを起動だけして false を返す (接続は待たない)
    bool OpenPipeLive();
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
    // live = true ならエンジンの起動を待たない
    bool TransactWithFallback(const std::string& request, const std::string& fallback,
                              bool live, std::string* response);
    // CONVSEG 系の要求 (+旧コマンド fallback) を送って応答を segments にパースする
    bool RequestSegments(const std::string& request, const std::string& fallback, bool live,
                         std::vector<ConversionSegment>* segments);
    // CONVSEG 系の応答1行を segments にパースする
    bool ParseSegmentsResponse(std::string response, std::vector<ConversionSegment>* segments);
    // CONVNBEST (旧エンジンには CONVCTX / CONVSEG) を送って入力全体の候補を得る
    bool RequestNBest(const std::wstring& kana, const ConversionContext& context,
                      const std::wstring& keys, bool live,
                      std::vector<SentenceCandidate>* candidates);
    // CONVNBEST の応答1行を candidates にパースする
    bool ParseNBestResponse(std::string response, std::vector<SentenceCandidate>* candidates);
    // SUGGEST の応答1行を candidates にパースする
    bool ParseSuggestResponse(std::string response, std::vector<SuggestCandidate>* candidates);
    // PREDICT・NEXTWORD の要求を現在の接続で送り、応答を candidates にパースする
    // (未接続ならパイプを1回だけ開いてみて、開けなければエンジンを起動だけする)
    bool RequestPredictions(const std::string& request,
                            std::vector<PredictionCandidate>* candidates);

    HANDLE pipe_ = INVALID_HANDLE_VALUE;
    ULONGLONG lastLaunchTick_ = 0; // 最後にエンジン起動を試みた時刻 (連続起動の抑止)
    // 最後にエンジンのプロセスを起動できた時刻。そこから一定時間は起動中とみなし、
    // 新たに起動せずにパイプができるのを待つ (起動できなかった試行は含めない)
    ULONGLONG launchedTick_ = 0;
    // 旧エンジン (CONVCTX/LEARN2 未対応) と判定したら以後は旧コマンドのみ送る
    bool legacyEngine_ = false;
    // CONVNBEST を知らないエンジンと判定したら以後は CONVCTX / CONVSEG で代用する
    // (CONVCTX には対応しているエンジンがあるため legacyEngine_ とは別に持つ)
    bool nbestUnsupported_ = false;
    // RERANK を知らないエンジンと判定したら、この接続では以後送らない
    bool rerankUnsupported_ = false;
    // SUGGEST を知らないエンジンと判定したら、この接続では以後送らない
    bool suggestUnsupported_ = false;
    // CONVSEG / CONVCTX の kinds を知らない (文節長として読んで ERR を返す) エンジンと判定したら、
    // この接続では以後 kinds を付けない
    bool segmentKindsUnsupported_ = false;
};
