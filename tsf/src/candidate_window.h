#pragma once

#include <windows.h>

#include <string>
#include <vector>

// 変換候補を表示するポップアップウィンドウ。
// フォーカスを奪わない (WS_EX_NOACTIVATE) 最前面ウィンドウとして表示する。
class CandidateWindow {
public:
    // 1ページに表示する候補数。行頭の番号はページ内相対 (1〜kPageSize) で、
    // 数字キーによる候補の直接選択もこのページ単位で対応付ける
    static constexpr size_t kPageSize = 9;

    CandidateWindow();
    ~CandidateWindow();

    // anchor (スクリーン座標、通常は composition の矩形) の直下に候補一覧を表示する
    bool Show(const RECT& anchor, const std::vector<std::wstring>& items, size_t selection);

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // 候補番号用フォントは候補文字列より一回り小さいサイズで内部生成する。
    // 作成に失敗したときは現在のフォントを維持する
    void SetFont(const std::wstring& face, int height);

    void Hide();

    // 表示中かどうか (Hide 済み・未表示なら false)
    bool Visible() const { return hwnd_ != nullptr; }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    void Paint(HDC hdc);

    HWND hwnd_;
    HFONT font_;        // 候補文字列用フォント
    HFONT numberFont_;  // 候補番号用フォント (候補文字列より控えめな小さいサイズ)
    std::vector<std::wstring> items_;
    size_t selection_;
    int lineHeight_;
    int numberColumnWidth_;  // 番号列の幅 (px)。kPageSize<=9 のため番号は常に1桁
    int numberFontHeight_;   // 番号フォントの文字高 (px、行内の垂直中央揃えに使う)
};
