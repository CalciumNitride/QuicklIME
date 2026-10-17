# キャレットの印 (テーマの caret_marker)

## 背景と目的

入力中に文書のどこへ文字が入るかが目に入りにくい。特にパネル保持入力 (設定 `input_buffer panel`) では
読みがパネルの入力行にあり、文書のキャレットはアプリの細い線だけになる。`candidate_position caret` でも、
パネルの左端は run で最初に出した位置に固定されるため、打鍵が進むとキャレットはパネルの左端から離れていく。

入力中・候補選択中・後続語の予測の表示中に、文書のキャレットの位置を IME 側の印で示す。

## 決定事項

### テーマのキー

見た目の一部としてテーマで指定する (設定ファイルのキーは足さない)。

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `caret_marker` | `off` / `pointer` / `triangle` | `triangle` | キャレットの印。`pointer` は候補パネルに吹き出しの尻尾を付けてキャレットを指す。`triangle` はキャレットの下端に上向きの三角を出す |
| `pointer_size` | 0〜3 | 0.6 | 尻尾の高さ (f に対する比率)。底辺は高さの 2 倍 |
| `triangle_size` | 0〜3 | 0.5 | 三角の高さ (f に対する比率)。形は正三角形 |
| 色 `caret_marker` | 色 | `accent` と同じ値 | 三角の色 (ライト・ダークで別の値を持てる) |

- 組み込みテーマ (`capsule` / `capsule-column` / `ghost`) はすべて `triangle`
- `caret_marker` は形のキーと色のキーで同じ名前になる。`border` / `shadow` と同じく、値が `off` / `pointer` /
  `triangle` なら形、`#` で始まれば色として読む
- 色を書かなければ、解決後の `accent` (ライト・ダーク別) と同じ値にする。テーマファイルで `accent` だけを
  変えたときも三角の色が追従するようにするため
- 尻尾は色のキーを持たず、パネルの `background` と `border` で描いてパネルとひと続きに見せる

### 出す期間

- run 中 (読みの入力中)
- 変換の候補選択中
- 後続語の予測の表示中
- それ以外 (run の終了・採用・確定・取消、フォーカス移動、IME オフ、パネルを隠したとき) は消す

### キャレットの位置

- composition があるとき (composition 入力の run、変換中): composition の末尾 (空の範囲) の矩形。取れなければ
  composition の矩形の右端を使う。composition 全体の矩形の右端は、折り返した composition では
  キャレットの位置にならないため、末尾の空の範囲を先に使う
- composition が無いとき (パネル保持入力の run、後続語の予測): 今の `CaretRect` と同じ (選択範囲の矩形 →
  `GetGUIThreadInfo` のシステムキャレット)
- 矩形が取れないとき、または入力位置がウィンドウの外を指して代わりの位置になったとき
  (`FallbackIfOutsideWindow`、docs/design/candidate-position-fallback.md) は印を出さない。代わりの位置は
  キャレットの位置ではなく、印が違う場所を指すため
- 更新は打鍵ごと・パネルの再描画ごと。スクロールやウィンドウの移動は追わない (パネルと同じ)

### pointer (吹き出し)

- 候補パネル (候補バーと変換の候補一覧の両方) の、キャレット行に向いた辺に二等辺三角形の尻尾を付ける。
  パネルがキャレット行の下にあれば上辺に上向き、上にあれば下辺に下向き
- 尻尾の先端の x はキャレットの矩形の左端。底辺の中心をその x に置き、パネルの角丸 (`panel_radius`) に
  かからない範囲に収める。範囲を外れるときは端で止める (先端もその位置にずらす)
- パネル本体の位置は尻尾の高さの分だけキャレット行から離す (尻尾の先端がキャレット行の下端・上端に届く)
- 尻尾を付けるときは、パネルの左端を通常の位置 (`Placement::x`。候補バーは run で最初に出した時点のキャレット、
  変換の候補一覧は composition の左端) から「角丸の半径 + 尻尾の半幅 (= 尻尾の高さ)」だけ左へずらす。
  通常の位置のままでは尻尾が角丸を避けて右へ押し出され、打ち始めのキャレットを指せないため。打ち始めは
  尻尾がキャレットの真下に来て、打鍵が進むと尻尾だけが右へ動く (パネルは動かない)。ずらした結果が
  作業領域からはみ出すときは今と同じく作業領域に収め、そのときは尻尾が右へ押し出されてよい。
  尻尾を付けないとき (尻尾を描けない条件・`triangle`・`off`) はずらさない
- 尻尾の塗りは `background`、縁は `border` (`border on` のとき)。パネルの縁は尻尾の底辺の部分だけ切り、
  外形をひと続きにする。影はパネルと同じく尻尾にも落とす
- 描けないときは何も出さない (三角に代えない)。テーマで選んだ見た目と違うものを出さないため。描けないのは:
  - `candidate_position center` (パネルがキャレットから離れている)
  - `style ghost` (尻尾を付ける枠が無い)
  - ぼかし (テーマの `blur on`、docs/design/theme-blur.md) でパネルを描いているとき (DWM の角丸ウィンドウの外に描けない)
  - パネルを出していないとき (`panel_input 0` で候補が無いときなど)

### triangle (下三角)

- キャレットの矩形の下端に、先端を上に向けた三角を出す。先端の x はキャレットの矩形の左端、先端の y は
  キャレットの矩形の下端
- 候補パネルとは別の、小さな layered window (`WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW |
  WS_EX_LAYERED | WS_EX_TRANSPARENT`) で出す。パネルの有無・表示位置 (`caret` / `center`)・`style`・
  ぼかしによらず出す。`WS_EX_TRANSPARENT` でクリックを下のアプリへ通す
