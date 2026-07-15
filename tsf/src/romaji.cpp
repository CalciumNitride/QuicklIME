#include "romaji.h"

#include <map>

namespace {

// ローマ字→かな変換テーブル (MS-IME 準拠の一般的なサブセット)
const std::map<std::wstring, std::wstring>& Table()
{
    static const std::map<std::wstring, std::wstring> table = {
        // 母音
        {L"a", L"あ"}, {L"i", L"い"}, {L"u", L"う"}, {L"e", L"え"}, {L"o", L"お"},
        // か行
        {L"ka", L"か"}, {L"ki", L"き"}, {L"ku", L"く"}, {L"ke", L"け"}, {L"ko", L"こ"},
        {L"kya", L"きゃ"}, {L"kyi", L"きぃ"}, {L"kyu", L"きゅ"}, {L"kye", L"きぇ"}, {L"kyo", L"きょ"},
        {L"qa", L"くぁ"}, {L"qi", L"くぃ"}, {L"qu", L"く"}, {L"qe", L"くぇ"}, {L"qo", L"くぉ"},
        {L"ga", L"が"}, {L"gi", L"ぎ"}, {L"gu", L"ぐ"}, {L"ge", L"げ"}, {L"go", L"ご"},
        {L"gya", L"ぎゃ"}, {L"gyi", L"ぎぃ"}, {L"gyu", L"ぎゅ"}, {L"gye", L"ぎぇ"}, {L"gyo", L"ぎょ"},
        // さ行
        {L"sa", L"さ"}, {L"si", L"し"}, {L"su", L"す"}, {L"se", L"せ"}, {L"so", L"そ"},
        {L"sya", L"しゃ"}, {L"syi", L"しぃ"}, {L"syu", L"しゅ"}, {L"sye", L"しぇ"}, {L"syo", L"しょ"},
        {L"sha", L"しゃ"}, {L"shi", L"し"}, {L"shu", L"しゅ"}, {L"she", L"しぇ"}, {L"sho", L"しょ"},
        {L"za", L"ざ"}, {L"zi", L"じ"}, {L"zu", L"ず"}, {L"ze", L"ぜ"}, {L"zo", L"ぞ"},
        {L"zya", L"じゃ"}, {L"zyi", L"じぃ"}, {L"zyu", L"じゅ"}, {L"zye", L"じぇ"}, {L"zyo", L"じょ"},
        {L"ja", L"じゃ"}, {L"ji", L"じ"}, {L"ju", L"じゅ"}, {L"je", L"じぇ"}, {L"jo", L"じょ"},
        {L"jya", L"じゃ"}, {L"jyi", L"じぃ"}, {L"jyu", L"じゅ"}, {L"jye", L"じぇ"}, {L"jyo", L"じょ"},
        // た行
        {L"ta", L"た"}, {L"ti", L"ち"}, {L"tu", L"つ"}, {L"te", L"て"}, {L"to", L"と"},
        {L"tya", L"ちゃ"}, {L"tyi", L"ちぃ"}, {L"tyu", L"ちゅ"}, {L"tye", L"ちぇ"}, {L"tyo", L"ちょ"},
        {L"cha", L"ちゃ"}, {L"chi", L"ち"}, {L"chu", L"ちゅ"}, {L"che", L"ちぇ"}, {L"cho", L"ちょ"},
        {L"tsa", L"つぁ"}, {L"tsi", L"つぃ"}, {L"tsu", L"つ"}, {L"tse", L"つぇ"}, {L"tso", L"つぉ"},
        {L"tha", L"てゃ"}, {L"thi", L"てぃ"}, {L"thu", L"てゅ"}, {L"the", L"てぇ"}, {L"tho", L"てょ"},
        {L"da", L"だ"}, {L"di", L"ぢ"}, {L"du", L"づ"}, {L"de", L"で"}, {L"do", L"ど"},
        {L"dya", L"ぢゃ"}, {L"dyi", L"ぢぃ"}, {L"dyu", L"ぢゅ"}, {L"dye", L"ぢぇ"}, {L"dyo", L"ぢょ"},
        {L"dha", L"でゃ"}, {L"dhi", L"でぃ"}, {L"dhu", L"でゅ"}, {L"dhe", L"でぇ"}, {L"dho", L"でょ"},
        // な行
        {L"na", L"な"}, {L"ni", L"に"}, {L"nu", L"ぬ"}, {L"ne", L"ね"}, {L"no", L"の"},
        {L"nya", L"にゃ"}, {L"nyi", L"にぃ"}, {L"nyu", L"にゅ"}, {L"nye", L"にぇ"}, {L"nyo", L"にょ"},
        {L"nn", L"ん"},
        // は行
        {L"ha", L"は"}, {L"hi", L"ひ"}, {L"hu", L"ふ"}, {L"he", L"へ"}, {L"ho", L"ほ"},
        {L"hya", L"ひゃ"}, {L"hyi", L"ひぃ"}, {L"hyu", L"ひゅ"}, {L"hye", L"ひぇ"}, {L"hyo", L"ひょ"},
        {L"fa", L"ふぁ"}, {L"fi", L"ふぃ"}, {L"fu", L"ふ"}, {L"fe", L"ふぇ"}, {L"fo", L"ふぉ"},
        {L"ba", L"ば"}, {L"bi", L"び"}, {L"bu", L"ぶ"}, {L"be", L"べ"}, {L"bo", L"ぼ"},
        {L"bya", L"びゃ"}, {L"byi", L"びぃ"}, {L"byu", L"びゅ"}, {L"bye", L"びぇ"}, {L"byo", L"びょ"},
        {L"pa", L"ぱ"}, {L"pi", L"ぴ"}, {L"pu", L"ぷ"}, {L"pe", L"ぺ"}, {L"po", L"ぽ"},
        {L"pya", L"ぴゃ"}, {L"pyi", L"ぴぃ"}, {L"pyu", L"ぴゅ"}, {L"pye", L"ぴぇ"}, {L"pyo", L"ぴょ"},
        // ま行
        {L"ma", L"ま"}, {L"mi", L"み"}, {L"mu", L"む"}, {L"me", L"め"}, {L"mo", L"も"},
        {L"mya", L"みゃ"}, {L"myi", L"みぃ"}, {L"myu", L"みゅ"}, {L"mye", L"みぇ"}, {L"myo", L"みょ"},
        // や行
        {L"ya", L"や"}, {L"yu", L"ゆ"}, {L"yo", L"よ"}, {L"ye", L"いぇ"},
        // ら行
        {L"ra", L"ら"}, {L"ri", L"り"}, {L"ru", L"る"}, {L"re", L"れ"}, {L"ro", L"ろ"},
        {L"rya", L"りゃ"}, {L"ryi", L"りぃ"}, {L"ryu", L"りゅ"}, {L"rye", L"りぇ"}, {L"ryo", L"りょ"},
        // わ行
        {L"wa", L"わ"}, {L"wi", L"うぃ"}, {L"wu", L"う"}, {L"we", L"うぇ"}, {L"wo", L"を"},
        // ヴ
        {L"va", L"ゔぁ"}, {L"vi", L"ゔぃ"}, {L"vu", L"ゔ"}, {L"ve", L"ゔぇ"}, {L"vo", L"ゔぉ"},
        // 小書き文字
        {L"xa", L"ぁ"}, {L"xi", L"ぃ"}, {L"xu", L"ぅ"}, {L"xe", L"ぇ"}, {L"xo", L"ぉ"},
        {L"la", L"ぁ"}, {L"li", L"ぃ"}, {L"lu", L"ぅ"}, {L"le", L"ぇ"}, {L"lo", L"ぉ"},
        {L"xya", L"ゃ"}, {L"xyu", L"ゅ"}, {L"xyo", L"ょ"},
        {L"lya", L"ゃ"}, {L"lyu", L"ゅ"}, {L"lyo", L"ょ"},
        {L"xtu", L"っ"}, {L"ltu", L"っ"}, {L"ltsu", L"っ"},
        {L"xn", L"ん"},
    };
    return table;
}

bool IsVowel(wchar_t c)
{
    return c == L'a' || c == L'i' || c == L'u' || c == L'e' || c == L'o';
}

// pending がテーブルのいずれかのキーの前方一致になっているか
bool IsPrefixOfAnyKey(const std::wstring& pending)
{
    const auto& table = Table();
    auto it = table.lower_bound(pending);
    return it != table.end() && it->first.size() > pending.size() &&
           it->first.compare(0, pending.size(), pending) == 0;
}

} // namespace

