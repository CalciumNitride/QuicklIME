#pragma once

#include <windows.h>

#include <string>
#include <vector>

// 変換候補を表示するポップアップウィンドウ。
// フォーカスを奪わない (WS_EX_NOACTIVATE) 最前面ウィンドウとして表示する。
class CandidateWindow {
public:
    CandidateWindow();
    ~CandidateWindow();

    // anchor (スクリーン座標、通常は composition の矩形) の直下に候補一覧を表示する
    bool Show(const RECT& anchor, const std::vector<std::wstring>& items, size_t selection);

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    void Hide();

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    void Paint(HDC hdc);

    HWND hwnd_;
    HFONT font_;
    std::vector<std::wstring> items_;
    size_t selection_;
    int lineHeight_;
};
