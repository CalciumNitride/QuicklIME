// 設定ツールの「見た目」ページに出す入力パネルの見本 (docs/spec/display.md「入力パネルの見本の描画」)。
// 設定ツールが LoadLibraryW したこの DLL の QuicklimePanelPreview を呼ぶ

#include <windows.h>

#include <cstring>
#include <new>
#include <string>
#include <vector>

#include "panel_theme.h"
#include "suggest_panel.h"

namespace {

enum PreviewSample : int {
    kSampleBar = 0,         // 入力パネル (Show)
    kSampleConversion = 1,  // 変換の候補一覧 (ShowConversion)
};

// 見本の位置の基準にする、プライマリモニタの作業領域の左上のキャレット。下に収まるので
// 縦積みでも並びを反転しない
RECT PreviewCaret(int fontSize)
{
    RECT work = {};
    MONITORINFO info = {};
    info.cbSize = sizeof(info);
    if (GetMonitorInfoW(MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY), &info)) {
        work = info.rcWork;
    } else {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
    }
    return {work.left, work.top, work.left + 1, work.top + fontSize};
}

bool DrawSample(SuggestPanel* panel, int sample, const RECT& caret)
{
    SuggestPanel::Placement placement;
    placement.position = SuggestPanel::Position::Caret;
    placement.caret = &caret;
    placement.x = caret.left;

    SuggestPanel::InputLine input;
    if (sample == kSampleBar) {
        input.text = L"きょうは";
        const std::vector<SuggestPanel::Item> items = {
            {L"今日は", SuggestPanel::ItemKind::Conversion},
            {L"京は", SuggestPanel::ItemKind::Conversion},
            {L"今日はいい天気", SuggestPanel::ItemKind::Prediction},
            {L"教派", SuggestPanel::ItemKind::Conversion},
            {L"kyouha", SuggestPanel::ItemKind::Ascii},
        };
        return panel->Show(placement, items, 1, &input) > 0;
    }
    input.segments = {L"今日は", L"いい", L"天気"};
    input.currentSegment = 0;
    const std::vector<std::wstring> candidates = {L"今日は", L"京は", L"教派", L"強は",
                                                  L"きょうは"};
    return panel->ShowConversion(placement, candidates, {}, 1, &input) > 0;
}

} // namespace

// テーマ themeName (設定 panel_theme の値)、フォント fontFace、大きさ fontSize (px) で、
// sample (0: 入力パネル、1: 変換の候補一覧) の見本を描く。
// 成功したら pixels (capacity バイト) に上から下へ並ぶ BGRA (premultiplied alpha) の画素を書き、
// S_OK を返す。capacity が足りなければ何も書かずに HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)
// を返す (pixels は null でもよい)。どちらでも *width・*height (px) と、テーマを解決したときの
// ダーク (*dark)・ぼかし (*blur) を書く。必要なバイト数は *width * *height * 4。
// 呼ぶたびにテーマを読み直してパネルを作り直す (状態を持たない)。ぼかしは再現せず、ぼかしの無い
// 描画で描く
extern "C" HRESULT WINAPI QuicklimePanelPreview(const wchar_t* themeName, const wchar_t* fontFace,
                                                int fontSize, int sample, BYTE* pixels,
                                                UINT32 capacity, UINT32* width, UINT32* height,
                                                BOOL* dark, BOOL* blur)
{
    if (themeName == nullptr || fontFace == nullptr || fontSize <= 0 || width == nullptr ||
        height == nullptr || dark == nullptr || blur == nullptr ||
        (sample != kSampleBar && sample != kSampleConversion)) {
        return E_INVALIDARG;
    }
    *width = 0;
    *height = 0;
    *dark = FALSE;
    *blur = FALSE;
    // 呼び出し元 (Rust の設定ツール) へ C++ の例外を出さない
    try {
        PanelThemeLoader loader;
        std::vector<std::wstring> log;
        loader.Refresh(themeName, true, &log);
        const PanelTheme& theme = loader.Get();
        *dark = theme.dark ? TRUE : FALSE;
        *blur = theme.blur ? TRUE : FALSE;

        SuggestPanel panel;
        panel.SetOffscreen(true);
        panel.SetFont(fontFace, fontSize);
        panel.SetTheme(theme);
        const RECT caret = PreviewCaret(fontSize);
        if (!DrawSample(&panel, sample, caret)) {
            return E_FAIL;
        }
        const std::vector<BYTE>& snapshot = panel.SnapshotPixels();
        *width = static_cast<UINT32>(panel.SnapshotWidth());
        *height = static_cast<UINT32>(panel.SnapshotHeight());
        if (pixels == nullptr || capacity < snapshot.size()) {
            return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
        }
        std::memcpy(pixels, snapshot.data(), snapshot.size());
        return S_OK;
    } catch (const std::bad_alloc&) {
        return E_OUTOFMEMORY;
    } catch (...) {
        return E_FAIL;
    }
}
