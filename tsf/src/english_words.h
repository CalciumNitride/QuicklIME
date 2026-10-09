#pragma once

// モードレス入力の自動英字判定ルール7 の語リスト (docs/design/modeless-detection.md)。
// ローマ字として読めるが日本語にならず、日本語を打つ場面がほぼない英単語だけに絞っている。
// SCOWL の語から、4文字以上で既定のローマ字テーブルで全体がかなになり、かなが Mozc 辞書の読みに
// 無いものを拾い、日本語の打鍵と同じ綴りになりうる語と、標準のローマ字 (l・v・q・x・c・fu 以外の
// f を含まない綴り) だけで打てる語を外して選んだ (remote は誤判定の余地が小さいので残している)。
// 照合は二分探索なので、英小文字のバイト順に並べておくこと

namespace english_words {

inline constexpr const wchar_t* kWords[] = {
    L"above", L"before", L"believe", L"define", L"delete", L"evaluate", L"everyone", L"examine",
    L"failure", L"favorite", L"feature", L"figure", L"file", L"five", L"give", L"have",
    L"initialize", L"invite", L"language", L"leave", L"life", L"like", L"line", L"live", L"love",
    L"mobile", L"module", L"move", L"movie", L"negative", L"override", L"pipeline", L"positive",
    L"queue", L"quite", L"quote", L"release", L"remote", L"remove", L"require", L"resolution",
    L"revision", L"rule", L"seven", L"solution", L"television", L"unique", L"value", L"vision",
    L"volume", L"vote", L"wave", L"while", L"whole",
};

} // namespace english_words
