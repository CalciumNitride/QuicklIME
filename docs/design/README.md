# 設計書の索引

機能ごとの設計書の一覧。各設計書は実装前に合意した設計で、後の設計書で一部または全部が置き換えられた
ものには、置き換えた設計書を併記する。全体の構成は [docs/architecture.md](../architecture.md)、
未着手の項目と既知の問題は [docs/roadmap.md](../roadmap.md) にある。

今の仕様は、領域ごとの仕様書 ([docs/spec/README.md](../spec/README.md)) に移している。仕様書のある領域は
仕様書を見る。仕様書へ移した節は、設計書の本文を仕様書への 1 行に置き換えてあり、全部移した設計書は
下の「archive」の節に並べる。仕様書と設計書の分担は [spec-docs.md](spec-docs.md)。

- [basics.md](basics.md): 設計書の無い基本機能 (辞書・学習・記号辞書・ユーザ辞書と単語登録・
  設定ファイルと設定ツール・エンジンの自動起動・インストーラ) の仕様と理由。
  数字の書き方の候補・対記号・生ローマ字の候補・ファンクションキー変換は
  [docs/spec/conversion.md](../spec/conversion.md) へ、予測・打鍵のファジィ照合・横並びのテーマでの
  サジェスト選択の ←→ は [docs/spec/suggest.md](../spec/suggest.md) へ、確定アンドゥ・読める文書の判定・
  ローマ字テーブル・パスワード欄・記号キーと Shift+英字で入る文字は [docs/spec/input.md](../spec/input.md) へ、
  キー割当 (`key.*`) は [docs/spec/keys.md](../spec/keys.md) へ、画面の部品の呼び名・言語バーは
  [docs/spec/display.md](../spec/display.md) へ、変換 (Viterbi と文節)・変換のタイプミス補正・日付時刻の候補は
  [docs/spec/engine.md](../spec/engine.md) へ移した
- [docs/protocol.md](../protocol.md): TSF 層とエンジンの間のプロトコル
- [docs/user/theme.md](../user/theme.md): 入力パネルのテーマファイルのリファレンス

## 入力方式

- 入力方式 (run・入力中の読みの表示 `input_buffer`・読める文書の判定・送り直し・確定アンドゥ・IME のオン/オフと
  起動時の状態・InputScope の欄・パスワード欄): [docs/spec/input.md](../spec/input.md) (設計書は下の「archive」の節)
- キー割当 (状態・機能・割り当てられないキー・割当の照合・状態ごとのキーの働き・数字キーと Ctrl+1〜9・設定ファイルの書式):
  [docs/spec/keys.md](../spec/keys.md) (設計書は下の「archive」の節)
- [newline-key.md](newline-key.md): 改行キー (既定 Ctrl+J)。対象アプリで Shift+Enter を送って改行を入れる。
  改行の入れ方・対象アプリ・機能は [docs/spec/input.md](../spec/input.md) へ移した (設定ツールの画面は本書)。
  設定ツールの「アプリ」タブは [app-settings.md](app-settings.md) で置き換え
- [no-learning-apps.md](no-learning-apps.md): 指定したアプリでは確定・採用の結果を学習に記録しない
- [forget-learning.md](forget-learning.md): 入力パネルで選んでいる候補の学習を Ctrl+Delete で消す。
  消す学習の種類とファイルは [learning-score.md](learning-score.md) で置き換え。キー割当は
  [docs/spec/keys.md](../spec/keys.md) へ移した

## 候補の表示

- 候補バー (バーの内容・表示条件・選択・採用・設定・エンジンへの問い合わせ): [docs/spec/suggest.md](../spec/suggest.md)
  (設計書は下の「archive」の節)
- 入力パネル (描き方・入力行・変換の候補一覧・表示位置・キャレット行の上に出すときの反転)・ぼかし・キャレットの印・
  入力モードの一時表示・入力モードのアイコンと言語バー: [docs/spec/display.md](../spec/display.md) (設計書は下の「archive」の節)
- [panel-theme.md](panel-theme.md): 入力パネルのテーマ (組み込みのカプセル・縦積み・ゴーストと、
  テーマファイル)。テーマ・縦積み・ゴースト・揃え・読み直しは [docs/spec/display.md](../spec/display.md) へ移した
  (設定ツールの画面は本書)
