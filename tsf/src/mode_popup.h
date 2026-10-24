#pragma once

#include <windows.h>

#include "icon_font.h"
#include "panel_theme.h"

struct ID2D1Factory;
struct ID2D1DCRenderTarget;
struct ID2D1SolidColorBrush;

// 入力モードの一時表示 (docs/spec/display.md「入力モードの一時表示」)。キャレット行のすぐ下 (作業領域の下端から
// はみ出すときは上) に「あ」/「A」を角丸の四角で出し、1 秒で消す。入力パネル・キャレットの印とは
// 別の小さな layered window で、フォーカスを奪わず (WS_EX_NOACTIVATE)、クリックは下のアプリへ通す
// (WS_EX_TRANSPARENT)
class ModePopup {
public:
    ModePopup();
    ~ModePopup();
    ModePopup(const ModePopup&) = delete;
    ModePopup& operator=(const ModePopup&) = delete;

    // caret の矩形を基準に、open なら「あ」、そうでなければ「A」を theme の色で出す (表示中なら
    // 出し直して 1 秒を数え直す)。fontSize は設定 candidate_font_size (角丸の半径に使う)。
    // 描けなければ隠して false
    bool Show(const RECT& caret, bool open, const PanelTheme& theme, int fontSize);
    void Hide();

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    bool EnsureTarget();
    void ReleaseTarget();

    HWND hwnd_;
    ID2D1Factory* factory_;
    ID2D1DCRenderTarget* target_;
    ID2D1SolidColorBrush* brush_;
    IconFont font_;
};
