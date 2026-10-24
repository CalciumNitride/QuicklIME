#pragma once

#include <windows.h>

#include <functional>
#include <string>
#include <vector>

#include "panel_theme.h"

struct ID2D1Factory;
struct ID2D1DCRenderTarget;
struct ID2D1HwndRenderTarget;
struct ID2D1RenderTarget;
struct ID2D1SolidColorBrush;
struct ID2D1StrokeStyle;
struct IDWriteFactory;
struct IDWriteTextFormat;

// 候補バー (横一列または縦積みの候補) と入力行を出すパネル、変換の候補一覧
// (docs/design/conversion-panel.md)、および未完成ローマ字の小さな
// カプセル。見た目は PanelTheme (docs/design/panel-theme.md) に従う。Direct2D で 32bpp の DIB に描き、UpdateLayeredWindow で画素ごとの不透明度つきで出す。
// ぼかし (SetBlur) では通常のウィンドウに DWM のアクリルを当て、その上に直接描く。
// フォーカスを奪わない (WS_EX_NOACTIVATE) 最前面ウィンドウとして表示する
class SuggestPanel {
public:
    // 1 ページに出す候補の最大数 (番号はページ内で 1〜9)
    static constexpr size_t kMaxItems = 9;

    // 候補の種類 (縦積みの種類のタグに出す)
    enum class ItemKind {
        Ascii,       // 英字
        Conversion,  // 全体変換
        Prediction,  // 予測
        Partial,     // 採用範囲を縮めた全体変換 (部分採用)
        NextWord,    // 読みなしの後続語予測 (横一列では「次の語」色の縁を付ける)
        // 打ち間違いを戻した全体変換と、ずれを使った予測 (横一列では「補正」色の縁を付ける。
        // 変換の候補一覧でも種類のタグを出す)
        Typo,
    };

    struct Item {
        std::wstring text;
        ItemKind kind = ItemKind::Conversion;
        // text のこの位置から後ろを文字の色の不透明度 0.5 で描く (採用範囲の外。npos なら全部を
        // ふつうに描く)
        size_t dimFrom = std::wstring::npos;
    };

    // 入力行の内容。パネルは内容を解釈せずにそのまま出す
    struct InputLine {
        std::wstring text;     // アクセント色の下線で出す文字列
        std::wstring pending;  // text に続けて副次色・点線の下線で出す文字列 (末尾にキャレット)
        // text・pending が空のときに副次色で出す文字列 (後続語予測の前文脈)
        std::wstring context;
        bool nextWordLabel = false;  // context の後ろに「次の語」のラベルを付ける
        // 変換中の内容。segments が空でなければ変換中の表示にし、上の 4 つは使わない
        std::wstring excluded;               // 範囲を縮めて外したかな (副次色・点線の下線)
        std::vector<std::wstring> segments;  // 文節ごとの変換結果
        size_t currentSegment = 0;           // 強調する文節 (segments の番号)
    };

    enum class Position {
        Caret,   // キャレット行の下 (入らなければ上)
        Center,  // キャレットのモニタの作業領域の中央
    };

    struct Placement {
        Position position = Position::Caret;
        // 位置の基準 (スクリーン座標)。null は取れなかったことを表す (Center のみで許す)
        const RECT* caret = nullptr;
        int x = 0;  // Caret のときのパネルの左端
        // キャレットの印 (docs/design/caret-marker.md) の先端の x (文書のキャレットの矩形の左端。
        // スクリーン座標)。pointer のときだけ使う
        int markerX = 0;
        // キャレット行に向いた辺に吹き出しの尻尾を付けて markerX を指す。描けないとき
        // (Center・style ghost・ぼかし・キャレット行に重なる位置) は付けない
        bool pointer = false;
        // Caret でキャレット行の下に出すとき、キャレットの下端に出す三角 (theme の triangleSize) の
        // 高さの分だけ下げる
        bool belowTriangle = false;
    };

    SuggestPanel();
    ~SuggestPanel();
    SuggestPanel(const SuggestPanel&) = delete;
    SuggestPanel& operator=(const SuggestPanel&) = delete;

    // 候補をテーマの並べ方 (横一列・縦積み) で並べ、input があれば上に入力行を置いて出す。
    // 候補はページに区切り (docs/spec/suggest.md「ページ送り」)、selection を含むページを出す。番号は
    // ページ内で 1 から振り、複数ページなら位置 (12/24) を出す。selection が範囲外ならどの候補も
    // 強調せず 1 ページ目を出す。出したページの候補の数を返す (候補が無く入力行だけを出したときも 0)。
    // 候補も入力行も無ければ隠す
    size_t Show(const Placement& placement, const std::vector<Item>& items, size_t selection,
                const InputLine* input);

