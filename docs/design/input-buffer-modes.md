# 入力方式の選択 (パネル保持入力と composition 入力)

## 背景と目的

文書書き込み入力 (打鍵したかなを文書に直接追記し、IME が覚えた run を後から置き換える方式) は、
アプリのエディタ実装に依存する問題を繰り返し起こした。追記前の照合、Chromium の選択と置換の制約、
読み戻せない文書での擬似 Backspace、Discord の内部モデルとの食い違いなど、アプリごとに個別の対応が要り、
今後も増える見込みが高い。

この方式を入れた動機は2つあった。

- ひらがなIME に近い使用感 (打鍵したかなが文書に入り、後から変換する)
- 半角前提の Web フォーム (1文字1枠の認証コード欄など) で composition が崩れること

後者を Chrome の検証ページで計測した結果 (下記「フォームの計測結果」)、崩れの原因は composition か
直接入力かではなく、IME を通った文字が全角になることと、composition 中のフォーカス移動・値の書き換え・
maxlength にあった。Chromium では、composition を使わずに挿入した文字列も、ページからは一瞬の composition
(compositionstart → compositionupdate → insertText) として届くため、直接入力にしても避けられない。
フォームに効いていたのは、run の外の半角数字の素通し (`digits=half`) である。

そこで文書書き込み入力を廃止し、入力方式を次の2つから設定で選べるようにする。フォームへの対応は
InputScope による素通し (docs/design/input-scope.md) で行う。

- パネル保持入力 (`panel`): 入力中の読みを候補パネルの入力行だけに持ち、採用・確定のときに文書へ挿入する
  (docs/design/panel-buffer.md を引き継ぐ)
- composition 入力 (`composition`): 入力中の読みを文書の composition (下線付きの未確定文字列) に出す。
  ほかの IME と同じ見た目

### フォームの計測結果

検証ページ (1文字1枠の6種の実装と、keydown / beforeinput / input / composition* / focus / blur の記録) を
Chrome で使った。

| IME | 数字 | 英字 |
|---|---|---|
| QuicklIME (panel、digits=half) | 6種とも素のキー入力と区別がつかず正常 | 挿入が一瞬の composition として届き、input ハンドラが次の枠へ移すと確定文字列が次の枠に入る |
| MS-IME (ひらがな) | 全角で composition に入る。`\D` の除去で消える、maxlength が composition 中に効かず確定時に切り詰められる、keydown が keyCode 229 で弾けない、フォーカス移動後に compositionend が元の枠へ届いて文字が消える | 全角で入り、英数字以外を除去する欄では消える |

## 決定事項

### 設定

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `input_buffer` | `composition` / `panel` | `composition` | 入力中の読みを出す場所 |

- 不正な値・廃止した値 (`document`) は既定値として扱う
- 方式は run を始める時点の設定で決め、その run が終わるまで変えない (今の `panelRun_` と同じ考え方)
- 設定ツールの「入力」タブで、今のチェックボックスを2択 (composition / パネル) に置き換える
- 設定 `append_only_apps` を廃止する (読み込まず、設定ツールの「アプリ」タブの「置換」の列とチェックボックスを削除する)

### 共通の run モデル

2つの方式は、入力中の読みを表示する場所だけが違う。run (読み `composer_`・候補バー・変換状態・モードレスの
英字区間) とキー処理は共通にする。

