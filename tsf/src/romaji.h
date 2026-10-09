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

    // モードレス入力 (設定 modeless) の有効/無効。無効なら自動英字判定は一切働かない。
    // 入力ごとに設定を渡し直さなくて済むよう、Clear() ではこのフラグを維持する
    void SetModeless(bool enabled) { modeless_ = enabled; }

    // 自動英字判定のうち、確定する直前にだけ効く判定 (判定ルール3):
    // 未変換ローマ字として n 以外の英小文字が1文字残っていれば英字と判定する
    // (「want」の t)。打ち途中で暴発しないよう、run / composition を無変換のまま
    // 確定する経路からのみ呼ぶ
    void FinishForCommit();

    // 末尾の1文字を削除する (未変換ローマ字があればそちらを優先)
    void Backspace();

    // 確定済みのかなの先頭 count 文字と、対応する打鍵列を取り除く
    // (候補バーで採用した部分を読みから外す。未変換ローマ字と英字モードは維持する)
    void RemoveFront(size_t count);

    void Clear();
    bool Empty() const;

    // composition 表示用: 確定済みかな + 未変換ローマ字
    std::wstring Display() const;

    // 確定済みのかな (未変換ローマ字を含まない。ライブ変換の変換対象)
    const std::wstring& ConfirmedKana() const { return kana_; }

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

    // 自動英字判定の成立時: ここまでの打鍵列 (Raw()) を1文字ずつそのままかな列に
    // 置き直して英字モードへ移る。以降は Shift+英字で入った英字モードと同じ扱い
    void SwitchToAscii();

    std::wstring kana_;             // 確定済みのかな
    std::vector<std::wstring> raw_; // kana_ の各文字に対応する打鍵列
    std::wstring pending_;          // 未変換のローマ字
    bool asciiMode_ = false;        // 英字モード (Shift+英字以降はアルファベットのまま)
    bool modeless_ = false;         // モードレス入力 (自動英字判定) が有効か
};
