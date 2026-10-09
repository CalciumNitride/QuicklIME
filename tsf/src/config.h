#pragma once

#include <windows.h>

#include <array>
#include <string>
#include <vector>

// キー割当を変えられる機能 (docs/design/keymap.md)。コア操作 (Enter/Esc/Tab/矢印など) と
// 印字キーは対象外。並び順は、同じ状態で同じキーが重なったときの優先順位を兼ねる
enum class KeyFunc {
    Convert,         // 変換・次候補・後置再変換 (既定 変換キー)
    NextCandidate,   // 次候補 (既定 Space)
    PrevCandidate,   // 前候補 (既定 Shift+Space)
    CommitRun,       // 確定 (既定 無変換キー)
    ConvertSymbol,   // 記号・日付変換 (既定 F4)
    ConvertUser,     // ユーザ登録語変換 (既定 F5)
    ToHiragana,      // ひらがな変換 (既定 F6)
    ToKatakana,      // カタカナ変換 (既定 F7)
    ToHalfKatakana,  // 半角カタカナ変換 (既定 F8)
    ToFullAscii,     // 全角英字変換 (既定 F9)
    ToHalfAscii,     // 半角英字変換 (既定 F10)
    UndoCommit,      // 確定アンドゥ (既定 Ctrl+Backspace)
    RegisterWord,    // 単語登録ツール起動 (既定 Ctrl+F7)
    OpenConfig,      // 設定ツール起動 (既定 Ctrl+F12)
    None,            // 割当なし (照合の「該当なし」も表す)
};

constexpr size_t kKeyFuncCount = static_cast<size_t>(KeyFunc::None);

// キー割当の照合に使う入力状態
enum class KeyState {
    Idle,       // 入力なし (run が無い)
    Run,        // run 中 (候補選択中でもサジェスト選択中でもない)
    Candidate,  // 候補選択中 (composition に昇格しているかは問わない)
    Suggest,    // サジェスト選択中 (候補バーで候補を選んでいる)
};

constexpr size_t kKeyStateCount = 4;

// 修飾キーの組と仮想キー
struct KeyBinding {
    bool ctrl = false;
    bool alt = false;
    bool shift = false;
    UINT vk = 0;

    bool operator==(const KeyBinding&) const = default;
};

// 1機能のキー割当。overridden[state] が立っている状態では keys の代わりに
// overrides[state] を使う (空なら、その状態では割当なし)
struct KeyAssignment {
    std::vector<KeyBinding> keys;
    std::array<bool, kKeyStateCount> overridden = {};
    std::array<std::vector<KeyBinding>, kKeyStateCount> overrides;
};

// 照合の結果
struct KeyMatch {
    KeyFunc func = KeyFunc::None;
    // Convert の割当に Shift を足した打鍵で一致した (候補選択中は前候補)
    bool shiftAdded = false;
};

// 機能が働く状態か
bool KeyFuncWorksIn(KeyFunc func, KeyState state);

// ユーザ設定 (config.tsv) のうち TSF 層で使う項目。
// エンジン向けのキー (learning, suggest など) はエンジンが同じファイルを読む
struct TsfConfig {
    // 診断用ログの出力先 (隠し設定 debug_log。空なら出さない)
    std::wstring debugLog;
    bool spaceFullwidth = true;    // run が無い Space で全角スペースを入れる
    bool digitsFullwidth = false;  // 数字キー・テンキーの数字を全角で入れる
    std::wstring punctComma = L"、";   // 読点 (VK_OEM_COMMA の非 Shift)
    std::wstring punctPeriod = L"。";  // 句点 (VK_OEM_PERIOD の非 Shift)
    std::wstring candidateFont = L"Yu Gothic UI";  // 候補ウィンドウのフォント名
    int candidateFontSize = 18;    // 候補ウィンドウのフォントの高さ (px)
    // モードレス入力 (英語の打鍵を自動で判定して英字のまま入れる。
    // docs/design/modeless.md)
    bool modeless = false;
    // 入力中に横一列の候補バーを出す (docs/design/candidate-bar.md)
    bool candidateBar = true;
    // 候補バーを出す確定済みかなの文字数の下限 (エンジンの PREDICT と同じ設定を読む)
    int minSuggestChars = 2;
    // 変換キーの候補選択を文節単位 (文節移動・伸縮・文節別の候補) にする。
    // オフなら入力全体の候補から選ぶ (docs/design/nbest.md)
    bool segmentUi = false;
    // LLM による候補の並べ替え (RERANK) を使う (docs/design/llm-rerank.md)。
    // バックエンドの選択 (llm_backend) はエンジンだけが読む
    bool llm = false;

    // 機能ごとのキー割当 (KeyFunc の並び順)
    std::array<KeyAssignment, kKeyFuncCount> keys = DefaultKeyAssignments();

    // state で打鍵 pressed に割り当てられた機能。状態別の上書きを基本の割当より優先し、
    // 重なれば KeyFunc の並び順で先の機能を採る。Convert の割当は Shift を足した打鍵にも
    // 一致する (明示的な割当がある打鍵ではそちらを優先する)
    KeyMatch FindFunc(KeyState state, const KeyBinding& pressed) const;

    static std::array<KeyAssignment, kKeyFuncCount> DefaultKeyAssignments();
};

// config.tsv (QUICKLIME_CONFIG_FILE > %APPDATA%\QuicklIME\config.tsv) のローダ。
// 書き込みは設定ツール (quicklime-config.exe) が行い、TSF 層は読むだけ。
// 形式は「キー\t値」の行ベース TSV (UTF-8、# 始まりはコメント)。
// 未知キーは無視し、不正な値はそのキーだけ既定値のまま (エラーにしない)。
// TextService (スレッドごとに1インスタンス) が所有するためロックは不要
class ConfigLoader {
public:
    // ファイルの更新時刻を確認し、初回と変更時だけ読み直して true を返す。
    // フォーカス切替などの軽いタイミングで毎回呼べる (通常は時刻比較のみ)。
    // ファイルが無い・読めない場合は既定値になる
    bool Refresh();

    const TsfConfig& Get() const { return config_; }

private:
    TsfConfig config_;
    FILETIME lastWrite_ = {};
    bool loaded_ = false;
};
