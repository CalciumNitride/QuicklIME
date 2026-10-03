# 直接入力方式 (composition を使わない入力と後置変換)

## 背景と目的

現在の TSF 層は、打鍵した文字を composition (下線付き未確定文字列) として保持し、
Enter で確定してはじめてアプリの文書に入る。この方式には2つの問題がある。

- Web 上の一部フォームで未確定文字列の挙動が安定しない。具体例:
  NEXON JAPAN の二次パスワード入力フォーム、半角前提のコード入力フォーム
  (全角で入ると挙動が崩れる)、電話番号・郵便番号など文字数固定や自動で次欄へ
  移動するフォーム
- ライブ変換で表示された結果を直す操作は、感覚的には「確定済みテキストの再変換」と
  同じなのに、composition の内側と外側で別の操作になっている

そこで、打鍵した文字を composition ではなく文書に直接入れ、IME が「自分が入れた
文字列 (surface) とその読み」を覚えておき、後から変換キーで置き換える方式
(後置変換) に切り替える。ひらがなIME (Esrille) と同じ考え方。

この方式を「直接入力方式 (direct)」、従来を「composition 方式 (composition)」と呼ぶ。
モードレス入力 (docs/design/modeless.md) はこの方式の上に載せる。

## 決定事項

### 方式の選択と切替

- 設定 `input_style` (値 `direct` / `composition`) で切り替える。既定は当面 `composition`
  とし、常用して安定したら `direct` に変える
- composition 方式のコードは削除せず残す。direct 方式で文書の読み取り・置換が
  できないアプリ (CUAS 経由の古いアプリ、ターミナルなど) では、その文書に限って
  自動的に composition 方式で動く (後述のフォールバック)
- 打鍵中は composition を使わず、変換中 (候補選択中) だけ composition を張る
  (後述「変換中の composition」)。フォームが崩れる主因は打鍵ごとの composition 更新で、
  変換中に限れば影響は小さい。一方、文書の選択による文節強調は Chromium 系で
  保てない (文字列を変えた後にキャレットが挿入末尾へ戻される) ため、変換中の強調は
  composition の表示属性で行う

### run モデル

direct 方式では、IME が以下をひとまとめにした「run」を1つだけ覚えておく。

| 項目 | 内容 |
|---|---|
| 読み | `composer_` (RomajiComposer)。既存のまま |
| surface | 現在文書に入っている、この run 由来の文字列 |
| 変換状態 | `segments_` / `selected_` / `segmentIndex_` / `converting_` (既存を流用) |
| ライブ変換状態 | `liveSegments_` (既存を流用) |
| 予測状態 | `predictions_` / `predictionIndex_` (既存を流用) |

run 中の毎打鍵は「キャレット直前の surface を読み取り、記憶と一致すれば新しい表示に
SetText で置き換える」1回の edit session で行う。一致しない場合 (キャレット移動・
アプリ側の編集・別欄への自動移動) は run を捨て、その打鍵を新しい run の先頭として
挿入する。この毎回の検証があるため、キャレット移動の監視 (ITfTextEditSink) は行わない。

run の終了 (= IME が忘れる) の契機:

- Enter (食べずにアプリへ渡す)、Esc、Space (スペース挿入後)、矢印・Tab・Home/End
  など編集キー (食べずにアプリへ渡す)
- フォーカス移動 (OnSetFocus)、IME オフ
- 置換時の検証不一致
- 候補選択中に印字キーが来たとき (選択を確定して新しい run を始める)

run 終了時に行うこと (既存の確定処理と同じ):

- ライブ変換結果・候補選択結果を表示中なら文節ごとに LEARN を送る
- 文脈補正用の文脈 (`SetCommitContext`) を更新する
- 確定アンドゥ用に `lastCommitText_` / `lastComposer_` を記憶する

### キーの意味 (direct 方式、IME オン時)

Space は常にスペース、変換は専用の変換キーで行う。

