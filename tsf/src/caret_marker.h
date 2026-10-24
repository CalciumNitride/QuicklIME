#pragma once

#include <windows.h>

#include "panel_theme.h"

struct ID2D1Factory;
struct ID2D1DCRenderTarget;
struct ID2D1SolidColorBrush;

// キャレットの印の三角 (テーマの caret_marker triangle。docs/spec/display.md「キャレットの印」)。
// キャレットの矩形の下端に先端を上に向けた三角を、入力パネルとは別の小さな layered window で出す。
// フォーカスを奪わず (WS_EX_NOACTIVATE)、クリックは下のアプリへ通す (WS_EX_TRANSPARENT)
class CaretMarker {
public:
    CaretMarker();
    ~CaretMarker();
    CaretMarker(const CaretMarker&) = delete;
    CaretMarker& operator=(const CaretMarker&) = delete;

    // 先端を (caret.left, caret.bottom) に置き、高さ height px の正三角形を color で
    // 出す (表示中なら動かして描き直す)。height が 1 未満か、描けなければ隠して false
    bool Show(const RECT& caret, int height, const PanelColor& color);
    void Hide();

    bool Visible() const { return hwnd_ != nullptr; }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    bool EnsureTarget();
    void ReleaseTarget();

    HWND hwnd_;
    ID2D1Factory* factory_;
    ID2D1DCRenderTarget* target_;
    ID2D1SolidColorBrush* brush_;
};
