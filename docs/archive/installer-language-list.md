# インストーラで利用者の言語の一覧に QuicklIME を足す

## 背景と目的

- インストーラは DLL を `regserver` で登録するだけで、`DllRegisterServer` は COM のクラス・入力プロファイル
  (既定で有効)・TSF のカテゴリを HKLM 側に登録する。利用者 (HKCU) の言語の一覧には足さない
- そのため、インストールしても利用者の言語の一覧 (日本語のキーボード) に QuicklIME が入らないことがある。
  このとき「既定の入力方式の上書き」の一覧などに QuicklIME が出ず、利用者が「キーボードの追加」で手で足す必要がある
- インストール時に、セットアップを起動した利用者の言語の一覧へ QuicklIME を足し、手での追加を不要にする

## 決定事項

### 足す方法

- 設定ツール (quicklime-config.exe) に引数 `--add-language-list` を足す。この引数のときは画面を出さず、
  二重起動の防止 (Mutex) より前に処理して終わる。中で `input.dll` の `InstallLayoutOrTip` を
  `L"0x0411:{QuicklIME の CLSID}{プロファイルの GUID}"`・`ILOT_INSTALL` で呼ぶ
  - `input.dll` には SDK の import library が無いので、System32 から `LoadLibraryExW`
    (`LOAD_LIBRARY_SEARCH_SYSTEM32`) で読み込み、`GetProcAddress` で呼ぶ
  - CLSID とプロファイルの GUID は Rust 側にも定数で持つ (TSF 層の tsf/src/globals.cpp と同じ値)。
    両方に、もう一方と同じ値であることをコメントで書く
  - 失敗してもダイアログなどは出さない
- インストーラの `[Run]` で `{app}\quicklime-config.exe --add-language-list` を動かす。`runasoriginaluser` を付け、
  セットアップを起動した利用者の権限 (昇格していない側) で動かす。終了を待つ

採らなかった案:

- `DllRegisterServer` の中で `InstallLayoutOrTip` を呼ぶ (CorvusSKK の方法): 足されるのは昇格した側のユーザの
  一覧で、UAC で別の管理者アカウントを入力すると利用者の一覧には入らない。開発版とインストール版の regsvr32 の
  切替のたびにも呼ばれてしまう
- DLL に rundll32 から呼ぶ関数を足し、インストール先の DLL を rundll32 で呼ぶ: 更新のときに DLL がアプリに
  読み込まれていると、`restartreplace` で再起動まで古い DLL が残る。古い DLL には関数が無く、rundll32 が
  エラーのダイアログを出してセットアップが止まる。exe はインストールの前に `taskkill` で止めるので、その場で
  置き換わる
- DLL の複製を `{tmp}` に入れて rundll32 で呼ぶ: `{tmp}` は昇格した側のユーザのフォルダにあり、別の管理者で
  昇格したときに利用者から読めない
- PowerShell の `Set-WinUserLanguageList`: 言語の一覧全体を書き直すので、ほかの言語・キーボードの設定を
  壊すおそれがあり、起動も遅い

### いつ足すか

- 新規のインストールでも更新でも毎回足す。すでに入っていれば変わらない
- 利用者が自分で一覧から外していても、更新すると戻る。これは受け入れる

### 言語の一覧に日本語が無いとき

- `InstallLayoutOrTip` に任せ、日本語ごと一覧に入れる

### 失敗したとき

- インストールは止めず、何も表示しない。設定ツールの終了コードも見ない
- 一覧に出ないときの手での追加の手順を README に書く

### アンインストール

- 言語の一覧からは外さない。アンインストーラは昇格して動き、セットアップを起動した利用者を確実には扱えないため。
  プロファイルの登録が消えるので、一覧の項目は無効になり表示されない

### 範囲外

- 既定の入力方式にはしない (`ILOT_DEFPROFILE` は使わない)
- TSF 層の DLL (`DllRegisterServer` / `DllUnregisterServer` を含む) と `scripts\dev-deploy.ps1` は変えない

## 変更対象ファイル

- engine/gui/src/bin/quicklime-config.rs: 引数 `--add-language-list` の処理と冒頭の「使い方」のコメント
- engine/gui/src/win32.rs (または同じ crate の適切な場所): `InstallLayoutOrTip` を呼ぶ処理と GUID の定数
- tsf/src/globals.cpp: CLSID とプロファイルの GUID に、設定ツールにも同じ値があることのコメント
- installer/installer.iss: `[Run]` の節
- docs/spec/runtime.md: 「インストーラ」に言語の一覧に足す処理・失敗時・アンインストールで外さないことと、
  採らなかった案の要点を書く
- docs/spec/tools.md: 設定ツールの起動の節に引数 `--add-language-list` を書く
- README.md: 「インストール」に、言語の一覧に足すことと、一覧に出ないときの手での追加の手順
  (設定 > 時刻と言語 > 言語と地域 > 日本語の言語のオプション > キーボードの追加) を書く

## 実装手順

1. 設定ツールの crate に、`InstallLayoutOrTip` を動的に読み込んで `ILOT_INSTALL` で呼ぶ関数を足す
2. quicklime-config.rs の main の先頭 (Mutex より前) で `--add-language-list` を見て、1 を呼んで終わる
3. installer.iss に `[Run]` を足す
4. 仕様書・README・globals.cpp のコメントを書き換える
5. 設定ツールをビルドする

## 検証方法

- 隔離した確認: ビルドした設定ツールを、管理者でない側から `quicklime-config.exe --add-language-list` で実行し、
  画面が出ずに終わること、`Get-WinUserLanguageList` の日本語の `InputMethodTips` に
  `0411:{D8FA8028-4371-40E9-8F49-4E465ECE9A41}{C0730986-A430-4595-8D18-A4103718C6C6}` が入ることを確かめる
- テストケース:
  1. 日本語のキーボードから QuicklIME を外した状態でインストーラを実行 → 一覧に QuicklIME が入り、
     `Win + Space` と「既定の入力方式の上書き」に出る
  2. すでに一覧に入っている状態で更新 → 一覧が重複しない。ほかのキーボード (Microsoft IME など) は変わらない
  3. UAC で別の管理者アカウントを入力してインストール → セットアップを起動した利用者の一覧に入る
     (確かめられる環境があれば)
  4. DLL がアプリに読み込まれた状態で更新 → エラーのダイアログが出ずにセットアップが終わり、一覧に QuicklIME が入る
  5. アンインストール → アンインストールは成功し、一覧に QuicklIME が出ない
  6. 開発版への `scripts\dev-deploy.ps1 -Dll` の切替 → 言語の一覧が変わらない
  7. 引数なしで設定ツールを起動 → 今までどおり設定の画面が出る

## 未決事項

なし