    // 変換の候補一覧。テーマの layout によらず縦積みにし、種類のタグは補正の候補 (typo[i] が true。
    // typo が candidates より短ければ足りない分は false) にだけ出す。selection を含む
    // ページ (kMaxItems 件。作業領域に収まらなければ収まる行数) を出し、番号はページ内で 1 から振る。
    // 複数ページなら最後の行の下に位置 (12/24) を出す。Caret では placement.caret (composition の
    // 矩形) の下 (入らなければ上) の placement.x に、Center では候補バーと同じ上端から下へ伸ばして出す。
    // candidates が空なら入力行だけを出し、input も無ければ隠す。
    // 候補一覧を出したときは 1 ページの件数 (数字キーの対応付けに使う) を、出さなかったときは 0 を返す
    size_t ShowConversion(const Placement& placement, const std::vector<std::wstring>& candidates,
                          const std::vector<bool>& typo, size_t selection, const InputLine* input);

    // 表示中のパネルの矩形 (影を除く。スクリーン座標)。非表示なら false
    bool WindowRect(RECT* rect) const;

    // 選択中の候補を変えて再描画する。Show の候補では selection は全体での番号で、別のページの
    // 候補なら Show で区切ったそのページに組み直して出す (範囲外は強調なしで 1 ページ目)
    void SetSelection(size_t selection);

    // 今出している候補のページの、先頭の候補の全体での番号と件数 (数字キーの対応付けに使う)
    size_t PageStart() const { return pageStart_; }
    size_t PageItemCount() const { return items_.size(); }

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // フォントが見つからなければ Yu Gothic UI を使う。表示中の内容には次の Show から効く
    void SetFont(const std::wstring& face, int height);

    // 見た目を差し替える。表示中なら、色だけが変わったときは描き直し、並べ方・形・寸法が
    // 変わったときは隠す (今のレイアウトは前の見た目で組んであるため)
    void SetTheme(const PanelTheme& theme);

    // 背景のぼかし (テーマの blur) を使うかどうか。変わったときは表示中なら隠し、次の表示から
    // 新しい方式でウィンドウを作る。ぼかしが使えない環境では layered window の描画に戻す
    void SetBlur(bool enabled);

    // 表示中のウィンドウに intervalMs 間隔のタイマーを付け、callback を呼ぶ (付け直すと前のものを
    // 置き換える)。非表示なら付けずに false。Hide で破棄されたタイマーは止まる
    bool StartTimer(UINT intervalMs, std::function<void()> callback);
    void StopTimer();

    void Hide();

    // 診断ログの出力先 (表示位置・ウィンドウの状態などを1行ずつ渡す)。
    // 未設定ならログの文字列も組み立てない
    void SetLogger(std::function<void(const std::wstring&)> logger) { logger_ = std::move(logger); }

    // ウィンドウを作らずに描く方式にする (設定ツールのプレビュー。docs/design/panel-preview.md)。
    // 有効なら Show・ShowConversion はウィンドウを出さず、layered window に出すのと同じ画像
    // (影・にじみの余白を含む) を Snapshot に残す。ぼかし (SetBlur) は使わない
    void SetOffscreen(bool enabled) { offscreen_ = enabled; }

    // offscreen で最後に描いた画像。上から下へ並ぶ BGRA (premultiplied alpha)。描いていなければ空
    const std::vector<BYTE>& SnapshotPixels() const { return snapshot_; }
    int SnapshotWidth() const { return snapshotWidth_; }
    int SnapshotHeight() const { return snapshotHeight_; }

    // 表示中かどうか (Hide 済み・未表示なら false)
    bool Visible() const { return hwnd_ != nullptr; }

    // テーマが候補を横一列に並べるか (変換の候補一覧はこれによらず縦積み)
    bool Horizontal() const { return theme_.layout == PanelLayout::Row; }

    // 縦に並ぶ入力パネルをキャレット行の上に出し、並びを反転している (1 番目の候補がキャレット側の
    // 下にある) か (docs/design/panel-above-caret.md)。表示していなければ false
    bool Reversed() const
    {
        return hwnd_ != nullptr && columnSide_ == ColumnSide::Above;
    }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    // 吹き出しの尻尾を付ける辺
    enum class TailSide {
        None,
        Top,     // パネルがキャレット行の下にあり、上向きの尻尾を付ける
        Bottom,  // パネルがキャレット行の上にあり、下向きの尻尾を付ける
    };

