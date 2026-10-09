#pragma once

#include <windows.h>

#include <functional>
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

    // 番号列・選択行の無い1行だけの表示 (追記型入力 F1 の未完成ローマ字の小窓)。
    // caret (スクリーン座標) の左上に重ねて表示する
    bool ShowInline(const RECT& caret, const std::wstring& text);

    // 候補バーの1候補
    struct BarItem {
        std::wstring text;
        bool partial = false;  // 部分採用の候補 (末尾に「…」を付けて表示する)
    };

    // 候補を横一列に並べる表示 (候補バー)。caret (スクリーン座標) の行の下 (入らなければ上)
    // の x の位置に出す。作業領域の右端に収まる先頭からの候補だけを表示し、表示した件数を返す
    // (0 なら表示していない)。selection が範囲外ならどの候補も強調しない
    size_t ShowBar(const RECT& caret, int x, const std::vector<BarItem>& items, size_t selection);

    // 表示中のウィンドウの矩形 (スクリーン座標)。非表示なら false
    bool WindowRect(RECT* rect) const;

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // 候補番号用フォントは候補文字列より一回り小さいサイズで内部生成する。
    // 作成に失敗したときは現在のフォントを維持する
    void SetFont(const std::wstring& face, int height);

    // 表示中のウィンドウに intervalMs 間隔のタイマーを付け、callback を呼ぶ (付け直すと前のものを
    // 置き換える)。非表示なら付けずに false。Hide で破棄されたタイマーは止まる
    bool StartTimer(UINT intervalMs, std::function<void()> callback);
    void StopTimer();

    void Hide();

    // 表示中かどうか (Hide 済み・未表示なら false)
    bool Visible() const { return hwnd_ != nullptr; }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    enum class Layout {
        Vertical,  // Show: 縦の候補一覧
        Inline,    // ShowInline: 未完成ローマ字の小窓
        Bar,       // ShowBar: 横一列の候補バー
    };

    void Paint(HDC hdc);
    void PaintBar(HDC hdc);

    HWND hwnd_;
    std::function<void()> timerCallback_;  // StartTimer の callback (空ならタイマーなし)
    HFONT font_;       // 候補文字列用フォント
    HFONT numberFont_;  // 候補番号用フォント (候補文字列より控えめな小さいサイズ)
    std::vector<std::wstring> items_;
    size_t selection_;
    Layout layout_;
    std::vector<int> barItemX_;      // 候補バーの各候補の左端 (クライアント座標)
    std::vector<int> barItemWidth_;  // 候補バーの各候補の幅
    int lineHeight_;
    int numberColumnWidth_;  // 番号列の幅 (px)。kPageSize<=9 のため番号は常に1桁
    int numberFontHeight_;   // 番号フォントの文字高 (px、行内の垂直中央揃えに使う)
};