- 読みなしの後続語予測: [docs/spec/suggest.md](../spec/suggest.md) (設計書は下の「archive」の節)
- [next-word-ngram.md](next-word-ngram.md): 日本語版ウィキペディアから数えた文節の連なりの統計 (next_word.tsv) を、
  読みなしの後続語予測で学習の後ろに足す。コーパス・文節への分け方・表・生成・エンジンは
  [docs/spec/suggest.md](../spec/suggest.md) へ移した (同梱は本書)
- 候補バーのページ送り: [docs/spec/suggest.md](../spec/suggest.md) (設計書は下の「archive」の節)
- 打鍵のファジィ照合・SUGGEST の並び・採用範囲の縮め: [docs/spec/suggest.md](../spec/suggest.md)、全体変換の補正は
  [docs/spec/engine.md](../spec/engine.md) (設計書は下の「archive」の節)

## 変換

- 変換エンジン (ラティスと Viterbi・文節の分け方と文節の候補・入力全体の N-best と候補の並び・変換のタイプミス補正・
  逆変換・数字の書き方の候補の位置・LLM の並べ替えと生成・評価セット): [docs/spec/engine.md](../spec/engine.md)
  (設計書は下の「archive」の節)
- [llm-rerank.md](llm-rerank.md): LLM (zenz 系モデル、別プロセス quicklime-llm) による N-best の並べ替え。並べ替えの
  規則・モデルと尤度・左文脈・依頼と結果は [docs/spec/engine.md](../spec/engine.md) へ移した (子プロセスの構成・
  モデルの配布・子プロセスの起動と監視・ビルドは本書)
- run が無いときのキャレット直前のかなの変換と範囲の伸縮、採用した文字列の記憶からの再変換:
  [docs/spec/conversion.md](../spec/conversion.md) (設計書は下の「archive」の節)
- 漢字を含む選択の再変換と数字の書き方の候補: [docs/spec/conversion.md](../spec/conversion.md)、逆変換と候補の位置は
  [docs/spec/engine.md](../spec/engine.md) (設計書は下の「archive」の節)
- [learning-score.md](learning-score.md): 学習を使った度合いの点数にする。確定した文節を辞書の語に分けて
  語の学習を Viterbi の単語コストの割引に、句の学習を文節の候補・読み全体の学習表記・予測に、文脈の句の学習を
  前文脈付きの予測と後続語の予測に使う (learning_stats.tsv)。予測の点数と並び (D) は [docs/spec/suggest.md](../spec/suggest.md) へ移した

## モードレス入力

- [modeless.md](modeless.md): モードレス入力 (自動英字判定) の基本。run 全体を英字にする遷移と Backspace
  の扱いは [modeless-detection.md](modeless-detection.md) の区間分割と
  [modeless-commit-candidate.md](modeless-commit-candidate.md) で置き換え
- [modeless-detection.md](modeless-detection.md): 判定の強化 (ルール4〜7) と、英単語の部分だけを英字にする
  英字区間。英字区間を閉じて日本語に戻る動作は [mixed-conversion.md](mixed-conversion.md)、run 終了時の
  ルール3・6・7 の見せ方は [modeless-commit-candidate.md](modeless-commit-candidate.md)
- [modeless-commit-candidate.md](modeless-commit-candidate.md): run 終了時の英字判定を書き換えではなく
  候補バーの英字候補として出す。きっかけの打鍵の Backspace で判定前に戻す
- [mixed-conversion.md](mixed-conversion.md): 英字混じり文の変換。英字区間を閉じて日本語に戻り、日本語の
  部分だけを変換する。英単語の大文字小文字の補正は [english-case.md](english-case.md)
- [english-case.md](english-case.md): 英単語の大文字小文字の補正 (`github` → `GitHub`) を候補として出す

## 辞書

- [dict-expansion.md](dict-expansion.md): 辞書の語彙補強 (Mozc の単漢字辞書と、mozc-ut 由来の UT 辞書)
- [dict-import.md](dict-import.md): 他の IME の辞書テキストの取り込み

## 設定・ツール

- [config-tabs.md](config-tabs.md): 設定ツールのメイン画面のタブ化。画面の部分は [slint-gui.md](slint-gui.md) で
  置き換え
- [app-settings.md](app-settings.md): 設定ツールの「アプリ」タブを、アプリを軸にした1つの一覧とアプリごとの
  編集欄にする (改行キー・起動時の IME の状態)。画面の部分は [slint-gui.md](slint-gui.md) で置き換え
