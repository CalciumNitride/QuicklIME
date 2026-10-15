#pragma once

#include <windows.h>

#include <functional>
#include <string>
#include <vector>

// 変換キーの候補選択で縦の候補一覧を表示するポップアップウィンドウ (候補バーと未完成ローマ字の
// 小窓は SuggestPanel)。フォーカスを奪わない (WS_EX_NOACTIVATE) 最前面ウィンドウとして表示する。
class CandidateWindow {
public:
    // 1ページに表示する候補数。行頭の番号はページ内相対 (1〜kPageSize) で、
    // 数字キーによる候補の直接選択もこのページ単位で対応付ける
    static constexpr size_t kPageSize = 9;

    CandidateWindow();
    ~CandidateWindow();

    // anchor (スクリーン座標、通常は composition の矩形) の直下に候補一覧を表示する
    bool Show(const RECT& anchor, const std::vector<std::wstring>& items, size_t selection);

    // 表示中のウィンドウの矩形 (スクリーン座標)。非表示なら false
    bool WindowRect(RECT* rect) const;

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // 候補番号用フォントは候補文字列より一回り小さいサイズで内部生成する。
    // 作成に失敗したときは現在のフォントを維持する
    void SetFont(const std::wstring& face, int height);

    void Hide();

    // 診断ログの出力先 (表示位置・ウィンドウの状態・WM_PAINT の受信などを1行ずつ渡す)。
    // 未設定ならログの文字列も組み立てない
    void SetLogger(std::function<void(const std::wstring&)> logger) { logger_ = std::move(logger); }

    // 表示中かどうか (Hide 済み・未表示なら false)
    bool Visible() const { return hwnd_ != nullptr; }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    void Paint(HDC hdc);
    // Show 系の呼び出しの結果を1行に出す。hwnd_ が null なら CreateWindowExW の失敗 (createError)
    void LogShow(const wchar_t* kind, const RECT& anchor, int x, int y, int width, int height,
                 bool created, DWORD createError, unsigned paints) const;

    HWND hwnd_;
    HFONT font_;       // 候補文字列用フォント
    HFONT numberFont_;  // 候補番号用フォント (候補文字列より控えめな小さいサイズ)
    std::vector<std::wstring> items_;
    size_t selection_;
    int lineHeight_;
    int numberColumnWidth_;  // 番号列の幅 (px)。kPageSize<=9 のため番号は常に1桁
    int numberFontHeight_;   // 番号フォントの文字高 (px、行内の垂直中央揃えに使う)
    std::function<void(const std::wstring&)> logger_;  // 診断ログ (空なら出さない)
    unsigned paintCount_;     // 前回の Show 系の呼び出し以降に受けた WM_PAINT の回数
    bool firstPaintLogged_;   // 今の hwnd_ で最初の WM_PAINT をログに出したか
    bool destroying_;         // Hide の DestroyWindow 中 (WM_DESTROY の由来の区別に使う)
};
