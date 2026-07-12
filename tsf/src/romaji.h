#pragma once

#include <string>

// ローマ字入力の途中状態 (未確定の子音) を保持する最小バッファ。
// フェーズ1では composition を持たないため、かなが確定した時点で
// 即座に文字列を返す。フェーズ2で本格的な変換テーブルに置き換える。
class RomajiBuffer {
public:
    // 英小文字を1文字受け取り、確定したかな文字列を返す (未確定なら空文字列)
    std::wstring Push(wchar_t c);

    // 入力途中の子音を破棄する
    void Clear();

private:
    std::wstring pending_;
};
