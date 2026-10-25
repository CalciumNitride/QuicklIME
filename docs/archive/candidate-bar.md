# 候補バー

入力モデル v2 (docs/archive/input-model-v2.md) の段階3。入力中は横一列の候補バーを常時表示し、
ライブ変換と予測サジェストをそこへ統合する。文書は採用したときにだけ書き換える (P1)。

## 背景と目的

段階2でライブ変換を削除し、run 中の文書にはかなだけが入るようになった。変換結果を見るには
変換キーで候補選択に入る必要がある。予測サジェストは縦の候補ウィンドウに出しており、
読める文書では選ぶたびに文書を書き換える (P1 に反する)。

本段階の目的は次のとおり。

- 変換キーを押さなくても変換結果が見え、1打で採用できるようにする (ライブ変換の作り直し)
- 予測サジェストを同じバーに統合し、縦の予測ウィンドウをなくす
- 長い入力を前から順に片付けられるようにする (先頭文節の部分採用)。後続の打鍵で前方の
  変換が変わる問題 (docs/archive/live-conversion-commit-notes.md) は、正しい時点で採用すれば
  以後変わらない、という形で解消する
- 選択では文書を書き換えず、採用時だけ書き換える。読める文書と追記のみの文書で挙動を揃える

## 決定事項

### バーの内容

[docs/spec/suggest.md](../spec/suggest.md)「候補バーの候補」へ移した

### 表示条件

[docs/spec/suggest.md](../spec/suggest.md)「表示の条件」へ移した

### 表示位置と見た目

見た目、画面中央に出す設定 (`candidate_position`)、入力行の表示 (`panel_input`) は
docs/archive/suggest-panel.md による。以下は `candidate_position caret` (既定) のときの位置。

- キャレット行のすぐ下に出す。下に入らなければ行の上に出す (縦に並ぶパネルの上下の決め方と上に出すときの
  並べ方は [panel-above-caret.md](panel-above-caret.md))
- x 座標の固定は [docs/spec/suggest.md](../spec/suggest.md)「表示の条件」へ移した
- 位置の基準は、未完成ローマ字の小窓と同じ矩形 (選択範囲の矩形、取れなければ
  システムキャレットの矩形) を使う
- 未完成ローマ字の小窓とは別のウィンドウにする
- 各候補の前にページ内の番号 (1〜9) を付け、選択中の候補を強調する。未選択のときはどれも強調しない
- 表示は `SuggestPanel` で行う。候補選択中の縦の表示は `CandidateWindow` のまま

### 選択

[docs/spec/suggest.md](../spec/suggest.md)「選択」へ移した

### 採用

[docs/spec/suggest.md](../spec/suggest.md)「採用」へ移した

### 採用せずに run が終わったとき

[docs/spec/suggest.md](../spec/suggest.md)「採用せずに run を終えたとき」へ移した

### 設定

[docs/spec/suggest.md](../spec/suggest.md)「表示の条件」と「設定」へ移した

### エンジン

[docs/spec/suggest.md](../spec/suggest.md)「SUGGEST の並び」と「旧エンジンとの互換」へ移した

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/candidate_window.h, tsf/src/candidate_window.cpp | 横一列の表示モード (番号・選択の強調・部分採用の「…」・作業領域に収まる件数の算出) |
| tsf/src/text_service.h, tsf/src/text_service.cpp | バーの状態 (候補の種類・読み・表記・文節) と作り直し。`UpdatePrediction` 系をバーに置き換え、選択時の文書置換 (`predictionInDocument_` の経路) を削除。候補選択中の Tab |
| tsf/src/text_service_direct.cpp | 採用 (全体・予測・先頭文節、読める文書と追記のみの文書)、確定キー・Enter・印字キー・Space からの採用、位置の固定、`candidate_bar` オフ時のキー処理 |
| tsf/src/romaji.h, tsf/src/romaji.cpp | 先頭の n 文字を切り離す操作 |
| tsf/src/config.h, tsf/src/config.cpp | `candidate_bar`・`min_suggest_chars` の読み込み |
| engine/src/bin/quicklime-config.rs | 「候補バーを表示する」チェックボックス |
| README.md | 候補バーの説明、キー操作、設定 |
| docs/archive/direct-input.md, docs/archive/keymap.md | サジェスト・確定キーの記述を本書に合わせて書き換える |
| docs/archive/input-model-v2.md | 段階表の詳細設計に本書を記し、未決事項から候補バーの項目を外す (本書の作成時に済ませる) |

## 実装手順

各手順の後にビルドが通り、既存の動作が壊れていないことを確かめる。

