#include "romaji.h"

#include <map>

namespace {

// フェーズ1用の最小ローマ字→かなテーブル (清音・濁音・半濁音の基本行のみ)
const std::map<std::wstring, std::wstring>& Table()
{
    static const std::map<std::wstring, std::wstring> table = {
        {L"a", L"あ"},  {L"i", L"い"},  {L"u", L"う"},  {L"e", L"え"},  {L"o", L"お"},
        {L"ka", L"か"}, {L"ki", L"き"}, {L"ku", L"く"}, {L"ke", L"け"}, {L"ko", L"こ"},
        {L"ga", L"が"}, {L"gi", L"ぎ"}, {L"gu", L"ぐ"}, {L"ge", L"げ"}, {L"go", L"ご"},
        {L"sa", L"さ"}, {L"si", L"し"}, {L"su", L"す"}, {L"se", L"せ"}, {L"so", L"そ"},
        {L"za", L"ざ"}, {L"zi", L"じ"}, {L"zu", L"ず"}, {L"ze", L"ぜ"}, {L"zo", L"ぞ"},
        {L"ta", L"た"}, {L"ti", L"ち"}, {L"tu", L"つ"}, {L"te", L"て"}, {L"to", L"と"},
        {L"da", L"だ"}, {L"di", L"ぢ"}, {L"du", L"づ"}, {L"de", L"で"}, {L"do", L"ど"},
        {L"na", L"な"}, {L"ni", L"に"}, {L"nu", L"ぬ"}, {L"ne", L"ね"}, {L"no", L"の"},
        {L"ha", L"は"}, {L"hi", L"ひ"}, {L"hu", L"ふ"}, {L"he", L"へ"}, {L"ho", L"ほ"},
        {L"ba", L"ば"}, {L"bi", L"び"}, {L"bu", L"ぶ"}, {L"be", L"べ"}, {L"bo", L"ぼ"},
        {L"pa", L"ぱ"}, {L"pi", L"ぴ"}, {L"pu", L"ぷ"}, {L"pe", L"ぺ"}, {L"po", L"ぽ"},
        {L"ma", L"ま"}, {L"mi", L"み"}, {L"mu", L"む"}, {L"me", L"め"}, {L"mo", L"も"},
        {L"ya", L"や"}, {L"yu", L"ゆ"}, {L"yo", L"よ"},
        {L"ra", L"ら"}, {L"ri", L"り"}, {L"ru", L"る"}, {L"re", L"れ"}, {L"ro", L"ろ"},
        {L"wa", L"わ"}, {L"wo", L"を"},
        {L"nn", L"ん"},
    };
    return table;
}

bool IsVowel(wchar_t c)
{
    return c == L'a' || c == L'i' || c == L'u' || c == L'e' || c == L'o';
}

} // namespace

std::wstring RomajiBuffer::Push(wchar_t c)
{
    const auto& table = Table();

    // 保持中の子音 + 今回の文字でテーブルを引く (例: "k" + 'a' -> か)
    std::wstring combo = pending_ + c;
    if (auto it = table.find(combo); it != table.end()) {
        pending_.clear();
        return it->second;
    }

    // 組み合わせが無く母音単独なら、未知の子音を捨てて母音だけ確定する
    if (IsVowel(c)) {
        pending_.clear();
        auto it = table.find(std::wstring(1, c));
        return it != table.end() ? it->second : std::wstring();
    }

    // 子音が続いた場合: "n" の後に別の子音が来たら「ん」を確定する (例: kanto -> かんと)
    std::wstring out;
    if (pending_ == L"n") {
        out = L"ん";
    }
    pending_ = c;
    return out;
}

void RomajiBuffer::Clear()
{
    pending_.clear();
}
