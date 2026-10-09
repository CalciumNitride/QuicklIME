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
    // Clear() (composition の終了) か、Backspace で英字区間が空になるまで維持される。
    // 打った文字から英字であることが明らかなので、その位置を英字区間の開始位置にする
    // (未変換ローマ字は PushKana と同じ規則で先に日本語区間へ救済する)
    void EnterAsciiMode();
    bool AsciiMode() const { return asciiMode_; }
    // 英字区間の開始位置 (確定済みかなの位置)。これより前は日本語区間で、0 なら run 全体が英字。
    // 英字モードでなければ 0
    size_t AsciiStart() const { return asciiStart_; }

    // モードレス入力 (設定 modeless) の有効/無効。無効なら自動英字判定は一切働かない。
    // 入力ごとに設定を渡し直さなくて済むよう、Clear() ではこのフラグを維持する
    void SetModeless(bool enabled) { modeless_ = enabled; }

    // 自動英字判定のうち、確定する直前にだけ効く判定。打ち途中で暴発しないよう、
    // run / composition を無変換のまま確定する経路 (と語の区切りになる記号キー) からのみ呼ぶ
    // - ルール3: 未変換ローマ字として n 以外の英小文字が1文字残っていれば英字と判定する
    //   (「want」の t)。境界は英単語辞書で決めるので要求にする
    // - ルール7: 打鍵列の全体、または末尾 (かなの境目から始まる5文字以上) が語リストの語と
    //   一致すれば、その語の頭から英字にする
    // - ルール6: afterAsciiCommit (直前の確定の末尾が ASCII 英字) なら、run の打鍵列が英単語辞書と
    //   完全一致するかを要求にする (英文の途中の「pen」)。助詞などと同じ形の語は除き、
    //   1文字の語は a と i だけをその場で英字にする
    void FinishForCommit(bool afterAsciiCommit);

    // 自動英字判定 (ルール1〜6) が成立し、英単語辞書での照合を待っている。
    // かなと未変換ローマ字は判定の時点のまま (根拠の打鍵は未変換ローマ字に残っている) なので、
    // Push・FinishForCommit の直後に呼び出し側が ResolveAscii で決着させる
    bool AsciiRequested() const { return asciiRequest_ != AsciiRequest::None; }
    // 要求が run 終了時の判定 (ルール3・6) によるものか。打ち終わった語なので、英単語辞書とは
    // 完全一致で照合する (打鍵中の判定 (ルール1・2・4・5) は打ち途中なので前方一致)
    bool AsciiRequestAtCommit() const
    {
        return asciiRequest_ == AsciiRequest::Commit || asciiRequest_ == AsciiRequest::Context;
    }
    // 要求がルール6 によるものか。英文中の短い語 (is・an) を拾うため、長さの制限を外して照合する
    bool AsciiRequestAnyLength() const { return asciiRequest_ == AsciiRequest::Context; }
    // 境界の候補: かなのかたまり (打鍵列を持つかな。「きょ」は1かたまり) ごとの打鍵列と、
    // 未変換ローマ字 (あれば最後の要素)。ASCIISTART にそのまま送る
    std::vector<std::wstring> AsciiRequestElements() const;
    // 英単語辞書での照合の結果 (matched なら AsciiRequestElements の element 番目の要素から
    // 一致) で要求を決着させる。一致しなかったときは判定の種類で扱いが変わる:
    // 根拠の強い判定 (ルール1〜3) は run 全体を英字にし、根拠の弱い判定 (ルール4・5) は
    // 英字にせずにかなへの変換を続け、ルール6 は先頭から一致したときだけ run 全体を英字にし、
    // c で始まる未変換ローマ字の保留中は保留を続ける
    void ResolveAscii(bool matched, size_t element);

    // 末尾の1文字を削除する (未変換ローマ字があればそちらを優先)。
    // モードレス入力が有効なときは、英字区間が空になったら英字モードを解除し、
    // 日本語区間の末尾から入力を続ける
    void Backspace();

    // 確定済みのかなの先頭 count 文字と、対応する打鍵列を取り除く
    // (候補バーで採用した部分を読みから外す。未変換ローマ字と英字モードは維持し、
    // 英字区間の開始位置は取り除いた分だけ前へずらす)
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

    // 未変換ローマ字を確定と同じ規則でかな列へ移す ("n" のみ「ん」へ救済し、残りはそのまま)
    void FlushPending();

    // 自動英字判定の成立時: かなの start 文字目以降の打鍵列 (+ 未変換ローマ字) を1文字ずつ
    // そのままかな列に置き直して英字モードへ移る。start より前 (日本語区間) は残す。
    // 以降は Shift+英字で入った英字モードと同じ扱い
    void SwitchToAscii(size_t start);

    // 根拠の弱い判定の綴りを run が含むか: ルール4 (c の直後に母音、打鍵列が5文字以上。
    // ローマ字テーブルに c 行のかながあるときだけ) とルール5 (th + 母音。thi を除く)
    bool HasWeakAsciiSpelling() const;

    // かなのかたまり (AsciiRequestElements の要素) ごとの先頭のかなの位置。
    // 未変換ローマ字の要素は kana_.size() とする
    std::vector<size_t> ElementPositions() const;

    std::wstring kana_;            // 確定済みのかな
    std::vector<std::wstring> raw_; // kana_ の各文字に対応する打鍵列
    std::wstring pending_;          // 未変換のローマ字
    bool asciiMode_ = false;        // 英字モード (Shift+英字以降はアルファベットのまま)
    size_t asciiStart_ = 0;         // 英字区間の開始位置 (kana_ の位置。英字モードでなければ 0)
    // 自動英字判定が成立し、ResolveAscii を待っている判定の種類
    enum class AsciiRequest {
        None,
        Strong,   // 打鍵中の根拠の強い判定 (ルール1・2)
        Weak,     // 打鍵中の根拠の弱い判定 (ルール4・5)
        Commit,   // run 終了時の判定 (ルール3)
        Context,  // 直前の確定が英字 (ルール6)
        Hold,     // c で始まる未変換ローマ字の保留中に、3文字で英単語と完全一致するか
    };
    AsciiRequest asciiRequest_ = AsciiRequest::None;
    bool modeless_ = false;         // モードレス入力 (自動英字判定) が有効か
};