- [config-input-display.md](config-input-display.md): 設定ツールで入力中の読みの表示先 (`input_buffer`・
  `panel_input`) を「入力」タブの 1 つのコンボで選び、入力モード表示のチェックボックスの文言を短くする
- [slint-gui.md](slint-gui.md): 設定ツールと単語登録ツールの画面を Slint (fluent スタイル) で作り直す。左のナビと
  ページのカード、変えたら即保存、キー割当・アプリの展開カード、結果とエラーの InfoBar
- [panel-preview.md](panel-preview.md): 設定ツールの「見た目」ページに、選んでいるテーマ・フォント・サイズの
  入力パネルの見本を、登録されている TSF の DLL の描画で出す。DLL の関数と見本の内容は
  [docs/spec/display.md](../spec/display.md) へ移した (設定ツールの側は本書)

## エンジン

- [engine-autostart.md](engine-autostart.md): エンジンの早期起動。最初の変換より前に起動しておき、起動中の変換は
  準備ができるまで待つ。named mutex による二重起動の防止。
  管理者権限で起動しても通常権限のアプリから接続できるパイプのアクセス権

## 互換

- 入力位置がウィンドウの外を指すアプリ (マビノギなど) での入力パネルなどの位置: [docs/spec/display.md](../spec/display.md)
  (設計書は下の「archive」の節)

## 配布

- [third-party-notices.md](third-party-notices.md): 配る exe に静的リンクしている Rust の crate の著作権表示と
  許諾文を、インストーラのビルド時に cargo-about で `LICENSE-rust-crates.txt` に生成して同梱する

## ドキュメント

- [spec-docs.md](spec-docs.md): 仕様書 (docs/spec/) と設計書の分離。仕様書の区分と移行の進め方、新しい変更の流れ

## archive (docs/archive/)

仕様書へ全部移した設計書、実験の記録、置き換わったドキュメントの構成の設計書と、採らなかった案・検討段階の草案。

- [conversion-range.md](../archive/conversion-range.md): run が無いときのキャレット直前のかなの変換と、候補選択中の
  ←→ による変換の範囲の伸縮。[docs/spec/conversion.md](../spec/conversion.md) へ移した
- [adopted-reconversion.md](../archive/adopted-reconversion.md): 採用・確定した文字列を読みとともに覚え、漢字を含む
  部分も変換キーで変換し直す。[docs/spec/conversion.md](../spec/conversion.md) へ移した
