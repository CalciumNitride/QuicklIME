# 改行キー

Enter で送信するアプリ (チャットアプリなど) で、IME のキー操作として改行を入れられるようにする。

## 背景と目的

Discord・Slack などでは Enter が送信で、改行は Shift+Enter になっている。アプリによっては
Ctrl+J も改行になるが、統一されていない。IME の機能として「改行キー」(既定 Ctrl+J) を設け、
対象のアプリではどこでも同じキーで改行を入れられるようにする。

## 決定事項

### 改行の入れ方

改行キーを食べて、アプリへ Shift+Enter を送る (`SendKeyWithModifiers`)。チャット系のアプリでは
Shift+Enter が改行の事実上の標準のため。

採らなかった案:

- TSF で `\n` を文字として挿入する: 改行コードの扱いがアプリごとに違い (`\r`・`\r\n`・`\n`)、
  Web のエディタが文字としての改行を改行として扱うかは不確実。TSF に対応していないアプリでは
  効かない
- 全アプリで有効にする: ターミナルでは Shift+Enter が Enter と同じ扱いになりやすく、コマンドが
  実行されてしまう。Excel では Shift+Enter がセルの移動になる。ブラウザ (ダウンロード)・
  VS Code (パネルの切替) では Ctrl+J のショートカットを奪う

### 対象アプリ (ホワイトリスト)

- ホストプロセスの exe 名 (`GetModuleFileNameW(nullptr, ...)` のファイル名部分) が対象リストに
  あるときだけ働く。照合は大文字小文字を区別しない
- 対象外のアプリでは、改行キーは割当なしのときと同じ扱い (アプリへ渡る。run 中なら Ctrl 併用の
  キーとして既存どおり run を終えてから渡る)
- 設定キー `newline_apps`。値は exe 名のカンマ区切り (前後の空白は無視)。値が空なら対象なし。
  設定ファイルに行が無いときは既定のリストを使う
- 既定のリスト: `Discord.exe,slack.exe,ms-teams.exe,Teams.exe,LINE.exe`。ブラウザは Ctrl+J を
  奪うため既定には入れない (必要ならユーザが足す)
- exe 名はプロセスごとに不変なので、TextService の生成時に一度だけ取得する

### 機能

| 機能 (`KeyFunc`) | 設定キー | 働く状態 | 既定の割当 | 動作 |
|---|---|---|---|---|
| `InsertNewline` | `key.insert_newline` | 全状態 | `Ctrl+J` | 対象アプリでは Shift+Enter を送る (下記) |

- 入力なし: 打鍵を食べて Shift+Enter を送る
- run 中・候補選択中・サジェスト選択中: Enter と同じ確定処理 (サジェスト選択中は選択中の候補の
  採用) をしてから Shift+Enter を送る。確定より先に改行が届かないよう、常に食べて確定の後に
  送り直す (`CommitAndResendEnter` の経路を、送り直す打鍵を Shift+Enter にして使う。
  擬似 Backspace を送ったときは書き換えが終わってから送る)
- IME オフのときは働かない (既存の機能キーと同じく `IsKeyEaten` が何も食べない)
- 対象アプリかどうかは `CanRunKeyFunc` で判定する。対象外なら一致しても食べない
- `KeyFunc` の並び順は `OpenConfig` の後 (`None` の前) に置く
- 送り直した Shift+Enter は擬似打鍵の目印 (`kPseudoKeyExtraInfo`) が付くので、IME 自身は
  処理しない

### 設定ツール

- キー割当の一覧に「改行」(`key.insert_newline`、全状態、既定 `Ctrl+J`) を追加する
- 対象アプリはテキストで入力させず、一覧から追加・削除する。読み込んだ設定に `newline_apps` の
  行が無ければ既定のリストを表示する。設定ファイルの形式 (カンマ区切り) と TSF 層は変えない

#### 「アプリ」タブ

設定ツールのタブ構成 (docs/design/config-tabs.md) の「アプリ」タブに置く。

- 見出し「改行キーの対象アプリ」と説明 (Ctrl+J などの改行キーで Shift+Enter を送るアプリ)
- 登録済みの exe 名の一覧 (ListView、単一選択)
- [実行中のアプリから追加...] [ファイルから追加...] [削除] [既定に戻す]
- 追加済みの exe 名 (大文字小文字を区別しない) は重ねて追加しない
- 変更は設定ツール内の状態に持ち、共通の [保存] で config.tsv に書く。[キャンセル] では書かない

#### 実行中のアプリから選ぶダイアログ

- 列は「exe 名」「ウィンドウタイトル」。単一選択で、[追加] かダブルクリックで1つ追加して閉じる
- 表示するのは、見えているトップレベルウィンドウ (表示中・所有者なし・タイトルが空でない) を
  持つプロセス。バックグラウンドのプロセスまで並べると探せないため
- exe 名で重複をまとめ (タイトルは最初に見つかったウィンドウのもの)、exe 名順に並べる
- 設定ツール自身と、編集中の一覧にすでにある exe は出さない
- exe のパスは `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` +
  `QueryFullProcessImageNameW` で取る。取れないプロセス (権限不足など) は飛ばす
- ストアアプリのウィンドウは所有プロセスが `ApplicationFrameHost.exe` になるため、子ウィンドウの
  うち別プロセスのものをたどって実際の exe を表示する。たどれなければ飛ばす
