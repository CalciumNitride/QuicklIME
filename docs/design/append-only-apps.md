# アプリごとの追記のみ扱い

指定したアプリでは、文書が読めても「追記のみ」の文書として扱い、文字の置き換えを擬似 Backspace で
行う。対象アプリは設定 `append_only_apps` に exe 名で並べ、設定ツールの「アプリ」タブ
(docs/design/app-settings.md) で選ぶ。既定は `Discord.exe`。

## 背景と目的

Discord の検索欄では、変換・サジェストの採用で run を置き換えると、最後のかなだけが消えて残りが
残る (「きょう」を「今日」にすると「きょ今日」になる)。

- ログでは、検索欄は「読める」文書に分類され、置換は `ReplaceRunEditSession` が run の範囲に
  composition の外から `SetText` をかける経路を通っていた。TSF から見た置換は成功している
- run 全体を選択してから選択範囲に `SetText` する方法を試したが、検索欄・入力欄の両方で
  「きょう今日」になった (選択範囲が置き換わらず、挿入だけが入る)
- Discord のエディタは、TSF 経由の範囲指定の編集を自分の内部モデルへ正しく取り込まない。
  範囲の渡し方を変えても直らないので、キーイベント (Backspace) で消す「追記のみ」の経路を使う

## 決定事項

### 判定と分類の読み替え

- 判定は TSF 層の `TextService::IsAppendOnlyApp()` で行う。ホストの exe 名 (`hostExeName_`) を
  `append_only_apps` と照合する (大文字小文字を区別しない。`IsNoLearningApp()` と同じ方法)
- 文書の分類の変更はすべて `TextService::SetAppendDocument` を通る。対象アプリで `Readable` に
  分類しようとしたときは、`AppendOnly` に読み替える
  - ログの理由には、元の理由に `append_only_apps` で読み替えたことを足す
  - `Unknown` (未判定) の扱いは変えない。フォーカス移動で未判定に戻り、2文字目以降の追記などで
    分類する流れはそのまま
- フォーカスした時点で最初から `AppendOnly` にする案は採らない。未判定を前提にした処理 (分類の
  契機、確定アンドゥの記憶の扱いなど) に影響が広がるため。分類の入口を1か所で読み替えれば足りる

### 対象アプリでの挙動

追記のみの文書 (docs/design/append-input.md) と同じになる。

- 変換: 擬似 Backspace で run を消し、空の composition を張って候補選択に入る
- サジェスト: Tab・↑↓ では文書を書き換えず、採用時に擬似 Backspace で消してから追記する
- 確定アンドゥ: 照合なしで擬似 Backspace を使うので、アプリへ渡したキー・run の外の Space・
  マウス操作・フォーカス移動で記憶を捨てる
- run の間と確定アンドゥの記憶がある間は、マウスフックでキャレット移動を検知する
- 失うもの: 置換前の文書との照合、文書から読む LLM の左文脈 (履歴で代用される)。
  Discord の入力欄も対象になるが、置換が確実に効くことを優先する

### 設定ファイル

- キー `append_only_apps`、値は exe 名のカンマ区切り (`newline_apps` と同じ形式。読み方も
  `ParseAppList` を共用する)
- 既定は `Discord.exe`。config.tsv に行が無ければ既定を使い、空の値で書かれていれば空 (対象なし)
  とする (`newline_apps` と同じ)
- 既定に Slack・Teams など他の Electron 系アプリは入れない。症状を確かめていないため
- TSF 層の設定の再読込で反映する (ほかのアプリ別設定と同じ)

### 設定ツールの「アプリ」タブ

docs/design/app-settings.md の一覧と編集欄に1項目ずつ足す。

```
┌──────────────────────────────────────────────────────────┐
│ exe 名              │ 改行キー │ 起動時の IME │ 学習   │ 置換 │
│ Discord.exe         │ ✓       │ 自動         │        │ BS   │
│ WindowsTerminal.exe │         │ オフ         │ しない │      │
└──────────────────────────────────────────────────────────┘
選択中のアプリの設定
  [ ] 改行キー (Ctrl+J など) で Shift+Enter を送る
  起動時の IME: [オン ▼]
  自動: IME のオン/オフをアプリやシステムに任せます
  [ ] このアプリでは学習しない
  [ ] 文字の置き換えを Backspace キーで行う (置き換えで文字が残るアプリ向け)
```

- 一覧に列「置換」を足す。対象アプリは `BS`、それ以外は空
- 編集欄にチェックボックス「文字の置き換えを Backspace キーで行う (置き換えで文字が残るアプリ向け)」
  を足す。変えるとすぐ一覧の列に反映する
- `AppSetting` に `append_only: bool` を足す。追加したアプリは `false` から始める
- 読み込み: `newline_apps` → `ime_start_apps` → `no_learning_apps` → `append_only_apps` の順に
  アプリを並べる (先のキーにあるアプリは重ねず、その行の設定を立てる)