| | パネル保持入力 | composition 入力 |
|---|---|---|
| 入力中の読み | パネルの入力行 (`panel_input` などによらず常に出す) | 文書の composition。打鍵ごとに読みの表示 (`composer_.Display()`、未完成のローマ字を含む) で更新する。表示属性は入力中の下線 |
| 候補パネルの入力行 | 常に出す | `panel_input` とテーマの設定に従う |
| 未完成のローマ字の小窓 | 出さない | 出さない (composition に含める) |
| 採用・確定 | キャレット位置へ挿入 (`InsertText`) | composition の文字列を採用した文字列にして終える |
| 先頭文節の採用 | 先頭文節を挿入し、残りのかなを入力行に残す | 先頭文節で composition を終え、残りのかなで composition を張り直す (`RestartCompositionEditSession` と同じく、終了と開始を1つの edit session で行う) |
| 変換 (候補選択) | 空の composition を張る (今と同じ) | run の composition をそのまま使う |
| 変換の取消 (降格) | composition を空で終え、読みを入力行に戻す | composition の文字列を読みの表示に戻す (composition は続く) |
| 変換の範囲の伸縮 | composition の中だけ (外したかなを戻すところまで) | 同じ |
| Backspace (run 中) | 読みの末尾を削る。空になれば run を終える | 同じ。空になれば composition を空で終える |
| Esc (run 中、バー・候補選択以外) | run を捨てる。文書には何も入らない | composition を空で終えて run を捨てる |
| マウスクリック・キャレット移動 | run を続け、採用・確定した時点のキャレット位置に入れる | アプリが composition を終えたら (`OnCompositionTerminated`)、アプリが残した文字列のまま run を終える (学習しない)。終えなければ run を続ける |
| フォーカス移動 | run を捨てる | run を終える。composition の文字列はアプリの扱いに任せる (学習しない) |
| IME オフ | 読みを挿入して run を終える | composition を見えているとおりに確定して run を終える |

- キー体系は両方式で同じにする (docs/design/keymap.md)。Space はスペース、変換は変換キー、Enter は確定してから
  アプリへ渡す、確定キーは確定だけ
- composition の開始と文字列の設定は別々の edit session で行う (開始と同じ session で SetText すると CUAS が
  WM_IME_COMPOSITION を生成しない制約。既存の `StartCompositionEditSession` の注記と同じ)
- composition を張れない (失敗・拒否) ときは、その run をパネル保持入力として続ける
- 候補パネルの位置は、composition 入力では composition の矩形から取る
- 後続語の予測・学習・前文脈・モードレス入力の判定は両方式で同じ。モードレスの英字切替は、
  composition 入力では composition の文字列を書き換えるだけになる
- 削除前の composition 方式の実装は `git show ba4b317^:tsf/src/text_service.cpp` にある。候補バー・パネル・
  モードレスはその後に入ったので、本書の共通 run の上に作り直す

### 送り直し

run を終えて文字列を入れてからアプリへ渡すキー (Enter・矢印・Tab・Home/End・Delete・PgUp/PgDn・Ctrl 併用) は、
両方式とも今のパネル保持入力の規則に従う。読めると判定された文書では食べずに `OnTestKeyDown` / `OnKeyDown` の
中で確定してから渡し、それ以外の文書では食べて確定の後に `SendInput` で送り直す (CUAS 経由のアプリは確定結果を
IME メッセージで後から受け取るため)。Alt 併用は送り直さず run も終えない (docs/design/panel-buffer.md)。

### 文書の分類

- 文書の分類は「読める」と「未判定」の2つにする。「追記のみ」を廃止する
- 判定は今のパネル保持入力と同じく、IME が直前に入れた文字列 (挿入・composition の確定) をキャレット直前から
  読み戻して行う。一致すれば「読める」、一致しなければ「未判定」のまま。`OnSetFocus` で未判定に戻す

### 文書の既存の文字列に触れる機能

run が無いときの変換 (後置再変換。docs/design/direct-input.md、conversion-range.md、adopted-reconversion.md) と
確定アンドゥは、文書を読み戻して置換できる場合だけ働く。擬似 Backspace による書き換えは廃止する。

| 機能 | 働く条件 | 働かないとき |
|---|---|---|
| 選択したかなの変換 | 選択を読み取れ、置換 (選択の文字列を消して composition に昇格) が照合込みで成功する | 変換キーをアプリへ渡す |
| キャレット直前のかな・採用済みの記憶の変換 | 読める文書 (今と同じ) | 変換キーをアプリへ渡す |
| 確定アンドゥ (Ctrl+Backspace) | 読める文書で、キャレット直前が直前に入れた文字列と一致する | アプリへ渡す |

- 確定アンドゥの再開先は run を始めたときの方式に従う (パネル保持入力は入力行、composition 入力は composition)
- 採用済みの記憶は、両方式とも未判定の文書でも記憶し、使う時点で読める文書と判定されていて照合で一致したものだけを
  使う (今のパネル保持入力と同じ)
- 後置再変換で昇格した composition を取り消すと (Esc・Backspace・Tab)、run を始めたときの方式の run に戻る
  (パネル保持入力では入力行、composition 入力では読みの composition)

