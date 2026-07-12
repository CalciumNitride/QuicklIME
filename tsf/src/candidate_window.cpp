#include "candidate_window.h"

#include "globals.h"

namespace {

constexpr wchar_t kWindowClassName[] = L"QuicklIMECandidateWindow";
constexpr int kPadding = 4;       // ウィンドウ内側の余白
constexpr int kLinePadding = 2;   // 行間の余白
constexpr size_t kPageSize = 9;   // 1ページに表示する候補数

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

// 表示用の行文字列 ("1 候補" の形式)
std::wstring LineText(size_t index, const std::wstring& item)
{
    return std::to_wstring(index + 1) + L" " + item;
}

} // namespace

CandidateWindow::CandidateWindow() : hwnd_(nullptr), font_(nullptr), selection_(0), lineHeight_(0)
{
    font_ = CreateFontW(-18, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE, DEFAULT_CHARSET,
                        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                        DEFAULT_PITCH | FF_DONTCARE, L"Yu Gothic UI");
}

CandidateWindow::~CandidateWindow()
{
    Hide();
    if (font_ != nullptr) {
        DeleteObject(font_);
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
    HGDIOBJ oldFont = SelectObject(hdc, font_);
    TEXTMETRICW tm = {};
    GetTextMetricsW(hdc, &tm);
    lineHeight_ = tm.tmHeight + kLinePadding * 2;

    int maxWidth = 0;
    for (size_t i = 0; i < items_.size(); ++i) {
        // 番号はページ内相対 (1〜9) なので幅の計算にはどれでも同じ1桁を使う
        const std::wstring line = LineText(i % kPageSize, items_[i]);
        SIZE size = {};
        GetTextExtentPoint32W(hdc, line.c_str(), static_cast<int>(line.size()), &size);
        maxWidth = max(maxWidth, static_cast<int>(size.cx));
    }
    SelectObject(hdc, oldFont);
    ReleaseDC(nullptr, hdc);

    // 1ページ分の行 + ページ位置表示 (候補が1ページに収まる場合は省略)
    const size_t rows = min(items_.size(), kPageSize);
    const bool showPageIndicator = items_.size() > kPageSize;
    const int width = maxWidth + kPadding * 2;
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
    HGDIOBJ oldFont = SelectObject(hdc, font_);
    SetBkMode(hdc, TRANSPARENT);

    RECT client = {};
    GetClientRect(hwnd_, &client);

    // 選択位置が含まれるページだけを描画する
    const size_t page = selection_ / kPageSize;
    const size_t begin = page * kPageSize;
    const size_t end = min(begin + kPageSize, items_.size());

    for (size_t i = begin; i < end; ++i) {
        const size_t row = i - begin;
        RECT lineRect = {client.left, kPadding + static_cast<LONG>(row) * lineHeight_,
                         client.right, kPadding + static_cast<LONG>(row + 1) * lineHeight_};

        if (i == selection_) {
            FillRect(hdc, &lineRect, GetSysColorBrush(COLOR_HIGHLIGHT));
            SetTextColor(hdc, GetSysColor(COLOR_HIGHLIGHTTEXT));
        } else {
            SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        }

        const std::wstring line = LineText(row, items_[i]);
        TextOutW(hdc, kPadding, lineRect.top + kLinePadding, line.c_str(),
                 static_cast<int>(line.size()));
    }

    // ページ位置表示 (例: "12/24" = 全24候補中の12番目を選択中)
    if (items_.size() > kPageSize) {
        SetTextColor(hdc, GetSysColor(COLOR_GRAYTEXT));
        const std::wstring indicator =
            std::to_wstring(selection_ + 1) + L"/" + std::to_wstring(items_.size());
        TextOutW(hdc, kPadding,
                 kPadding + static_cast<LONG>(end - begin) * lineHeight_ + kLinePadding,
                 indicator.c_str(), static_cast<int>(indicator.size()));
    }

    SelectObject(hdc, oldFont);
}