| キー | 状態 | 挙動 |
|---|---|---|
| 英字・記号 | run なし | 新しい run を開始して挿入 (選択があればそれを置換する。通常の入力と同じ) |
| 数字・テンキー | run なし、設定 digits=half | 食べない (アプリに素のキーを渡す)。電話番号・郵便番号など自動で次欄へ移動するフォームは keydown/keyup/input をそのまま受け取る必要があるため。run は始まらない (数字で始まる run の助数詞サジェストは諦める) |
| 数字・テンキー | run なし、設定 digits=full | 新しい run を開始して全角で挿入 |
| 数字・テンキー | run 中 | 読みに追加し surface を置換 |
| 同上 | run 中 (かな表示・ライブ表示) | 読みに追加し surface を置換 |
| 同上 | 候補選択中・サジェスト選択中 | 選択を確定して run 終了、新しい run を開始 |
| Space | run なし | スペース挿入 (設定 space に従う)。食べずに通す場合も既存どおり |
| Space | run 中 | run 終了 (未変換ローマ字は `Commit()` の救済で付ける) + スペース挿入 |
| Space | 候補選択中 | 次候補 (変換中は composition 方式の経路で動くため、composition 方式と同じ) |
| 変換キー | run 中 (かな表示・ライブ表示) | 変換開始 (候補選択へ) |
| 変換キー | 候補選択中 | 次候補。Shift+変換キー は前候補 |
| 変換キー | run なし、ひらがな・カタカナ・ー のみの選択あり | 選択文字列を読みとして run を作り変換開始 (後置再変換) |
| 変換キー | run なし、選択なし | 何もしない (食べない) |
| Enter | 候補選択中・サジェスト選択中 | 選択を確定して run 終了 (食べる) |
| Enter | それ以外 | 食べない (アプリで改行)。run 終了 |
| Esc | 候補選択中 | 変換前の表示 (かな or ライブ表示) に戻す |
| Esc | サジェスト選択中 | 選択解除 (既存どおり) |
| Esc | run 中 (上記以外) | run 終了。文字は消さない (食べる) |
| Esc | run なし | 食べない |
| Backspace | run 中 (かな表示・ライブ表示) | 読みの末尾を削って surface を置換。空になれば run 終了 |
| Backspace | 候補選択中 | 変換前の表示に戻す (既存の CancelConversion 相当) |
| Backspace | run なし | 食べない |
| ↑↓、数字 1〜9、PgUp/PgDn、←→、Shift+←→ | 候補選択中 | 既存どおり (候補移動・番号選択・文節移動・伸縮) |
| Tab / ↑↓ | サジェスト表示中 | 既存どおり (サジェスト選択) |
| 矢印・Tab・PgUp/PgDn | run 中 (上記以外) | 食べない。run 終了 |
| F4〜F10 (機能キー) | run 中 | 既存どおり (未変換なら全文を1文節とする) |
| Ctrl+Backspace (確定アンドゥ) | run なし | 直前の run を復元する (surface を読み比べ、読みのかな表示に置き換えて run 再開) |
| Ctrl+Backspace | run 中 | run を終えて直ちに復元する = 表示を読みのかなに戻す (段階2 ではライブ変換・候補選択の結果をかなに戻す操作になる) |
| Ctrl+M / Ctrl+H | run 中 | Enter / Backspace の読み替え (既存どおり) |

composition 方式では Space による変換を従来どおり残す (既存の操作を壊さない)。
変換キーは composition 方式でも Space と同じ変換操作として使える。

### 変換キー

- 機能 `KeyFunc::Convert` を追加する。設定キーは `key.convert`、既定値は `Convert`
  (VK_CONVERT、JIS 配列の変換キー)
- 設定ツールで `Ctrl+Space` に変更できるようにする。KeyBinding は現状の
  `ctrl + vk` のままとし、値の表記は `Convert` または `Ctrl+Space` の2択
  (US 配列用の代替が Ctrl+Space)。将来 Shift 併用などを増やすときに KeyBinding を拡張する
- composition 方式では、変換キーの設定に関わらず Space による変換も従来どおり使える

### 変換中の composition