### 削除するもの

- 文書書き込み入力 (`input_buffer document`): run の追記 (`AppendRunEditSession` の追記の経路)、追記前の照合、
  置換の edit session のうち run の作り直しに使う部分、`surface_` / `surfaceCaret_` / `surfaceSelectLength_`、
  未完成のローマ字の小窓 (`pendingPanel_`)、追記のみの文書で run を終えるための `OnEndEdit` の処理
- 擬似 Backspace (`SendPseudoBackspaces` と目印の打鍵)、それが要る採用の判定 (`AdoptionNeedsPseudoBackspace`)
- run と確定アンドゥのためのマウスフック (`UpdateMouseHook`)
- `append_only_apps` と `SetAppendDocument` での読み替え
- 後置再変換・確定アンドゥのうち、追記のみの文書向けの経路

照合 (`MatchRunRange`)、選択だけを設定する session、昇格 (`PromoteRun`) など、読める文書での後置再変換・
確定アンドゥが使うものは残す。

## 採らなかった案

- パネル保持入力への一本化: 読みが文書に出ないので、ほかの IME から移る人には分かりにくい
- composition 入力で Space を変換、Enter を確定だけにする古典的なキー体系: 方式ごとにキーの意味が変わり、
  キーマップと状態別の処理が二重になる。変換キーの割当 (`key.convert`) で近づけられる
- 後置再変換・確定アンドゥを追記のみの文書でも擬似 Backspace で残す案: 不安定さの原因そのものを残すことになる
- 後置再変換・確定アンドゥの全廃: 読める文書では安定して動いており、ひらがなIME に近い使用感の一部を担う

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h, tsf/src/config.cpp | `input_buffer` の値 (`composition` / `panel`、既定 `composition`)。`append_only_apps` の削除 |
| tsf/src/text_service.h | run の方式の持ち方 (`panelRun_` を方式の列挙に)、文書書き込み入力の状態の削除、文書の分類の2値化 |
| tsf/src/text_service_direct.cpp | 文書書き込み入力の経路と擬似 Backspace の削除、composition 入力の run (開始・更新・採用・先頭文節の採用・降格・終了)、後置再変換と確定アンドゥの条件 |
| tsf/src/text_service.cpp | `IsKeyEaten` / `HandleKey` の分岐、`OnSetFocus`・IME オフ・`OnCompositionTerminated`・`OnEndEdit` での run の終え方、パネルの入力行の判定、マウスフックの削除 |
| tsf/src/edit_session.h, tsf/src/edit_session.cpp | 使わなくなった session の削除 (ファイル名は変えない) |
| engine/src/bin/quicklime-config.rs | 「入力」タブの2択、「アプリ」タブの「置換」の列と `append_only_apps` の削除 |
| README.md | 「入力方式」の節を2方式の説明に書き直す。「読める文書と追記のみの文書」「パネル保持入力」の節を整理する |
| docs/design/README.md | 本書を足し、direct-input.md・append-input.md・panel-buffer.md・append-only-apps.md に置き換えを併記する |
| docs/architecture.md | 入力方式の記述があれば合わせる |

text_service_direct.cpp のファイル名は変えない (変えると履歴を追いにくくなるため)。

## 実装手順

各手順の後にビルドが通り、パネル保持入力のテストが通ることを確かめる。

1. 文書書き込み入力・擬似 Backspace・マウスフック・`append_only_apps` を削除する。この時点の既定は `panel` にしておく
2. 文書の分類を2値にし、後置再変換・確定アンドゥを上記の条件にする
3. composition 入力の run を足す (開始・打鍵ごとの更新・Backspace・Esc・採用・先頭文節の採用・確定・送り直し)
4. composition 入力の変換 (run の composition での候補選択、降格、範囲の伸縮)、`OnCompositionTerminated`・
   フォーカス移動・IME オフ
5. 既定を `composition` にし、設定ツールを直す
6. README.md・docs/design/README.md・docs/architecture.md を直す

## 検証方法

ビルド: `cmake --build tsf/build --config Debug` と `cd engine && cargo build`。反映は `scripts\dev-deploy.ps1 -Dll`、
テスト後は `-Restore` で戻す。テストは新しく起動したアプリで行う (メモ帳は `taskkill /IM Notepad.exe /F` してから開く)。
config.tsv は `QUICKLIME_CONFIG_FILE` で隔離し、`debug_log` で文書の分類・送り直しを確かめる。