- `candidate_position caret` でパネルをキャレット行の下に出すときは、パネルを三角の高さの分だけ下げ、
  三角とパネルを重ねない。上に出すときは重ならないので位置を変えない
- 点滅させない

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/panel_theme.h / panel_theme.cpp | `caret_marker`・`pointer_size`・`triangle_size`・色 `caret_marker` の読み込みと既定値。組み込みテーマ 3 つに `caret_marker triangle`。色を書かなかったときの `accent` からの補完 |
| tsf/src/suggest_panel.h / suggest_panel.cpp | `Placement` にキャレットの x (印の先端) と「pointer を付ける」「三角の分だけ下げる」を足す。pointer の尻尾の描画 (外形・縁・影) と、尻尾の分のウィンドウの大きさ・位置の計算。描けない条件の判定 |
| tsf/src/caret_marker.h / caret_marker.cpp (新規) | 三角の layered window。表示 (位置と色・大きさ)・非表示 |
| tsf/CMakeLists.txt | caret_marker.cpp を足す |
| tsf/src/text_service.h / text_service.cpp / text_service_direct.cpp | キャレットの位置の取得 (composition の末尾 → `CaretRect`、代わりの位置なら無し)。パネルの表示・再描画・消去に合わせて印を出す・消す。テーマに従って pointer と triangle を振り分ける |
| docs/theme.md | キーの説明 |
| docs/design/README.md | 索引に 1 行 |

## 実装手順

1. テーマのキーを足す (読み込み・既定・範囲・組み込みテーマ・色の補完)
2. キャレットの位置の取得を作る (composition の末尾の空の範囲 → composition の右端 → `CaretRect`。
   代わりの位置かどうかを返せるようにする)
3. `CaretMarker` (三角の window) を作り、TextService から出す期間に合わせて出す・消す
4. `candidate_position caret` でパネルを下に出すとき、三角の分だけ下げる
5. `SuggestPanel` に pointer の尻尾を足す (候補バー・変換の候補一覧)。描けない条件では付けない
6. docs/theme.md と docs/design/README.md を書く

## 検証方法

隔離環境 (CLAUDE.md「エンジン検証の隔離」) のメモ帳で行う。テーマは `QUICKLIME_THEME_DIR` に置いた
テーマファイルで切り替える。

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳・既定 (`capsule`・`caret`・composition 入力) | `kyouha` を 1 文字ずつ打つ | `k` の時点からキャレット (composition の末尾) の下端に三角が出て、打鍵ごとに右へ動く。パネルは三角と重ならない |
| 2 | 1 と同じ | `kyouha` → Enter | 確定後、後続語の予測が出ている間は三角が残り、予測が消えると三角も消える |
| 3 | 1 と同じ | `kyouha` → 変換キー → Esc → Esc | 候補選択中も composition の末尾に三角が出る。run を捨てると消える |
| 4 | パネル保持入力 (`input_buffer panel`) | `kyouha` | 文書は変わらず、文書のキャレットの下に三角が出る |
| 5 | 4 と同じ | `kyouha` の途中で文書をクリックしてキャレットを別の行へ動かし、続けて打つ | 次の打鍵で三角がクリックした位置へ移る |
| 6 | `candidate_position center` | `kyouha` | パネルは画面中央、三角はキャレットの下に出る |
| 7 | `panel_theme capsule-blur` | `kyouha` | ぼかしのパネルと三角が両方出る |
| 8 | `style ghost` | `kyouha` | 三角が出る |
| 9 | テーマに `caret_marker pointer` (`capsule`・`caret`) | `kyouhaiitenki` を 1 文字ずつ打つ | 三角は出ず、パネルの上辺の尻尾がキャレットを指し、打鍵ごとに右へ動く。パネルの右端の角丸までで止まる |
| 9b | 9 と同じ | 行の途中で `kyouha` → 変換キー | 打ち始めの `k` の時点で尻尾がキャレットの真下にあり、パネルはその分だけ左へ出ている。変換の候補一覧でも尻尾が composition の末尾を指す |
| 9c | 9 と同じで、画面左端の行頭に入力 | `kyouha` | パネルは作業領域の左端に収まり、尻尾は角丸の右に押し出される |
| 10 | 9 と同じで、画面下端付近の行に入力 | `kyouha` | パネルが行の上に出て、尻尾が下辺から下向きにキャレットを指す。縦積みのテーマでは並びが反転し、入力行 (入力行を出すとき) が尻尾のすぐ上、1 番目の候補がそのすぐ上にある |
| 11 | 9 と同じ | `kyouha` → 変換キー | 縦の候補一覧にも尻尾が付き、composition の末尾を指す |
| 12 | 9 の設定で `candidate_position center` / テーマに `blur on` / `style ghost` のそれぞれ | `kyouha` | 尻尾も三角も出ない |
| 13 | 9 の設定で `panel_input 0` | `k` | パネルが無いので何も出ない。候補が出た時点で尻尾が付く |
| 14 | テーマに `caret_marker off` | `kyouha` | 尻尾も三角も出ず、今の見た目と同じ |
| 15 | テーマに `accent #E24B4A` だけを書く | `kyouha` | 三角も赤になる。`caret_marker #3B6D11` を足すとその色になる |
| 16 | 入力位置がウィンドウの外を指すアプリ (マビノギなど) | 入力 | パネルはウィンドウ中央の代わりの位置に出て、印は出ない |
| 17 | メモ帳 | 三角が出ている状態で、三角の上をクリック | クリックが下の文書に届く (キャレットが動く) |

## 未決事項

なし