run が変換状態に入るとき、run の範囲に composition を張り (「昇格」)、候補選択中は
composition 方式の変換経路と表示属性 (入力中の下線・現在文節の強調) をそのまま使う。

- 昇格の契機: run 中の変換キー、run 中の F4〜F10 (`ApplyFunctionKey`。変換状態に入る
  キー)、後置再変換 (選択したかなを読みにした run の変換開始)
- 昇格の edit session (`PromoteRunEditSession`): `MatchRunRange` で run の範囲を
  特定・照合し (`surface_` / `surfaceCaret_`。後置再変換では選択開始が run 先頭)、
  その範囲で `StartComposition` し、入力中の表示属性を付けて末尾に潰す。文字列は変えない
- 昇格に成功したら run の文書上の状態 (`surface_` / `surfaceCaret_` /
  `surfaceSelectLength_`) を捨てて `promoted_` を立てる。`composer_`・ライブ変換の状態・
  文脈は composition にそのまま引き継ぐ (学習・確定アンドゥの記憶はしない)。
  以後 `UsingDirectStyle()` は composition が生きている間 false を返すので、
  キー判定・処理は composition 方式の経路に乗る。そこから既存の
  `StartConversion` / `ApplyFunctionKey` を呼ぶ
- 昇格の照合が不一致なら、今の不一致と同じく run を捨てる (文書は触らない)。
  `StartComposition` が失敗・拒否されたら、従来の direct 方式の変換
  (選択による文節強調。後述「置換 edit session」) にフォールバックする
- 確定 (Enter・候補番号・Ctrl+M など composition 方式の確定) は composition 方式の
  確定処理 (学習・文脈・確定アンドゥの記憶) をそのまま使う。確定後は composition が
  無いので direct 方式に戻る
- 降格: `promoted_` の composition が生きていて変換状態でなくなったとき (Esc・Backspace
  での変換取消、Tab でのサジェスト移行、候補選択中の印字キーで確定して新しい
  composition が始まったとき) は、composition をその表示文字列のまま終了し、
  その文字列を `surface_` (キャレットは末尾) とする direct 方式の run に戻す
  (`composer_`・ライブ変換の状態は維持、学習はしない)。判定は `HandleKey` の
  composition 経路の後処理1か所で行う
- composition が無くなったら (確定・アプリによる終了 `OnCompositionTerminated`)
  `promoted_` を下ろす
- 候補ウィンドウの位置は composition 方式と同じ (composition の矩形)

フォールバック時 (昇格できなかった文書) の変換は、**現在文節の範囲を選択状態にする**
ことで強調する。候補ウィンドウの位置は選択範囲の矩形 (`ITfContextView::GetTextExt`) の
直下 (取れなければキャレット/マウス位置)。

### 置換 edit session

文字列の置換と選択の設定は別々の edit session で行う。Chromium 系 (Chrome・Vivaldi) は
次の2つの制約を持つため (診断ログで確認):

- 文書の選択が潰れておらず、置換範囲がその選択と一致しない置換は正しく反映されない
  (旧文字列が残り、新しい文字列がその後ろに挿入される)。潰れたキャレットからの置換と、
  選択と同じ範囲の置換は正しく反映される
- 文字列を変えた edit session 内で設定した選択は捨てられ、キャレットは挿入した文字列の
  末尾に置かれる。文字列を変えない session での選択設定は反映される

そこで2つの edit session を用意する。どちらも冒頭で run の位置を特定して照合する。

照合 (両 session 共通): 入力は `expected` (現在の surface)、`caretOffset` (現在の
選択開始位置が surface の何文字目か)。`GetSelection` → 選択開始に潰し、
`ShiftEnd(+(expected.size() - caretOffset))`、`ShiftStart(-caretOffset)` で run 範囲を作り、
`GetText` で `expected` と比較する。一致しなければ何もせず「不一致」を返す。
`GetText` 自体が失敗したら「非対応」を返す。

- `ReplaceRunEditSession` (置換): 入力は照合用の値と `newText`。`expected` が空なら
  照合せずに選択位置へ挿入する (選択が非空なら選択を newText で置き換える)。
  `SetText(newText)` の後は常に末尾に潰す
