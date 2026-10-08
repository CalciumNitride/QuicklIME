# 追記型入力への一本化と composition 方式の廃止

入力モデル v2 (docs/design/input-model-v2.md) の段階2。実験 0a
(docs/archive/experiment-0a-append-input.md) で採用を決めた F1 方式を本実装とし、
composition 方式を廃止する。

## 背景と目的

direct 方式は、空の入力欄の冒頭と CUAS 経由のアプリ (WezTerm など) で composition 方式に
フォールバックしていた。実験 0a では、未完成のローマ字を文書の外 (キャレット付近の小窓) に
出し、かなを追記だけで入れる方式 (F1) を試した。その結果、メモ帳・Chrome・Word・WezTerm の
すべてで、フォールバックなしに入力・変換できることを確認した。

本段階の目的は次のとおり。

- direct 方式を常に F1 で動かし、置換型の打鍵経路とフォールバックを取り除く
- composition 方式 (設定 `input_style`) を廃止する
- 試作で残った問題を直す: 修飾キーを押したままの擬似 Backspace、追記のみの文書での
  キャレット移動の見落とし、追記のみの文書でのサジェストと確定アンドゥ

## 決定事項

### 入力の経路

- IME オン時の打鍵は、常に F1 の経路 (試作の `TypeAppend` 系) で処理する
- 削除するもの:
  - 置換型の打鍵経路 (未完成のローマ字を文書に入れて毎打鍵で置換する処理) と、
    `DirectCapability` によるフォールバック判定、composition 方式へのフォールバック
  - F2 (未完成のローマ字の composition) のコード
  - 隠し設定 `direct_append`
- 文書の分類は、試作の3分類 (未判定 / 読める / 追記のみ) に一本化する
- 置換 edit session (`ReplaceRunEditSession` など) は、読める文書での変換・英字切替・
  サジェストの採用に使うので残す
- 隠し設定 `debug_log` は診断用に残す。出す内容は試作のまま

### composition 方式の廃止

- 設定 `input_style` を廃止する。config.tsv に残っていても読み飛ばす
  (`composition` が書かれていても direct 方式で動く)
- 設定ツール (quicklime-config.exe) から「入力方式」の項目を削除する。保存時に
  `input_style` を書き出さない
- 削除するもの: 変換前の composition 入力 (未確定文字列への打鍵、Space による変換開始、
  Enter での確定、composition 上のサジェスト・確定アンドゥ)
- 残すもの: 候補選択中の composition (昇格した run の変換)、文節 UI (文節移動・伸縮・
  文節別の候補選択)、F4〜F10 の変換、Shift+英字の英字モード。これらは今と同じく
  run を composition に昇格して動かす
- `HandleKeyComposition` は、候補選択中 (昇格した composition) のキー処理だけを
  受け持つ形に縮める。名前を実態に合わせて変えてよい

### ライブ変換の廃止

- ライブ変換は本段階で削除し、段階3の候補バーで作り直す
- 削除するもの: 設定 `live_conversion` (config.tsv に残っていても読み飛ばす)、
  設定ツールの「ライブ変換」チェックボックス、TSF 層のライブ変換の処理
  (`LiveDisplayText`・`liveSegments_` など)
- エンジンは変更しない (CONVSEG の live 指定などは段階3で使う見込みのため残す)

### 擬似 Backspace と修飾キー

追記のみの文書で入れた文字を消すには、Backspace の擬似打鍵を送る (試作と同じ)。
変換キーなど修飾キー付きの操作から送ると、アプリには Ctrl+Backspace (単語削除) や
Alt+Backspace (元に戻す) として届いてしまう。

- 擬似打鍵の列を次の順にする
  1. 送信時点で押されている修飾キー (Ctrl・Shift・Alt・Win の左右) を離す打鍵
  2. Backspace x n
  3. 1 で離した修飾キーのうち、送信時点で物理的に押されているものを押し直す打鍵
  4. 目印の打鍵 (VK_F24)
- 押されているかは `GetAsyncKeyState` で調べる
- 擬似打鍵として送った修飾キーの打鍵は、Backspace と同じく IME では処理せずアプリへ渡す
- Alt を離す打鍵でアプリのメニューが開かないよう、Alt を離す前に無害な打鍵
  (VK_NONAME など、メニューの起動を打ち消すもの) を挟む。具体的な打鍵は実装時に確かめる
- 既知の制約 (受け入れる): 擬似打鍵を送っている数 ms の間にユーザが修飾キーを物理的に
  離すと、アプリから見て押されたままになることがある。もう一度押して離せば直る

