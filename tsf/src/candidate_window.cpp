#include "candidate_window.h"

#include "globals.h"

namespace {

constexpr wchar_t kWindowClassName[] = L"QuicklIMECandidateWindow";
constexpr int kPadding = 4;       // ウィンドウ内側の余白
constexpr int kLinePadding = 2;   // 行間の余白
constexpr int kNumberGap = 6;     // 番号列と候補文字列の間の余白
constexpr size_t kPageSize = CandidateWindow::kPageSize; // 1ページに表示する候補数

// 候補番号のフォント高を候補文字列より一回り小さくして控えめにする
// (候補文字列を主役として見やすくするため)
int NumberFontHeight(int candidateHeight)
{
    return max(candidateHeight * 3 / 4, 10);
}

// ウィンドウクラスを一度だけ登録する
bool EnsureWindowClass()
{
    static bool registered = false;
    if (registered) {
        return true;
    }

    WNDCLASSEXW wc = {};
    wc.cbSize = sizeof(wc);
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = CandidateWindow::WndProc;
    wc.hInstance = globals::dllInstance;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    wc.lpszClassName = kWindowClassName;
    if (RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return false;
    }
    registered = true;
    return true;
}

} // namespace

CandidateWindow::CandidateWindow()
    : hwnd_(nullptr), font_(nullptr), numberFont_(nullptr), selection_(0), lineHeight_(0),
      numberColumnWidth_(0), numberFontHeight_(0)
{
    SetFont(L"Yu Gothic UI", 18);
}

void CandidateWindow::SetFont(const std::wstring& face, int height)
{
    HFONT font = CreateFontW(-height, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE, DEFAULT_CHARSET,
                             OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                             DEFAULT_PITCH | FF_DONTCARE, face.c_str());
    if (font == nullptr) {
        return;
    }
    HFONT numberFont = CreateFontW(-NumberFontHeight(height), 0, 0, 0, FW_NORMAL, FALSE, FALSE,
                                   FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                                   CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                                   DEFAULT_PITCH | FF_DONTCARE, face.c_str());
    if (numberFont == nullptr) {
        DeleteObject(font);
        return;
    }
    if (font_ != nullptr) {
        DeleteObject(font_);
    }
    if (numberFont_ != nullptr) {
        DeleteObject(numberFont_);
    }
    font_ = font;
    numberFont_ = numberFont;
    if (hwnd_ != nullptr) {
        InvalidateRect(hwnd_, nullptr, TRUE);
    }
}

CandidateWindow::~CandidateWindow()
{
    Hide();
    if (font_ != nullptr) {
        DeleteObject(font_);
    }
    if (numberFont_ != nullptr) {
        DeleteObject(numberFont_);
    }
}

bool CandidateWindow::Show(const RECT& anchor, const std::vector<std::wstring>& items,
                           size_t selection)
{
    if (items.empty() || !EnsureWindowClass()) {
        return false;
    }
    items_ = items;
    selection_ = selection;

    // フォントで各行の寸法を測ってウィンドウサイズを決める
    HDC hdc = GetDC(nullptr);

    // 番号列の幅 (kPageSize<=9 のため番号は常に1桁 "9" で代表させて測ればよい)
    HGDIOBJ oldFont = SelectObject(hdc, numberFont_);
    TEXTMETRICW numberTm = {};
    GetTextMetricsW(hdc, &numberTm);
    numberFontHeight_ = numberTm.tmHeight;
    SIZE numberSize = {};
    GetTextExtentPoint32W(hdc, L"9", 1, &numberSize);
    numberColumnWidth_ = numberSize.cx;

    SelectObject(hdc, font_);
    TEXTMETRICW tm = {};
    GetTextMetricsW(hdc, &tm);
    lineHeight_ = tm.tmHeight + kLinePadding * 2;

    int maxItemWidth = 0;
    for (const auto& item : items_) {
        SIZE size = {};
        GetTextExtentPoint32W(hdc, item.c_str(), static_cast<int>(item.size()), &size);
        maxItemWidth = max(maxItemWidth, static_cast<int>(size.cx));
    }
    SelectObject(hdc, oldFont);
    ReleaseDC(nullptr, hdc);

    // 1ページ分の行 + ページ位置表示 (候補が1ページに収まる場合は省略)
    const size_t rows = min(items_.size(), kPageSize);
    const bool showPageIndicator = items_.size() > kPageSize;
    const int width = kPadding * 2 + numberColumnWidth_ + kNumberGap + maxItemWidth;
    const int height =
        static_cast<int>(rows + (showPageIndicator ? 1 : 0)) * lineHeight_ + kPadding * 2;
    const int x = anchor.left;
    const int y = anchor.bottom + 2;

    if (hwnd_ == nullptr) {
        hwnd_ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                                kWindowClassName, L"", WS_POPUP | WS_BORDER, x, y, width, height,
                                nullptr, nullptr, globals::dllInstance, this);
        if (hwnd_ == nullptr) {
            return false;
        }
    }

    SetWindowPos(hwnd_, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE);
    ShowWindow(hwnd_, SW_SHOWNA);
    InvalidateRect(hwnd_, nullptr, TRUE);
    return true;
}

