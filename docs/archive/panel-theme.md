# 入力パネルのテーマ (見た目のカスタマイズ・縦積み・ゴースト)

## 背景と目的

入力パネル (docs/archive/suggest-panel.md) はモックの案 A (カプセル) の見た目で作った。
色と寸法は `PanelTheme` にまとめてあるが、ユーザは変えられない。本書では次の 3 つを入れる。

- 見た目をユーザが変えられるようにする (色・形・大きさ)
- 候補を縦に積む並べ方を足す
- モックの案 D (ゴースト HUD: 枠を持たず、大きな文字で入力と第一候補を出す) を足す

これらはどれも「見た目の違い」なので、まとめて**テーマ**として扱う。A・縦積み・D は
組み込みのテーマとして持ち、ユーザは組み込みのテーマを土台に、変えたい値だけを書いた
テーマファイルを作れるようにする。

## 決定事項

### 方式: 値を書くテーマファイル

[docs/spec/display.md](../spec/display.md)「方式」へ移した

### テーマの選び方と置き場所

[docs/spec/display.md](../spec/display.md)「テーマの選び方」へ移した

### 組み込みのテーマ

[docs/spec/display.md](../spec/display.md)「組み込みテーマ」と「テーマファイルの読み込み」へ移した (設定ツールでの表記は [docs/user/theme.md](../user/theme.md)「テーマの選び方」)

### テーマファイルの形式

[docs/spec/display.md](../spec/display.md)「テーマファイルの読み込み」へ移した (書式は [docs/user/theme.md](../user/theme.md)「テーマファイルの書式」)

### キー

[docs/spec/display.md](../spec/display.md)「候補の描き方」と「テーマファイルの読み込み」へ移した (キーの一覧は [docs/user/theme.md](../user/theme.md)「キー」)

### 組み込みテーマの値

[docs/spec/display.md](../spec/display.md)「組み込みテーマ」へ移した (値は [docs/user/theme.md](../user/theme.md)「組み込みテーマの値」)

### 縦積み (`layout column`)

[docs/spec/display.md](../spec/display.md)「候補の描き方」「キャレットの下 (`caret`)」「キャレット行の上に出すときの反転」「画面中央 (`center`)」へ移した

### ゴースト (`style ghost`)

[docs/spec/display.md](../spec/display.md)「入力行」と「候補の描き方」へ移した

### 揃え (`align`)

[docs/spec/display.md](../spec/display.md)「入力行」「候補の描き方」「画面中央 (`center`)」へ移した

### 読み直し

[docs/spec/display.md](../spec/display.md)「読み直し」へ移した

### 設定ツール

- 「表示」タブで、「候補の表示位置」の直前に「入力パネルの見た目」のコンボボックスを置く。
  項目は組み込みのテーマ (上の表記) に続けて、テーマフォルダの `*.tsv` のファイル名 (拡張子なし)。
  組み込みと同名のファイルは組み込みの項目として 1 つだけ出す
- その横に「テーマのフォルダを開く」ボタンを置く。フォルダが無ければ作ってから開く
- config.tsv の書き出しでは `# 候補ウィンドウ` の節、`candidate_position` の前に `panel_theme` を書く

### ドキュメント

- ユーザ向けのテーマファイルのリファレンス (キー・値・範囲・例・組み込みテーマの値) を
  docs/theme.md に書き、README.md から参照する
- .claude/CLAUDE.md の隔離用の環境変数の一覧に `QUICKLIME_THEME_DIR` を足す
- docs/archive/suggest-panel.md の未決事項のうち、見た目のカスタマイズと案 D は本書へ参照を移す

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/panel_theme.h, tsf/src/panel_theme.cpp (新規) | `PanelTheme` を suggest_panel.h から移し、新しい項目を足す。テーマファイルの解釈、組み込みテーマの文字列、名前の解決、更新時刻の確認 |
| tsf/src/suggest_panel.h, tsf/src/suggest_panel.cpp | 形の選択肢、縦積み、ゴースト、揃え、種類のタグ、文字の影 |
| tsf/src/text_service.h, tsf/src/text_service.cpp, tsf/src/text_service_direct.cpp | テーマの読み直し、候補の種類を渡す、`input_row always`・`blur never` の反映 |
| tsf/src/config.h, tsf/src/config.cpp | `panel_theme` の読み込み |
| tsf/CMakeLists.txt | panel_theme.cpp の追加 |
| engine/src/bin/quicklime-config.rs | 「入力パネルの見た目」コンボ、テーマのフォルダを開くボタン、`panel_theme` の読み書き |
| docs/theme.md (新規), README.md, .claude/CLAUDE.md, docs/archive/suggest-panel.md | 上の「ドキュメント」 |

## 実装手順

1. `PanelTheme` を panel_theme.{h,cpp} に移し、テーマファイルの解釈と組み込みの `capsule` を、今と同じ
   見た目になるように作る。`panel_theme` と `QUICKLIME_THEME_DIR`、読み直しをつなぐ