void RomajiComposer::Push(wchar_t c)
{
    pending_ += c;
    Convert();
}

void RomajiComposer::PushKana(const std::wstring& kana, const std::wstring& raw)
{
    // 未変換ローマ字が残っていたら確定と同じ規則で救済してから記号を足す
    if (pending_ == L"n") {
        AppendKana(L"ん", L"n");
    } else if (!pending_.empty()) {
        AppendKana(pending_, pending_);
    }
    pending_.clear();
    AppendKana(kana, raw);
}

void RomajiComposer::AppendKana(const std::wstring& kana, const std::wstring& raw)
{
    for (size_t i = 0; i < kana.size(); ++i) {
        kana_ += kana[i];
        raw_.push_back(i == 0 ? raw : L"");
    }
}

void RomajiComposer::Convert()
{
    const auto& table = Table();

    // 先頭からマッチする限り繰り返し変換する
    for (;;) {
        if (pending_.empty()) {
            return;
        }

        // 完全一致: かなを確定
        if (auto it = table.find(pending_); it != table.end()) {
            AppendKana(it->second, pending_);
            pending_.clear();
            return;
        }

        // 前方一致: 続きの入力を待つ
        if (IsPrefixOfAnyKey(pending_)) {
            return;
        }

        // 促音: 同じ子音の連続 ("kk" など。"nn" はテーブル側で ん になる)
        if (pending_.size() >= 2 && pending_[0] == pending_[1] && !IsVowel(pending_[0]) &&
            pending_[0] != L'n') {
            AppendKana(L"っ", pending_.substr(0, 1));
            pending_.erase(0, 1);
            continue;
        }

        // ん: "n" の後に変換を継続できない子音が来た場合
        if (pending_[0] == L'n' && pending_.size() >= 2) {
            AppendKana(L"ん", L"n");
            pending_.erase(0, 1);
            continue;
        }

        // どの規則にも合わない先頭文字はそのままかな列へ残す
        // (MS-IME と同様。英単語の打鍵 "apple" の p などが見えるようにする)
        AppendKana(pending_.substr(0, 1), pending_.substr(0, 1));
        pending_.erase(0, 1);
    }
}

