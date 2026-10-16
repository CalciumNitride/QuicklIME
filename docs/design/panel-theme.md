# サジェストパネルのテーマ (見た目のカスタマイズ・縦積み・ゴースト)

## 背景と目的

サジェストパネル (docs/design/suggest-panel.md) はモックの案 A (カプセル) の見た目で作った。
色と寸法は `PanelTheme` にまとめてあるが、ユーザは変えられない。本書では次の 3 つを入れる。

- 見た目をユーザが変えられるようにする (色・形・大きさ)
- 候補を縦に積む並べ方を足す
- モックの案 D (ゴースト HUD: 枠を持たず、大きな文字で入力と第一候補を出す) を足す

これらはどれも「見た目の違い」なので、まとめて**テーマ**として扱う。A・縦積み・D は
組み込みのテーマとして持ち、ユーザは組み込みのテーマを土台に、変えたい値だけを書いた
テーマファイルを作れるようにする。

## 決定事項

### 方式: 値を書くテーマファイル

- 見た目は値 (数値・色・選択肢) を書いたテーマファイルで決める
- 採らなかった案: スクリプト (Lua・JavaScript など) を DLL に組み込み、描画や配置を書けるようにする。
  IME の DLL は全アプリ (ゲームを含む) に読み込まれ、打鍵ごとに描画するため、スクリプトの不具合や
  重い処理がそのまま全アプリの入力を止め、遅延にもなる。任意のコードが全プロセスで動くことにもなる。
  値の指定でも、色・形 (選択肢)・大きさを変えるという目的は満たせる

### テーマの選び方と置き場所

- config.tsv に `panel_theme` (テーマ名、既定 `capsule`) を足す。TSF 層が読む
- テーマ名の解決:
  1. ユーザのテーマフォルダの `<テーマ名>.tsv` があればそれを使う
  2. 無ければ同名の組み込みテーマ
  3. どちらも無ければ組み込みの `capsule` を使い、診断ログに出す
- ユーザのテーマフォルダは `%APPDATA%\QuicklIME\themes\`。環境変数 `QUICKLIME_THEME_DIR` で上書きできる
  (エンジン検証の隔離用。他の `QUICKLIME_*` と同じ扱い)
- 組み込みのテーマと同じ名前のユーザのテーマファイルを置くと、組み込みのものより優先される
  (組み込みのテーマを全体的に手直ししたいときに使う)

### 組み込みのテーマ

| 名前 | 設定ツールでの表記 | 内容 |
|---|---|---|
| `capsule` | カプセル (横一列) | 今の見た目 (案 A) |
| `capsule-column` | カプセル (縦積み) | 案 A の配色で候補を縦に積み、行に種類のタグを付ける (モックの案 B の一覧に近い) |
| `ghost` | ゴースト | 案 D。枠を持たず、大きな文字で入力と第一候補を出す |

- 組み込みのテーマは、ユーザのテーマファイルと同じ形式の文字列として DLL に持ち、同じ読み込み処理で
  解釈する (形式と実装を一致させるため)
- 読み込みの順: すべてのキーの既定値 → `base` で指定した組み込みテーマ → テーマファイル自身の行。
  後から読んだ値が勝つ

### テーマファイルの形式

config.tsv と同じく「キー\t値」の行ベース TSV (UTF-8、`#` 始まりはコメント、空行は無視)。

```
# 例: カプセルを角ばらせ、アクセントを緑にする
base	capsule
panel_radius	0.4
chip_shape	rounded
accent	#2E9E6B
dark.background	#101014EE
```

- `base`: 土台にする組み込みテーマの名前。省略時は `capsule`。ユーザのテーマは土台にできない
  (循環を避けるため)。ファイルの何行目に書いてもよい (他のキーより先に適用する)
- 色の値は `#RRGGBB` または `#RRGGBBAA` (AA は不透明度)
- 色のキーは、そのまま書くとライト・ダークの両方に効き、`light.` / `dark.` を前に付けると
  その配色だけに効く。同じファイルに両方あれば `light.` / `dark.` が勝つ
- 未知のキーは無視する。不正な値 (形式が違う、選択肢に無い) はそのキーだけ無視する (土台の値のまま)。
  数値は下の範囲に丸める。無視したキーは診断ログに出す
- 色以外のキー (並べ方・形・寸法) はライト・ダークで共通

### キー

#### 描き方と並べ方