- 保存: 対象アプリを一覧の順に `append_only_apps` へ書く。空なら空の値で書く
- 4つの設定がすべて既定 (改行キーなし・自動・学習する・置換は通常) のアプリは、どのキーにも書かれず、
  開き直すと一覧から消える (app-settings.md と同じ)
- [既定に戻す] では、既定の一覧 (改行キーの既定のアプリ) に戻り、`Discord.exe` の「置換」が `BS` に
  なる

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h / config.cpp | `TsfConfig::appendOnlyApps` (既定 `DefaultAppendOnlyApps()` = `Discord.exe`) と `append_only_apps` の読み込み |
| tsf/src/text_service.h / text_service.cpp | `IsAppendOnlyApp()` |
| tsf/src/text_service_direct.cpp | `SetAppendDocument` での `Readable` → `AppendOnly` の読み替え |
| engine/src/bin/quicklime-config.rs | `AppSetting::append_only`、`append_only_apps` の読み書きと既定値、一覧の列と編集欄のチェックボックス、単体テスト |
| README.md | 「アプリ」タブと config.tsv のキーの説明に追記のみ扱いを足す |
| docs/design/app-settings.md | アプリ別の設定が4つになったことを冒頭の説明に反映し、本書を指す |
| docs/design/README.md | 本書を索引に足す |

## 実装手順

1. TSF 層: 設定の読み込みと既定値、`IsAppendOnlyApp()`、`SetAppendDocument` での読み替え
2. 設定ツール: `AppSetting` と読み書き・既定値 (単体テストを含む)、一覧の列、編集欄のチェックボックス
3. ビルド (TSF の Debug、エンジンの `cargo build`) と `cargo test`
4. ドキュメント

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、Discord はトレイからも完全に終了してから
起動し直す。config.tsv に `debug_log` を設定し、ログで文書の分類 (`append_only_apps` による読み替え)
と擬似 Backspace の送信を確かめる。メモ帳での確認は、隔離した環境 (`QUICKLIME_CONFIG_FILE` など)
から `taskkill /IM Notepad.exe /F` の後に起動して行う。

### テストケース

| # | 環境・設定 | 操作 | 期待 |
|---|---|---|---|
| 1 | Discord の検索欄 (既定の設定) | `kyou` → 変換キー → 確定キー | 「今日」になる。ログで分類が「追記のみ」(読み替え) になり、擬似 Backspace を送っている |
| 2 | Discord の検索欄 | `kyou` → Tab でサジェストを選ぶ → Enter | 選択中は文書が変わらず、Enter で「今日」になる |
| 3 | Discord の入力欄 | 1・2 と同じ操作 (送信はしない) | 「今日」になる |
| 4 | Discord の入力欄 | `kyouha` → 変換キー → 確定 → Ctrl+Backspace | 「きょうは」に戻り、run が再開する |
| 5 | Discord の入力欄 | `kyouha` → 変換キー → 候補選択中に Esc | かなが1回だけ残り、入力を続けられる |
| 6 | Discord の入力欄 | `kyou` → 別の位置をクリック → 変換キー | run が終わっていて、文字が消されない |
| 7 | Discord の入力欄 | `kyouha` → Ctrl+J | 「きょうは」が確定し、改行が入る (送信されない) |
| 8 | Discord の入力欄 | `kyouha` → 変換キー → ←→ で範囲を伸縮 → 確定 | 選んだ範囲だけが変換され、元のかなが残らない |
| 9 | メモ帳 (既定の設定) | `kyouha` → 変換 → 確定、サジェストの採用 | 今までどおり「読める」文書として置き換わる |
| 10 | `append_only_apps Notepad.exe` | メモ帳で 1・2 の操作 | ログで分類が「追記のみ」(読み替え) になり、正しく置き換わる |
| 11 | `append_only_apps notepad.exe` (小文字) | 10 と同じ | 同上 |
| 12 | `append_only_apps` を空の値で書く | Discord の検索欄で 1 の操作 | 「読める」文書のまま (元の症状が出る) |
| 13 | config.tsv に `append_only_apps` の行なし | 設定ツールの「アプリ」タブを開く | Discord.exe の行の「置換」が `BS`、ほかの既定のアプリは空 |
| 14 | 13 と同じ | Discord.exe のチェック「文字の置き換えを Backspace キーで行う」を外して保存 | `append_only_apps` が空の値で書かれる。Discord.exe は `newline_apps` に残る |
| 15 | 任意 | アプリを足して「文字の置き換えを Backspace キーで行う」だけをオンにして保存し、開き直す | そのアプリが一覧に残り、「置換」列が `BS` |
| 16 | 任意 | [既定に戻す] → 保存 | `append_only_apps Discord.exe` が書かれる |
| 17 | 任意 | 設定ツールで対象アプリを足して保存し、アプリを開き直さずに入力する | 設定の再読込で反映され、次の分類から「追記のみ」になる |

## 未決事項

- Discord の検索欄で「擬似 Backspace → 空の composition → 確定」が正しく動くかは、実装後の
  テストケース 1 で確かめる。動かなければ設計を見直す
