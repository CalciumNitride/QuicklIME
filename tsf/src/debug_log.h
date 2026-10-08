#pragma once

#include <string>

// 実験用ログ (隠し設定 debug_log)。path へ「時刻 pid=... tid=... message」の1行を
// UTF-8 で追記する。IME の DLL は複数のプロセスから同じファイルへ書くため、
// 1行ごとに開いて追記モードで書き、閉じる。失敗は無視する
void WriteDebugLog(const std::wstring& path, const std::wstring& message);