| キー | 値 | 内容 |
|---|---|---|
| `style` | `capsule` / `ghost` | 描き方。`capsule` は背景・縁・影のあるパネル、`ghost` は枠を持たず、背景の代わりに放射状のにじみを描く |
| `layout` | `row` / `column` | 候補を横一列に並べるか、縦に積むか |
| `align` | `left` / `center` | 入力行と候補の揃え。`ghost` の既定は `center`、他は `left` |
| `input_row` | `setting` / `always` | `setting` は設定 `panel_input` に従う。`always` は設定によらず入力行を出す (未完成ローマ字の小窓も出さない) |
| `blur` | `allow` / `never` | 設定 `panel_blur` を効かせるか。`never` なら `panel_blur 1` でも layered window で描く |

#### 形

| キー | 値 | 内容 |
|---|---|---|
| `chip_shape` | `pill` / `rounded` / `underline` | 候補の形。`pill` は両端が半円、`rounded` は角丸 (`chip_radius`)、`underline` は背景を持たず、選択中だけ下線を引く |
| `badge_shape` | `circle` / `square` / `plain` / `none` | 候補番号の形。`plain` は数字だけ (形なし)、`none` は番号を出さない |
| `selection` | `fill` / `soft` / `text` | 選択中の候補の強調。`fill` はアクセント色で塗って文字を `selected_text` に、`soft` はアクセント色を薄く敷いて文字をアクセント色に、`text` は文字だけをアクセント色の太字にする |
| `border` | `on` / `off` | パネルの外周の縁 |
| `shadow` | `on` / `off` | パネルの影 |
| `kind_tag` | `on` / `off` | 候補の種類のタグ (英字・変換・予測・文節・次の語)。`layout column` のときだけ効く |
| `text_shadow` | `on` / `off` | 文字の下に薄い影を敷く (背景の上で読みやすくする) |

- `layout row` の `chip_shape pill` / `rounded` では、未選択の候補も `chip` 色の背景を持つ (今と同じ)。
  `layout column` では、未選択の行は背景を持たず、選択中の行だけ `selection` に従って強調する
  (強調の形は `chip_shape` に従う)
- `underline` で `selection fill` / `soft` を指定したときは、下線の色だけをアクセント色にする

#### 寸法

`candidate_font_size` (以下 f) に対する比率。`_px` で終わるものは f によらない画素数。
今の `PanelTheme` の項目をそのままキーにし、新しい項目を足す。

| キー | 範囲 | 内容 |
|---|---|---|
| `panel_radius`, `panel_padding` | 0〜10 | パネルの角丸の半径・内側の余白 |
| `input_text_size`, `input_row_height` | 0.5〜5 | 入力行の文字の高さ・入力行の高さ |
| `row_gap` | 0〜5 | 入力行と区切り線、区切り線と候補の間 |
| `chip_height`, `chip_padding`, `chip_gap`, `chip_radius` | 0〜5 | 候補 1 件の高さ (縦積みでは行の高さ)・左右の余白・候補どうしの間隔・`rounded` の角丸 |
| `badge_size`, `badge_text_size`, `badge_gap` | 0〜5 | 番号の形の大きさ・番号の文字の高さ・番号と候補の間 |
| `label_text_size`, `label_gap` | 0〜5 | 「次の語」ラベルの文字の高さ・前文脈との間 |
| `tag_text_size` | 0.3〜3 | 種類のタグの文字の高さ |
| `ghost_text_size` | 0.5〜5 | `ghost` の入力行の文字の高さ (`input_text_size` の代わりに使う) |
| `ghost_opacity` | 0〜1 | `ghost` の第一候補 (ゴースト) の不透明度 |
| `shadow_size`, `shadow_offset` | 0〜5 | 影の広がり・下へのずらし |
| `center_width`, `center_width_ratio` | 5〜100 / 0.1〜1 | 画面中央に出すときの幅 (f の倍数) と、作業領域の幅に対する上限 |
| `line_width_px`, `underline_px`, `caret_width_px` | 0〜10 | 縁・区切り線の太さ、下線の太さ、キャレットの太さ |
| `shadow_steps` | 1〜32 | 影を近似する角丸矩形の枚数 |

#### 色

今の `PanelTheme` の色をそのままキーにし、新しい色を足す。

| キー | 内容 |
|---|---|
| `background`, `border`, `text`, `secondary`, `chip`, `separator`, `accent`, `selected_text`, `next_word`, `shadow`, `blur_tint` | 今の `PanelTheme` と同じ |
| `scrim` | `ghost` のにじみの色 (内側はこの濃さのまま、外へ向かって透明になる) |
| `tag_ascii`, `tag_conversion`, `tag_prediction`, `tag_partial`, `tag_next_word` | 種類のタグの文字の色。タグの背景はこの色を不透明度 0.16 で敷く |