### 追記のみの文書でのキャレット移動の検知 (マウス)

追記のみの文書では文書を読み戻せないため、キャレットが動いたことを見落とすと、
擬似 Backspace が移動先の手前の文字を消す。キー操作による移動は、既存の処理
(Ctrl・Alt 併用キー、編集キー、Esc で run を終える) で対処済み。残るのはマウス操作である
(マウス対応の vim やクリックでカーソルを動かせる TUI など)。

- 追記のみの文書で、次のどちらかが有効な間だけ、IME の動いているスレッドに
  マウスフック (`SetWindowsHookEx(WH_MOUSE, ..., GetCurrentThreadId())`。
  そのスレッドだけを対象にするフック) を仕掛ける
  - run
  - 確定アンドゥの記憶 (直前の確定文字列と読み)
- 次のマウスメッセージが来たら、文書には触れずに run を終え、確定アンドゥの記憶も捨てる
  - ボタンを押す操作 (左・右・中・X ボタン。クライアント領域・非クライアント領域とも)
  - ホイール (縦・横)
- どちらも無効になったら、フックを外す
- フックはマウスメッセージを止めず、必ず次のフックへ渡す
- マウスで実際にキャレットが動いたかは区別しない。誤って run を終えたときの不便は
  「その run を変換できない」だけで、文字を誤って消す事故より軽いため
- 読める文書は、変換・置換の前に文書を読み戻して照合するので、対象外とする

### 追記のみの文書でのサジェスト

- 表示する (試作では出さなかった)
- Tab・↑↓ による選択は、候補ウィンドウ上の選択だけを動かし、文書は書き換えない
- 選択中に採用したとき (現状の Enter による確定) に、擬似 Backspace で run の文字列を消してから、
  候補の文字列を追記する
- 読める文書のサジェストは現状のまま (選択するたびに文書の文字列を置き換える)。
  サジェストの見せ方は段階3の候補バーで作り直す

### 追記のみの文書での確定アンドゥ

- 確定アンドゥ (既定 Ctrl+Backspace) は、擬似 Backspace で直前の確定文字列を消してから、
  読みのかなを追記して run を再開する
- 確定アンドゥの記憶は、上のマウスフックの対象にする
- 読めると判定されていない文書 (追記のみ・未判定) では、次の契機でも確定アンドゥの記憶を捨てる。
  照合なしに擬似 Backspace で消すため、確定後に文書が変わりうる操作の後には働かせない
  - アプリへ渡したキー (run を終えた Enter 自身を含む。vim の挿入モードなどで改行が入るため)
  - run の外の Space
- フォーカス移動では、すべての文書で確定アンドゥの記憶を捨てる

### F1 の小窓の位置

- 選択範囲の矩形 (`GetSelectionExtentEditSession`) が取れないときは、
  `GetGUIThreadInfo` のシステムキャレットの矩形を代わりに使う。それも取れなければ表示しない

### 試作から引き継ぐ挙動

次は試作の挙動をそのまま本実装の仕様とする。

- 文書の分類: 未判定の文書は2文字目以降の追記時に読み戻して「読める」か「追記のみ」に分ける。
  `OnSetFocus` で未判定に戻す
- 追記のみの文書では、`OnEndEdit` で IME 由来でない選択変更が来たら run を終える
  (通知が届くアプリでは、マウスフックと二重に効く)
- Backspace: 未完成のローマ字があればそれを削る。無ければアプリに渡し、run の末尾から
  表示上の1文字を削る
- 追記のみの文書では、アプリへ渡すキー (Enter など) で run を終えるとき、モードレス入力の
  ルール3 (未変換の子音1文字で英字にする) を適用しない
- 擬似 Backspace を送ってから目印の打鍵を受け取るまでの Backspace は、すべてアプリへ渡す
- 未完成のローマ字は、Esc・Enter・矢印などで run を終えるときは文書にそのまま残し、
  フォーカス移動・IME オフ・キャレット移動で終えるときは捨てる

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/text_service_direct.cpp | 置換型の打鍵経路・フォールバック・F2 の削除、F1 の一本化、修飾キー付きの擬似打鍵、マウスフック、追記のみの文書のサジェスト・確定アンドゥ、小窓の位置の代替 |
| tsf/src/text_service.cpp | composition 方式の入力処理の削除 (候補選択中の処理だけ残す)、ライブ変換の削除、キー処理の分岐の整理 |
| tsf/src/text_service.h | 上記に伴うメンバの削除・追加 (`DirectCapability`・`liveSegments_`・F2 の状態の削除、マウスフックの状態の追加) |
| tsf/src/config.h, tsf/src/config.cpp | `input_style`・`direct_append`・`live_conversion` の削除 (読み飛ばす) |
| tsf/src/edit_session.h, tsf/src/edit_session.cpp | F2 用・フォールバック判定用で不要になった edit session の削除 |
| engine/src/bin/quicklime-config.rs | 「入力方式」「ライブ変換」の項目の削除 |
| README.md | 「入力方式」「モードレス入力」の節を新しい挙動に書き直す |
| docs/design/direct-input.md | フォールバック判定・composition 方式・ライブ変換の記述を、本書の内容に合わせて書き換える |
| docs/design/modeless.md | composition 方式とライブ変換に触れている箇所を直す |
| docs/archive/experiment-0a-append-input.md | docs/design/ から移す |

