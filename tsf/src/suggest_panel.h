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
    // 番号 (1〜9) を付けて出せる候補の数
    static constexpr size_t kMaxItems = 9;

    // 候補の種類 (縦積みの種類のタグに出す)
    enum class ItemKind {
        Ascii,       // 英字
        Conversion,  // 全体変換
        Prediction,  // 予測
        Partial,     // 先頭文節 (部分採用。末尾に「…」を付けて表示する)
        NextWord,    // 読みなしの後続語予測 (横一列では「次の語」色の縁を付ける)
    };

    struct Item {
        std::wstring text;
        ItemKind kind = ItemKind::Conversion;
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
        // Caret のとき、未完成ローマ字のカプセル (キャレットに重ねて出す) の下端より下に出す
        bool belowCapsule = true;
    };

    SuggestPanel();
    ~SuggestPanel();
    SuggestPanel(const SuggestPanel&) = delete;
    SuggestPanel& operator=(const SuggestPanel&) = delete;

    // 候補をテーマの並べ方 (横一列・縦積み) で並べ、input があれば上に入力行を置いて出す。
    // 収まる先頭からの候補だけを表示し、表示した候補の数を返す (候補が無く入力行だけを出したときも 0)。
    // 候補も入力行も無ければ隠す。selection が範囲外ならどの候補も強調しない
    size_t Show(const Placement& placement, const std::vector<Item>& items, size_t selection,
                const InputLine* input);

    // 変換の候補一覧。テーマの layout によらず縦積みにし、種類のタグは出さない。selection を含む
    // ページ (kMaxItems 件。作業領域に収まらなければ収まる行数) を出し、番号はページ内で 1 から振る。
    // 複数ページなら最後の行の下に位置 (12/24) を出す。Caret では placement.caret (composition の
    // 矩形) の下 (入らなければ上) の placement.x に、Center では候補バーと同じ上端から下へ伸ばして出す。
    // candidates が空なら入力行だけを出し、input も無ければ隠す。
    // 候補一覧を出したときは 1 ページの件数 (数字キーの対応付けに使う) を、出さなかったときは 0 を返す
    size_t ShowConversion(const Placement& placement, const std::vector<std::wstring>& candidates,
                          size_t selection, const InputLine* input);

    // 未完成ローマ字のカプセル。caret (スクリーン座標) の左上に重ねて出す
    bool ShowCapsule(const RECT& caret, const std::wstring& text);

    // 表示中のパネルの矩形 (影を除く。スクリーン座標)。非表示なら false
    bool WindowRect(RECT* rect) const;

    // 選択中の候補を変えて再描画する
    void SetSelection(size_t selection);

    // 描画フォントを差し替える (設定変更の反映用)。height は px 単位の文字高。
    // フォントが見つからなければ Yu Gothic UI を使う。表示中の内容には次の Show から効く
    void SetFont(const std::wstring& face, int height);

    // 見た目を差し替える。表示中なら、色だけが変わったときは描き直し、並べ方・形・寸法が
    // 変わったときは隠す (今のレイアウトは前の見た目で組んであるため)
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
    IDWriteTextFormat* textFormat_;    // 候補文字列・カプセル (f。収まらなければ末尾を「…」で切る)
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

    // 今のレイアウト (座標は影を除いたパネルの左上を原点とする px)
    Kind kind_;
    bool conversion_;  // 変換の候補一覧 (ShowConversion) を出している
    std::vector<Item> items_;
    std::vector<std::wstring> itemTexts_;  // 表示する候補文字列 (「…」を付けたもの)
    std::vector<ItemBox> itemBoxes_;
    float inputTop_;  // 入力行の上端 (パネルの左上を原点とする px)
    bool separator_;  // 入力行と候補の間に区切り線を引く
    std::wstring pageText_;  // 変換の候補一覧の位置の表示 (空なら出さない)
    float pageTop_;          // 位置の表示の上端 (パネルの左上を原点とする px)
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
