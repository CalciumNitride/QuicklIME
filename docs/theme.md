# 候補パネルのテーマ

候補バー (入力中の候補と次の語の予測)、変換キーで出る候補一覧と未完成ローマ字の小窓の見た目は、
テーマで決まる。組み込みのテーマから選ぶか、組み込みのテーマを土台に、変えたい値だけを書いたテーマファイルを作る。
変換キーで出る候補一覧は、`layout` によらず縦に積み、種類のタグ (`kind_tag`) は出さない。

## テーマの選び方

設定ツールの「表示」タブの「候補の見た目」で選ぶ (config.tsv の `panel_theme`、既定 `capsule`)。

| 名前 | 設定ツールでの表記 | 内容 |
|---|---|---|
| `capsule` | カプセル (横一列) | 半透明・角丸・影のパネルに、丸いチップ状の候補を横一列に並べる |
| `capsule-column` | カプセル (縦積み) | カプセルの配色で候補を縦に積み、各行の右に候補の種類のタグを付ける |
| `ghost` | ゴースト | 枠を持たず、大きな文字で入力と第一候補を出し、その下に番号付きの候補を並べる |

- テーマファイルは、テーマフォルダ `%APPDATA%\QuicklIME\themes\` に `<テーマ名>.tsv` として置く
  (設定ツールの「テーマのフォルダを開く」で開ける。フォルダが無ければ作られる)。置いたファイルは
  「候補の見た目」に、拡張子を除いたファイル名で出る
- 名前の解決: テーマフォルダの `<テーマ名>.tsv` → 同名の組み込みテーマ → どちらも無ければ `capsule`
- 組み込みのテーマと同じ名前のファイル (`capsule.tsv` など) を置くと、組み込みのものの代わりに
  そのファイルが使われる
- 環境変数 `QUICKLIME_THEME_DIR` でテーマフォルダを別の場所にできる (検証用)
- 反映: フォーカスの移動や IME のオン/オフのときに、選んでいるテーマファイルの更新時刻を確かめ、
  設定かテーマファイルが変わっていれば読み直す。Windows のアプリモード (ライト/ダーク) も
  このときに読み直す

## ファイルの形式

config.tsv と同じく「キー<TAB>値」の行の並び (UTF-8)。`#` で始まる行はコメント、空行は無視する。

```
# 例: カプセルを角ばらせ、アクセントを緑にする
base	capsule
panel_radius	0.4
chip_shape	rounded
accent	#2E9E6B
dark.background	#101014EE
```

- `base`: 土台にする組み込みテーマの名前 (`capsule` / `capsule-column` / `ghost`)。省略時は `capsule`。
  ユーザのテーマは土台にできない。何行目に書いてもよい
- 値はすべてのキーの既定値 (`capsule` と同じ) → `base` のテーマ → ファイル自身の行の順に重ね、
  後から読んだ値が勝つ。同じキーが何行もあれば後の行が勝つ
- 色は `#RRGGBB` または `#RRGGBBAA` (AA は不透明度。00 で透明、FF で不透明)
- 色のキーは、そのまま書くとライト・ダークの両方に効き、`light.` / `dark.` を前に付けるとその配色の
  ときだけ効く。同じファイルに両方あれば、行の順によらず `light.` / `dark.` の付いた方が勝つ
- 色以外のキー (並べ方・形・寸法) はライト・ダークで共通。`light.` / `dark.` は付けられない
- 知らないキーや不正な値 (形式が違う・選択肢に無い) の行は無視し、土台の値のまま使う。
  数値は下の範囲に収める。無視した行・範囲に収めた値は診断ログ (設定 `debug_log`) に出る

## キー

### 描き方と並べ方

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `style` | `capsule` / `ghost` | `capsule` | 描き方。`capsule` は背景・縁・影のあるパネル。`ghost` は枠を持たず、背景の代わりに放射状のにじみを描き、入力行の後ろに「→ 候補」の形で選択中 (未選択なら先頭) の候補を薄く出す |
| `layout` | `row` / `column` | `row` | 候補を横一列に並べるか、縦に積むか |
| `align` | `left` / `center` | `left` | 入力行と候補の揃え |
| `input_row` | `setting` / `always` | `setting` | `setting` は設定 `panel_input` (「入力中の読みも候補と同じ場所に表示する」) に従う。`always` は設定によらず入力行を出す (未完成ローマ字の小窓も出さない) |
| `blur` | `allow` / `never` | `allow` | 設定 `panel_blur` (「候補の背景をぼかす」) を効かせるか。`never` ならぼかさずに描く |