- アイコンは表示しない

#### ファイルから追加

標準の「ファイルを開く」ダイアログ (`GetOpenFileNameW`、フィルタ `*.exe`) で選び、
ファイル名部分だけを追加する

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h, tsf/src/config.cpp | `KeyFunc::InsertNewline`、設定キー `key.insert_newline`・既定割当 `Ctrl+J`・働く状態 (全状態)。`TsfConfig` に対象アプリのリスト (`newline_apps`) と既定値、そのパース |
| tsf/src/text_service.cpp, tsf/src/text_service.h | ホストの exe 名の取得と保持。`CanRunKeyFunc` で対象アプリの判定。機能の実行 (入力なし: Shift+Enter を送る。それ以外: 確定してから送り直す) |
| tsf/src/text_service_direct.cpp | 必要なら、確定してから送り直す経路を Shift+Enter に使えるようにする |
| engine/src/bin/quicklime-config.rs | キー割当の一覧への追加、`newline_apps` の読み書き、「アプリ」タブの中身と実行中のアプリから選ぶダイアログ、テスト |
| engine/Cargo.toml | windows-sys の feature に `Win32_System_Threading` を追加 (プロセスの exe パスの取得) |
| README.md | キー操作の一覧と設定項目に改行キーと対象アプリを追記 |
| docs/design/keymap.md | 「機能」の表に `InsertNewline` を追加 (本書を参照) |

## 実装手順

各手順の後にビルドが通ることを確かめる。

1. config: `KeyFunc::InsertNewline` と既定割当、`newline_apps` のパースと既定値
2. TSF 層: exe 名の取得、`CanRunKeyFunc` での判定、機能の実行 (入力なし・入力中)
3. 設定ツール: キー割当の一覧への追加、「アプリ」タブ (タブ化 docs/design/config-tabs.md の後に行う)、
   実行中のアプリから選ぶダイアログ、ファイルから追加、読み書きとテスト
4. ドキュメント: README.md、keymap.md

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。
config.tsv は `QUICKLIME_CONFIG_FILE` で隔離したものを使い、`debug_log` を設定する。
メモ帳で確かめるケースは、`newline_apps` に `Notepad.exe` を足して行う。

### テストケース

| # | 環境 | 設定・操作 | 期待 |
|---|---|---|---|
| 1 | Discord (既定の設定) | 入力なしで Ctrl+J | 改行が入り、送信されない |
| 2 | Discord | `kyouha` → Ctrl+J | 「きょうは」が確定し、その後に改行が入る。送信されない |
| 3 | Discord | `kyouha` → 変換キー → Ctrl+J | 変換結果が確定し、その後に改行が入る |
| 4 | Discord | `kyouha` → Tab でサジェストを選ぶ → Ctrl+J | 選んだ候補が採用され、その後に改行が入る |
| 5 | Discord | IME オフで Ctrl+J | IME は何もしない (アプリ本来の Ctrl+J) |
| 6 | Chrome (既定の設定。対象外) | 入力なしで Ctrl+J | ダウンロード画面が開く (奪わない) |
| 7 | Chrome (対象外) | `kyouha` → Ctrl+J | 既存どおり run を終えてから Ctrl+J がアプリへ渡る |
| 8 | WezTerm (対象外) | 入力なしで Ctrl+J | シェルへ Ctrl+J (LF) がそのまま届く |
| 9 | メモ帳 (`newline_apps` に `notepad.exe` を小文字で足す) | `kyouha` → 変換キー → Ctrl+J | 確定して改行が入る (大文字小文字を区別しない) |
| 10 | Discord | `newline_apps` を空にする | Ctrl+J はアプリへ渡る |
| 11 | Discord | `key.insert_newline Ctrl+Enter` | Ctrl+Enter で改行が入り、Ctrl+J はアプリへ渡る |
| 12 | 設定ツール | 対象アプリと改行キーの割当を変えて保存し、開き直す | 保存した内容が表示され、config.tsv に `newline_apps`・`key.insert_newline` が書かれている |
| 13 | 設定ツール | `newline_apps` の行が無い config.tsv で開く | 「アプリ」タブに既定のリスト (5つ) が表示される |
| 14 | Slack・Teams・LINE | 入力なしで Ctrl+J | 改行が入り、送信されない |
| 15 | 設定ツール | メモ帳を起動して [実行中のアプリから追加...] | 一覧に Notepad.exe がタイトル付きで出る。選んで追加すると対象アプリの一覧に入る。設定ツール自身と登録済みの exe は出ない |
| 16 | 設定ツール | 電卓 (ストアアプリ) を起動して [実行中のアプリから追加...] | ApplicationFrameHost.exe ではなく CalculatorApp.exe として出る |
| 17 | 設定ツール | [ファイルから追加...] で `C:\Windows\System32\notepad.exe` を選ぶ | `notepad.exe` が追加される。すでに Notepad.exe があれば重ならない |
| 18 | 設定ツール | 「アプリ」タブで削除・追加してからキャンセル | config.tsv は変わらない。開き直すと元の一覧 |
| 19 | 設定ツール | 「アプリ」タブで [既定に戻す] → 保存 | 既定の5つが config.tsv に書かれる |

## 未決事項

- 既定のリストの exe 名 (`ms-teams.exe`・`Teams.exe`・`LINE.exe`) が実際のプロセス名と合っているかは、
  実機で確かめる
