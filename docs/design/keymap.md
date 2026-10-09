# キーマップ刷新

入力モデル v2 (docs/design/input-model-v2.md) の段階1。キー割当を「状態 x キー → 機能」の
表に置き換え、Enter をアプリへ渡すキーとして分離し、確定キーを新設する。

## 背景と目的

現状のキー割当には次の制約がある。

- 割当を変えられる機能は11個で、1機能に1キーしか割り当てられない
- 機能ごとに使える修飾キーが固定されている (run 中の機能は無修飾 F1-F12、run が無いときの
  機能は Ctrl+F1-F12 / Ctrl+Backspace、変換キーは `Convert` / `Ctrl+Space` の2択)
- 候補選択中・サジェスト選択中の Enter は確定だけを行い、改行・送信には2打かかる
- 候補選択中の処理が、composition に昇格した経路 (`HandleKeyConverting`) と昇格できなかった
  経路 (`HandleKeyDirect`) に分かれており、Space と Tab の挙動が文書の種類によって違う

本段階の目的は次のとおり。

- 割当を変えられる機能を広げ、任意の修飾キー + 仮想キーを、1機能に複数割り当てられるようにする
- 状態 (入力なし / run 中 / 候補選択中 / サジェスト選択中) ごとに割当を上書きできるようにする
- Enter を常にアプリへ渡し、確定キー (`CommitRun`) を新設する
- 候補選択中のキー処理を、昇格の有無によらず同じ挙動にする

## 決定事項

### 状態

| 状態 | 設定上の名前 | 判定 |
|---|---|---|
| 入力なし | `idle` | run が無い |
| run 中 | `run` | run があり、候補選択中でもサジェスト選択中でもない |
| 候補選択中 | `candidate` | `converting_` (composition に昇格しているかどうかは問わない) |
| サジェスト選択中 | `suggest` | 候補バー (docs/design/candidate-bar.md) で候補を選んでいる (`barIndex_ >= 0`) |

### 割当の対象外にするキー

次のキーは機能に割り当てられない (設定ファイルに書かれても無視し、設定ツールでは受け付けない)。

- コア操作: Enter・Esc・Tab・Backspace・Delete・↑↓←→・Home・End・PgUp・PgDn の、
  無修飾と Shift 併用。Ctrl+M・Ctrl+H (Enter・Backspace の読み替え。Shift を
  足した打鍵は対象にする)
- 印字キー (英字・数字・記号・テンキーの数字と演算子) の、無修飾と Shift 併用
- IME の切替キー (半角/全角・VK_KANJI・VK_IME_ON・VK_IME_OFF)。修飾キーによらない
- 修飾キー単体、Alt 併用、Win キー併用。Alt 併用の打鍵は key event sink に届かない
  アプリがあり (メモ帳で確認)、F10 と同じ preserved key で受けて送り直す方式は
  アプリ本来の Alt の動作 (メニュー・アクセラレータ) を再現しきれないため

コア操作の Ctrl 併用 (Ctrl+Backspace など) と Space・変換・無変換・ファンクションキーは
割当の対象にする。

### 機能

| 機能 (`KeyFunc`) | 設定キー | 働く状態 | 既定の割当 | 動作 |
|---|---|---|---|---|
| `Convert` | `key.convert` | 全状態 | `Convert` | 入力なし: 後置再変換。run 中・サジェスト選択中: 変換開始。候補選択中: 次候補 |
| `NextCandidate` | `key.next_candidate` | 候補選択中 | `Space` | 次候補 |
| `PrevCandidate` | `key.prev_candidate` | 候補選択中 | `Shift+Space` | 前候補 |
| `CommitRun` | `key.commit_run` | run 中・候補選択中・サジェスト選択中 | `NonConvert` | 下記「確定キー」 |
| `ConvertSymbol` | `key.convert_symbol` | run 中・候補選択中・サジェスト選択中 | `F4` | 記号・日付変換 |
| `ConvertUser` | `key.convert_user` | 同上 | `F5` | ユーザ登録語変換 |
| `ToHiragana` | `key.to_hiragana` | 同上 | `F6` | ひらがな変換 |
| `ToKatakana` | `key.to_katakana` | 同上 | `F7` | カタカナ変換 |
| `ToHalfKatakana` | `key.to_half_katakana` | 同上 | `F8` | 半角カタカナ変換 |
| `ToFullAscii` | `key.to_full_ascii` | 同上 | `F9` | 全角英字変換 |
| `ToHalfAscii` | `key.to_half_ascii` | 同上 | `F10` | 半角英字変換 |
| `UndoCommit` | `key.undo_commit` | 入力なし | `Ctrl+Backspace` | 確定アンドゥ |
| `RegisterWord` | `key.register_word` | 入力なし | `Ctrl+F7` | 単語登録ツールの起動 |
| `OpenConfig` | `key.open_config` | 入力なし | `Ctrl+F12` | 設定ツールの起動 |