void CandidateWindow::SetSelection(size_t selection)
{
    selection_ = selection;
    if (hwnd_ != nullptr) {
        InvalidateRect(hwnd_, nullptr, TRUE);
    }
}

void CandidateWindow::Hide()
{
    if (hwnd_ != nullptr) {
        DestroyWindow(hwnd_);
        hwnd_ = nullptr;
    }
    items_.clear();
    selection_ = 0;
}

LRESULT CALLBACK CandidateWindow::WndProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam)
{
    if (msg == WM_NCCREATE) {
        // CreateWindowExW の最終引数で渡した this を関連付ける
        auto* cs = reinterpret_cast<CREATESTRUCTW*>(lparam);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA,
                          reinterpret_cast<LONG_PTR>(cs->lpCreateParams));
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    auto* self = reinterpret_cast<CandidateWindow*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    switch (msg) {
    case WM_PAINT: {
        PAINTSTRUCT ps = {};
        HDC hdc = BeginPaint(hwnd, &ps);
        if (self != nullptr) {
            self->Paint(hdc);
        }
        EndPaint(hwnd, &ps);
        return 0;
    }
    case WM_MOUSEACTIVATE:
        // クリックされてもフォーカスを奪わない
        return MA_NOACTIVATE;
    default:
        break;
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}

void CandidateWindow::Paint(HDC hdc)
{
    SetBkMode(hdc, TRANSPARENT);
    HGDIOBJ oldFont = SelectObject(hdc, font_);

    RECT client = {};
    GetClientRect(hwnd_, &client);

    // 選択位置が含まれるページだけを描画する
    const size_t page = selection_ / kPageSize;
    const size_t begin = page * kPageSize;
    const size_t end = min(begin + kPageSize, items_.size());

    const int itemX = kPadding + numberColumnWidth_ + kNumberGap;

    for (size_t i = begin; i < end; ++i) {
        const size_t row = i - begin;
        RECT lineRect = {client.left, kPadding + static_cast<LONG>(row) * lineHeight_,
                         client.right, kPadding + static_cast<LONG>(row + 1) * lineHeight_};
        const bool isSelected = (i == selection_);

        if (isSelected) {
            FillRect(hdc, &lineRect, GetSysColorBrush(COLOR_HIGHLIGHT));
        }

        // 候補番号: 候補文字列より控えめ (一回り小さいフォント・グレー系の色) に描画し、
        // 数字を変換するときでも候補本体と混同しないようにする
        SelectObject(hdc, numberFont_);
        SetTextColor(hdc, GetSysColor(isSelected ? COLOR_HIGHLIGHTTEXT : COLOR_GRAYTEXT));
        const std::wstring number = std::to_wstring(row + 1);
        const LONG numberY = lineRect.top + (lineHeight_ - numberFontHeight_) / 2;
        TextOutW(hdc, kPadding, numberY, number.c_str(), static_cast<int>(number.size()));

        // 候補文字列: 主役として現状どおりのフォント・色で描画する
        SelectObject(hdc, font_);
        SetTextColor(hdc, GetSysColor(isSelected ? COLOR_HIGHLIGHTTEXT : COLOR_WINDOWTEXT));
        TextOutW(hdc, itemX, lineRect.top + kLinePadding, items_[i].c_str(),
                 static_cast<int>(items_[i].size()));
    }

    // ページ位置表示 (例: "12/24" = 全24候補中の12番目を選択中)
    if (items_.size() > kPageSize) {
        SelectObject(hdc, font_);
        SetTextColor(hdc, GetSysColor(COLOR_GRAYTEXT));
        const std::wstring indicator =
            std::to_wstring(selection_ + 1) + L"/" + std::to_wstring(items_.size());
        TextOutW(hdc, kPadding,
                 kPadding + static_cast<LONG>(end - begin) * lineHeight_ + kLinePadding,
                 indicator.c_str(), static_cast<int>(indicator.size()));
    }

    SelectObject(hdc, oldFont);
}
