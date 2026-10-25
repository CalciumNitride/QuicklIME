# キー割当

IME がオンのときに、打鍵を入力の状態と割当に照らして、どの機能にするか・アプリへ渡すかを決める仕様。状態、割り当てられる
機能、割り当てられないキー、割当の照合、打鍵の処理の流れ、状態ごとのキーの働き、数字キーと Ctrl+1〜9、設定ファイルの書式を扱う。

- 利用者向けの説明は [キー割当](../user/keymap.md) と [入力と変換](../user/input.md#キー操作) にある
- 状態ごとの表のセルは働きの要約で、働きの中身は各仕様書にある。run・読みの編集・run の終わり方・送り直し・確定アンドゥ・
  改行キー・IME のオン/オフは [input.md](input.md)、変換キーの候補選択 (変換の対象・範囲の伸縮・文節 UI・確定と取消・F4〜F10) は
  [conversion.md](conversion.md)、候補バーの選択と採用・採用範囲の縮め・後続語の予測は [suggest.md](suggest.md)
- 設定ツールのキー割当の画面 (機能ごとのカード・キーの取り込み・重なりの検査) は [tools.md](tools.md)「キー割当のページ」
- 実装は tsf/src/config.cpp (割当の読み込み・割り当てられるかの判定・照合)、tsf/src/text_service.cpp (`IsKeyEaten`・
  `HandleKey`・`RunKeyFunc`・候補選択中の `HandleKeyConverting`・F10 の `OnPreservedKey`)、tsf/src/text_service_direct.cpp
  (`IsKeyEatenDirect`・`HandleKeyDirect`・`EndRunIfPassthroughKey`・`PanelNumberKey`・`DismissNextWordForKey`)。
  設定ツールの読み書きと検査は engine/gui/src/keys.rs で、TSF 層と同じ規則にする

## 状態

割当の照合に使う入力の状態 (tsf/src/text_service.cpp `CurrentKeyState`)。

| 状態 | 設定上の名前 | 判定 |
|---|---|---|
| 入力なし | `idle` | run が無い。後続語の予測を出していて選んでいないときを含む |
| run 中 | `run` | run があり、候補選択中でもサジェスト選択中でもない |
| 候補選択中 | `candidate` | 変換キー・F4〜F10 で変換の候補を選んでいる |
| サジェスト選択中 | `suggest` | 候補バーで候補を選んでいる。run の無いときに出す後続語の予測で候補を選んでいるときを含む |

- 設定上の名前は状態別の上書き (下記「設定ファイルの書式」) に使う
- キーの働きは、同じ状態の中でも run の有無 (サジェスト選択中の run のバーと後続語の予測)、後続語の予測を出しているか、
  候補バーがあるか、設定 `segment_ui` などで分かれる (下記「run の無い状態のキー」「run のある状態のキー」)

## 機能

割当を変えられる機能 (tsf/src/config.h `KeyFunc`)。

| 機能 | 設定キー | 働く状態 | 既定の割当 | 働き |
|---|---|---|---|---|
| `Convert` | `key.convert` | すべて | `Convert` (変換) | 入力なし: 入力なしの対象の変換 ([conversion.md](conversion.md)「変換の対象」)。run 中・サジェスト選択中: 変換を始める。候補選択中: 次候補 |
| `NextCandidate` | `key.next_candidate` | 候補選択中 | `Space` | 次候補 |
| `PrevCandidate` | `key.prev_candidate` | 候補選択中 | `Shift+Space` | 前候補 |
| `CommitRun` | `key.commit_run` | 入力なし以外 | `NonConvert` (無変換) | 確定キー。run 中: 見えているとおりに run を終える ([input.md](input.md)「run の終わり方」)。サジェスト選択中: 選んでいる候補を採用する ([suggest.md](suggest.md)「採用」「選択と採用」)。候補選択中: 確定する ([conversion.md](conversion.md)「確定と取消」) |
| `ConvertSymbol` | `key.convert_symbol` | 入力なし以外 | `F4` | 記号・日付変換 ([conversion.md](conversion.md)「ファンクションキー変換 (F4〜F10)」) |
| `ConvertUser` | `key.convert_user` | 入力なし以外 | `F5` | ユーザ語変換 (同上) |
| `ToHiragana` | `key.to_hiragana` | すべて | `F6` | ひらがな変換 (同上。入力なしの対象は `Convert` と同じ) |
| `ToKatakana` | `key.to_katakana` | すべて | `F7` | カタカナ変換 (同上) |
| `ToHalfKatakana` | `key.to_half_katakana` | すべて | `F8` | 半角カタカナ変換 (同上) |
| `ToFullAscii` | `key.to_full_ascii` | すべて | `F9` | 全角英字変換 (同上) |
| `ToHalfAscii` | `key.to_half_ascii` | すべて | `F10` | 半角英字変換 (同上) |
| `UndoCommit` | `key.undo_commit` | 入力なし | `Ctrl+Backspace` | 確定アンドゥ ([input.md](input.md)「確定アンドゥ」) |
| `RegisterWord` | `key.register_word` | 入力なし | `Ctrl+F7` | 単語登録ツールを起動する ([dictionary.md](dictionary.md)「単語登録ツール」) |
| `OpenConfig` | `key.open_config` | 入力なし | `Ctrl+F12` | 設定ツールを起動する |
| `InsertNewline` | `key.insert_newline` | すべて | `Ctrl+J` | 改行キー ([input.md](input.md)「改行キー」)。入力なし: アプリへ Shift+Enter を送る。run 中・run のバーのサジェスト選択中・候補選択中: Enter と同じに run を終える・採用する・確定してから Shift+Enter を送り直す |
| `NextWordNext` | `key.next_word_next` | 入力なし・サジェスト選択中 | `Ctrl+Down` | 後続語の予測の次の候補を選ぶ (未選択から先頭) |
| `NextWordPrev` | `key.next_word_prev` | 入力なし・サジェスト選択中 | `Ctrl+Up` | 後続語の予測の前の候補を選ぶ (未選択から末尾、先頭から末尾) |
| `ForgetLearning` | `key.forget_learning` | サジェスト選択中 | `Ctrl+Delete` | 選んでいる候補の学習を消す ([suggest.md](suggest.md)「学習の削除の後の候補」) |

- 表の並びは、同じ状態で同じキーが重なったときの優先の順を兼ねる (下記「割当の照合」)
- `Convert` の割当は、Shift を足した打鍵にも一致する。候補選択中は Shift を足すと前候補になり、ほかの状態では Shift の有無で
  働きは変わらない
- `CommitRun` は打鍵をアプリへ渡さない。run を終えたら後続語の予測を出す ([suggest.md](suggest.md)「出すとき・消すとき」)
- `ConvertSymbol`・`ConvertUser` が入力なしで働かない理由は [conversion.md](conversion.md)「変換の対象」
- `NextWordNext`・`NextWordPrev` に ↑ / ↓ を割り当てたときは、キャレット行の上に反転して出した入力パネルで向きを入れ替える
  ([display.md](display.md)「キャレット行の上に出すときの反転」)
- `ForgetLearning` は、英字候補・学習でない候補を選んでいるとき・エンジンに接続できないときも打鍵を食べて何もしない。
  アプリへ渡すとキャレットの後ろの文字が消えるため
- 候補バーと後続語の予測の選択、範囲の伸縮、文節の移動と伸縮、変換の取消、番号での採用・確定は割当の機能にしない。
  コア操作 (矢印・Tab・Esc・Backspace など) と Ctrl+1〜9 に固定する (下記の表と「数字キーと Ctrl+1〜9」)

## 割り当てられないキー

次のキーは機能に割り当てられない。設定ファイルに書いても捨て、設定ツールは取り込まずに理由を出す (tsf/src/config.cpp
`IsAssignable`、engine/gui/src/keys.rs `unassignable_reason`)。

| キー | 理由 |
|---|---|
| コア操作 (Enter・Esc・Tab・Backspace・Delete・↑・↓・←・→・Home・End・PageUp・PageDown) の、修飾なしと Shift 併用 | 状態ごとに決まった働き (下記の表) に使う |
| 印字キー (英字・数字・記号・テンキーの数字と演算記号) の、修飾なしと Shift 併用 | 読みの入力に使う |
| Ctrl+M・Ctrl+H (Shift なし) | Enter・Backspace の読み替えに使う |
| Ctrl+1〜9 (上段の数字。Shift なし) | 番号での採用・確定に使う (下記「数字キーと Ctrl+1〜9」) |
| IME の切替キー (`VK_KANJI`・`VK_OEM_AUTO`・`VK_OEM_ENLW`・`VK_IME_ON`・`VK_IME_OFF`) | 修飾キーによらず IME のオン/オフに使う ([input.md](input.md)「IME のオン/オフ」) |
| 修飾キーだけ | |
| Alt 併用 | Alt 併用の打鍵が key event sink に届かないアプリがある (メモ帳で確認) |
| Win 併用 | 設定の表記に Win が無く、Win を押している打鍵は照合しない |

- Insert・Space・変換・無変換・カタカナ/ひらがな・ファンクションキーは修飾なしでも割り当てられる。コア操作と印字キーの
  Ctrl 併用 (Ctrl+Backspace・Ctrl+Shift+1 など)・Ctrl+0 も割り当てられる
- 採らなかった案: Alt 併用の打鍵を F10 と同じく preserved key で受け、割当に一致しなければアプリへ送り直す。アプリ本来の
  Alt の働き (メニュー・アクセラレータ) を再現しきれない

## 割当の照合

- 打鍵の Ctrl・Shift・Alt の組と仮想キーが割当と完全に一致したときに、その機能にする (tsf/src/config.cpp `TsfConfig::FindFunc`)。
  Win を押している打鍵は照合しない
- 今の状態で働かない機能の割当は照合しない
- 状態別の上書きのある機能は、その状態では上書きのキーだけを使い、基本の割当を使わない。上書きで一致したものを基本の割当で
  一致したものより優先し、それでも重なれば「機能」の表の並びで先の機能を採る。設定ツールは同じ状態で同じキーを重ねさせない
- `Convert` の Shift: どの割当にも一致しない Shift 併用の打鍵は、Shift を除いた打鍵が `Convert` の割当に一致すれば、Shift を
  足した `Convert` とする。Shift を足した打鍵に明示的な割当があれば、そちらが先に一致する
- モードレスが有効で英字モード中の、候補選択中の Space (Ctrl・Alt なし) は照合しない。割当によらず確定して半角スペースを入れる
  (英文の語の区切りのため)
- 一致した機能の打鍵を食べるか (tsf/src/text_service.cpp `CanRunKeyFunc`)
  - 入力なし: `Convert`・F6〜F10 は入力なしの対象があるとき ([conversion.md](conversion.md)「変換の対象」)、`UndoCommit` は
    確定アンドゥが働くとき ([input.md](input.md)「確定アンドゥ」) だけ食べる。ほかの機能は常に食べる
  - 入力なし以外: 常に食べる
  - `InsertNewline`: 状態によらず、改行キーの対象のアプリ (`newline_apps`) のときだけ食べる
  - `NextWordNext`・`NextWordPrev`: 状態によらず、後続語の予測を出しているときだけ食べる。run のバーを選んでいるときは食べない
- 食べない打鍵はアプリへ渡す。run・候補選択を終えるかは、割当の無い打鍵と同じに決める (下記の表。run 中の Ctrl+J なら、
  割当の無い Ctrl 併用として run を終えて渡す)
- 割当に一致しない打鍵は、キーの決まった働き (下記の表) に進む。たとえば候補選択中の Space の割当を外すと、Space は確定して
  スペースを入れる

### F10

- 修飾なしの F10 は preserved key で受ける (tsf/src/text_service.cpp `OnPreservedKey`)。F10 は WM_SYSKEYDOWN で届き、TSF に
  対応していないアプリ (WezTerm など) では key event sink まで届かないことがあるため
- 今の状態で F10 に一致する割当が無いとき、一致しても食べないとき (入力なしで対象が無いなど)、IME オフ・InputScope の欄では、
  フォーカスのウィンドウへ WM_SYSKEYDOWN の F10 を送り、アプリ本来の F10 の働き (メニュー) を再現する (Mozc と同じ)

## 打鍵の処理の流れ

key event sink の `OnTestKeyDown` / `OnKeyDown` で、打鍵ごとに次の順に処理する。

1. キー入力が無効な文書なら、食べずに、残っている run を捨てる ([input.md](input.md)「キー入力が無効な文書」)
2. 後続語の予測を出していれば、予測のまま扱うキーかを確かめ、そうでなければ予測を消す (`DismissNextWordForKey`。
   [suggest.md](suggest.md)「出すとき・消すとき」)。以降は消した後の状態 (入力なし) で照合する
3. 入力なしで、変換キー・F6〜F10・確定アンドゥに一致する打鍵なら、先に文書を分類する ([input.md](input.md)「読める文書の判定」)
4. 食べずに渡すキーで run・候補選択を終えるもの (下記の表の「run を終えて渡す」「確定して渡す」のうち、5 で食べないもの) は、
   ここで終える (`EndRunIfPassthroughKey`)。読める文書で後続語の予測を選んでいるときの Enter・Ctrl+M の採用もここで行う
5. 食べるかを決める (`IsKeyEaten`)
   - IME オフと InputScope の欄 ([input.md](input.md)「InputScope の欄」) では何も食べない
   - 読めると判定されていない文書の run 中に run を終えて渡すキーは、食べて run を終えてから送り直す
     ([input.md](input.md)「送り直し」)
   - 割当に一致すれば上記「割当の照合」、一致しなければ下記の表に従う
6. 食べた打鍵を処理する (`HandleKey`)。食べた打鍵の key-up も食べる。端末エミュレータは key-up もアプリへ送り、確定に使った
   Enter などの key-up が素通りすると入力の誤送信につながるため

- 食べないキーはホストが `OnKeyDown` を呼ばずに処理することがあるため、2〜4 は `OnTestKeyDown` でも行う
- IME の切替キーと修飾なしの F10 は preserved key で、key event sink より先に受ける

## run の無い状態のキー

| キー | 入力なし | 後続語の予測を選択中 |
|---|---|---|
| 印字キー (英字・記号・数字・テンキー) | run を始める。`digits half` では数字キーとテンキーをアプリへ渡す ([input.md](input.md)「印字キー」) | 予測を入れ、その打鍵で run を始める (`digits half` の数字も) |
| Space | `space full` で修飾なしなら全角スペースを入れる ([input.md](input.md)「印字キー」)。それ以外はアプリへ渡す | 予測を入れてスペースを入れる |
| Enter・Ctrl+M | アプリへ渡す | 予測を入れてからアプリへ渡す |
| 確定キー | 働かない (アプリへ渡す) | 予測を入れ、次の語の予測を出す |
| Esc | アプリへ渡す | 予測を消す (キーは食べる) |
| Tab / Shift+Tab、↓ / ↑ | アプリへ渡す | 次 / 前の候補を選ぶ |
| ← / → | アプリへ渡す | 横一列のテーマで修飾なしなら前 / 次の候補を選ぶ。それ以外は予測を消す |
| Ctrl+1〜9 | アプリへ渡す。後続語の予測を出していれば、その番号の予測を入れ、次の語の予測を出す | その番号の予測を入れ、次の語の予測を出す |
| `NextWordNext`・`NextWordPrev` (Ctrl+↓・Ctrl+↑) | 後続語の予測を出していれば先頭 / 末尾を選ぶ。出していなければアプリへ渡す | 次 / 前の候補を選ぶ |
| `ForgetLearning` (Ctrl+Delete) | 働かない (アプリへ渡す) | 選んでいる予測の学習を消す |
| 変換キー・F6〜F10 | 入力なしの対象があれば変換する ([conversion.md](conversion.md)「変換の対象」)。無ければアプリへ渡す | 予測を消す |
| F4・F5 | 働かない (アプリへ渡す) | 予測を消す |
| `UndoCommit` (Ctrl+Backspace) | 確定アンドゥ。働かないときはアプリへ渡す | 予測を消す |
| `RegisterWord`・`OpenConfig` (Ctrl+F7・Ctrl+F12) | ツールを起動する | 予測を消す |
| 改行キー (Ctrl+J) | 対象のアプリなら Shift+Enter を送る。対象外ならアプリへ渡す | 予測を消す |
| Backspace・Delete・Home・End・PgUp・PgDn・Insert、割当の無い Ctrl 併用・Alt 併用、その他のキー | アプリへ渡す | 予測を消す |

- 「予測を消す」は、予測を消してから入力なしの列のとおりに処理する (Esc を除く)
- 後続語の予測を出していて選んでいない間 (入力なし) は、`NextWordNext`・`NextWordPrev` と Ctrl+1〜9 だけが予測に働く。
  ほかのキーは予測を消してから入力なしの列のとおりに処理する
- 予測を入れる働きの中身 (スペースの幅、`digits half` の数字を run にする理由) は [suggest.md](suggest.md)「選択と採用」。
  Enter・Ctrl+M の送り直しは [input.md](input.md)「送り直し」
- IME オフ・InputScope の欄・キー入力が無効な文書では、どのキーも食べない ([input.md](input.md))

## run のある状態のキー

| キー | run 中 | サジェスト選択中 (run のバー) | 候補選択中 |
|---|---|---|---|
| 印字キー (英字・記号・数字・テンキー) | 読みに足す ([input.md](input.md)「印字キー」) | 選んでいる候補を採用し、その打鍵から続ける | 確定し、その打鍵で次の run を始める (`digits half` の数字も) |
| Space | 読みとスペースを入れて run を終える ([input.md](input.md)「run の終わり方」) | 採用してスペースを入れる ([suggest.md](suggest.md)「採用の操作」) | 既定の割当では次候補、Shift+Space は前候補。割当が無ければ確定してスペースを入れる。モードレスの英字モード中は割当によらず確定して半角スペースを入れる |
| Enter・Ctrl+M | 見えているとおりに run を終えて渡す | 採用して run を終えて渡す | 確定して渡す |
| 確定キー | 見えているとおりに run を終える | 採用する (採用範囲を縮めた全体変換は run を続ける) | 確定する |
| Esc | run を捨てる ([input.md](input.md)「読みの編集」) | 選択を外す | 変換を取り消す |
| Backspace・Ctrl+H | 読みを 1 字削る ([input.md](input.md)「読みの編集」) | 読みを 1 字削る (選択は外れる) | 変換を取り消す |
| Tab / Shift+Tab | 候補バーがあれば次 / 前の候補を選ぶ。無ければ run を終えて渡す | 次 / 前の候補を選ぶ | 変換を取り消し、候補バーの先頭を選ぶ (Shift+Tab も同じ) |
| ↓ / ↑ | 候補バーがあれば次 / 前の候補を選ぶ。無ければ run を終えて渡す | 次 / 前の候補を選ぶ | 次候補 / 前候補 |
| ← / → | run を終えて渡す | 横一列のテーマなら前 / 次の候補を選ぶ。縦積みは run を終えて渡す | `segment_ui` 0: 範囲の伸縮 (伸縮しない候補選択では確定して渡す)。`segment_ui` 1: 文節の移動 |
| Shift+← / Shift+→ | run を終えて渡す | 採用範囲を 1 文節縮める / 伸ばす ([suggest.md](suggest.md)「採用範囲の縮め」) | `segment_ui` 0: 確定して渡す。`segment_ui` 1: 今の文節の伸縮 |
| PgUp / PgDn | run を終えて渡す | run を終えて渡す | `segment_ui` 0: 確定して渡す。`segment_ui` 1: 先頭 / 末尾の文節へ |
| Home・End・Delete・Insert | run を終えて渡す | run を終えて渡す | 確定して渡す |
| Ctrl+1〜9 | 候補バーがあれば、その番号の候補を採用する。無ければ run を終えて渡す | その番号の候補を採用する | その番号の候補で確定する |
| 変換キー | 変換を始める | 変換を始める | 次候補 (Shift+変換キーは前候補) |
| F4〜F10 | その変換を始める | その変換を始める | 今の候補選択にかける ([conversion.md](conversion.md)「ファンクションキー変換 (F4〜F10)」) |
| `ForgetLearning` (Ctrl+Delete) | run を終えて渡す | 選んでいる候補の学習を消す | 確定して渡す |
| 改行キー (Ctrl+J。対象のアプリ) | 見えているとおりに run を終え、Shift+Enter を送る | 採用して run を終え、Shift+Enter を送る | 確定し、Shift+Enter を送る |
| 割当の無い Ctrl 併用 (入力なしの機能・後続語の予測の選択の割当、対象外のアプリの改行キーを含む) | run を終えて渡す | run を終えて渡す | 確定して渡す |
| Alt 併用 (Ctrl+Alt を含む) | run を続けたまま渡す | run を続けたまま渡す | 確定して渡す |
| その他のキー (割当の無いファンクションキーなど) | run を続けたまま渡す | run を続けたまま渡す | 候補選択を続けたまま渡す |

- 「run を終えて渡す」は、見えているとおりの読みを文書に入れて run を終えてから、キーをアプリへ渡す ([input.md](input.md)
  「run の終わり方」)。サジェスト選択中でも、選んでいる候補を採用しない (Enter・Ctrl+M・改行キーを除く)。読めると判定されて
  いない文書では、食べて run を終えてから送り直す ([input.md](input.md)「送り直し」)
- 「確定して渡す」は、候補選択を確定してからキーをアプリへ渡す ([conversion.md](conversion.md)「確定と取消」)。食べて
  送り直すのは、読めると判定されていない文書の Enter・Ctrl+M と、改行キーだけ
- 候補バーの選択の動き (未選択からの前は何もしない、端での循環) は [suggest.md](suggest.md)「選択」。`candidate_bar` 0 では
  候補バーが無いので、run 中の Tab・↑ / ↓ は run を終えて渡す
- 候補選択中の Tab・↑ / ↓ は、使い道が無いときも食べる (アプリにキャレットやフォーカスを動かさせないため)
- ← / → が範囲の伸縮・文節の移動になる条件と、伸縮できない端でキーを食べることは [conversion.md](conversion.md)「範囲の伸縮」
  「文節 UI (`segment_ui` 1)」
- キャレット行の上に反転して出した縦積みの入力パネルでは、↑ / ↓ を見た目の向きに合わせて入れ替える
  ([display.md](display.md)「キャレット行の上に出すときの反転」)
- Alt 併用で run を終えない理由は [input.md](input.md)「run の終わり方」
- 確定キーと候補バーの Ctrl+1〜9 で run を終えたときは、後続語の予測を出す ([suggest.md](suggest.md)「出すとき・消すとき」)

## 数字キーと Ctrl+1〜9

### 数字キー

- 上段の数字キー (修飾なし) は、どの状態でもほかの印字キーと同じに扱い、候補の番号の選択には使わない。番号での採用・確定は
  Ctrl+1〜9 で行う
- 数字の打鍵が状態によって入力と選択に分かれず、選んだ候補を入れるのに 2 打鍵かからないようにするため
- 採らなかった案
  - 数字キーを、候補バーで選んでいるときだけ「選んだ候補の採用」にする: 数字の打鍵が状態で入力と採用に分かれたままで、
    選んでいないときは番号で採用できない
  - 設定で切り替えられるようにする: 選ぶだけの操作は Tab / ↑ / ↓ で残るので設けない

### Ctrl+1〜9

上段の数字 1〜9 に Ctrl を足した打鍵 (Shift・Alt なし) は、候補を出している入力パネルがあるとき、今出しているページ内の
その番号の候補を採用・確定する (tsf/src/text_service_direct.cpp `PanelNumberKey`)。

| 入力パネル | 働き |
|---|---|
| run の候補バー (選んでいるかによらない) | その番号の候補を確定キーと同じく採用する ([suggest.md](suggest.md)「採用の操作」) |
| 後続語の予測 (選んでいるかによらない) | その番号の予測を確定キーと同じく入れ、次の語の予測を出す ([suggest.md](suggest.md)「選択と採用」) |
| 変換の候補一覧 | 今の文節 (入力全体の候補選択では入力全体) をその番号の候補にして、全体を確定する。アプリへキーは渡さず、後続語の予測も出さない |

- 入力パネルが出ている間は、ページ内に無い番号でも食べて何もしない。アプリのショートカット (ブラウザのタブの切り替えなど)
  より入力パネルの操作を優先する。後続語の予測は確定の直後に出るので、出ている間はアプリの Ctrl+数字が効かない
- 入力パネルが無いときは割当の無い Ctrl 併用と同じ (run 中は run を終えて渡し、入力なしはアプリへ渡す)
- Ctrl+0・テンキーの Ctrl+数字・Ctrl+Shift+数字は対象外で、機能に割り当てられる
- 割当の機能にせず固定にし、Ctrl+1〜9 (Shift なし) は割り当てられないキーにする。割当を許すと、割り当てた番号の候補だけが
  採用できなくなるため
- 採らなかった案: 後続語の予測では、選んでいるときだけ Ctrl+数字を食べる。アプリのショートカットとの衝突は避けられるが、
  入力パネルが出ている間は IME が食べる、という規則のほうが分かりやすい

## 設定ファイルの書式

キー割当は設定ファイル (config.tsv) の `key.` で始まる行に書く (tsf/src/config.cpp `ApplyKeyLine`、engine/gui/src/keys.rs)。

```
key.convert<TAB>Convert,Ctrl+Space
key.commit_run@candidate<TAB>Ctrl+Enter
key.next_candidate@candidate<TAB>none
```

- 基本の割当は `key.<機能>` の行、状態別の上書きは `key.<機能>@<状態>` の行 (状態は上記「状態」の設定上の名前)。値はキーの
  カンマ区切りか `none` (割当なし)
- 上書きは、その機能が働く状態にだけ書ける。働かない状態の上書きの行は捨てる
- 上書きした状態では基本の割当を使わない。上書きの値が `none` なら、その状態では割当なし
- キーの表記は、修飾キー `Ctrl+`・`Shift+`・`Alt+` を前に付けてキー名を続ける。修飾キーの順は問わない。同じ修飾キーを 2 度
  書いた表記と、ほかの名前の修飾キーは読めない。設定ツールは `Ctrl+`・`Shift+` の順で書き出す

| キー名 | キー |
|---|---|
| `A`〜`Z`、`0`〜`9`、`F1`〜`F24` | 英字・上段の数字・ファンクションキー |
| `Space`・`Enter`・`Esc`・`Tab`・`Backspace`・`Delete`・`Insert`・`Home`・`End`・`PageUp`・`PageDown`・`Up`・`Down`・`Left`・`Right` | 名前のとおり |
| `Convert`・`NonConvert`・`Kana` | 変換・無変換・カタカナ/ひらがな |
| `VK_xx` | 上記以外。`xx` は 16 進 2 桁の仮想キーコード (`VK_00` は読めない) |

- 各キーの前後の半角スペースは除く。読めないキー・割り当てられないキー (上記「割り当てられないキー」)・重複したキーは、
  そのキーだけを捨てる
- 1 つも残らない値の行は捨てる (基本の割当は既定のまま、上書きは無いまま)。`none` とは区別する
- 同じ設定キーの行が複数あれば、後の行が勝つ
- 既定の割当は上記「機能」の表
