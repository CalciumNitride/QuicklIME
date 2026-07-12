#pragma once

#include <windows.h>

// COM サーバとしての登録 (HKCR\CLSID\{...})
BOOL RegisterComServer();
void UnregisterComServer();

// TSF への入力プロファイル登録 (入力方式一覧に表示されるようにする)
BOOL RegisterProfile();
void UnregisterProfile();

// TSF カテゴリ登録 (キーボードTIPであること、対応機能の宣言)
BOOL RegisterCategories();
void UnregisterCategories();