- [next-word-prediction.md](../archive/next-word-prediction.md): 読みなしの後続語予測。確定の直後に次の語を候補バーに
  出し、予測候補の順位にも前文脈を使う。[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [suggest-paging.md](../archive/suggest-paging.md): 候補バー (run のサジェスト・後続語の予測) のページ送り。合計の
  上限を `max_predictions` (9 / 18 / 27 / 36) にする。[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [input-model-v2.md](../archive/input-model-v2.md): 入力モデル v2 の総論 (文書を自動で書き換えない・1キー1機能
  などの原則と段階の区切り)。今も成り立つ原則は [docs/spec/input.md](../spec/input.md) へ移した
- [direct-input.md](../archive/direct-input.md): 直接入力方式 (打鍵した文字を文書に直接入れ、run を覚えて変換キーで
  後置変換する)。[input-buffer-modes.md](../archive/input-buffer-modes.md) で廃止。今も成り立つ理由 (CUAS の文書の
  読み戻し・Chromium の置き換えの制約・半角数字の素通し) は [docs/spec/input.md](../spec/input.md) へ移した
- [append-input.md](../archive/append-input.md): 追記型入力 (未完成のローマ字を小窓に出し、かなを文書に追記する。
  読める文書と追記のみの文書・擬似 Backspace)。[input-buffer-modes.md](../archive/input-buffer-modes.md) で廃止
- [append-only-apps.md](../archive/append-only-apps.md): 指定したアプリ (既定 Discord.exe) を追記のみの文書として扱う。
  [input-buffer-modes.md](../archive/input-buffer-modes.md) で廃止
- [panel-buffer.md](../archive/panel-buffer.md): パネル保持入力 (設定 `input_buffer panel`)。
  [docs/spec/input.md](../spec/input.md) へ移した
- [input-buffer-modes.md](../archive/input-buffer-modes.md): 入力方式の選択 (設定 `input_buffer` の `composition` /
  `panel`)、2 方式で共通の run、送り直し、文書の分類。[docs/spec/input.md](../spec/input.md) へ移した
- [ime-start-state.md](../archive/ime-start-state.md): アプリの起動時の IME の状態 (オン・オフ・自動) をアプリごとに
  決める。[docs/spec/input.md](../spec/input.md) へ移した
- [keymap.md](../archive/keymap.md): キー割当を状態別の表にし、Enter をアプリへ渡すキーに分け、確定キーを設ける。
  [docs/spec/keys.md](../spec/keys.md) へ移した。設定ツールの画面は [slint-gui.md](slint-gui.md) で置き換え
- [number-key-adopt.md](../archive/number-key-adopt.md): 数字キーをどの状態でも文字キーと同じに扱い、番号での採用・確定を
  Ctrl+1〜9 にする (候補バー・後続語の予測・候補選択中)。[docs/spec/keys.md](../spec/keys.md) へ移した
- [input-scope.md](../archive/input-scope.md): 数字・電話番号・メールアドレス・URL などの InputScope の欄では、IME オンの
  ままキーを食べない (設定 `input_scope_passthrough`)。[docs/spec/input.md](../spec/input.md) へ移した
- [candidate-bar.md](../archive/candidate-bar.md): 候補バー。入力中に変換結果と予測候補を横一列に出し、採用時だけ
  文書を書き換える。[docs/spec/suggest.md](../spec/suggest.md) と [docs/spec/display.md](../spec/display.md) へ移した
- [suggest-panel.md](../archive/suggest-panel.md): 入力パネル。候補バーの見た目の刷新 (カプセル)、画面中央への
  表示、入力行、背景のぼかし。[docs/spec/display.md](../spec/display.md) へ移した
- [conversion-panel.md](../archive/conversion-panel.md): 変換キーの候補一覧を入力パネルで出す (テーマ・表示位置・
  変換中の入力行)。[docs/spec/display.md](../spec/display.md) へ移した
- [panel-above-caret.md](../archive/panel-above-caret.md): 縦に並ぶ入力パネルの上下を出しうる最大の高さで決め、
  キャレット行の上に出すときは並びを反転して入力行と各候補の位置を動かさない。[docs/spec/display.md](../spec/display.md) へ移した
- [caret-marker.md](../archive/caret-marker.md): 文書のキャレットの位置を印 (テーマの `caret_marker`。パネルの吹き出しの
  尻尾か、キャレットの下の三角) で示す。[docs/spec/display.md](../spec/display.md) へ移した
- [theme-blur.md](../archive/theme-blur.md): ぼかしをテーマのキー `blur` (`on` / `off`) だけで決め、組み込みテーマ
  `capsule-blur` を足す。[docs/spec/display.md](../spec/display.md) へ移した
- [mode-icon.md](../archive/mode-icon.md): タスクバーの入力モードのアイコン (あ・A・入力欄でないときの ×) を、同梱の
  M PLUS Rounded 1c で透明な背景に描く。[docs/spec/display.md](../spec/display.md) へ移した
- [mode-popup.md](../archive/mode-popup.md): オン/オフの切替時と別の入力欄へのフォーカス移動時に、キャレットの近くに
  入力モード (あ / A) を約 1 秒出す (設定 `mode_popup`)。[docs/spec/display.md](../spec/display.md) へ移した
- [candidate-position-fallback.md](../archive/candidate-position-fallback.md): 入力位置がウィンドウの外を指すアプリ
  (マビノギなど) で、入力パネルなどをウィンドウ中央に出す。[docs/spec/display.md](../spec/display.md) へ移した
- [nbest.md](../archive/nbest.md): 入力全体の N-best。候補バーと変換キーの候補に使い、文節 UI を設定 `segment_ui`
  のオプションにする。[docs/spec/engine.md](../spec/engine.md)・[docs/spec/conversion.md](../spec/conversion.md)・
  [docs/spec/suggest.md](../spec/suggest.md) へ移した
- [unified-suggest.md](../archive/unified-suggest.md): 打鍵をエンジンに送り、打鍵の上でずれを測るファジィ照合と、全体変換・
  予測・補正の候補を 1 つの点数で並べる SUGGEST。先頭文節の候補をなくし、選んだ全体変換の採用範囲を
  Shift+← / → で縮める。[docs/spec/suggest.md](../spec/suggest.md) と [docs/spec/engine.md](../spec/engine.md) へ移した
- [typo-conversion.md](../archive/typo-conversion.md): 入力全体の候補に、打ち間違いを 1 打直した読みで変換した補正の候補を
  足す (計測の結果を含む)。[docs/spec/engine.md](../spec/engine.md) と [docs/spec/display.md](../spec/display.md) へ移した
- [typo-segment-ui.md](../archive/typo-segment-ui.md): 文節 UI (`segment_ui` 1) の変換で、補正の最良経路に沿って文節を
  区切り、打ち間違えた語を含む文節の候補に補正の表記を足す。[docs/spec/engine.md](../spec/engine.md) と
  [docs/spec/conversion.md](../spec/conversion.md) へ移した
- [selection-reconversion.md](../archive/selection-reconversion.md): 漢字を含む選択範囲を、エンジンの逆変換 (REVERSE) で
  推測した読みから変換キー・F6〜F10 で変換し直す。[docs/spec/conversion.md](../spec/conversion.md) と
  [docs/spec/engine.md](../spec/engine.md) へ移した
- [number-forms.md](../archive/number-forms.md): 数字で打った数に、漢数字・桁区切り・万の単位まじり・全角/半角の候補を足す。
  [docs/spec/conversion.md](../spec/conversion.md) と [docs/spec/engine.md](../spec/engine.md) へ移した
- [llm-generate.md](../archive/llm-generate.md): Vulkan 版で、9 文字以上の読みの並べ替えに LLM の貪欲法で生成した候補を
  1 件足し、候補バーの全体変換の枠を並べ替えの結果の上位から埋め直す。[docs/spec/engine.md](../spec/engine.md) と
  [docs/spec/suggest.md](../spec/suggest.md) へ移した
- [eval-sets.md](../archive/eval-sets.md): 変換精度の評価セット (AJIMEE-Bench と、実ユーザの learning_stats.tsv の
  再生評価)。[docs/spec/engine.md](../spec/engine.md)「評価セット」へ移した

- [experiment-0b-llm.md](../archive/experiment-0b-llm.md): LLM による順位補正の遅延・メモリ・精度の計測の仕様
- [experiment-0b-llm-results.md](../archive/experiment-0b-llm-results.md): 上の計測の結果
- [experiment-0c-llm-gen.md](../archive/experiment-0c-llm-gen.md): LLM による候補の生成 (自由生成と、統計変換を
  ドラフトにした照合) の精度・速さ・副作用の計測の仕様
- [experiment-0c-llm-gen-results.md](../archive/experiment-0c-llm-gen-results.md): 上の計測の結果
- [docs-reorganization.md](../archive/docs-reorganization.md): ドキュメントの構成 (architecture・設計書の索引・
  basics・roadmap の分担)。[spec-docs.md](spec-docs.md) に置き換わった
- [user-docs.md](../archive/user-docs.md): 利用者向けドキュメントの分割 (README・docs/user/・docs/development.md・
  設計書の分担と、設定一覧 docs/user/settings.md)

- [experiment-0a-append-input.md](../archive/experiment-0a-append-input.md): 追記型入力の2方式 (F1 / F2)
  の試作と比較。F1 を [append-input.md](../archive/append-input.md) で採用
- [live-conversion-commit-notes.md](../archive/live-conversion-commit-notes.md): ライブ変換の結果を途中で
  確定する手段の検討。後続の打鍵で前方の変換が変わる問題は [docs/spec/suggest.md](../spec/suggest.md)「採用」で扱う
- [single-unit-conversion-notes.md](../archive/single-unit-conversion-notes.md): 連文節変換をやめ、文節を
  見せない変換にする案の検討
- [modeless-detection-notes.md](../archive/modeless-detection-notes.md): モードレス入力の英語判定の強化案と
  Meltype の調査。採った項目は [modeless-detection.md](modeless-detection.md)
- [llm-rerank-notes.md](../archive/llm-rerank-notes.md): Zenzai・mozkey の LLM 補正の調査。並べ替えは
  [llm-rerank.md](llm-rerank.md) で実装し、フィードバック学習は [roadmap](../roadmap.md) の未着手
- [rakukan-llm-notes.md](../archive/rakukan-llm-notes.md): rakukan の LLM 変換 (生成の前後処理・jinen・運用) の調査。
  生成は [llm-generate.md](../archive/llm-generate.md) で実装