- `SelectRunRangeEditSession` (選択のみ): 入力は照合用の値と `selectOffset` /
  `selectLength` (run 内の範囲。長さ 0 なら `selectOffset` の位置に潰す)。
  文字列は変えずに選択だけを設定する

`TextService` は、文書の選択が run 内のどこにあるか (`surfaceCaret_`) に加えて、
選択の長さ (`surfaceSelectLength_`) を持つ。表示の更新 (`ReplaceRunRange`) は次の順に行う:

1. 選択が run 末尾に潰れたキャレットでないとき (`surfaceCaret_ != surface_.size()` または
   `surfaceSelectLength_ > 0`。候補選択中・後置再変換の開始時など) は、
   `SelectRunRangeEditSession` で run 末尾に潰す。失敗は置換の失敗と同じ扱いにする
2. `ReplaceRunEditSession` で置換する (潰れたキャレットからの置換になる)
3. 置換後に選択する範囲があれば (`selectLength > 0`)、`SelectRunRangeEditSession` で
   その範囲を選択する。失敗しても置換は済んでいるので run は続け、選択は末尾に潰れた
   ものとして状態を持つ (強調が付かないだけ。不一致なら次の操作の手順 1 で検出される)

通常の打鍵では選択は常に末尾に潰れているので、手順 1・3 は走らず1 session のまま。
手順 1・3 が走るのは昇格できなかった文書での変換 (フォールバック) と、F6 などで
選択範囲を伴う表示更新をするときに限られる。Chromium 系では手順 3 の選択も後から
戻されうるが、その場合は次の操作の照合が不一致になり run を捨てる (文書は壊さない)。

候補選択中の確定 (`CommitRunDirect`) で選択を末尾に潰す処理も、同じ文字列での置換では
なく `SelectRunRangeEditSession` で行う。

後置再変換の開始時は、ユーザの選択 = run 全体なので `surfaceCaret_ = 0`、
`surfaceSelectLength_ = surface_.size()` とする (手順 1 で末尾に潰してから置換する)。

出力は3値: 成功 / 不一致 / 非対応。呼び出し側の対応:

- 不一致: run を捨てて、その打鍵を新規 run として挿入し直す (`expected` 空で再実行)
- 非対応: その文書を composition 方式にフォールバックする (下記)

矩形取得は別の `GetSelectionExtentEditSession` (現在の選択範囲の `GetTextExt`) で行う。
SetText 直後は同じロック内でレイアウトが更新されていないアプリがあるため、
既存の `GetTextExtentEditSession` と同様に session を分ける。

### フォールバック判定

- 判定は文書 (フォーカス中の ITfDocumentMgr) 単位。`directCapable_` を
  未判定 / 可 / 不可 の3値で持ち、`OnSetFocus(ITfDocumentMgr*)` で未判定に戻す
- CUAS 経由 (IMM32 アプリ。WezTerm など) の文書は、composition の外の文字列を
  同じロック内では読み戻せても次の打鍵では読めない (「ka」が「kあ」になる)。
  挿入直後の読み戻しでは判定できない。また `TS_SS_TRANSITORY` フラグは Chrome も
  立てる (Chromium は IMM32 互換の再変換経路を IME に使わせるため意図的に transitory
  を宣言している) ので、CUAS の判別には使えない
- そこで「選択位置の周りに読める文字があるか」で判定する。未判定の文書で run を
  始めるとき、置換 session の冒頭で選択を潰した範囲のクローンに `ShiftStart(-1)` と
  `ShiftEnd(+1)` を試し、どちらかが動けば周辺テキストが読める文書とみなして直接挿入する。
  どちらも動かない (文書が空、または CUAS のように composition の外に文字が無い) なら、
  この run だけ composition 方式で入力する (`directCapable_` は未判定のまま。composition が
  生きている間は既存の `UsingDirectStyle()` が false を返すので composition 経路に乗る)。
  composition が確定して文書に文字が入れば、次の run の判定で周辺テキストが読めて
  direct 方式に移る。CUAS では確定文字列が文書に残らないため毎回 composition 方式になる。
  代償は「空の入力欄での最初の run だけ下線付きになる」こと。CUAS で1文字残る
  制限は無くなる
