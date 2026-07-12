#pragma once

#include <string>

// ローマ字入力を逐次かなへ変換するコンポーザ。
// 「確定済みのかな列」と「まだ変換できない未変換ローマ字列」を保持し、
// composition にはこの2つを連結した文字列を表示する。
class RomajiComposer {
public:
    // 英小文字を1文字受け取り、変換を進める
    void Push(wchar_t c);

    // かな1文字を直接追加する (ー 、 。 など記号キー用)。
    // 未変換ローマ字が残っていれば先に確定処理をしてから追加する
    void PushKana(const std::wstring& kana);

    // 末尾の1文字を削除する (未変換ローマ字があればそちらを優先)
    void Backspace();

    void Clear();
    bool Empty() const;

    // composition 表示用: 確定済みかな + 未変換ローマ字
    std::wstring Display() const;

    // 確定用文字列: 未変換ローマ字は "n" のみ「ん」へ救済し、残りはそのまま付ける
    std::wstring Commit() const;

private:
    // 未変換ローマ字の先頭を可能な限りかなへ変換する
    void Convert();

    std::wstring kana_;    // 確定済みのかな
    std::wstring pending_; // 未変換のローマ字
};

// ひらがなをカタカナに変換する (対象外の文字はそのまま)
std::wstring ToKatakana(const std::wstring& kana);
