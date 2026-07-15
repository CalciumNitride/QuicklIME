#pragma once

#include <string>
#include <vector>

// ローマ字入力を逐次かなへ変換するコンポーザ。
// 「確定済みのかな列」と「まだ変換できない未変換ローマ字列」を保持し、
// composition にはこの2つを連結した文字列を表示する。
// かな1文字ごとに由来の打鍵列も保持し、打鍵をそのまま候補に出す
// 生ローマ字候補 (英単語入力用) に使う。
class RomajiComposer {
public:
    // 英小文字を1文字受け取り、変換を進める
    void Push(wchar_t c);

    // かな1文字を直接追加する (ー 、 。 など記号キー用)。
    // raw にはそのキーの打鍵文字 (「-」など) を渡す。
    // 未変換ローマ字が残っていれば先に確定処理をしてから追加する
    void PushKana(const std::wstring& kana, const std::wstring& raw);

    // 英字モードに入る (Shift+英字での大文字入力時)。
    // 以降の入力をローマ字変換せずアルファベットのまま続けるための状態で、
    // Clear() (composition の終了) まで維持される
    void EnterAsciiMode() { asciiMode_ = true; }
    bool AsciiMode() const { return asciiMode_; }

    // 末尾の1文字を削除する (未変換ローマ字があればそちらを優先)
    void Backspace();

    void Clear();
    bool Empty() const;

    // composition 表示用: 確定済みかな + 未変換ローマ字
    std::wstring Display() const;

    // 確定用文字列: 未変換ローマ字は "n" のみ「ん」へ救済し、残りはそのまま付ける
    std::wstring Commit() const;

    // 打鍵した文字列そのもの (生ローマ字候補用)
    std::wstring Raw() const;

    // Commit() が返す文字列の [pos, pos+len) 区間に対応する打鍵列。
    // 文節単位の英数変換 (F9/F10) 用
    std::wstring RawRange(size_t pos, size_t len) const;

private:
    // 未変換ローマ字の先頭を可能な限りかなへ変換する
    void Convert();

    // かなを1かたまり追加する。raw (由来の打鍵列) は先頭のかな文字に対応付け、
    // 2文字目以降には空を対応付ける (Backspace はかな1文字単位のため)
    void AppendKana(const std::wstring& kana, const std::wstring& raw);

    std::wstring kana_;             // 確定済みのかな
    std::vector<std::wstring> raw_; // kana_ の各文字に対応する打鍵列
    std::wstring pending_;          // 未変換のローマ字
    bool asciiMode_ = false;        // 英字モード (Shift+英字以降はアルファベットのまま)
};
