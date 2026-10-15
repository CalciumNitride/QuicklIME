#pragma once

#include <windows.h>

#include <functional>
#include <string>
#include <vector>

struct ID2D1Factory;
struct ID2D1DCRenderTarget;
struct ID2D1HwndRenderTarget;
struct ID2D1RenderTarget;
struct ID2D1SolidColorBrush;
struct ID2D1StrokeStyle;
struct IDWriteFactory;
struct IDWriteTextFormat;

// 色 (アルファは乗算前の値)
struct PanelColor {
    float r = 0.0f;
    float g = 0.0f;
    float b = 0.0f;
    float a = 1.0f;
};

// サジェストパネルの見た目 (docs/design/suggest-panel.md)。寸法は設定 candidate_font_size
// (以下 f) に対する比率、px と書いたものは f によらない画素数
struct PanelTheme {
    float panelRadius = 1.4f;      // パネルの角丸の半径 (パネルの高さの半分まで)
    float panelPadding = 0.6f;     // パネルの内側の余白
    float inputTextSize = 1.15f;   // 入力行の文字の高さ
    float inputRowHeight = 1.7f;   // 入力行の高さ (下線を含む)
    float rowGap = 0.35f;          // 入力行と区切り線、区切り線とチップ列の間
    float chipHeight = 1.9f;       // チップの高さ (両端は半円)
    float chipPadding = 0.65f;     // チップの左右の余白
    float chipGap = 0.35f;         // チップどうしの間隔
    float badgeSize = 0.95f;       // 候補番号のバッジの直径
    float badgeTextSize = 0.62f;   // 候補番号の文字の高さ
    float badgeGap = 0.35f;        // バッジと候補文字列の間
    float labelTextSize = 0.62f;   // 入力行の「次の語」ラベルの文字の高さ
    float labelGap = 0.5f;         // 前文脈と「次の語」ラベルの間
    float shadowSize = 0.8f;       // 影の広がり
    float shadowOffset = 0.2f;     // 影を下へずらす量
    float centerWidth = 36.0f;     // 画面中央に出すときのパネルの幅
    float centerWidthRatio = 0.9f; // 画面中央に出すときの幅の上限 (作業領域の幅に対する比率)
    float lineWidthPx = 1.0f;      // 外周・区切り線・バッジ・次の語の縁の太さ (px)
    float underlinePx = 2.0f;      // run の文字列の下線の太さ (px)
    float caretWidthPx = 2.0f;     // 入力行のキャレットの太さ (px)
    int shadowSteps = 10;          // 影を近似する角丸矩形の枚数

    PanelColor background;  // パネルの背景
    PanelColor border;      // 外周の縁
    PanelColor text;        // 文字
    PanelColor secondary;   // 副次色 (候補番号・未完成ローマ字・前文脈)
    PanelColor chip;        // チップの背景
    PanelColor separator;   // 入力行とチップ列の区切り線
    PanelColor accent;      // 選択中のチップ・run の下線・キャレット
    PanelColor selectedText;  // 選択中のチップの文字とバッジ
    PanelColor nextWord;    // 読みなしの後続語予測の縁と「次の語」ラベル
    PanelColor shadow;      // 影 (a は重ねたときの最大不透明度)
    PanelColor blurTint;    // ぼかし (panel_blur) の上に重ねる色

    static PanelTheme Dark();
    static PanelTheme Light();
    // Windows のアプリモード (AppsUseLightTheme。値が無ければライト) に合わせた配色
    static PanelTheme ForAppMode();
};

// 候補バー (横一列のチップ) と入力行を出すパネル、および未完成ローマ字の小さなカプセル。
// Direct2D で 32bpp の DIB に描き、UpdateLayeredWindow で画素ごとの不透明度つきで出す。
// ぼかし (SetBlur) では通常のウィンドウに DWM のアクリルを当て、その上に直接描く。
// フォーカスを奪わない (WS_EX_NOACTIVATE) 最前面ウィンドウとして表示する
class SuggestPanel {
public:
    // 番号 (1〜9) を付けて出せる候補の数
    static constexpr size_t kMaxItems = 9;