### テストケース (composition 入力)

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | `kyouha` | 下線付きで `k` → `ky` → `きょ` … `きょうは` と文書に出る。小窓は出ない |
| 2 | メモ帳 | `kyouha` → Backspace ×5 | `きょう` → … と縮み、空になると下線が消える。次の Backspace でアプリの文字が消える |
| 3 | メモ帳 | `kyouha` → Esc | 文書に何も残らない |
| 4 | メモ帳 | `kyouha` → 無変換 / Space / Enter | `きょうは` / `きょうは　` / `きょうは` + 改行 (下線が消える) |
| 5 | メモ帳 | `kyouhaiitenki` → Tab で全体変換を選ぶ → 無変換 | 選んだ候補で確定する。Ctrl+Z 1回で確定文字列がまとめて消える |
| 6 | メモ帳 | `kisyanokisyagakisyasuru` → 先頭文節を選ぶ → 無変換 | `記者の` が確定し、`きしゃがきしゃする` が下線付きで残る |
| 7 | メモ帳 | `kyouhaiitenki` → 変換キー ×2 → → / ← → Enter | 候補選択・範囲の伸縮ができ、Enter で確定して改行も入る |
| 8 | メモ帳 | `kyouha` → 変換キー → Esc → `i` | 変換前の `きょうは` (下線) に戻り、`きょうはい` に続く |
| 9 | メモ帳 | `kyouha` → ← | `きょうは` が確定し、キャレットが1文字左へ動く |
| 10 | メモ帳 | `kyouha` → 文書の別の位置をクリック | メモ帳が composition を終えれば `きょうは` が残って run が終わる。続けて打つと新しい run |
| 11 | メモ帳 | `kyouha` → IME オフ | `きょうは` が確定してから IME がオフになる |
| 12 | メモ帳 (モードレス ON) | `apple`、`kyouhagithub` | composition の中で英字に切り替わる |
| 13 | メモ帳 | `kyouha` → 無変換 → Ctrl+Backspace | 確定した `きょうは` が消え、下線付きの `きょうは` の run に戻る |
| 14 | メモ帳 | 文書の `きょう` を選択して変換キー → Enter / Esc | 候補選択に入り確定できる。Esc では下線付きの `きょう` の run に戻る |
| 15 | Chrome の textarea・Gmail 本文 | 1〜9 | 同じ挙動。文字の重複・消失がない |
| 16 | Word | 1〜9、13、14 | 同じ挙動 |
| 17 | WezTerm | `kyouha` → 変換キー → Enter / `kyouha` → ← | 確定結果が入ってから Enter・← が処理される (ログで送り直しを確認) |
| 18 | WezTerm | `kyouha` → 無変換 → Ctrl+Backspace | 何も起きずアプリへ渡る (読める文書と判定されないため) |
| 19 | Discord の入力欄 | `kyouha` → 変換キー → 確定 / Tab で候補を採用 / Enter | どれも選んだ文字列だけが入り、かなが残らない |
| 20 | 検証ページ (1文字1枠) | 数字を打つ | 6種とも半角で1文字ずつ入る (digits=half の素通し) |

### テストケース (パネル保持入力と共通)

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 21 | メモ帳 | `input_buffer panel` で docs/design/panel-buffer.md のテストケース 1〜29 | 追記のみの文書の擬似 Backspace に関するもの (21: WezTerm の確定アンドゥ) を除き、今どおり。21 は何も起きずアプリへ渡る |
| 22 | メモ帳 | config.tsv に `input_buffer document` | composition 入力として動く |
| 23 | メモ帳 | run の途中で config.tsv の `input_buffer` を書き換えて保存 | その run は始めたときの方式のまま。次の run から切り替わる |
| 24 | 設定ツール | 「入力」タブの2択を切り替えて保存 / 「アプリ」タブ | config.tsv の `input_buffer` が書き換わる。「置換」の列が無い。`append_only_apps` の行が書かれない |
| 25 | 読めない文書 (WezTerm) | run なしで変換キー・F6 | アプリへ渡る (擬似 Backspace が送られない) |

## 未決事項

- なし