    // 縦に並ぶ入力パネルを Caret で出す側
    enum class ColumnSide {
        None,   // 決めていない (横一列・Center・どちら側にも 1 行も入らない)
        Below,  // キャレット行の下
        Above,  // キャレット行の上 (並びを反転する)
    };

    // 候補 1 件の表示位置 (影を除いたパネルの左上を原点とする px)。高さは行の高さ
    struct ItemBox {
        float left;
        float top;
        float width;
    };

    bool EnsureFactories();
    bool EnsureTarget();
    void ReleaseTarget();
    // text を format で1行に並べたときの幅 (px)
    float TextWidth(const std::wstring& text, IDWriteTextFormat* format) const;
    // Show で区切った候補の page ページ目を組んで出し、出した候補の数を返す (selection は全体での番号)
    size_t LayoutPage(size_t page, size_t selection, const wchar_t* kind);
    // placement (Caret) で縦の入力パネルを出す側を *side に決め、1 ページに出す行数を返す。
    // どちら側にも 1 行も入らなければ *side を None にして 0 を返す
    size_t FitColumn(const Placement& placement, const RECT& work, bool hasInput, size_t total,
                     ColumnSide* side) const;
    // 縦積みで上から組んだ入力行・区切り線・候補・位置の表示を、高さ height のパネルの中で
    // 上下に反転する
    void MirrorColumn(int height);
    // Show の候補で selection を選んでいるときの位置の表示 (1 ページなら空)
    std::wstring PageText(size_t selection) const;
    // 今のレイアウトをウィンドウ (bodyX_, bodyY_ の位置) に描いて出す
    bool Render(const wchar_t* kind, const RECT* anchor);
    // 今のレイアウトを、memory に選択した rect の大きさの DIB に描く
    HRESULT DrawLayered(HDC memory, const RECT& rect);
    // offscreen の方式で今のレイアウトを描き、Snapshot に残す
    bool RenderOffscreen(const wchar_t* kind);
    // ぼかしのウィンドウを作る。使えなければ作りかけを破棄して false (理由はログに出す)
    bool CreateBlurWindow(const wchar_t* kind);
    // 今のレイアウトを残したまま、ぼかしのウィンドウとその描画先を破棄する
    void DestroyBlurWindow();
    bool EnsureHwndTarget();
    void ReleaseHwndTarget();
    // ぼかしのウィンドウに今のレイアウトを描く
    HRESULT DrawBlur();
    void Draw();
    void DrawShadow();
    // ghost の背景 (放射状のにじみ)
    void DrawScrim();
    void DrawInputLine(float left, float top, float width);
    // 変換中の入力行 (外したかなと文節。現在文節を強調する)
    void DrawConversionInput(float left, float top, float width);
    void DrawItem(size_t index, float left, float top);
    // text_shadow on なら下に影を敷いて文字を描く
    void DrawLabel(const std::wstring& text, IDWriteTextFormat* format, float left, float top,
                   float right, float bottom, const PanelColor& color);
    // layered window の方式で、パネルの外側に取る余白 (影・ghost のにじみの分)
    int LayeredMargin() const;
    // 次の Render がぼかしの方式で描くか
    bool BlurExpected() const;
    // placement で尻尾を付けるときの尻尾の高さ (px)。付けられなければ 0
    int PointerHeight(const Placement& placement) const;
    // placement で三角の分だけパネルを下げる量 (px)。下げなければ 0
    int TriangleHeight(const Placement& placement) const;
    // 決めたパネルの位置 (bodyY_・bodyHeight_) とキャレット行から、尻尾を付ける辺を決める
    void PlaceTail(const Placement& placement, int pointerHeight);
    // 尻尾の分としてパネルの上・下に取るウィンドウの余白 (px)
    int TopTailPad() const;
    int BottomTailPad() const;
    // 尻尾の先端の x (ウィンドウ内の座標)。bodyLeft・bodyRight はパネルの左右端 (ウィンドウ内の座標)
    float TailTipX(float bodyLeft, float bodyRight, float radius) const;
    // 入力行の末尾に出すゴースト (ghost のとき選択中または先頭の候補。無ければ空)
    std::wstring GhostText() const;
    void LogShow(const wchar_t* kind, const RECT* anchor, bool created, DWORD error) const;

    HWND hwnd_;
    std::function<void()> timerCallback_;  // StartTimer の callback (空ならタイマーなし)
    std::function<void(const std::wstring&)> logger_;  // 診断ログ (空なら出さない)
    bool destroying_;  // Hide の DestroyWindow 中 (WM_DESTROY の由来の区別に使う)

