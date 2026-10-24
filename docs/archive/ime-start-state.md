# 起動時の IME の状態 (アプリごと)

アプリの起動時に IME をオンで始めるか、オフで始めるか、自動 (アプリ・システムに任せる) にするかを、
アプリ (exe) ごとに設定できるようにする。設定ツールでの見せ方は docs/design/app-settings.md。

## 背景と目的

`ActivateEx` は IME のオン/オフ (OPENCLOSE compartment) が未設定のときだけオンにしている。
それでも、アプリによってはオフで始まり、入力を始める前に毎回オンに切り替える手間がかかる。
一方で、ターミナルのように英字の入力が主なアプリでは、オフで始まってほしい。

## 決定事項

### 設定

config.tsv に次の項目を足す。TSF 層が読む。

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `ime_start_apps` | `exe名:状態` のカンマ区切り。状態は `on` / `off` | 空 | 起動時の IME の状態をアプリごとに決める |

- 例: `Notepad.exe:on,WindowsTerminal.exe:off`
- exe 名の照合は大文字小文字を区別しない (`newline_apps` と同じ)。状態の `on` / `off` も区別しない
- 区切りの `:` は最後のものを使い、exe 名と状態のそれぞれから前後の空白・タブを除く。状態が `on` / `off` 以外の項目や、
  exe 名が空の項目は捨てる。同じ exe 名が重なったら最初の項目を使う
- 一覧に無いアプリは「自動」: 今の挙動 (未設定のときだけオンにする) のまま
  - 「自動」は一覧に載せないことで表す。値に `auto` のような状態は設けない

### 適用するタイミング

- プロセスで最初に文書へフォーカスしたとき (`ActivateEx` の時点ですでにフォーカスがあればそこ、無ければ
  最初の `OnSetFocus`。フォーカス先が null のときは数えない) に1回だけ適用する。プロセス全体で1つの
  フラグを持つ (TextService はスレッドごとに作られるため、インスタンスのメンバにはしない)
  - `ActivateEx` の時点で適用しても効かない。メモ帳では `ActivateEx` の時点の compartment は未設定で、
    そこでオンにしても、最初の文書フォーカスまでにシステムが 0 に書き換える (IMM の入力コンテキストの
    オン/オフとの同期と思われる)
  - 同じプロセスの2つ目以降のスレッド、ほかの IME から QuicklIME へ切り替えたときの再アクティブ化、
    新しく開いたウィンドウには適用しない。ユーザがアプリの中で切り替えたオン/オフを上書きしないため
- 対象のアプリで `on` なら compartment の値にかかわらずオン、`off` ならオフにする
- 一覧に無いアプリ (自動) では何もしない。`ActivateEx` の「未設定ならオンにする」処理は残すが、
  多くのアプリではその後にシステムの値で上書きされる
- 設定は `ActivateEx` の中で、適用より前に読み込んでおく
- 適用の直後と、その後の OPENCLOSE の変更 (`OnChange`) を、最初の文書フォーカスから 5 秒だけ診断ログに出す。
  適用した後でシステムに書き換えられていないかを確かめるため (メモ帳では書き換えられない)

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h, tsf/src/config.cpp | `TsfConfig` に `ime_start_apps` (exe 名と状態の組の一覧) とそのパース |
| tsf/src/text_service.cpp, tsf/src/text_service.h | プロセスで最初の文書フォーカスでの1回だけの適用 (ホストの exe 名 `hostExeName_` で照合)。設定の読み込みを適用より前にする |
| engine/src/bin/quicklime-config.rs | `ime_start_apps` の読み書き (画面は docs/design/app-settings.md) |
| README.md | 設定項目に起動時の IME の状態を足す |
| docs/design/README.md | 本書を索引に足す |

## 実装手順

各手順の後にビルドが通ることを確かめる。

1. TSF 層: `ActivateEx` と最初の文書フォーカスで compartment の値を診断ログに出し、実機で値の変化を
   確かめる。結果に応じて適用するタイミングを決める (上記)
2. config: `ime_start_apps` のパース
3. TSF 層: プロセスで1回だけの適用
4. 設定ツール: docs/design/app-settings.md の実装に含める
5. ドキュメント: README.md

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、`QUICKLIME_CONFIG_FILE` で隔離した config.tsv と
`debug_log` を使う。各ケースの前に `taskkill /IM Notepad.exe /F` でメモ帳を終了してから開き直す。

### テストケース

| # | 設定 | 操作 | 期待 |
|---|---|---|---|
| 1 | `ime_start_apps` なし | メモ帳を起動する | 今と同じ挙動 (自動) |
| 2 | `Notepad.exe:on` | メモ帳を起動する | IME オンで始まる (言語バーが「あ」) |
| 3 | `notepad.exe:OFF` | メモ帳を起動する | IME オフで始まる (大文字小文字を区別しない) |
| 4 | `Notepad.exe:on` | 起動後に IME をオフにし、新しいウィンドウ (Ctrl+N) を開く | 新しいウィンドウでもオンに戻されない (Windows 11 のメモ帳は同じプロセス) |
| 5 | `Notepad.exe:on` | 起動後に IME をオフにし、Microsoft IME へ切り替えてから QuicklIME に戻す | オンに戻されない |
| 6 | `Notepad.exe:off` | 起動してからメモ帳と別のアプリを行き来する | メモ帳は手で切り替えるまでオフのまま。別のアプリの状態は変わらない |
| 7 | `WindowsTerminal.exe:off,Notepad.exe:on` | 両方を起動する | ターミナルはオフ、メモ帳はオンで始まる |
| 8 | `Notepad.exe:yes,:on, Notepad.exe : off ` | メモ帳を起動する | 不正な項目は捨て、`Notepad.exe : off` が効いてオフで始まる |
| 9 | `Notepad.exe:on,notepad.exe:off` | メモ帳を起動する | 最初の項目が効いてオンで始まる |
| 10 | `Notepad.exe:on` | メモ帳の起動中に設定を `Notepad.exe:off` に変えて保存し、メモ帳に戻る | 起動済みのメモ帳の状態は変わらない。メモ帳を開き直すとオフで始まる |