- 直接挿入した文書は仮判定 (`Provisional`) とし、2打鍵目以降の置換が成功した時点で可にする。
  その置換で run 範囲が `ShiftStart` で作れない (shifted が 0) 場合は、周辺テキストが
  読めない文書とみなして不可にし、run を捨ててその打鍵から composition 方式にする
  (キャレット移動による通常の不一致は、範囲は作れて内容が違うので区別できる)
- 置換 session で `GetText` / `SetText` 自体が失敗したら不可にする (従来どおり)
- ターミナル (WezTerm) は既存コメントから CUAS 経由で composition の扱いに癖があると
  分かっているため、フォールバックの検証対象にする

### 確定アンドゥ

direct 方式では run 中の Backspace で読みを直せるため出番は減るが、run 終了後の
復元用に残す。`lastCommitText_` を `expected`、`lastComposer_.Display()` を `newText`
として `ReplaceRunEditSession` を実行し、成功したら run を再開する (composition は
開始しない)。復元後はこの run の間ライブ変換を止める (既存と同じ理由)。

### 既知のトレードオフ (受け入れる)

- アプリの Undo 履歴に置換が1打鍵ごとに積まれる
- 逐次検索するフォームでは、途中のかな状態でも input イベントが飛ぶ
- 選択で文節を強調するため、選択変更に反応するエディタ (Slack 等) で見え方が変わる可能性
- 漢字からの再変換 (読みの逆引き) は対象外。後置再変換は IME が読みを覚えている run と、
  ひらがな・カタカナのみの選択範囲に限る

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/config.h / config.cpp | `inputStyle` (direct/composition)、`KeyFunc::Convert` と `key.convert` のパース (`Convert` / `Ctrl+Space`) |
| tsf/src/edit_session.h / edit_session.cpp | `ReplaceRunEditSession`、`SelectRunRangeEditSession`、`GetSelectionExtentEditSession` を追加 |
| tsf/src/text_service.h | run 状態 (`surface_`、`directCapable_`、run 中フラグ)、direct 方式用メソッド宣言 |
| tsf/src/text_service_direct.cpp (新規) | direct 方式のキー処理 `HandleKeyDirect`、`IsKeyEatenDirect`、run の開始/更新/終了、変換・候補選択・サジェスト・ライブ変換の direct 版、後置再変換、フォールバック判定。TextService のメンバ関数を別翻訳単位に分けるだけで、クラスは増やさない |
| tsf/src/text_service.cpp | `IsKeyEaten` / `HandleKey` / `OnSetFocus` / `OnChange` (IME オフ) / `UndoCommit` で方式による分岐。`StartConversion` の候補生成部分 (エンジン問い合わせ + 生ローマ字候補 + 対記号同期) を表示から切り離して両方式で共用できるようにする |
| tsf/CMakeLists.txt | text_service_direct.cpp の追加 |
| engine/src/bin/quicklime-config.rs | 設定項目 `input_style` (ラジオまたはコンボ) と `key.convert` のコンボ (Convert / Ctrl+Space) を追加。全キー書き出しに含める |
| README.md / docs/roadmap.md | 方式の説明と設定項目の追記 (仕様のスナップショットのみ) |

エンジン (Rust) とプロトコルは変更しない。エンジン側の config.rs は未知キーを無視するため
`input_style` / `key.convert` の追加で壊れない。

## 実装手順

### 段階1: 直接入力の基盤 (変換なし)

1. config: `input_style` と `key.convert` の読み込み、既定値。設定ツールにも項目追加
2. edit_session: `ReplaceRunEditSession` (3値の結果)、`GetSelectionExtentEditSession`
3. text_service_direct.cpp: run の開始 (新規挿入 + 読み戻しによる判定)、かな入力・記号・
   数字の置換更新、Backspace、Space (run 終了 + スペース)、Enter/Esc/矢印/Tab の通過と
   run 終了、フォーカス移動・IME オフでの run 終了