### 形

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `chip_shape` | `pill` / `rounded` / `underline` | `pill` | 候補の形。`pill` は両端が半円、`rounded` は角丸 (`chip_radius`)、`underline` は背景を持たず、選択中だけ下線を引く |
| `badge_shape` | `circle` / `square` / `plain` / `none` | `circle` | 候補番号の形。`plain` は数字だけ、`none` は番号を出さない (数字キーでの選択はできる) |
| `selection` | `fill` / `soft` / `text` | `fill` | 選択中の候補の強調。`fill` はアクセント色で塗って文字を `selected_text` に、`soft` はアクセント色を薄く敷いて文字をアクセント色に、`text` は文字だけをアクセント色の太字にする |
| `border` | `on` / `off` | `on` | パネルの外周の縁 |
| `shadow` | `on` / `off` | `on` | パネルの影 |
| `kind_tag` | `on` / `off` | `off` | 候補の種類のタグ (英字・変換・予測・文節・次の語)。`layout column` のときだけ効く |
| `text_shadow` | `on` / `off` | `off` | 文字の下に薄い影 (ダークでは黒、ライトでは白) を敷き、下の背景によらず読めるようにする |

- `border` と `shadow` は色のキーと同じ名前。値が `on` / `off` なら形、`#` で始まれば色として読む
- `layout row` の `pill` / `rounded` では、未選択の候補も `chip` 色の背景を持つ。
  `layout column` では、未選択の行は背景を持たず、選択中の行だけを `selection` に従って強調する
- `underline` で `selection fill` / `soft` のときは、下線の色だけがアクセント色になる
- 次の語の予測の候補は、`layout row` の `pill` / `rounded` では `next_word` 色の縁が付く

### 寸法

設定 `candidate_font_size` (候補のフォントの大きさ、以下 f) に対する比率。`_px` で終わるものは
f によらない画素数。

| キー | 範囲 | 既定 | 内容 |
|---|---|---|---|
| `panel_radius` | 0〜10 | 1.4 | パネルの角丸の半径 (パネルの高さの半分まで) |
| `panel_padding` | 0〜10 | 0.6 | パネルの内側の余白 |
| `input_text_size` | 0.5〜5 | 1.15 | 入力行の文字の高さ |
| `input_row_height` | 0.5〜5 | 1.7 | 入力行の高さ |
| `row_gap` | 0〜5 | 0.35 | 入力行と区切り線、区切り線と候補の間 |
| `chip_height` | 0〜5 | 1.9 | 候補 1 件の高さ (縦積みでは行の高さ) |
| `chip_padding` | 0〜5 | 0.65 | 候補の左右の余白 |
| `chip_gap` | 0〜5 | 0.35 | 候補どうしの間隔 (縦積みでは行の間) |
| `chip_radius` | 0〜5 | 0.5 | `chip_shape rounded` の角丸の半径 |
| `badge_size` | 0〜5 | 0.95 | 番号の形の大きさ |
| `badge_text_size` | 0〜5 | 0.62 | 番号の文字の高さ |
| `badge_gap` | 0〜5 | 0.35 | 番号と候補の間 |
| `label_text_size` | 0〜5 | 0.62 | 入力行の「次の語」ラベルの文字の高さ |
| `label_gap` | 0〜5 | 0.5 | 前文脈と「次の語」ラベルの間 (`ghost` では入力とゴーストの間にも使う) |
| `tag_text_size` | 0.3〜3 | 0.62 | 種類のタグの文字の高さ |
| `ghost_text_size` | 0.5〜5 | 1.7 | `ghost` の入力行の文字の高さ (`input_text_size` の代わりに使い、入力行の高さも同じ比で伸ばす) |
| `ghost_opacity` | 0〜1 | 0.42 | `ghost` の入力行に出す候補 (ゴースト) の不透明度 |
| `shadow_size` | 0〜5 | 0.8 | 影の広がり |
| `shadow_offset` | 0〜5 | 0.2 | 影を下へずらす量 |
| `center_width` | 5〜100 | 36 | 画面中央に出すとき (設定 `candidate_position center`) のパネルの幅 |
| `center_width_ratio` | 0.1〜1 | 0.9 | 画面中央に出すときの幅の上限 (作業領域の幅に対する比率) |
| `line_width_px` | 0〜10 | 1 | 縁・区切り線・番号の形の線の太さ (整数に丸めて描く。1 未満は 1) |
| `underline_px` | 0〜10 | 2 | 入力中のかなの下線と、`underline` の選択中の下線の太さ |
| `caret_width_px` | 0〜10 | 2 | 入力行のキャレットの太さ |
| `shadow_steps` | 1〜32 | 10 | 影を近似して重ねる角丸矩形の枚数 (整数に丸める) |

### 色