2. 形の選択肢 (`chip_shape`・`badge_shape`・`selection`・`border`・`shadow`・`text_shadow`) と `align` を
   横一列のカプセルに実装する
3. 候補の種類を `SuggestPanel::Item` に持たせ、縦積みと種類のタグ、組み込みの `capsule-column` を実装する
4. ゴーストと組み込みの `ghost`、`input_row always`・`blur never` を実装する
5. 設定ツールとドキュメント

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、メモ帳などのテスト用アプリで行う。
config.tsv は `QUICKLIME_CONFIG_FILE`、テーマフォルダは `QUICKLIME_THEME_DIR` で隔離し、`debug_log` を設定する。
TSF 層のテスト前に `taskkill /IM Notepad.exe /F` でメモ帳を終了しておく。

### テストケース

| # | 環境 | 設定・操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | `panel_theme` 未設定で `kyouha` | 今と同じ見た目 (案 A) で出る |
| 2 | メモ帳 | `panel_theme capsule-column`、`kyouha` | 候補が縦に積まれ、各行の右に種類のタグ (変換・予測など) が出る |
| 3 | メモ帳 | 2 で Tab ×3 → Shift+Tab | 選択が下へ 3 行動き、1 行戻る。選択中の行だけ薄いアクセント色で強調される |
| 4 | メモ帳 | 2 で `kisyanokisyagakisyasuru` | 先頭文節の候補に「文節」のタグと末尾の「…」が付く |
| 5 | メモ帳 | 2 で `kyouha` → 無変換 | 後続語予測の行に「次の語」のタグが付く |
| 6 | メモ帳 | 2 で `candidate_position center`、`kyouhaiitenki` を 1 文字ずつ打つ | パネルの上端・入力行・先頭の行が動かず、下へ伸び縮みする |
| 7 | 画面下端付近の入力欄 | 2 で `candidate_position caret`、`kyouha` | 1 打目からキャレット行の上に出て、下端がキャレット行の上に揃う。入力行がいちばん下、1 番目の候補がそのすぐ上にあり、打鍵で候補の数が変わっても入力行と各候補の位置は動かない |
| 8 | メモ帳 | `panel_theme ghost`、`candidate_position center`、`panel_input 0`、`kyouha` | 枠の無い大きな文字で「きょうは → 今日は」が中央に出て、下に番号付きの候補が並ぶ。キャレット付近にローマ字の小窓は出ず、`ky` の段階から入力行に出る |
| 9 | メモ帳 | 8 で Tab を押していく | ゴーストが選択中の候補に変わり、候補列の選択中の候補がアクセント色の太字になる |
| 10 | メモ帳 | 8 で `kyouha` → 無変換 | 前文脈と「次の語」ラベルに続けて、後続語予測の第一候補がゴーストで出る |
| 11 | メモ帳 | 8 を白い背景と暗い背景のウィンドウの上で行う | どちらでも文字が読める |
| 12 | メモ帳 | `panel_theme ghost`、`panel_blur 1` | ぼかしにならず、にじみの描画で出る |
| 13 | メモ帳 | `panel_theme capsule-column`、`panel_blur 1` | 縦積みでぼかしが効く |
| 14 | メモ帳 | テーマフォルダに `mine.tsv` (`base capsule`、`accent #E0457B`、`chip_shape rounded`、`badge_shape none`) を置き、`panel_theme mine` | アクセントが変わり、候補が角丸で番号の無い表示になる |
| 15 | メモ帳 | 14 の `mine.tsv` を入力の途中で書き換えて保存 | 次にフォーカスを移すなどの契機で反映される |
| 16 | メモ帳 | 14 に `dark.accent #FFB000` を足し、アプリモードを切り替える | ダークのときだけアクセントが変わる |
| 17 | メモ帳 | `mine.tsv` に不正な行 (`accent red`、`chip_shape star`、未知のキー) を混ぜる | その行だけ無視され、他の行は効く。診断ログに無視したキーが出る |
| 18 | メモ帳 | `panel_theme nothing` (存在しない名前) | `capsule` で出る。診断ログに出る |
| 19 | メモ帳 | テーマフォルダに `capsule.tsv` (`accent #E0457B`) を置き、`panel_theme capsule` | 組み込みの `capsule` の代わりにこのファイルが使われる |
| 20 | 設定ツール | 「入力パネルの見た目」でテーマを切り替えて保存 | config.tsv に `panel_theme` が書かれ、入力に反映される。フォルダの `mine.tsv` も選べる。「テーマのフォルダを開く」でフォルダが開く |
| 21 | WezTerm (追記型入力) | 2・8 と同じ | 2・8 と同じ |

## 未決事項

- 入力中のかなを文書に書かず、パネルだけに持つ設定は docs/archive/panel-buffer.md で扱う
