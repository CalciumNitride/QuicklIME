# 仕様書と設計書の分離

## 背景と目的

設計書 (docs/design/*.md) は変更ごとの差分として書かれ、後の設計書が前の設計書の一部を置き換えても、
前の設計書の本文は古いまま残る。今の仕様を知るには索引の「〜で置き換え」の連鎖をたどる必要があり
(例: candidate-bar.md → nbest.md → suggest-panel.md → unified-suggest.md → suggest-paging.md)、
1 本の設計書だけを読むと古い仕様を今の仕様と取り違える。

実際に、conversion-range.md の「読める文書では run の先頭より左にも広げられる」は
input-buffer-modes.md で「composition の中だけ」に置き換わっているが、conversion-range.md の本文と
実装 (tsf/src/text_service_direct.cpp の `ResizeConversionRange`) が食い違ったまま残っている。

今の仕様を機能の領域ごとの仕様書に一本化し、コードと同じ変更で書き換える。設計書は実装前の合意の
置き場所に戻し、実装が終わって内容を仕様書へ移したら archive へ移す。

## 決定事項

### 2 層の役割

| 層 | 置き場所 | 中身 | 更新のタイミング |
|---|---|---|---|
| 仕様書 | docs/spec/ | 今の挙動と、コードから復元できない WHY・採らなかった案の要点 | コードの変更と同じ commit で書き換える |
| 設計書 | docs/design/ | 実装前の合意 (背景・決定事項・変更対象・手順・検証・未決事項) | 実装中に設計が変わったときだけ書き換える |

- 仕様書は最新のスナップショットだけを書く。完了日・「当初は〜」「〜を廃止した」などの経緯、
  設計書の名前による置き換えの注記は書かない
- 採らなかった案は、仕様書に 1〜2 行の要点 (案と採らない理由) を残す。同じ案を後で検討し直さないため。
  詳しい比較が要るときは archive の設計書へリンクする
- 利用者向けの操作の説明は今どおり README.md・docs/user/ に書く。仕様書は開発者向けで、操作を
  重複して説明せず、内部の仕様と WHY を書く。挙動を変えたら仕様書と docs/user/ の両方を同じ commit で直す
- docs/protocol.md (プロトコル) と docs/architecture.md (全体の構成) は今も最新のスナップショットなので、
  場所を変えずにそのまま使う。docs/spec/README.md からリンクする
- キーの状態別の挙動の表は docs/spec/keys.md だけに置く。各仕様書は機能の中身を書き、キーの表は
  keys.md を参照する (状態別の表が複数の仕様書に分かれると、それ自体が食い違うため)

### 仕様書の区分

| ファイル | 領域 | 主な移行元 |
|---|---|---|
| README.md | 仕様書の索引。protocol.md・architecture.md・docs/user/ へのリンク | docs/design/README.md |
| input.md | 入力方式 (run・`input_buffer`・読める文書の判定・確定と取消・確定アンドゥ・改行キー・ローマ字テーブル・InputScope・パスワード欄・起動時の IME の状態) | input-model-v2, input-buffer-modes, panel-buffer, newline-key, ime-start-state, input-scope, basics |
| keys.md | キー割当 (状態・機能・割当の照合・状態別のコア操作の表・設定ファイルの書式) | keymap, number-key-adopt, basics |
| conversion.md | 変換キーの候補選択 (変換の対象・`segment_ui`・範囲の伸縮・文節 UI・後置再変換・採用済みの記憶・F4〜F10・数字の書き方・生ローマ字・記号・対記号) | nbest, conversion-range, adopted-reconversion, selection-reconversion, typo-segment-ui, number-forms, basics |
| engine.md | 変換エンジン (Viterbi・文節の分け方・N-best・タイプミス補正・LLM の並べ替えと生成・評価セット) | nbest, typo-conversion, llm-rerank, llm-generate, eval-sets, basics |
| dictionary.md | 辞書 (システム辞書・語彙補強・取り込み・ユーザ辞書・単語登録ツール) | dict-expansion, dict-import, basics |
| suggest.md | 候補バーと予測 (SUGGEST・ファジィ照合・予測・後続語予測・文節の連なりの統計・ページ送り・採用範囲の縮め) | candidate-bar, unified-suggest, suggest-paging, next-word-prediction, next-word-ngram, basics |
| learning.md | 学習 (語・句・文脈の句の学習・点数・学習を消す・学習しないアプリ) | learning-score, forget-learning, no-learning-apps, basics |
| modeless.md | モードレス入力と英字 (判定・英字区間・英字混じりの変換・英字候補・大文字小文字の補正) | modeless, modeless-detection, modeless-commit-candidate, mixed-conversion, english-case |
| display.md | 表示 (画面の部品の呼び名・入力パネル・テーマ・ぼかし・表示位置・キャレットの印・入力モードのアイコンとポップアップ・言語バー) | suggest-panel, panel-theme, theme-blur, conversion-panel, panel-above-caret, caret-marker, mode-icon, mode-popup, basics |
| tools.md | 設定ファイルと設定ツール (config.tsv・画面の構成・アプリごとの設定・入力パネルの見本) | slint-gui, config-tabs, app-settings, config-input-display, panel-preview, basics |
| runtime.md | 実行と配布 (エンジンの自動起動・二重起動の防止・パイプのアクセス権・表示位置のフォールバック・インストーラ・第三者の著作権表示) | engine-autostart, candidate-position-fallback, third-party-notices, basics |

- 実験の設計書 (experiment-*.md) と、docs の構成の設計書 (docs-reorganization.md・user-docs.md・本書) は
  仕様書へ移さない。実験の結果で今の仕様の WHY になる数値は、該当する仕様書に要点を書いてリンクする
- 区分に収まらない機能が出てきたら、仕様書を足す前に既存の区分に入るかを先に検討する

### 移行の進め方

一度に全部は移さない。ある領域の機能に手を入れるとき、または食い違いを見つけたときに、その領域の
仕様書を作る。

- 仕様書は設計書の写しにしない。コード・docs/user/・後の設計書と照らし、今の挙動を書く。コードを読んでも
  今の挙動が確定できない項目は、推測で書かずに報告する
- 移行元の設計書は、内容が全部仕様書に移ったら docs/archive/ へ移す
- 一部の節だけを移した設計書 (複数の領域にまたがる nbest.md など) は、移した節の本文を
  「docs/spec/<ファイル>.md「<見出し>」へ移した」という 1 行に置き換える。古い本文を残すと、
  今回と同じ取り違えが起きるため。残りの節は、その領域の仕様書を作るときに移す
- basics.md も同じ扱いにする。節を移したら 1 行に置き換え、全部移したら archive へ移す
- 移行が終わるまで、docs/design/README.md (設計書の索引) は今の形で残す。移した設計書の行は
  仕様書へのリンクに書き換え、archive へ移した設計書は「archive」の節へ移す
- 仕様書をまだ作っていない領域は、今どおり設計書と索引を見る

### 新しい変更の流れ

1. 設計書を書いて合意する (今どおり)。「変更対象ファイル」に、書き換える仕様書と節を含める
2. 実装と同じ commit で仕様書を書き換える。実装を implementer に委譲するときは、仕様書の書き換えも
   委譲の範囲に含める
3. 実装後、設計書の内容が仕様書に移ったことを確かめて、設計書を docs/archive/ へ移す
   (同じ commit でよい)
- 仕様書をまだ作っていない領域の変更では、その領域の仕様書を作ってから変更を書き込む
  (上の「移行の進め方」)。仕様書を作らずに設計書だけを足すことはしない
- 設計書の要らない小さな変更 (basics.md に書いていた類) は、該当する仕様書に直接書く

### 運用ルール (.claude/CLAUDE.md)

「設計書とロードマップ」の節を次の内容に書き換える。

- 今の仕様は docs/spec/ の仕様書に書き、コードの変更と同じ commit で書き換える。経緯は書かない
- 設計書 (docs/design/) は実装前の合意の置き場所。「変更対象ファイル」に書き換える仕様書を含め、
  実装後に内容を仕様書へ移して docs/archive/ へ移す
- 仕様書の無い領域を変えるときは、先にその領域の仕様書を作る (本書の「移行の進め方」)
- docs/roadmap.md の扱いは今どおり
- 「機能を足したら docs/design/README.md に 1 行足す」「basics.md に書く」は、移行中は仕様書の
  無い領域にだけ適用する

「画面の部品の呼び名」の参照先 (basics.md) は、display.md を作るまで今のままにする。

### 最初に移行する領域

conversion.md (変換キーの候補選択) を最初に作る。食い違いが見つかった領域で、移行元が
conversion-range.md・adopted-reconversion.md・selection-reconversion.md と、nbest.md・
typo-segment-ui.md・number-forms.md・basics.md の一部にまとまっている。

- engine.md はまだ作らないので、nbest.md の N-best の生成・候補の並び、typo-segment-ui.md の補正の求め方、
  selection-reconversion.md の逆変換のエンジン側などのエンジン側の節は移さずに残す
- keys.md もまだ作らないので、conversion.md にはキーの状態別の表を書かず、候補選択中のキーの挙動は
  機能の説明の中に書く。keys.md を作るときに表へ移す

### 採らなかった案

- 設計書を常に最新に書き換え続ける: 設計書が実装前の合意の記録でなくなり、同じ機能を複数の設計書が
  書いている状態は変わらないため
- 古い設計書の冒頭に「この節は〜で置き換え」の注記を足すだけにする: 置き換えの判断を人が毎回
  正しく行う必要があり、注記の漏れが今回の取り違えと同じ形で残るため
- 全部の設計書を一度に移す: 約 1 万 1 千行あり、コードとの照合を一度に終えられないため

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| docs/spec/README.md | 新設 (仕様書の索引。区分の表と、まだ作っていない仕様書は「未作成」と書く) |
| docs/spec/conversion.md | 新設 |
| docs/design/conversion-range.md, adopted-reconversion.md, selection-reconversion.md | 全部移したら docs/archive/ へ移す。エンジン側の節が残るものは、移した節だけ 1 行に置き換える |
| docs/design/nbest.md, typo-segment-ui.md, number-forms.md, basics.md | 移した節を 1 行に置き換える |
| docs/design/README.md | 冒頭に仕様書 (docs/spec/) への案内を足す。移した設計書の行を書き換え、archive へ移したものは archive の節へ移す。本書を「ドキュメント」の節に足す |
| docs/archive/ | 上の設計書の移動先 |
| .claude/CLAUDE.md | 「設計書とロードマップ」の節を書き換える |
| docs/architecture.md, README.md | docs/ の説明に docs/spec/ を足す (該当の記述があれば) |

## 実装手順

1. docs/spec/README.md を作る
2. docs/spec/conversion.md を、移行元の設計書・basics.md・docs/user/input.md とコード
   (tsf/src/text_service.cpp・text_service_direct.cpp ほか) を照らして書く。設計書とコードが食い違う箇所は
   コードの挙動で書き、食い違いの一覧を報告する
3. 移行元の設計書の移した節を 1 行に置き換え、全部移したものを docs/archive/ へ移す (`git mv`)
4. docs/design/README.md を更新する
5. .claude/CLAUDE.md と docs/architecture.md・README.md を更新する

## 検証方法

- 移行元の各設計書の決定事項が、conversion.md に対応するか、意図して残した節 (エンジン側) か、
  今のコードに無いものとして報告されているかのどれかであること。対応表を報告に含める
- conversion-range.md の範囲の伸縮が、今の実装どおり「composition の中だけ (外したかなを戻すところまで)」で
  書かれていること
- conversion.md に経緯・設計書名による置き換えの注記が無いこと
- archive へ移したファイルを指す相対リンク (docs/ 全体と README.md) が新しい場所を指していること
- 新設・書き換えたファイル内の相対リンクのリンク先が存在すること
- ファイルに絵文字を使っていないこと

## 未決事項

- ドキュメントの機械的なチェック (docs/user/settings.md の設定キーと config.cpp の照合、仕様書に書いた
  識別子がコードにあるかの確認) を作るか。作る場合は別の設計書にする