- `Convert` に割り当てたキーは、Shift を足した打鍵にも一致する。候補選択中は Shift を
  足すと前候補になる (既存の Shift+変換キー の挙動)。それ以外の状態では Shift の有無で
  動作は変わらない。Shift を足した打鍵に他の機能が明示的に割り当てられていれば、そちらを優先する
- 文節移動・文節伸縮・変換の取消・サジェストの移動・候補番号の選択は、割当機能にしない。
  コア操作 (矢印・Esc など) と数字キーに固定する

### 割当の照合

- 打鍵の修飾キー (Ctrl・Shift) の組と仮想キーが、割当と完全に一致したときに機能が働く
  (`Convert` の Shift の扱いは上記の例外)
- 現在の状態で働かない機能の割当は照合しない
- 同じ状態で同じキーが複数の機能に割り当てられているときは、状態別の上書きを基本の割当より
  優先し、それでも重なる場合は上の表の並び順で先の機能を採る。設定ツールでは重なりを保存させない
- 入力なしの状態では、機能を実行できるときだけ打鍵を食べ、実行できなければアプリへ渡す
  (`Convert`: 後置再変換できる選択があるとき。`UndoCommit`: 確定アンドゥの記憶があるとき。
  他は常に食べる)。他の状態では、割当に一致した打鍵は常に食べる
- 割当に一致しない打鍵は、キー本来の処理に進む (コア操作・印字キー・Space のスペース挿入・
  アプリへの受け渡し)。たとえば候補選択中の Space の割当を外すと、Space は
  「確定してスペースを入れる」(サジェスト選択中と同じ) になる
- Alt を押している打鍵は照合しない (割当の対象外のため)。F10 だけは既存どおり preserved key
  で受け、現在の状態の割当に一致しなければアプリへ再送する

### コア操作の挙動

候補選択中は、昇格の有無によらず次の挙動に揃える。

| キー | run 中 | 候補選択中 | サジェスト選択中 |
|---|---|---|---|
| Enter / Ctrl+M | かなのまま run を終えてアプリへ渡す | 確定してからアプリへ渡す | 選択中の候補を採用して run を終えてからアプリへ渡す |
| Esc | run を忘れる (文字は残す) | 変換を取り消す | 選択を解除する |
| Backspace / Ctrl+H | 1字削除 | 変換を取り消す | 1字削除 (既存どおり) |
| Tab / Shift+Tab | 候補バーの選択へ (バーが無ければ run を終えてアプリへ渡す) | 変換を取り消してバーの先頭を選ぶ | 次 / 前の候補 |
| ↑ / ↓ | 候補バーの選択へ (バーが無ければ run を終えてアプリへ渡す) | 前候補 / 次候補 | 前 / 次の候補 |
| ← / → / Shift+← / Shift+→ | run を終えてアプリへ渡す | 確定してアプリへ渡す (`segment_ui` 1 では文節の移動 / 伸縮) | run を終えてアプリへ渡す |
| PgUp / PgDn | run を終えてアプリへ渡す | 確定してアプリへ渡す (`segment_ui` 1 では先頭 / 末尾の文節へ) | run を終えてアプリへ渡す |
| Home / End / Delete | run を終えてアプリへ渡す | 確定してアプリへ渡す | run を終えてアプリへ渡す |
| 1〜9 | 数字を入力 | 候補番号で選択 | 候補番号で選択 |

