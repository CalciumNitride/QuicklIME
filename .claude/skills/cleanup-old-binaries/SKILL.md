---
name: cleanup-old-binaries
description: QuicklIME のビルド・デプロイで溜まった旧DLL/旧exe (.old, .old2, .locked-* 等の退避ファイル) をゴミ箱へ削除する。「旧DLL消して」「掃除して」「溜まってるファイル削除して」等と言われたときに使う
---

`scripts\cleanup-old-binaries.ps1` を実行する。

対象は tsf のビルド出力 (build/build32 の Debug/Release)、engine のビルド出力
(target/debug, target/release)、常用インストール環境 (Program Files) の
DLL/exe に残る退避ファイル (`.old`, `.old2`, `.old-<日時>`, `.locked-<タイムスタンプ>` 等)。
現役ファイルとビルドシステムの管理ファイル (`.recipe`) は対象外。

## 実行手順

1. `powershell -File scripts\cleanup-old-binaries.ps1` を実行する
   (対象一覧の確認だけしたい場合は `-WhatIf` を付ける)
2. 「使用中の可能性、スキップ」と報告されたファイルは、無理に別の削除方法
   (PowerShell の VisualBasic API、Shell.Application の MoveHere 等) を試さないこと。
   使用中のファイルに対してこれらは不可解なエラーになったりハングしたりする
   ([[trash-command-pitfalls]] 参照。Shell.Application 経由の削除がタイムアウトし
   強制停止する事態になった実例がある)。一覧をユーザーに報告し、手動削除に委ねてよい
3. 結果 (削除件数・スキップ件数) を要約してユーザーに報告する

## 背景・既知の罠

- `trash` (npm trash-cli) に Windows パスを渡す際、バックスラッシュ区切りだと
  内部で glob として誤解釈され、エラーなしで静かに何もしない
  (スクリプト内でフォワードスラッシュに変換済み)
- `Get-ChildItem -Filter "name.*"` は Win32 のレガシーなワイルドカード規則により
  "name" 自身 (現役ファイル) にもマッチしてしまう罠がある
  (スクリプト内では正規表現マッチで回避済み。2026-07-22 に発見、危うく現役の
  QuicklIME.dll / quicklime-engine.exe を削除対象に含めるところだった)
- 日本語コメントを含む `.ps1` は BOM 付き UTF-8 で保存すること。BOM 無しだと
  Windows PowerShell が Shift_JIS として誤読し、構文エラーになることがある
