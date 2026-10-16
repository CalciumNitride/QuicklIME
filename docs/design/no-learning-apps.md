# アプリごとの学習の停止

指定したアプリでは、確定・採用の結果を学習に記録しない。対象アプリは設定 `no_learning_apps` に
exe 名で並べ、設定ツールの「アプリ」タブ (docs/design/app-settings.md) で選ぶ。

## 背景と目的

ターミナルやゲームのように、ふだんの文章と違う語 (コマンド・略語・一時的な文字列) を打つアプリで
学習が進むと、ほかのアプリでの候補の並びが崩れる。学習の有効/無効 (設定 `learning`) は全体にしか
効かないので、アプリを選んで学習だけを止められるようにする。

## 決定事項

### 止める範囲

- 対象アプリでは、TSF 層が学習のコマンド `LEARN` / `LEARN2` / `LEARNSEG` / `LEARNWORD`
  (docs/protocol.md) をエンジンへ送らない
- 既存の学習データは、対象アプリでも候補の並びに使う (全体の `learning=0` と同じ扱い)。
  記録の停止だけで、変換・予測・後続語予測の結果は対象外のアプリと変わらない
- 単語登録ツールでの手動登録 (`ADDWORD`) は止めない。人が明示的に行う操作のため
- 前文脈 (`contextSurface_`、直前に確定した文節) はメモリ上の状態で記録ではないので、これまでどおり
  更新して後続語予測と文脈補正に使う

### 判定する層

- 判定は TSF 層で行う。ホストの exe 名 (`hostExeName_`) を `no_learning_apps` と照合する
  (大文字小文字を区別しない。`IsNewlineApp()` と同じ方法)
- エンジン側で判定する案は採らない。エンジンはどのアプリからの要求かを知らないので、
  プロトコルに exe 名を足す必要がある。TSF 層で送らないようにすればエンジンとプロトコルは変えずに済む
- 学習の送信箇所は複数あるため、判定は `TextService::IsNoLearningApp()` 1つにまとめ、各送信箇所の
  手前で使う

### 設定ファイル

- キー `no_learning_apps`、値は exe 名のカンマ区切り (`newline_apps` と同じ形式。読み方も
  `ParseAppList` を共用する)。既定は空 (どのアプリでも学習する)
- 例: `no_learning_apps\tWindowsTerminal.exe,wezterm-gui.exe`
- TSF 層の設定の再読込で反映する (ほかのアプリ別設定と同じ)

### 設定ツールの「アプリ」タブ

docs/design/app-settings.md の一覧と編集欄に1項目ずつ足す。

```
┌───────────────────────────────────────────────────┐
│ exe 名              │ 改行キー │ 起動時の IME │ 学習   │
│ Discord.exe         │ ✓       │ 自動         │        │
│ WindowsTerminal.exe │         │ オフ         │ しない │
└───────────────────────────────────────────────────┘
選択中のアプリの設定
  [ ] 改行キー (Ctrl+J など) で Shift+Enter を送る
  起動時の IME: [オン ▼]
  自動: IME のオン/オフをアプリやシステムに任せます
  [ ] このアプリでは学習しない
```

- 一覧に列「学習」を足す。学習しないアプリは `しない`、学習するアプリは空
- 編集欄にチェックボックス「このアプリでは学習しない」を足す。変えるとすぐ一覧の列に反映する
- `AppSetting` に `no_learning: bool` を足す。追加したアプリは `false` から始める
- 読み込み: `newline_apps` → `ime_start_apps` → `no_learning_apps` の順にアプリを並べる
  (先のキーにあるアプリは重ねず、その行の設定を立てる)
- 保存: 学習しないアプリを一覧の順に `no_learning_apps` へ書く。空なら空の値で書く
- 3つの設定がすべて既定 (改行キーなし・自動・学習する) のアプリは、どのキーにも書かれず、開き直すと
  一覧から消える (app-settings.md と同じ)