## 実装手順

各手順の後にビルドが通ること、既存の動作が壊れていないことを確かめる。

1. F1 への一本化: `direct_append` を廃止して direct 方式を常に F1 で動かす。
   置換型の打鍵経路・`DirectCapability`・フォールバック・F2 を削除する
2. composition 方式の廃止: `input_style` を廃止し、変換前の composition 入力の処理を削除する。
   候補選択中の処理は残す
3. ライブ変換の削除
4. 擬似打鍵の修飾キー対策
5. マウスフック
6. 追記のみの文書のサジェストと確定アンドゥ、小窓の位置の代替
7. 設定ツールの項目削除
8. ドキュメントの更新 (README.md、direct-input.md、modeless.md)、experiment-0a の移動

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。
config.tsv に `debug_log` を設定し、ログで文書の分類と擬似打鍵を確かめる。

### テストケース

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 (空) | `kyouha` → 変換キー → Enter | 冒頭から追記で入り、変換・確定できる。未完成のローマ字は小窓に出る |
| 2 | Chrome の空の欄・文字数固定の欄 | かなと数字の入力、変換 | 冒頭から追記で入り、欄の移動が誤動作しない |
| 3 | Word | `kyouha` → 変換キー → Enter | 読める文書として変換できる |
| 4 | WezTerm (シェル) | `kyouha` → 変換キー → Enter | 擬似 Backspace で消してから変換結果が入る |
| 5 | WezTerm、変換キー = Ctrl+Space | `kyouha` → Ctrl+Space → (Ctrl を押したまま) Space で次候補 → Enter | 単語ごと消えない。Ctrl を押したままの次候補が効く。確定後に Ctrl が押されたままにならない |
| 6 | WezTerm、変換キー = Ctrl+Space | 5 と同じ操作を、Ctrl をすぐ離して行う | 同上 |
| 7 | WezTerm の vim (`set mouse=a`、挿入モード) | `kyou` → 別の行をクリック → 変換キー | ログで run が終わっている。文字が消されない |
| 8 | WezTerm の vim (`set mouse=a`、挿入モード) | `kyou` → ホイールでスクロール → 変換キー | 同上 |
| 9 | WezTerm | `kyouha` → 変換キー → Enter (候補選択中の確定) → Ctrl+Backspace | 「きょうは」が読みに戻り、run が再開する。`kyouha` → Enter (アプリへ渡る) → Ctrl+Backspace では働かず、文字が消されない |
| 10 | WezTerm | `kyouha` → 変換キー → Enter → クリック → Ctrl+Backspace | 確定アンドゥが働かず、文字が消されない |
| 11 | WezTerm | `kyouha` → Tab でサジェストを選ぶ → Enter | 選択中は文書が変わらず、Enter で候補に置き換わる |
| 12 | メモ帳 | `kyouha` → Tab でサジェストを選ぶ → Enter | 現状どおり、選択するたびに文書が置き換わり Enter で確定 |
| 13 | config.tsv に `input_style composition`・`live_conversion 1` を残したまま | 各アプリで入力 | direct 方式 (F1) で動き、ライブ変換されない |
| 14 | メモ帳 (モードレス ON) | `apple`、`kyouhagithub` | 英字に切り替わる (既存の判定どおり) |
| 15 | メモ帳 | `kyouha` → 変換キー → ←→・Shift+←→ で文節を操作 → Enter | 文節 UI が今までどおり動く |
| 16 | メモ帳 | `kyouha` → F7 / F10 | カタカナ / 半角英字に変換される |
| 17 | 設定ツール | 開いて保存 | 「入力方式」「ライブ変換」の項目が無く、保存した config.tsv に `input_style`・`live_conversion` が書かれない |

## 未決事項

- Alt を離すときにメニューの起動を打ち消す打鍵の具体値 (実装時に確かめる)
