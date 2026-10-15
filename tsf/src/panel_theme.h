#pragma once

#include <windows.h>

#include <string>
#include <vector>

// 色 (アルファは乗算前の値)
struct PanelColor {
    float r = 0.0f;
    float g = 0.0f;
    float b = 0.0f;
    float a = 1.0f;

    bool operator==(const PanelColor&) const = default;
};

enum class PanelStyle {
    Capsule,  // 背景・縁・影のあるパネル
    Ghost,    // 枠を持たず、背景の代わりに放射状のにじみを描く
};

enum class PanelLayout {
    Row,     // 候補を横一列に並べる
    Column,  // 候補を縦に積む
};

enum class PanelAlign {
    Left,
    Center,
};

enum class ChipShape {
    Pill,       // 両端が半円
    Rounded,    // 角丸 (chipRadius)
    Underline,  // 背景を持たず、選択中だけ下線を引く
};

enum class BadgeShape {
    Circle,
    Square,
    Plain,  // 数字だけ
    None,   // 番号を出さない
};

enum class SelectionStyle {
    Fill,  // アクセント色で塗り、文字を selectedText に
    Soft,  // アクセント色を薄く敷き、文字をアクセント色に
    Text,  // 文字だけをアクセント色の太字に
};

// サジェストパネルの見た目 (docs/design/panel-theme.md)。ライト・ダークを解決した1組。
// 寸法は設定 candidate_font_size (以下 f) に対する比率、Px で終わるものは f によらない画素数
struct PanelTheme {
    PanelStyle style = PanelStyle::Capsule;
    PanelLayout layout = PanelLayout::Row;
    PanelAlign align = PanelAlign::Left;
    bool inputRowAlways = false;  // input_row always (設定 panel_input によらず入力行を出す)
    bool blurAllowed = true;      // blur allow (設定 panel_blur を効かせる)
    ChipShape chipShape = ChipShape::Pill;
    BadgeShape badgeShape = BadgeShape::Circle;
    SelectionStyle selection = SelectionStyle::Fill;
    bool drawBorder = true;   // パネルの外周の縁
    bool drawShadow = true;   // パネルの影
    bool kindTag = false;     // 候補の種類のタグ (layout column のときだけ効く)
    bool textShadow = false;  // 文字の下に薄い影を敷く
    bool dark = false;        // ダークの配色として解決したか (文字の影の色に使う)

    float panelRadius = 1.4f;      // パネルの角丸の半径 (パネルの高さの半分まで)
    float panelPadding = 0.6f;     // パネルの内側の余白
    float inputTextSize = 1.15f;   // 入力行の文字の高さ
    float inputRowHeight = 1.7f;   // 入力行の高さ (下線を含む)
    float rowGap = 0.35f;          // 入力行と区切り線、区切り線と候補の間
    float chipHeight = 1.9f;       // 候補 1 件の高さ (縦積みでは行の高さ)
    float chipPadding = 0.65f;     // 候補の左右の余白
    float chipGap = 0.35f;         // 候補どうしの間隔
    float chipRadius = 0.5f;       // chip_shape rounded の角丸の半径
    float badgeSize = 0.95f;       // 候補番号の形の大きさ
    float badgeTextSize = 0.62f;   // 候補番号の文字の高さ
    float badgeGap = 0.35f;        // 番号と候補文字列の間
    float labelTextSize = 0.62f;   // 入力行の「次の語」ラベルの文字の高さ
    float labelGap = 0.5f;         // 前文脈と「次の語」ラベルの間
    float tagTextSize = 0.62f;     // 種類のタグの文字の高さ
    float ghostTextSize = 1.7f;    // ghost の入力行の文字の高さ (inputTextSize の代わり)
    float ghostOpacity = 0.42f;    // ghost の第一候補 (ゴースト) の不透明度
    float shadowSize = 0.8f;       // 影の広がり
    float shadowOffset = 0.2f;     // 影を下へずらす量
    float centerWidth = 36.0f;     // 画面中央に出すときのパネルの幅
    float centerWidthRatio = 0.9f; // 画面中央に出すときの幅の上限 (作業領域の幅に対する比率)
    float lineWidthPx = 1.0f;      // 外周・区切り線・バッジ・次の語の縁の太さ (px)
    float underlinePx = 2.0f;      // run の文字列・選択中の候補の下線の太さ (px)
    float caretWidthPx = 2.0f;     // 入力行のキャレットの太さ (px)
    int shadowSteps = 10;          // 影を近似する角丸矩形の枚数

    PanelColor background;    // パネルの背景
    PanelColor border;        // 外周の縁
    PanelColor text;          // 文字
    PanelColor secondary;     // 副次色 (候補番号・未完成ローマ字・前文脈)
    PanelColor chip;          // 候補の背景
    PanelColor separator;     // 入力行と候補の区切り線
    PanelColor accent;        // 選択中の候補・run の下線・キャレット
    PanelColor selectedText;  // selection fill の選択中の候補の文字と番号
    PanelColor nextWord;      // 読みなしの後続語予測の縁と「次の語」ラベル
    PanelColor shadow;        // 影 (a は重ねたときの最大不透明度)
    PanelColor blurTint;      // ぼかし (panel_blur) の上に重ねる色
    PanelColor scrim;         // ghost のにじみの中心の色 (外へ向かって透明になる)
    PanelColor tagAscii;      // 種類のタグの文字の色 (背景はこの色を不透明度 0.16 で敷く)
    PanelColor tagConversion;
    PanelColor tagPrediction;
    PanelColor tagPartial;
    PanelColor tagNextWord;

    bool operator==(const PanelTheme&) const = default;
    // 色以外 (並べ方・形・寸法) が同じか
    bool SameShape(const PanelTheme& other) const;

    // すべてのキーの既定値 (組み込みの capsule と同じ見た目)
    static PanelTheme Defaults(bool dark);
};

// Windows のアプリモードがライトか (AppsUseLightTheme。値が無ければライト)
bool AppsUseLightTheme();

// テーマファイルの形式の文字列 text を theme に重ねる。base の組み込みテーマを先に重ね、
// 次に light./dark. の付かない行、最後に dark (false ならライト) に合う接頭辞の行を重ねる。
// source はログに出す名前。builtinName は text が組み込みテーマのときのその名前 (自身を
// base にしない判定に使う。ユーザのファイルなら空)。無視・丸めた行は log に足す
void ApplyThemeText(const std::wstring& source, const std::wstring& builtinName,
                    const std::wstring& text, bool dark, PanelTheme* theme,
                    std::vector<std::wstring>* log);

// 組み込みテーマの文字列。無ければ nullptr
const wchar_t* BuiltinThemeText(const std::wstring& name);

// 設定 panel_theme のテーマを解決して読み込む。テーマフォルダ
// (QUICKLIME_THEME_DIR > %APPDATA%\QuicklIME\themes) の <名前>.tsv > 同名の組み込み > capsule の順。
// TextService (スレッドごとに1インスタンス) が所有するためロックは不要
class PanelThemeLoader {
public:
    // name のテーマファイルの更新時刻を確かめ、force か、名前・更新時刻が変わったときだけ
    // (アプリモードも含めて) 読み直して true を返す。読み直しで無視した行などは log に足す
    bool Refresh(const std::wstring& name, bool force, std::vector<std::wstring>* log);

    const PanelTheme& Get() const { return theme_; }

private:
    PanelTheme theme_ = PanelTheme::Defaults(false);
    std::wstring name_;
    FILETIME lastWrite_ = {};
    bool exists_ = false;
    bool loaded_ = false;
};