    struct Item {
        std::wstring text;
        bool partial = false;   // 部分採用の候補 (末尾に「…」を付けて表示する)
        bool nextWord = false;  // 読みなしの後続語予測の候補 (「次の語」色の縁を付ける)
    };

    // 入力行の内容。パネルは内容を解釈せずにそのまま出す
    struct InputLine {
        std::wstring text;     // アクセント色の下線で出す文字列
        std::wstring pending;  // text に続けて副次色・点線の下線で出す文字列 (末尾にキャレット)
        // text・pending が空のときに副次色で出す文字列 (後続語予測の前文脈)
        std::wstring context;
        bool nextWordLabel = false;  // context の後ろに「次の語」のラベルを付ける
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
        // Caret のとき、未完成ローマ字のカプセル (キャレットに重ねて出す) の下端より下に出す
        bool belowCapsule = true;
    };

    SuggestPanel();
    ~SuggestPanel();
    SuggestPanel(const SuggestPanel&) = delete;
    SuggestPanel& operator=(const SuggestPanel&) = delete;

    // 候補を横一列に並べ、input があれば上に入力行を置いて出す。収まる先頭からの候補だけを
    // 表示し、表示した候補の数を返す (候補が無く入力行だけを出したときも 0)。
    // 候補も入力行も無ければ隠す。selection が範囲外ならどの候補も強調しない
    size_t Show(const Placement& placement, const std::vector<Item>& items, size_t selection,
                const InputLine* input);

    // 未完成ローマ字のカプセル。caret (スクリーン座標) の左上に重ねて出す
    bool ShowCapsule(const RECT& caret, const std::wstring& text);

    // 表示中のパネルの矩形 (影を除く。スクリーン座標)。非表示なら false
    bool WindowRect(RECT* rect) const;

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // フォントが見つからなければ Yu Gothic UI を使う。表示中の内容には次の Show から効く
    void SetFont(const std::wstring& face, int height);

    // 配色と寸法の比率を差し替える。表示中なら描き直す
    void SetTheme(const PanelTheme& theme);

    // 背景のぼかし (設定 panel_blur) を使うかどうか。変わったときは表示中なら隠し、次の表示から
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

    // 表示中かどうか (Hide 済み・未表示なら false)
    bool Visible() const { return hwnd_ != nullptr; }

    // ウィンドウクラス登録から参照するため public にしている
    static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam);

private:
    enum class Kind {
        Panel,    // Show
        Capsule,  // ShowCapsule
    };

    bool EnsureFactories();
    bool EnsureTarget();
    void ReleaseTarget();
    // text を format で1行に並べたときの幅 (px)
    float TextWidth(const std::wstring& text, IDWriteTextFormat* format) const;
    // 今のレイアウトをウィンドウ (bodyX_, bodyY_ の位置) に描いて出す
    bool Render(const wchar_t* kind, const RECT* anchor);
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
    void DrawInputLine(float left, float top, float width);
    void DrawChip(size_t index, float left, float top);
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
    IDWriteTextFormat* textFormat_;    // 候補文字列・カプセル (f)
    IDWriteTextFormat* inputFormat_;   // 入力行
    IDWriteTextFormat* badgeFormat_;   // 候補番号 (中央揃え)
    IDWriteTextFormat* labelFormat_;   // 「次の語」ラベル
    std::wstring face_;
    int fontSize_;
    PanelTheme theme_;
    bool blur_;          // ぼかしを使う設定
    bool blurFailed_;    // API が使えずぼかしをやめた (SetBlur で有効にし直すまで試さない)
    bool blurActive_;    // 今のウィンドウがぼかしの方式

    // 今のレイアウト (座標は影を除いたパネルの左上を原点とする px)
    Kind kind_;
    std::vector<Item> items_;
    std::vector<std::wstring> itemTexts_;  // 表示する候補文字列 (「…」を付けたもの)
    std::vector<float> chipX_;
    std::vector<float> chipWidth_;
    size_t selection_;
    bool hasInput_;
    InputLine input_;
    std::wstring capsuleText_;
    int bodyX_;       // パネル (影を除く) の左上 (スクリーン座標)
    int bodyY_;
    int bodyWidth_;
    int bodyHeight_;
    int margin_;      // 影のためにパネルの外側に取る余白 (ぼかしの方式では 0)
};