    ID2D1Factory* d2dFactory_;
    IDWriteFactory* dwriteFactory_;
    ID2D1DCRenderTarget* target_;          // layered window の描画先
    ID2D1HwndRenderTarget* hwndTarget_;    // ぼかしのウィンドウの描画先
    ID2D1RenderTarget* drawTarget_;        // Draw が描く先 (上の2つのうち描画中のもの)
    // target_ と hwndTarget_ は同時には持たず、brush_ は今ある方で作ったもの
    ID2D1SolidColorBrush* brush_;
    ID2D1StrokeStyle* dotStroke_;  // 未完成ローマ字の点線の下線
    IDWriteTextFormat* textFormat_;    // 候補文字列 (f。収まらなければ末尾を「…」で切る)
    IDWriteTextFormat* boldFormat_;    // selection text の選択中の候補文字列
    IDWriteTextFormat* inputFormat_;   // 入力行
    IDWriteTextFormat* badgeFormat_;   // 候補番号 (中央揃え)
    IDWriteTextFormat* labelFormat_;   // 「次の語」ラベル
    IDWriteTextFormat* tagFormat_;     // 種類のタグ (中央揃え)
    std::wstring face_;
    int fontSize_;
    PanelTheme theme_;
    bool blur_;          // ぼかしを使う設定
    bool blurFailed_;    // API が使えずぼかしをやめた (SetBlur で有効にし直すまで試さない)
    bool blurActive_;    // 今のウィンドウがぼかしの方式
    bool offscreen_;     // ウィンドウを作らずに描く (SetOffscreen)
    std::vector<BYTE> snapshot_;  // offscreen で描いた画像
    int snapshotWidth_;
    int snapshotHeight_;

    // 今のレイアウト (座標は影を除いたパネルの左上を原点とする px)
    bool conversion_;  // 変換の候補一覧 (ShowConversion) を出している
    std::vector<Item> items_;
    std::vector<std::wstring> itemTexts_;  // 表示する候補文字列
    std::vector<ItemBox> itemBoxes_;
    float inputTop_;  // 入力行の上端 (パネルの左上を原点とする px)
    bool separator_;  // 入力行と候補の間に区切り線を引く
    float separatorTop_;  // 区切り線の上端 (パネルの左上を原点とする px)
    std::wstring pageText_;  // 位置の表示 (空なら出さない)
    float pageTop_;          // 位置の表示の上端 (パネルの左上を原点とする px)
    // 位置の表示を横一列のチップと同じ行の pageLeft_ (パネルの左上を原点とする px) に出す
    // (false なら最後の行の下に align に従って出す)
    bool pageInline_;
    float pageLeft_;
    size_t pageStart_;  // 出しているページの先頭の候補の全体での番号
    size_t selection_;  // 選択中の候補 (出しているページ内の番号。範囲外なら強調なし)

    // Show で受け取った全候補とページの区切り (SetSelection でページを組み直すのに使う)
    std::vector<Item> allItems_;
    std::vector<std::wstring> allTexts_;  // 表示する候補文字列
    std::vector<float> allWidths_;        // 切り詰めずに出したときの候補 1 件の幅
    std::vector<size_t> pageStarts_;      // 各ページの先頭の候補の番号 (候補が無ければ空)
    size_t page_;                         // 出しているページ
    float pageLabelWidth_;  // 位置の表示に取る幅 (選択によって変わらないよう最大の幅。1 ページなら 0)
    float inputWidth_;      // 入力行に要る幅
    Placement placement_;   // Show で受け取った位置の基準 (caret は placementCaret_ を指す)
    RECT placementCaret_;
    RECT work_;             // 出すモニタの作業領域
    int pointerHeight_;     // 尻尾の高さ (Show の時点で決めたもの)
    ColumnSide columnSide_;  // 縦の入力パネルを出す側 (Show・ShowConversion の時点で決めたもの)
    bool hasInput_;
    InputLine input_;
    int bodyX_;       // パネル (影を除く) の左上 (スクリーン座標)
    int bodyY_;
    int bodyWidth_;
    int bodyHeight_;
    int margin_;      // 影のためにパネルの外側に取る余白 (ぼかしの方式では 0)
    TailSide tail_;   // 吹き出しの尻尾を付ける辺
    int tailHeight_;  // 尻尾の高さ (px。底辺は高さの 2 倍)
    int markerX_;     // 尻尾の先端の x (スクリーン座標。角丸にかかるときは Draw で寄せる)
};