4. `IsKeyEaten` / `HandleKey` の入口で `inputStyle` と `directCapable_` を見て分岐
5. フォールバック: 非対応時に composition 方式へ切り替える
6. 確定アンドゥの direct 版
7. 手動テスト (下記「段階1」)

この段階では変換キーは効かない (ひらがなが直接入るだけ)。ライブ変換 ON でも
かな表示にとどめる。

### 段階2: 変換・予測・ライブ変換の direct 版

1. `StartConversion` から候補生成部分を切り出し (`BuildConversionSegments` など)、
   composition 方式は従来どおり表示する
2. 変換キー: run 中の変換開始。候補選択中の表示更新は「surface を `ConvertedText()` で
   置換 + 現在文節を選択」で行う。`CycleCandidate` / `MoveSegment` / `ResizeSegment` /
   `SelectCandidateByNumber` / `DirectConvert` / `ConvertToSymbols` / `ConvertToShortcuts`
   の表示部分 (`UpdateConvertingDisplay`) を方式で分岐させ、状態操作は共用する
3. 候補ウィンドウの位置を選択範囲の矩形から取る
4. 候補選択中の Enter / Space / Esc / Backspace / 印字キー
5. run 終了時の LEARN 送信、文脈補正の更新、文節伸縮の学習 (既存の `PrepareConversionCommit`
   / `LearnResizedSegments` を流用)
6. サジェスト: 候補選択で surface を候補表記に置換、Enter で確定して run 終了
7. ライブ変換: 毎打鍵の置換文字列を `LiveText() + pending` にする。変換キーで
   候補選択へ (既存の StartConversion がライブ状態から変換へ移る処理と同じ)
8. 後置再変換: run なしで変換キー → `GetSelectionTextEditSession` で選択を読み、
   ひらがな・カタカナ・ー のみなら読みとして run を作って変換開始
9. composition 方式でも変換キーを Space と同じ変換操作 (変換開始・次候補) として
   食べる。Space による変換はそのまま残す
10. 手動テスト (下記「段階2」)

## 検証方法

ビルド: `cmake --build tsf/build --config Debug` と `cd engine && cargo build`。
反映は `scripts\dev-deploy.ps1 -Dll` (開発版へ切替)。テスト後は `-Restore` で戻す。
テストは新規に起動したメモ帳・ブラウザで行う (`taskkill /IM Notepad.exe /F` してから開く)。

設定ファイルは `QUICKLIME_CONFIG_FILE` で隔離し、`input_style\tdirect` を入れて試す。

### テストケース (段階1)

| # | 操作 | 期待 |
|---|---|---|
| 1 | メモ帳で `kyou` と打つ | 1打鍵ごとに `k` → `き` → `きy` → `きょ` → `きょう` と文書の文字が置き換わる。下線は付かない |
| 2 | 続けて Backspace ×2 | `きょ` → `き`。さらに Backspace で `き` が消え run 終了。もう1回 Backspace はアプリに渡る |
| 3 | `kyou` Space `ha` | `きょう　は` (space=full)。Space の後は新しい run |
| 4 | `kyou` Enter | 改行が入る (Enter はアプリに渡る)。`きょう` は残る |
| 5 | `kyou` Esc | 何も変わらない。続けて `ha` を打つと `きょうは` ではなく新 run として `は` が続く (見た目は同じ、Backspace で `は` だけ消える) |
| 6 | `kyou` の後にマウスで行頭へキャレット移動し `a` | 行頭に `あ` が入り、`きょう` は変わらない (不一致で run 捨て) |
| 7 | `kyou` の後に Ctrl+Backspace ... 段階1では run 終了後 (Space 後) に Ctrl+Backspace | 直前の run が読みのかな表示に戻り、Backspace で編集できる |
| 8 | 半角/全角で IME オフ → オン | run が終了している (オン後の打鍵は新 run) |
| 9 | Chrome で NEXON の二次パスワード欄、郵便番号自動移動欄、コード入力欄 | 数字が半角で1文字ずつ入り、自動移動後も崩れない。全角かなを入れた場合も composition 方式より挙動が素直 |
| 10 | WezTerm (CUAS) | フォールバックして composition 方式で入力できる。最初の1文字が残る場合はその旨を記録する |
| 11 | Word / Excel のセル | 置換が効くこと。効かなければフォールバックが働くこと |
| 12 | `input_style\tcomposition` | 従来どおりの挙動 (回帰) |

