# ぼかしのテーマへの統合

## 背景と目的

入力パネルの背景のぼかし (アクリル) は、設定 `panel_blur` (設定ツールの「候補の背景をぼかす」) と
テーマのキー `blur` (`allow` / `never`) の両方が許したときだけ掛かる。ぼかしで描くパネルには
`caret_marker pointer` の尻尾を描けない (docs/design/caret-marker.md) ため、テーマで吹き出しを選んでも
設定次第で吹き出しが消える。

ぼかすかどうかをテーマだけで決め、見た目の組み合わせをテーマの中で完結させる。

## 決定事項

### テーマのキー

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `blur` | `on` / `off` | `off` | パネルの背景をぼかし (アクリル) にする。Windows の透明効果がオフのときや使えない環境では通常の描画で出る |

- `allow` / `never` は読まない (互換を取らない)。不正な値として無視して土台の値のまま使い、診断ログに出す
- `blur on` と `caret_marker pointer` が同じテーマにあるときはぼかしを優先し、尻尾は出さない (今と同じ)。
  docs/theme.md に書き、テーマを読み込んだときに診断ログにも 1 行出す
- ぼかしが使えずに通常の描画に戻ったとき (透明効果オフなど) は、尻尾を出す (今と同じ。実際に描く方式で判定する)

### 組み込みテーマ

- `capsule`・`capsule-column`・`ghost` は `blur off`
- `capsule-blur` を足す。中身は `base capsule` に `blur on` (印は capsule と同じ `triangle`)。設定ツールの
  「入力パネルの見た目」での表記は「カプセル (ぼかし)」

### 設定

- 設定 `panel_blur` を廃止する。TSF 層は読まず、設定ツールの「表示」タブの「候補の背景をぼかす」の
  チェックボックスを削除する。既存の config.tsv に残った `panel_blur` の行は、知らないキーとして無視する
  (ぼかしていた人は「カプセル (ぼかし)」かテーマファイルの `blur on` に移る)

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/panel_theme.h / panel_theme.cpp | `blurAllowed` を `blur` (on/off、既定 off) にする。組み込みの文字列の `blur` の値、`capsule-blur` の追加。`blur on` と `pointer` の両方があるときのログ |
| tsf/src/config.h / config.cpp | `panelBlur` と `panel_blur` の読み込みを削除する |
| tsf/src/text_service.cpp | `SetBlur` に渡す値をテーマの `blur` だけにする |
| tsf/src/suggest_panel.h / suggest_panel.cpp | コメント中の `panel_blur` の記述をテーマの `blur` に直す (動作は変えない) |
| engine/src/bin/quicklime-config.rs | `panel_blur` のフィールド・読み書き・チェックボックス・テストを削除する。`BUILTIN_THEMES` に `capsule-blur` を足し、テストの期待値を直す |
| docs/theme.md | `blur` の説明・組み込みテーマの表と値・`blur_tint` の説明・吹き出しとの関係 |
| README.md | `panel_blur` の項を、テーマの `blur` と「カプセル (ぼかし)」の説明に置き換える |
| docs/design/README.md | 索引に 1 行 |

## 実装手順

1. テーマの `blur` を on/off にし、組み込みテーマと `capsule-blur` を直す
2. 設定 `panel_blur` を TSF 層から削除し、ぼかしをテーマだけで決める
3. 設定ツールからチェックボックスと `panel_blur` を削除し、組み込みテーマの一覧に `capsule-blur` を足す
4. docs を直す

## 検証方法

- `cargo test` (engine) で設定ツールのテストが通る
- 隔離環境のメモ帳 (tsf/build/test-caret-marker/ の config.tsv とテーマ) で確かめる

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | `panel_theme capsule` | `kyouha` | ぼかさずに描く |
| 2 | `panel_theme capsule-blur` | `kyouha` | ぼかしで描き、キャレットの下に三角が出る |
| 3 | `panel_theme capsule` で config.tsv に `panel_blur 1` を残す | `kyouha` | ぼかさない (設定は効かない) |
| 4 | テーマファイル `base capsule-blur` + `caret_marker pointer` | `kyouha` | ぼかしで描き、尻尾も三角も出ない。診断ログに両方が指定されている旨が出る |
| 5 | 4 のテーマで、Windows の透明効果をオフ | `kyouha` | 通常の描画になり、尻尾が出る |
| 6 | テーマファイルに `blur allow` | 読み込み | 無視され (土台の値)、診断ログに出る |
| 7 | 設定ツール | 「表示」タブを開く | 「候補の背景をぼかす」が無く、「入力パネルの見た目」に「カプセル (ぼかし)」がある。保存した config.tsv に `panel_blur` が無い |

## 未決事項

なし
