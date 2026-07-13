#include "kana_forms.h"

#include <map>

namespace {

// ひらがな1文字を全角カタカナへ (範囲外はそのまま)
wchar_t HiraganaToKatakanaChar(wchar_t c)
{
    // ぁ (U+3041) - ゖ (U+3096) はカタカナと 0x60 ずれで対応する
    if (c >= 0x3041 && c <= 0x3096) {
        return static_cast<wchar_t>(c + 0x60);
    }
    return c;
}

// 全角カタカナ → 半角カタカナ (濁点・半濁点は2文字に分解)
const std::map<wchar_t, const wchar_t*>& HalfwidthKatakanaTable()
{
    static const std::map<wchar_t, const wchar_t*> table = {
        {L'ァ', L"ｧ"}, {L'ィ', L"ｨ"}, {L'ゥ', L"ｩ"}, {L'ェ', L"ｪ"}, {L'ォ', L"ｫ"},
        {L'ア', L"ｱ"}, {L'イ', L"ｲ"}, {L'ウ', L"ｳ"}, {L'エ', L"ｴ"}, {L'オ', L"ｵ"},
        {L'カ', L"ｶ"}, {L'キ', L"ｷ"}, {L'ク', L"ｸ"}, {L'ケ', L"ｹ"}, {L'コ', L"ｺ"},
        {L'ガ', L"ｶﾞ"}, {L'ギ', L"ｷﾞ"}, {L'グ', L"ｸﾞ"}, {L'ゲ', L"ｹﾞ"}, {L'ゴ', L"ｺﾞ"},
        {L'サ', L"ｻ"}, {L'シ', L"ｼ"}, {L'ス', L"ｽ"}, {L'セ', L"ｾ"}, {L'ソ', L"ｿ"},
        {L'ザ', L"ｻﾞ"}, {L'ジ', L"ｼﾞ"}, {L'ズ', L"ｽﾞ"}, {L'ゼ', L"ｾﾞ"}, {L'ゾ', L"ｿﾞ"},
        {L'タ', L"ﾀ"}, {L'チ', L"ﾁ"}, {L'ツ', L"ﾂ"}, {L'テ', L"ﾃ"}, {L'ト', L"ﾄ"},
        {L'ダ', L"ﾀﾞ"}, {L'ヂ', L"ﾁﾞ"}, {L'ヅ', L"ﾂﾞ"}, {L'デ', L"ﾃﾞ"}, {L'ド', L"ﾄﾞ"},
        {L'ッ', L"ｯ"},
        {L'ナ', L"ﾅ"}, {L'ニ', L"ﾆ"}, {L'ヌ', L"ﾇ"}, {L'ネ', L"ﾈ"}, {L'ノ', L"ﾉ"},
        {L'ハ', L"ﾊ"}, {L'ヒ', L"ﾋ"}, {L'フ', L"ﾌ"}, {L'ヘ', L"ﾍ"}, {L'ホ', L"ﾎ"},
        {L'バ', L"ﾊﾞ"}, {L'ビ', L"ﾋﾞ"}, {L'ブ', L"ﾌﾞ"}, {L'ベ', L"ﾍﾞ"}, {L'ボ', L"ﾎﾞ"},
        {L'パ', L"ﾊﾟ"}, {L'ピ', L"ﾋﾟ"}, {L'プ', L"ﾌﾟ"}, {L'ペ', L"ﾍﾟ"}, {L'ポ', L"ﾎﾟ"},
        {L'マ', L"ﾏ"}, {L'ミ', L"ﾐ"}, {L'ム', L"ﾑ"}, {L'メ', L"ﾒ"}, {L'モ', L"ﾓ"},
        {L'ャ', L"ｬ"}, {L'ュ', L"ｭ"}, {L'ョ', L"ｮ"},
        {L'ヤ', L"ﾔ"}, {L'ユ', L"ﾕ"}, {L'ヨ', L"ﾖ"},
        {L'ラ', L"ﾗ"}, {L'リ', L"ﾘ"}, {L'ル', L"ﾙ"}, {L'レ', L"ﾚ"}, {L'ロ', L"ﾛ"},
        {L'ワ', L"ﾜ"}, {L'ヲ', L"ｦ"}, {L'ン', L"ﾝ"}, {L'ヴ', L"ｳﾞ"},
        {L'ー', L"ｰ"}, {L'。', L"｡"}, {L'、', L"､"},
        {L'「', L"｢"}, {L'」', L"｣"}, {L'・', L"･"},
    };
    return table;
}

} // namespace

namespace kana_forms {

std::wstring ToKatakana(const std::wstring& text)
{
    std::wstring result;
    result.reserve(text.size());
    for (wchar_t c : text) {
        result += HiraganaToKatakanaChar(c);
    }
    return result;
}

std::wstring ToHalfwidth(const std::wstring& text)
{
    const auto& table = HalfwidthKatakanaTable();
    std::wstring result;
    result.reserve(text.size());
    for (wchar_t c : text) {
        const wchar_t katakana = HiraganaToKatakanaChar(c);
        if (auto it = table.find(katakana); it != table.end()) {
            result += it->second;
        } else if (katakana >= 0xFF01 && katakana <= 0xFF5E) {
            // 全角英数記号 (！ など) → 半角
            result += static_cast<wchar_t>(katakana - 0xFEE0);
        } else if (katakana == 0x3000) {
            result += L' '; // 全角スペース
        } else {
            result += katakana;
        }
    }
    return result;
}

std::wstring ToFullwidthAscii(const std::wstring& text)
{
    std::wstring result;
    result.reserve(text.size());
    for (wchar_t c : text) {
        if (c >= 0x21 && c <= 0x7E) {
            result += static_cast<wchar_t>(c + 0xFEE0);
        } else if (c == L' ') {
            result += static_cast<wchar_t>(0x3000); // 全角スペース
        } else {
            result += c;
        }
    }
    return result;
}

} // namespace kana_forms