### テストケース (段階2)

| # | 操作 | 期待 |
|---|---|---|
| 1 | `kyouhaiitenki` 変換キー | `今日はいい天気` に置き換わり、先頭文節 `今日は` が選択状態、候補ウィンドウがその直下に出る |
| 2 | 続けて 変換キー ×2、Shift+変換キー | 候補が進み、戻る |
| 3 | → で文節移動、Shift+→ で伸縮 | 選択範囲が該当文節に移る。伸縮で再変換される |
| 4 | Enter | 選択が末尾に潰れ、run 終了。改行は入らない。LEARN が送られる (エンジンログか learning.tsv で確認) |
| 5 | 候補選択中に Space | 次候補 (composition 方式と同じ) |
| 6 | 候補選択中に Esc | 変換前のかな表示に戻る (run は続く) |
| 7 | 候補選択中に `a` | 確定して `あ` の新 run が続く |
| 8 | ライブ変換 ON で `kyouhaiitenki` | 打鍵ごとに `今日はいい天気` 側の表示に置き換わる。Space でスペースが入り run 終了 |
| 9 | ライブ変換 ON で打った後、変換キー | 候補選択に入り直せる |
| 10 | `きょう` と打って Space で run を終えた後、`きょう` をマウスで選択して変換キー | `今日` などの候補選択に入る (後置再変換)。漢字を選択して変換キーは何も起きない |
| 11 | `ky` (2文字以上のかな) でサジェストが出て Tab | surface がサジェスト候補に置き換わる。Enter で確定、Esc で戻る |
| 12 | 候補選択中に F6〜F10、F4、F5 | 既存どおりの直接変換・特殊変換が選択文節に効く |
| 13 | 英字を含む入力 `apple` 変換キー | 生ローマ字候補 apple / Apple / APPLE が出る |
| 14 | 変換キーを `Ctrl+Space` に設定 | Ctrl+Space で変換開始・次候補 |
| 15 | Chrome / Vivaldi の textarea で `kyouhaiitenki` 変換キー ×4、数字キー、→、F7、Enter | 変換キーで下線付きの composition になり現在文節が強調される。候補送り・文節移動・F7 が効き、文字列が追加されない。Enter で確定し、続く打鍵は下線なし (direct) |
| 16 | Chrome / Vivaldi でかなを選択して変換キー ×3、→、Enter | 同上 (後置再変換から composition に昇格) |
| 17 | Chrome / Vivaldi で `kyou` F6 → Enter | `きょう` の composition になり、Enter で確定できる |
| 18 | 候補選択中に Esc / Backspace | 変換前のかな表示に戻り、下線が消えて direct の run に戻る (続けて打鍵・Backspace で読みを編集できる) |
| 19 | 候補選択中に `a` | 確定して `あ` が下線なしの direct の run として続く |
| 20 | 候補選択中に Tab (サジェストあり) | direct の run に戻ってサジェスト選択に移る |
| 21 | メモ帳で 15〜20 | 同じ挙動 (回帰確認) |

## 未決事項

- CUAS 経由のアプリを打鍵前に判定する確実な方法。現状は「読み戻し失敗で不可」の
  楽観判定なので、文書先頭で1文字残る制限がある。実機で問題が大きければ、
  `OnSetFocus` 時に読み取り専用 session で先行テキストの `GetText` を試す事前判定を追加する
- 選択による文節強調が崩れるエディタ (Slack、Notion、Google Docs) の実態。問題があれば
  候補ウィンドウ内に現在文節の読みを出す代替案を検討する
- `input_style` の既定を `direct` に切り替える時期