- [既定に戻す] では `no_learning_apps` も空になる

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h / config.cpp | `TsfConfig::noLearningApps` と `no_learning_apps` の読み込み |
| tsf/src/text_service.h / text_service.cpp | `IsNoLearningApp()`。`Learn` / `LearnSegmentBoundaries` / `LearnWords` を呼ぶ箇所の手前で判定する |
| tsf/src/text_service_direct.cpp | `engine_.Learn` を呼ぶ箇所の手前で判定する |
| engine/src/bin/quicklime-config.rs | `AppSetting::no_learning`、`no_learning_apps` の読み書き、一覧の列と編集欄のチェックボックス、単体テスト |
| README.md | 「アプリ」タブと config.tsv のキーの説明に学習の停止を足す |
| docs/design/app-settings.md | アプリ別の設定が3つになったことを冒頭の説明に反映し、本書を指す |
| docs/design/README.md | 本書を索引に足す |

## 実装手順

1. TSF 層: 設定の読み込みと `IsNoLearningApp()`、学習の送信箇所 (text_service.cpp の `engine_.Learn` /
   `LearnSegmentBoundaries` / `LearnWords`、text_service_direct.cpp の `engine_.Learn` 3か所) での判定
2. 設定ツール: `AppSetting` と読み書き (単体テストを含む)、一覧の列、編集欄のチェックボックス
3. ビルド (TSF の Debug、エンジンの `cargo build`) と `cargo test`
4. ドキュメント

## 検証方法

TSF 層は `QUICKLIME_PIPE_NAME` / `QUICKLIME_LEARN_FILE` / `QUICKLIME_CONFIG_FILE` で隔離した
PowerShell からメモ帳を起動して確かめる (`taskkill /IM Notepad.exe /F` で既存のメモ帳を終了してから)。
学習ファイルは `QUICKLIME_LEARN_FILE` の learning.tsv と、同じディレクトリの learning_context.tsv /
learning_boundary.tsv / learning_word.tsv を見る。

### テストケース

| # | 設定 | 操作 | 期待 |
|---|---|---|---|
| 1 | `no_learning_apps Notepad.exe` | メモ帳で変換して2番目の候補を選んで確定する | 学習ファイルが増えない |
| 2 | 1 と同じ | メモ帳で候補バーの予測候補を採用する | 学習ファイルが増えない |
| 3 | 1 と同じ | メモ帳で文節を伸縮して確定する | learning_boundary.tsv / learning_word.tsv が増えない |
| 4 | 1 と同じ | 別のアプリ (WordPad など、同じ隔離環境から起動) で 1 の操作をする | 学習ファイルに記録される |
| 5 | `no_learning_apps notepad.exe` (小文字) | 1 の操作 | 学習ファイルが増えない |
| 6 | 1 と同じ。既存の学習で「きしゃ → 記者」が先頭 | メモ帳で「きしゃ」を変換する | 「記者」が先頭 (既存の学習は使う) |
| 7 | 1 と同じ | メモ帳で確定した直後に後続語予測を出す | 前文脈に基づく予測が出る (前文脈は更新される) |
| 8 | 1 と同じ | メモ帳から単語登録ツールで語を登録する | ユーザ辞書に登録される |
| 9 | `no_learning_apps` の行なし | 設定ツールの「アプリ」タブを開く | 既定の5アプリの「学習」列が空 |
| 10 | `newline_apps Discord.exe`、`no_learning_apps discord.exe,WindowsTerminal.exe` | 設定ツールで開く | Discord.exe (✓・自動・しない)、WindowsTerminal.exe (空・自動・しない) の2行 |
| 11 | 10 と同じ | Discord.exe のチェック「このアプリでは学習しない」を外して保存 | `no_learning_apps WindowsTerminal.exe` が書かれる。Discord.exe は `newline_apps` に残る |
| 12 | 任意 | アプリを足して「このアプリでは学習しない」だけをオンにして保存し、開き直す | そのアプリが一覧に残り、「学習」列が `しない` |
| 13 | 任意 | [既定に戻す] → 保存 | `no_learning_apps` が空で書かれる |
| 14 | 任意 | 設定ツールで対象アプリを足して保存し、メモ帳を開き直さずに 1 の操作 | 設定の再読込で反映され、学習ファイルが増えない |