1. `RomajiComposer` の先頭切り離しと、config の `candidate_bar`・`min_suggest_chars`
2. `CandidateWindow` の横一列表示
3. バーの作り直し: 打鍵ごとの CONVNBEST・PREDICT から候補を組み立てて表示する
   (採用はまだ既存のサジェスト採用の経路のまま)。縦の予測ウィンドウを置き換える
4. 選択で文書を書き換えないようにする (読める文書の置換経路の削除)
5. 採用: 全体変換・予測候補 (確定キー・Enter・印字キー・Space)
6. 採用: 先頭文節の部分採用 (run の継続、前文脈の受け渡し、位置の取り直し)
7. `candidate_bar` オフの挙動と設定ツール
8. ドキュメントの更新 (README.md、direct-input.md、keymap.md)

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。
config.tsv は `QUICKLIME_CONFIG_FILE` で隔離したものを使い、`debug_log` を設定する。

### テストケース

| # | 環境 | 設定・操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | `ki` → Backspace → `kyouha` | 「き」(1文字) ではバーが出ない。「きょ」(2文字) 以降、行の下に全体変換・予測候補が出る。文書はかなのまま |
| 2 | メモ帳 | `kyouhaiitenkidesu` を打つ間 | バーの x 位置が動かない。文書は一度もかな以外に書き換わらない |
| 3 | メモ帳 | `kyouhaiitenki` → 無変換 | 全体変換 (例: 今日はいい天気) に置き換わり run が終わる。Ctrl+Backspace で読みに戻る |
| 4 | メモ帳 | `kyouha` → Tab → Tab → Shift+Tab → Shift+Tab | 選択が1件目 → 2件目 → 1件目 → 未選択と動く。文書は変わらない |
| 5 | メモ帳 | `sinagawa` → Tab (予測候補を選ぶ) → Enter | 予測候補に置き換わってから改行が入る |
| 6 | メモ帳 | `kyouha` → Tab → `a` | 選択中の候補を採用し、続けて「あ」から新しい run になる |
| 7 | メモ帳 | `kyouha` → Tab → Space | 採用してスペースが入る |
| 8 | メモ帳 | `kisyanokisyagakisyasuru` → 先頭文節 (「記者の…」) を選ぶ → 無変換 | 「記者の」だけ置き換わり、「きしゃがきしゃする」が run として続く。バーは文脈つきで作り直され、x 位置は採用した時点のキャレット位置で取り直される |
| 9 | メモ帳 | 8 に続けて無変換 | 残りはかなのまま run が終わる (`記者のきしゃがきしゃする`) |
| 10 | WezTerm | 3・5・8 と同じ操作 | 擬似 Backspace で消してから採用部分 (+ 残りのかな) が入る。選択中は文書が変わらない |
| 11 | Chrome の textarea・検索欄 | 3・5 と同じ操作 | 3・5 と同じ。検索欄の Enter は採用した文字列で送信される |
| 12 | メモ帳 | `kyouha` → 変換キー | バーが消え、縦の候補ウィンドウで候補選択になる。Esc で戻るとバーが再び出る |
| 13 | メモ帳 | `kyouha` → 変換キー → Tab | 変換を取り消し、バーの先頭が選ばれる |
| 14 | メモ帳 | `kyouhak` → 無変換 | `きょうはk` のまま run が終わる (Enter と同じ救済) |
| 15 | メモ帳 (モードレス ON) | `kyouhat` → 無変換 | 全体変換を採用せず、見えているとおり (`きょうはt`) で終える |
| 16 | メモ帳 (モードレス ON) | `Apple` (英字モード) | バーが出ない |
| 17 | メモ帳 | `kyouha` → Enter | かなのまま改行 (自動で採用しない) |
| 18 | メモ帳 | `candidate_bar 0`。`kyouha` → Tab / 無変換 | バーが出ない。Tab はアプリへ渡る (run が終わる)。無変換はかなのまま run を終える |
| 19 | メモ帳 | `suggest 0` | バーに予測候補が出ず、全体変換・先頭文節だけが出る |
| 20 | メモ帳 | `min_suggest_chars 4` | かな4文字以上でバーが出る |
| 21 | メモ帳 | エンジンを終了して入力 | バーが出ず、かなの入力は止まらない |
| 22 | 画面右端付近の入力欄 | 長い予測候補が並ぶ入力 | 作業領域からはみ出さない。収まらない候補は次のページに送られ、Tab で届く |
| 23 | 設定ツール | 「候補バーを表示する」を切り替えて保存 | config.tsv に `candidate_bar` が書かれ、入力に反映される |

## 未決事項

- 打鍵ごとの往復は [unified-suggest.md](unified-suggest.md) の SUGGEST で 1 回にする (上記「エンジン」)
