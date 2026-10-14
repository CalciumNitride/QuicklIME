#pragma once

#include <string>
#include <utility>
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

    // 自動英字判定のうち、run を終えたら成り立つ判定。打ち途中で暴発しないよう、
    // 候補バーの英字候補を作るときに複製に対してだけ呼ぶ (読みそのものは書き換えない)
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
    // 未変換ローマ字の保留中 (c で始まるもの、ルール1 で未変換ローマ字が4文字未満のもの) は保留を続ける
    void ResolveAscii(bool matched, size_t element);

    // 開いている英字区間の文字列 (英字モードでなければ空)
    std::wstring AsciiSection() const;
    // 英字区間の先頭 length 文字で区間を閉じ、残り + c を日本語区間の打鍵として打ち直せるか:
    // 残り + c が、自動英字判定を止めたローマ字の照合で素通しの英字を出さずに読めること
    bool CanCloseAscii(size_t length, wchar_t c) const;
    // 英字区間を先頭 length 文字で閉じ、残りを自動英字判定を止めて打ち直してから c を通常どおり
    // Push する (docs/design/mixed-conversion.md)。c で自動英字判定の要求が出ることがあるので、
    // 呼び出し側は Push と同じく直後に ResolveAscii で決着させる
    void CloseAscii(size_t length, wchar_t c);
    // 閉じた英字区間があるか
    bool HasClosedAscii() const { return !closedAscii_.empty(); }
    // 英字区間 (開いている・閉じたもの) と、どの英字区間にも入らない日本語区間の両方がある run か
    bool MixedAscii() const;

    // 末尾の1文字を削除する (未変換ローマ字があればそちらを優先)。
    // モードレス入力が有効なときは、英字区間が空になったら英字モードを解除し、
    // 日本語区間の末尾から入力を続ける。閉じた位置まで戻ったら、最後に閉じた英字区間を開き直す。
    // 自動判定で英字にした後、判定のきっかけの打鍵まで消したら判定前の日本語の状態に戻す。
    // 消した後は残った未変換ローマ字に打鍵中の判定をやり直すので、呼び出し側は Push と同じく
    // 直後に ResolveAscii で決着させる
    void Backspace();
    // Backspace で判定前の日本語の状態に戻るか (文書の英字を IME が書き換える必要がある)
    bool BackspaceRestores() const;

    // 確定済みのかなの先頭 count 文字と、対応する打鍵列を取り除く
    // (候補バーで採用した部分を読みから外す。未変換ローマ字と英字モードは維持し、
    // 英字区間の開始位置と閉じた英字区間の位置は取り除いた分だけ前へずらす)
    void RemoveFront(size_t count);

    // 確定済みのかなの先頭に kana を足す。打鍵列は文字そのもの (文書から読んだかなで変換の範囲を
    // 左へ広げる。RemoveFront の逆で、英字区間の開始位置と閉じた英字区間の位置は足した分だけ
    // 後ろへずらす)
    void PrependKana(const std::wstring& kana);

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

    // 閉じた位置 (最後に閉じた英字区間の終わりの kana_ の位置。無ければ 0)。
    // これより前は自動英字判定の対象外
    size_t ClosedEnd() const { return closedAscii_.empty() ? 0 : closedAscii_.back().second; }
    // 閉じた位置より後ろの打鍵列 (+ 未変換ローマ字)
    std::wstring RawAfterClosed() const;
    // 閉じた位置より後ろのかなと未変換ローマ字が空なら、最後に閉じた英字区間を開き直す
    void ReopenClosedAscii();
    // Backspace の1回分 (判定前の状態への復元と判定のやり直しを除く)
    void BackspaceStep();

    std::wstring kana_;            // 確定済みのかな
    std::vector<std::wstring> raw_; // kana_ の各文字に対応する打鍵列
    std::wstring pending_;          // 未変換のローマ字
    bool asciiMode_ = false;        // 英字モード (Shift+英字以降はアルファベットのまま)
    size_t asciiStart_ = 0;         // 英字区間の開始位置 (kana_ の位置。英字モードでなければ 0)
    // 閉じた英字区間の (開始位置, 終わり) の積み (kana_ の位置。前から順)。Backspace で開き直すと
    // 1つ前の区間の終わりが閉じた位置に戻るため、最後の1つだけでなく積んで持つ
    std::vector<std::pair<size_t, size_t>> closedAscii_;
    // 自動判定で英字に切り替える直前の読みの状態と、そのときの打鍵数 (Raw() の文字数) の積み
    struct Snapshot {
        std::wstring kana;
        std::vector<std::wstring> raw;
        std::wstring pending;
        bool asciiMode;
        size_t asciiStart;
        std::vector<std::pair<size_t, size_t>> closedAscii;
    };
    struct AutoSwitch {
        Snapshot state;
        size_t rawCount;
    };
    std::vector<AutoSwitch> autoSwitches_;
    // 自動英字判定が成立し、ResolveAscii を待っている判定の種類
    enum class AsciiRequest {
        None,
        Strong,   // 打鍵中の根拠の強い判定 (ルール2、未変換ローマ字が4文字以上のルール1)
        Weak,     // 打鍵中の根拠の弱い判定 (ルール4・5)
        Commit,   // run 終了時の判定 (ルール3)
        Context,  // 直前の確定が英字 (ルール6)
        // 未変換ローマ字の保留中 (c で始まるもの・ルール1 で辞書に一致しなかったもの) に英単語と
        // 一致するか。一致しなければ保留を続ける
        Hold,
    };
    AsciiRequest asciiRequest_ = AsciiRequest::None;
    bool modeless_ = false;         // モードレス入力 (自動英字判定) が有効か
};
