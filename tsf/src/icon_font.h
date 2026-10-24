#pragma once

#include <windows.h>

struct IDWriteFactory;
struct IDWriteFontFace;
struct IDWriteFontFileLoader;

// 入力モードの字 (あ・A・×) を描くフォント。DLL のリソースに入れた M PLUS Rounded 1c Regular の
// サブセットを読み、読めなければ Yu Gothic UI を使う (docs/spec/display.md「入力モードのアイコン」)。
// タスクバーのアイコン (LangBarButton) と入力モードの一時表示 (ModePopup) がそれぞれ持つ
class IconFont {
public:
    IconFont();
    ~IconFont();
    IconFont(const IconFont&) = delete;
    IconFont& operator=(const IconFont&) = delete;

    // font face (最初の呼び出しで作って使い回す)。作れなければ nullptr
    IDWriteFontFace* Face();

private:
    IDWriteFactory* factory_;
    IDWriteFontFileLoader* loader_;  // factory_ に登録したリソースのフォントの loader
    IDWriteFontFace* face_;
};

// 一辺 size px の正方形に 1 字を置く glyph run の値
struct IconGlyph {
    UINT16 index = 0;
    float emSize = 0.0f;
    float advance = 0.0f;
    float originX = 0.0f;  // 描く原点 (整数 px に丸めてある)
    float originY = 0.0f;
};

// 「あ」の字形の外接矩形の長辺が inkLong px になる文字サイズで、ch の字形の外接矩形を一辺 size px の
// 正方形の上下左右の中央に置く。字形が引けなければ false
bool PlaceIconGlyph(IDWriteFontFace* face, wchar_t ch, float inkLong, float size, IconGlyph* glyph);