入力なしの状態では、コア操作はすべてアプリへ渡す。

- 候補選択は既定で入力全体から1つ選ぶ (docs/design/nbest.md)。文節の操作は設定 `segment_ui` 1
  (文節 UI) のときだけ使える
- 候補選択中の Shift+Tab は Tab と同じ (変換を取り消してバーの先頭を選ぶ)
- サジェスト選択中の選択の移動では文書を書き換えない。採用の操作と結果は
  docs/design/candidate-bar.md に定める
- 候補選択中の、割当の無い Ctrl / Alt 併用と Insert は、Home / End / Delete と同じく
  確定してからアプリへ渡す
- モードレスの英字モード中は、候補選択中の Space も割当より優先して「確定して半角スペース」
  にする (英文の語の区切り)

### Enter

- 候補選択中・サジェスト選択中の Enter (Ctrl+M も同じ) は、確定してから元の打鍵をアプリへ渡す
- 読める文書では、打鍵を食べずに、`OnTestKeyDown` / `OnKeyDown` の中で確定を済ませてから
  アプリへ渡す (`EndRunIfPassthroughKey` と同じ仕組み)。composition に昇格した候補選択中も
  この経路で確定する。確定は TSF の文書へ同期的に書き込まれるので、改行より先に入る
- 読めると判定されていない文書 (追記のみ・未判定) では、Enter を食べて確定し、確定の後に
  元の打鍵を `SendInput` で送り直す。送り直した打鍵は run が無い状態で届くので、IME は食べない
  - 候補選択中 (composition に昇格している): CUAS 経由のアプリ (WezTerm など) は確定結果を
    IME メッセージで後から受け取るため、食べずに渡すと確定結果より先に Enter が処理される。
    送り直した打鍵は入力キューの後ろに並ぶので、確定結果のメッセージより後に処理される
  - サジェスト選択中 (候補バーの採用): 擬似 Backspace で文書を書き換えるので、書き換えが
    終わった後 (目印の打鍵を受け取った後) に送り直す
- 確定アンドゥの記憶の扱いは append-input.md のとおり。Enter はアプリへ渡したキーなので、
  読めると判定されていない文書では、Enter で確定した直後の確定アンドゥは働かない。確定アンドゥを
  使うには確定キーで確定する

### 確定キー (`CommitRun`)

- run 中: 候補バーの全体変換を採用して run を終える。全体変換が無いとき・ルール3で英字に
  なるときは、かなのまま (英字になるときは英字で) run を終える。末尾の未完成のローマ字は、
  Enter などでアプリへ渡すときと同じ救済 (読める文書ではモードレスのルール3を適用、
  未完成のローマ字は文書に残す) を通す
- 候補選択中: 確定する
- サジェスト選択中: 選んでいる候補を採用する (先頭文節は残りのかなで run を続ける。
  docs/design/candidate-bar.md)
- 打鍵はアプリへ渡さない。確定アンドゥの記憶は残る

### 設定ファイル (config.tsv)

- 書式: `key.<機能>\t<キー>[,<キー>...]` が基本の割当、
  `key.<機能>@<状態>\t<キー>[,<キー>...]` が状態別の上書き。値 `none` は割当なし
- 上書きは、その機能が働く状態にだけ書ける。上書きした状態では基本の割当を使わない
- キーの表記: 修飾キーを `Ctrl+`・`Shift+` の順に前置し、キー名を続ける。`Alt+` を含む
  キーは対象外として捨てる
  (読み込みでは修飾キーの順序を問わない。書き出しはこの順)
- キー名:
  - `A`〜`Z`、`0`〜`9`、`F1`〜`F24`
  - `Space`・`Enter`・`Esc`・`Tab`・`Backspace`・`Delete`・`Insert`・`Home`・`End`・
    `PageUp`・`PageDown`・`Up`・`Down`・`Left`・`Right`
  - `Convert` (変換)・`NonConvert` (無変換)・`Kana` (カタカナ/ひらがな)
  - 上記以外は `VK_xx` (16進2桁の仮想キーコード)