void RomajiComposer::Backspace()
{
    if (!pending_.empty()) {
        pending_.pop_back();
    } else if (!kana_.empty()) {
        kana_.pop_back();
        raw_.pop_back();
    }
}

void RomajiComposer::Clear()
{
    kana_.clear();
    raw_.clear();
    pending_.clear();
    asciiMode_ = false;
}

bool RomajiComposer::Empty() const
{
    return kana_.empty() && pending_.empty();
}

std::wstring RomajiComposer::Display() const
{
    return kana_ + pending_;
}

std::wstring RomajiComposer::Commit() const
{
    if (pending_ == L"n") {
        return kana_ + L"ん";
    }
    return kana_ + pending_;
}

std::wstring RomajiComposer::Raw() const
{
    std::wstring raw;
    for (const auto& r : raw_) {
        raw += r;
    }
    return raw + pending_;
}

std::wstring RomajiComposer::RawRange(size_t pos, size_t len) const
{
    // Commit() は kana_ + 未変換ローマ字 (pending_ == "n" のみ「ん」1文字に救済)。
    // kana_ の各文字には raw_ が対応し、それ以降は pending_ の文字がそのまま並ぶ
    std::wstring raw;
    for (size_t i = pos; i < pos + len; ++i) {
        if (i < raw_.size()) {
            raw += raw_[i];
        } else if (pending_ == L"n") {
            raw += L"n";
        } else if (i - raw_.size() < pending_.size()) {
            raw += pending_[i - raw_.size()];
        }
    }
    return raw;
}