| キー | 内容 | ダークの既定 | ライトの既定 |
|---|---|---|---|
| `background` | パネルの背景 | `#2D3437EB` | `#FAFBF7EB` |
| `border` | 外周の縁 | `#FFFFFF14` | `#00000014` |
| `text` | 文字 | `#FAFBF7` | `#2D3437` |
| `secondary` | 副次色 (番号・未完成ローマ字・前文脈) | `#9AA5A3` | `#6B7573` |
| `chip` | 候補の背景 | `#FFFFFF12` | `#0000000B` |
| `separator` | 入力行と候補の区切り線 | `#FFFFFF17` | `#00000014` |
| `accent` | 選択中の候補・入力中のかなの下線・キャレット | `#7DC7B0` | `#2A7D66` |
| `selected_text` | `selection fill` の選択中の候補の文字と番号 | `#2D3437` | `#FFFFFF` |
| `next_word` | 次の語の予測の候補の縁と「次の語」ラベル | `#E3B873` | `#A8741F` |
| `shadow` | 影 (AA は重なったところの最大の不透明度) | `#00000073` | `#00000047` |
| `blur_tint` | ぼかし (`panel_blur 1`) の上に重ねる色 | `#2D343799` | `#FAFBF7B3` |
| `scrim` | `ghost` のにじみの色 (内側はこの濃さのまま、外へ向かって透明になる) | `#000000BF` | `#FFFFFFE6` |
| `tag_ascii` | 種類のタグ「英字」の文字の色 | `#9AA5A3` | `#6B7573` |
| `tag_conversion` | 種類のタグ「変換」の文字の色 | `#7DC7B0` | `#2A7D66` |
| `tag_prediction` | 種類のタグ「予測」の文字の色 | `#9AA5A3` | `#6B7573` |
| `tag_partial` | 種類のタグ「文節」の文字の色 | `#C08CF0` | `#8A4FC8` |
| `tag_next_word` | 種類のタグ「次の語」の文字の色 | `#E3B873` | `#A8741F` |

- 種類のタグの背景は、文字の色を不透明度 0.16 で敷く
- `blur_tint` は `panel_blur 1` のときだけ使い、そのときは `background`・`border`・`shadow` を描かない
  (背景・縁・影は Windows が描く)

## 縦積み (`layout column`)

- 1 行に番号・候補・種類のタグ (`kind_tag on` のとき右端) を並べる。行の高さは `chip_height`
- 候補は最大 9 行。作業領域の高さに収まらない後ろの候補は出さず、選択もできない
- 操作は横一列と同じ (Tab / ↓ で次、Shift+Tab / ↑ で前、数字で直接選ぶ)。ただし横一列と違い、
  ← / → では選択を動かさない
- キャレットの下に出すとき (`candidate_position caret`): 幅はいちばん幅の広い行に合わせる
  (作業領域の幅まで)。下に入らなければキャレット行の上に、下端をキャレット行の上に揃えて出す
- 画面中央に出すとき (`candidate_position center`): 幅は横一列と同じ固定幅。パネルの上端は、
  入力行と 9 行を出したときの高さで決めた位置に固定し、候補の数が変わると下へ伸び縮みする
- 幅に収まらない行は、末尾を「…」で切る

## ゴースト (`style ghost`)

- 入力行の文字は `ghost_text_size` の大きさで、入力中のかな・未完成ローマ字・キャレットに続けて
  「→ 候補」を `text` 色・`ghost_opacity` の不透明度で出す。Tab などで選ぶと、選んだ候補に変わる
- 次の語の予測では、前文脈と「次の語」ラベルに続けて同じく出す
- 候補は入力行の下に並べる (区切り線は引かない)
- `align center` では、入力行と候補をパネルの中央に揃える。キャレットの下に出すときも、パネルの
  左端の位置は `left` と同じ
- 組み込みの `ghost` は `blur never` のため、`panel_blur 1` でもぼかさない
- 未完成ローマ字の小窓 (`panel_input 0` かつ `input_row setting` のとき) も、にじみの上に文字だけを出す

## 組み込みテーマの値

`capsule` は上の表の既定値そのもの (`style capsule`・`layout row`・`align left`・`input_row setting`・
`blur allow`・`chip_shape pill`・`badge_shape circle`・`selection fill`・`border on`・`shadow on`・
`kind_tag off`・`text_shadow off`)。

`capsule-column` (`base capsule` に次を重ねる):

```
layout	column
panel_radius	0.7
chip_shape	rounded
chip_height	1.7
badge_shape	square
selection	soft
kind_tag	on
```

`ghost` (`base capsule` に次を重ねる):

```
style	ghost
align	center
input_row	always
blur	never
chip_shape	underline
badge_shape	plain
selection	text
border	off
shadow	off
text_shadow	on
```

## 例

縦積みで、番号を出さず、選択を塗りつぶしにする:

```
base	capsule-column
badge_shape	none
selection	fill
```

ゴーストの文字をさらに大きくし、ダークのときのにじみを濃くする:

```
base	ghost
ghost_text_size	2.2
dark.scrim	#000000E0
```