- 読めない値・対象外のキーは、そのキーだけを捨てる。1つも残らなければ既定の割当のまま
  (`none` と区別する)
- 既存の設定ファイルの `key.*` の値 (`F4`・`Ctrl+F7`・`Ctrl+Backspace`・`Convert`・
  `Ctrl+Space`・`none`) は新しい書式でもそのまま読める

### 設定ツール

- 「キー割当」の欄を、機能ごとの一覧 (ListView) に置き換える。列は「機能」「キー」
  「状態別の上書き」。キーは表記をカンマ区切りで、上書きは `候補選択中: Space` のように表示する
- 一覧の行を選んで「編集」を押すと、その機能の編集ダイアログを開く
  - 基本の割当のキー一覧と、「追加」「削除」ボタン
  - その機能が働く状態ごとに「上書きする」チェックボックスとキー一覧・「追加」「削除」。
    チェックした直後の上書きは基本の割当の写しにする (チェックだけでは動作を変えない)
  - 「追加」は、押したキーを取り込むダイアログ (WM_KEYDOWN と WM_SYSKEYDOWN を受ける)。
    対象外のキーは取り込まずに理由を表示する
- 保存時に、同じ状態で同じキーが複数の機能に割り当てられていれば、重なっている機能と状態を
  示して保存しない
- 「既定に戻す」ボタンで、全機能の割当を既定に戻す (保存は別途)

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h, tsf/src/config.cpp | `KeyFunc` に `NextCandidate`・`PrevCandidate`・`CommitRun` を追加。`KeyBinding` を修飾キー3つ + vk にし、機能ごとの基本の割当と状態別の上書きを複数キーで持つ。新しい表記のパース。状態とキーから機能を引く照合関数 (`FindPlainFunc`・`FindCtrlFunc` を置き換える) |
| tsf/src/text_service.cpp, tsf/src/text_service.h | `IsKeyEaten`・`HandleKeyConverting`・`OnPreservedKey` (F10) を照合関数で書き直す。候補選択中の Space・Tab を本書の挙動に揃える。状態の判定関数 |
| tsf/src/text_service_direct.cpp | `IsKeyEatenDirect`・`HandleKeyDirect`・`EndRunIfPassthroughKey` を照合関数で書き直す。Enter の「確定してから渡す」、擬似 Backspace が要る場合の Enter の送り直し、`CommitRun` |
| engine/src/bin/quicklime-config.rs | キー割当の一覧・編集ダイアログ・キー取り込み・重なりの検査、新しい書式の読み書き |
| README.md | 「入力方式」のキー操作の記述、キー割当の書式と対象外のキー |
| docs/design/direct-input.md | キー処理の表と「変換キー」の節を本書に合わせて書き換える |
| docs/design/input-model-v2.md | 段階表の詳細設計に本書を記し、未決事項からコア操作の範囲を外す (本書の作成時に済ませる) |

## 実装手順

各手順の後にビルドが通り、既存の動作が壊れていないことを確かめる。

1. config: 新しい `KeyBinding`・割当の持ち方・表記のパースと照合関数。既定の割当は
   現状の挙動を再現する (この時点ではキー処理側は差し替えない)
2. キー処理の差し替え: `IsKeyEaten` 系・`HandleKey` 系・F10 を照合関数で書き直す。
   候補選択中の Space・Tab の統一、Shift+Space の前候補
3. 確定キー `CommitRun`
4. Enter の「確定してから渡す」(同期で確定する経路と、擬似 Backspace 後に送り直す経路)
5. 設定ツール
6. ドキュメントの更新 (README.md、direct-input.md)

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。
config.tsv は `QUICKLIME_CONFIG_FILE` で隔離したものを使い、`debug_log` を設定する。

### テストケース