### 組み込みテーマの値

`capsule` は今の `PanelTheme::Dark()` / `Light()` と寸法の既定値をそのまま書いたもの。加えて
`style capsule`、`layout row`、`align left`、`input_row setting`、`blur allow`、`chip_shape pill`、
`badge_shape circle`、`selection fill`、`border on`、`shadow on`、`kind_tag off`、`text_shadow off`。
新しい項目の既定値は次のとおり (全テーマ共通の既定値として持つ)。

| キー | 既定値 |
|---|---|
| `chip_radius` | 0.5 |
| `tag_text_size` | 0.62 |
| `ghost_text_size` | 1.7 |
| `ghost_opacity` | 0.42 |
| `scrim` | ダーク #000000 (0.75)、ライト #FFFFFF (0.9) |
| `tag_ascii` | ダーク #9AA5A3、ライト #6B7573 |
| `tag_conversion` | ダーク #7DC7B0、ライト #2A7D66 |
| `tag_prediction` | ダーク #9AA5A3、ライト #6B7573 |
| `tag_partial` | ダーク #C08CF0、ライト #8A4FC8 |
| `tag_next_word` | ダーク #E3B873、ライト #A8741F |

既定の配色 (色の値は docs/theme.md「色」) はロゴの色 (スレート #2D3437・ミント #7DC7B0・
オフホワイト #FAFBF7) から取る。

- ダークの `selected_text` はスレートにする。ミントは明るく、白文字ではコントラストが足りない
- ライトの `accent` は暗いティール (#2A7D66) にする。ミントのままでは白い背景の上で下線・キャレットが見えにくい
- `next_word` はアンバー系にし、アクセントのミントと区別する

`capsule-column` (`base capsule` に次を重ねる):

| キー | 値 |
|---|---|
| `layout` | `column` |
| `panel_radius` | 0.7 |
| `chip_shape` | `rounded` |
| `chip_height` | 1.7 |
| `badge_shape` | `square` |
| `selection` | `soft` |
| `kind_tag` | `on` |

`ghost` (`base capsule` に次を重ねる):

| キー | 値 |
|---|---|
| `style` | `ghost` |
| `align` | `center` |
| `input_row` | `always` |
| `blur` | `never` |
| `chip_shape` | `underline` |
| `badge_shape` | `plain` |
| `selection` | `text` |
| `border` | `off` |
| `shadow` | `off` |
| `text_shadow` | `on` |

### 縦積み (`layout column`)

- 1 行に、番号 (`badge_shape`)・候補文字列・種類のタグ (`kind_tag on` のとき右端) を並べる。行の高さは `chip_height`
- 候補の種類は、`TextService` のバーの候補の種類 (英字・全体変換・予測・先頭文節) と後続語予測から決める。
  タグの表記は「英字」「変換」「予測」「文節」「次の語」。`SuggestPanel::Item` に種類を持たせ、
  今の `partial`・`nextWord` はこの種類で表す
- キー操作は横一列と同じ (Tab / ↓ で次、Shift+Tab / ↑ で前、数字で直接)
- パネルの幅:
  - `caret`: いちばん幅の広い行に合わせる。ただし作業領域の幅を超えない。収まらない行は末尾を「…」で切る
  - `center`: 横一列と同じ固定幅。収まらない行は末尾を「…」で切る
- 出せる行数は最大 9。作業領域の高さに収まらない後ろの候補は出さず、選択の対象からも外す
  (横一列で幅に収まらない候補と同じ扱い)
- 位置:
  - `caret`: キャレット行の下に出し、下に入らなければ行の上に出す。上に出すときはパネルの下端を
    キャレット行の上に揃える (候補が増えると上へ伸びる)
  - `center`: 入力行と 9 行を出したときの高さを H として、パネルの上端を「作業領域の縦の中央 − H / 2」に
    固定する。候補の数が変わっても上端は動かず、下へ伸び縮みする (入力行と先頭の候補が打鍵ごとに動かない)

### ゴースト (`style ghost`)

- パネルの背景・縁・影の代わりに、パネル全体に楕円の放射状のにじみを描く。中心から半径の 45% までは `scrim`、
  そこから外周に向かって不透明度 0 になる。にじみが切れて見えないよう、ウィンドウには外側の余白を取る
- 入力行: 文字の高さは `ghost_text_size`。run の文字列 (アクセント色の下線)・未完成ローマ字 (今と同じ)・
  キャレットに続けて、選択中の候補 (未選択なら先頭の候補) を「→ 候補」の形で `text` 色・
  `ghost_opacity` の不透明度で出す (ゴースト)。部分採用の候補は末尾に「…」を付ける
  - run が無い後続語予測では、前文脈と「次の語」ラベルに続けて同じくゴーストを出す
- 候補の並び: 入力行の下に、`badge_shape plain` の番号付きで出す。`layout row` では横一列
  (`chip_gap` 間隔)、`layout column` では縦積み
- `text_shadow on` では、文字の下に黒 (ダーク) / 白 (ライト) の不透明度 0.35 の同じ文字を
  1px ずらして描く (ぼかしは掛けない)
- `align center` では入力行と候補を中央揃えにする。打鍵とともに入力行は左右へ伸びる
- 位置の規則 (`caret` / `center`) はカプセルと同じ。キャレット位置でも使える
- `blur never` のため、`panel_blur 1` でも layered window で描く

### 揃え (`align`)

- `left`: 今と同じ (左詰め)
- `center`: 入力行・候補列 (`row` では列全体、`column` では各行) をパネルの幅の中で中央に置く。
  `caret` で `center` のときは、パネルの中心ではなく左端を今と同じ x に置く (位置の規則は変えない)

### 読み直し

- テーマは `TextService::RefreshConfig` のたびに、選んでいるテーマファイルの更新時刻を確かめ、
  config が変わったとき・テーマファイルが変わったときだけ読み直す (打鍵ごとには読まない)
- アプリモード (ライト/ダーク) の読み直しも同じ契機にする (今は config が変わったときだけ)
- 読み直した結果は `PanelTheme` (ライト・ダークを解決した 1 組) として両パネルに渡す
- `input_row always` は `TextService` の入力行の判定 (`PanelInputActive` など) にも効かせる。
  `blur never` はパネルに渡すぼかしの有効・無効に効かせる

### 設定ツール

- 「表示」タブで、「候補の表示位置」の直前に「候補の見た目」のコンボボックスを置く。
  項目は組み込みのテーマ (上の表記) に続けて、テーマフォルダの `*.tsv` のファイル名 (拡張子なし)。
  組み込みと同名のファイルは組み込みの項目として 1 つだけ出す
- その横に「テーマのフォルダを開く」ボタンを置く。フォルダが無ければ作ってから開く
- config.tsv の書き出しでは `# 候補ウィンドウ` の節、`candidate_position` の前に `panel_theme` を書く

### ドキュメント

- ユーザ向けのテーマファイルのリファレンス (キー・値・範囲・例・組み込みテーマの値) を
  docs/theme.md に書き、README.md から参照する
- .claude/CLAUDE.md の隔離用の環境変数の一覧に `QUICKLIME_THEME_DIR` を足す
- docs/design/suggest-panel.md の未決事項のうち、見た目のカスタマイズと案 D は本書へ参照を移す

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/panel_theme.h, tsf/src/panel_theme.cpp (新規) | `PanelTheme` を suggest_panel.h から移し、新しい項目を足す。テーマファイルの解釈、組み込みテーマの文字列、名前の解決、更新時刻の確認 |
| tsf/src/suggest_panel.h, tsf/src/suggest_panel.cpp | 形の選択肢、縦積み、ゴースト、揃え、種類のタグ、文字の影 |
| tsf/src/text_service.h, tsf/src/text_service.cpp, tsf/src/text_service_direct.cpp | テーマの読み直し、候補の種類を渡す、`input_row always`・`blur never` の反映 |
| tsf/src/config.h, tsf/src/config.cpp | `panel_theme` の読み込み |
| tsf/CMakeLists.txt | panel_theme.cpp の追加 |
| engine/src/bin/quicklime-config.rs | 「候補の見た目」コンボ、テーマのフォルダを開くボタン、`panel_theme` の読み書き |
| docs/theme.md (新規), README.md, .claude/CLAUDE.md, docs/design/suggest-panel.md | 上の「ドキュメント」 |

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
| 7 | 画面下端付近の入力欄 | 2 で `candidate_position caret`、`kyouha` | キャレット行の上に出て、下端がキャレット行の上に揃う |
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
| 20 | 設定ツール | 「候補の見た目」でテーマを切り替えて保存 | config.tsv に `panel_theme` が書かれ、入力に反映される。フォルダの `mine.tsv` も選べる。「テーマのフォルダを開く」でフォルダが開く |
| 21 | WezTerm (追記型入力) | 2・8 と同じ | 2・8 と同じ |

## 未決事項

- 入力中のかなを文書に書かず、パネルだけに持つ設定は docs/design/panel-buffer.md で扱う