| # | 環境 | 設定・操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | 既定の設定。`kyouha` → 変換キー → Enter | 確定して改行まで入る (1打) |
| 2 | Chrome の textarea (composition に昇格する) | 1 と同じ | 同上。確定文字列の後に改行が入る |
| 3 | Chrome の検索欄など Enter で送信する欄 | `kyouha` → 変換キー → Enter | 確定した文字列で送信される |
| 4 | WezTerm | `kyouha` → 変換キー → Enter | 確定した文字列の後に Enter がシェルへ届く |
| 5 | メモ帳 | `kyouha` → Tab でサジェストを選ぶ → Enter | 採用して確定し、改行が入る |
| 6 | WezTerm | `kyouha` → Tab でサジェストを選ぶ → Enter | 文書が候補に書き換わった後に Enter が届く (順序が逆にならない) |
| 7 | メモ帳 | `kyouha` → 無変換 | run が終わる。改行は入らない。続けて Ctrl+Backspace で読みに戻る |
| 8 | メモ帳 | `kyouha` → 変換キー → 無変換 | 確定する。改行は入らない |
| 9 | メモ帳 | `kyouha` → Tab → 無変換 | サジェストを採用して確定する |
| 10 | メモ帳 (モードレス ON) | `kyouhat` → 無変換 | Enter で終えたときと同じ救済になる |
| 11 | WezTerm | `kyouha` → 変換キー → 無変換 → Ctrl+Backspace | 読みに戻る。`kyouha` → 変換キー → Enter → Ctrl+Backspace では働かず、文字が消されない |
| 12 | メモ帳と Chrome の textarea (昇格の有無の両方) | `kyouha` → 変換キー → Space → Shift+Space | 次候補 → 前候補。どちらの文書でも同じ |
| 13 | メモ帳と Chrome の textarea | `kyouha` → 変換キー → Tab | 変換を取り消してサジェスト選択に移る。どちらの文書でも同じ |
| 14 | メモ帳 | `key.next_candidate@candidate none` を設定。`kyouha` → 変換キー → Space | 確定してスペースが入る |
| 15 | メモ帳 | `key.convert Convert,Ctrl+Space` | 変換キーと Ctrl+Space の両方で変換・次候補。Shift+どちらかで前候補 |
| 16 | メモ帳 | `key.to_katakana F7,Ctrl+K` | run 中の F7・Ctrl+K の両方でカタカナになる。run が無いときの Ctrl+K はアプリへ渡る |
| 17 | メモ帳 | `key.commit_run@candidate Ctrl+Enter` | 候補選択中は Ctrl+Enter で確定し、無変換は効かない (アプリへ渡る)。run 中は無変換で確定する |
| 18 | メモ帳 | `key.to_half_ascii none` | run 中の F10 がアプリへ渡る (メニューへフォーカス) |
| 19 | メモ帳 | `key.convert_symbol F4,Alt+S` | Alt+S は無視され、run 中の Alt+S はアプリへ渡る。F4 は記号変換になる |
| 20 | 既存の config.tsv (`key.convert Ctrl+Space`・`key.undo_commit Ctrl+F1` など) | 各キー | 既存どおりに働く |
| 21 | config.tsv | `key.to_katakana Enter`、`key.to_katakana A`、`key.to_katakana Shift+Tab` | 無視され、既定の F7 のまま |
| 22 | 設定ツール | 機能の編集で、キー取り込みに Enter・A・Ctrl+H・Alt+S を押す | 取り込まれず、理由が表示される |
| 23 | 設定ツール | 2つの機能の候補選択中に同じキーを割り当てて保存 | 重なりが表示され保存されない |
| 24 | 設定ツール | 割当と上書きを変えて保存し、開き直す | 保存した割当が表示され、config.tsv が本書の書式になっている |
| 25 | メモ帳 | 候補選択中に ←→・Shift+←→・PgUp/PgDn・↑↓・1〜9・Esc・Backspace | 既存どおり |
| 26 | メモ帳 | run が無いときに Ctrl+Backspace (記憶なし)・変換キー (選択なし) | アプリへ渡る |

## 未決事項

- なし
