# archive の一覧

使い終わってもう参照しない文書の置き場所。仕様書へ内容を移し終えた設計書、実験の記録、置き換わった
ドキュメントの構成の設計書と、採らなかった案・検討段階の草案を置く。今の仕様は
[docs/spec/README.md](../spec/README.md) を見る。

- [fuzzy-level.md](fuzzy-level.md): ファジィ入力の段階 (−1〜3) を F2 / Shift+F2 で run ごとに変え、標準の段階を設定
  `fuzzy_level` にする。機能の名前をファジィ入力に改め、補正の語の枠を補正した読みごとにする。
  [docs/spec/engine.md](../spec/engine.md)・[docs/spec/suggest.md](../spec/suggest.md)・[docs/spec/keys.md](../spec/keys.md)・
  [docs/spec/conversion.md](../spec/conversion.md)・[docs/spec/display.md](../spec/display.md)・
  [docs/spec/tools.md](../spec/tools.md)・[docs/protocol.md](../protocol.md) へ移した
- [typo-tail.md](typo-tail.md): 補正で読みの末尾 1 文字を変えず、打ちかけの読みで補正が候補バーの先頭に出るのを
  減らす。[docs/spec/engine.md](../spec/engine.md)・[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [name-candidates.md](name-candidates.md): 変換の候補と予測でシステム辞書の人名を 3 件に抑え、辞書の候補の枠を
  40 件に広げる。人名だけを出す人名変換 (F3) を足す。[docs/spec/engine.md](../spec/engine.md)・
  [docs/spec/suggest.md](../spec/suggest.md)・[docs/spec/conversion.md](../spec/conversion.md)・
  [docs/spec/keys.md](../spec/keys.md)・[docs/spec/dictionary.md](../spec/dictionary.md) へ移した
- [panel-font-gdi-name.md](panel-font-gdi-name.md): 入力パネルのフォントを GDI の面名 (太さ違いの面名を含む) から
  DirectWrite のフォントへ解決し、設定ツールの選択肢からビットマップ・ベクタのフォントを外す。
  [docs/spec/display.md](../spec/display.md)・[docs/spec/tools.md](../spec/tools.md) へ移した
- [installer-language-list.md](installer-language-list.md): インストール時に、セットアップを起動した利用者の言語の
  一覧に QuicklIME を足す (設定ツールの `--add-language-list`)。[docs/spec/runtime.md](../spec/runtime.md)・
  [docs/spec/tools.md](../spec/tools.md) へ移した
- [appcontainer.md](appcontainer.md): AppContainer のアプリ (スタートメニューの検索欄など) での入力。パイプと
  設定ファイルの権限、AppContainer からのエンジンとツールの起動、入力パネルなどのウィンドウのオーナー。
  [docs/spec/runtime.md](../spec/runtime.md)・[docs/spec/tools.md](../spec/tools.md)・[docs/spec/display.md](../spec/display.md) へ移した
- [conversion-range.md](conversion-range.md): run が無いときのキャレット直前のかなの変換と、候補選択中の
  ←→ による変換の範囲の伸縮。[docs/spec/conversion.md](../spec/conversion.md) へ移した
- [adopted-reconversion.md](adopted-reconversion.md): 採用・確定した文字列を読みとともに覚え、漢字を含む
  部分も変換キーで変換し直す。[docs/spec/conversion.md](../spec/conversion.md) へ移した
- [next-word-prediction.md](next-word-prediction.md): 読みなしの後続語予測。確定の直後に次の語を候補バーに
  出し、予測候補の順位にも前文脈を使う。[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [suggest-paging.md](suggest-paging.md): 候補バー (run のサジェスト・後続語の予測) のページ送り。合計の
  上限を `max_predictions` (9 / 18 / 27 / 36) にする。[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [input-model-v2.md](input-model-v2.md): 入力モデル v2 の総論 (文書を自動で書き換えない・1キー1機能
  などの原則と段階の区切り)。今も成り立つ原則は [docs/spec/input.md](../spec/input.md) へ移した
- [direct-input.md](direct-input.md): 直接入力方式 (打鍵した文字を文書に直接入れ、run を覚えて変換キーで
  後置変換する)。[input-buffer-modes.md](input-buffer-modes.md) で廃止。今も成り立つ理由 (CUAS の文書の
  読み戻し・Chromium の置き換えの制約・半角数字の素通し) は [docs/spec/input.md](../spec/input.md) へ移した
- [append-input.md](append-input.md): 追記型入力 (未完成のローマ字を小窓に出し、かなを文書に追記する。
  読める文書と追記のみの文書・擬似 Backspace)。[input-buffer-modes.md](input-buffer-modes.md) で廃止
- [append-only-apps.md](append-only-apps.md): 指定したアプリ (既定 Discord.exe) を追記のみの文書として扱う。
  [input-buffer-modes.md](input-buffer-modes.md) で廃止
- [panel-buffer.md](panel-buffer.md): パネル保持入力 (設定 `input_buffer panel`)。
  [docs/spec/input.md](../spec/input.md) へ移した
- [input-buffer-modes.md](input-buffer-modes.md): 入力方式の選択 (設定 `input_buffer` の `composition` /
  `panel`)、2 方式で共通の run、送り直し、文書の分類。[docs/spec/input.md](../spec/input.md) へ移した
- [ime-start-state.md](ime-start-state.md): アプリの起動時の IME の状態 (オン・オフ・自動) をアプリごとに
  決める。[docs/spec/input.md](../spec/input.md) へ移した
- [keymap.md](keymap.md): キー割当を状態別の表にし、Enter をアプリへ渡すキーに分け、確定キーを設ける。
  [docs/spec/keys.md](../spec/keys.md) へ移した。設定ツールの画面は [docs/spec/tools.md](../spec/tools.md) へ移した
- [number-key-adopt.md](number-key-adopt.md): 数字キーをどの状態でも文字キーと同じに扱い、番号での採用・確定を
  Ctrl+1〜9 にする (候補バー・後続語の予測・候補選択中)。[docs/spec/keys.md](../spec/keys.md) へ移した
- [input-scope.md](input-scope.md): 数字・電話番号・メールアドレス・URL などの InputScope の欄では、IME オンの
  ままキーを食べない (設定 `input_scope_passthrough`)。[docs/spec/input.md](../spec/input.md) へ移した
- [candidate-bar.md](candidate-bar.md): 候補バー。入力中に変換結果と予測候補を横一列に出し、採用時だけ
  文書を書き換える。[docs/spec/suggest.md](../spec/suggest.md) と [docs/spec/display.md](../spec/display.md) へ移した
- [suggest-panel.md](suggest-panel.md): 入力パネル。候補バーの見た目の刷新 (カプセル)、画面中央への
  表示、入力行、背景のぼかし。[docs/spec/display.md](../spec/display.md) へ移した
- [conversion-panel.md](conversion-panel.md): 変換キーの候補一覧を入力パネルで出す (テーマ・表示位置・
  変換中の入力行)。[docs/spec/display.md](../spec/display.md) へ移した
- [panel-above-caret.md](panel-above-caret.md): 縦に並ぶ入力パネルの上下を出しうる最大の高さで決め、
  キャレット行の上に出すときは並びを反転して入力行と各候補の位置を動かさない。[docs/spec/display.md](../spec/display.md) へ移した
- [caret-marker.md](caret-marker.md): 文書のキャレットの位置を印 (テーマの `caret_marker`。パネルの吹き出しの
  尻尾か、キャレットの下の三角) で示す。[docs/spec/display.md](../spec/display.md) へ移した
- [theme-blur.md](theme-blur.md): ぼかしをテーマのキー `blur` (`on` / `off`) だけで決め、組み込みテーマ
  `capsule-blur` を足す。[docs/spec/display.md](../spec/display.md) へ移した
- [mode-icon.md](mode-icon.md): タスクバーの入力モードのアイコン (あ・A・入力欄でないときの ×) を、同梱の
  M PLUS Rounded 1c で透明な背景に描く。[docs/spec/display.md](../spec/display.md) へ移した
- [mode-popup.md](mode-popup.md): オン/オフの切替時と別の入力欄へのフォーカス移動時に、キャレットの近くに
  入力モード (あ / A) を約 1 秒出す (設定 `mode_popup`)。[docs/spec/display.md](../spec/display.md) へ移した
- [candidate-position-fallback.md](candidate-position-fallback.md): 入力位置がウィンドウの外を指すアプリ
  (マビノギなど) で、入力パネルなどをウィンドウ中央に出す。[docs/spec/display.md](../spec/display.md) へ移した
- [nbest.md](nbest.md): 入力全体の N-best。候補バーと変換キーの候補に使い、文節 UI を設定 `segment_ui`
  のオプションにする。[docs/spec/engine.md](../spec/engine.md)・[docs/spec/conversion.md](../spec/conversion.md)・
  [docs/spec/suggest.md](../spec/suggest.md) へ移した
- [unified-suggest.md](unified-suggest.md): 打鍵をエンジンに送り、打鍵の上でずれを測るファジィ照合と、全体変換・
  予測・補正の候補を 1 つの点数で並べる SUGGEST。先頭文節の候補をなくし、選んだ全体変換の採用範囲を
  Shift+← / → で縮める。[docs/spec/suggest.md](../spec/suggest.md) と [docs/spec/engine.md](../spec/engine.md) へ移した
- [typo-conversion.md](typo-conversion.md): 入力全体の候補に、打ち間違いを 1 打直した読みで変換した補正の候補を
  足す (計測の結果を含む)。[docs/spec/engine.md](../spec/engine.md) と [docs/spec/display.md](../spec/display.md) へ移した
- [typo-segment-ui.md](typo-segment-ui.md): 文節 UI (`segment_ui` 1) の変換で、補正の最良経路に沿って文節を
  区切り、打ち間違えた語を含む文節の候補に補正の表記を足す。[docs/spec/engine.md](../spec/engine.md) と
  [docs/spec/conversion.md](../spec/conversion.md) へ移した
- [selection-reconversion.md](selection-reconversion.md): 漢字を含む選択範囲を、エンジンの逆変換 (REVERSE) で
  推測した読みから変換キー・F6〜F10 で変換し直す。[docs/spec/conversion.md](../spec/conversion.md) と
  [docs/spec/engine.md](../spec/engine.md) へ移した
- [number-forms.md](number-forms.md): 数字で打った数に、漢数字・桁区切り・万の単位まじり・全角/半角の候補を足す。
  [docs/spec/conversion.md](../spec/conversion.md) と [docs/spec/engine.md](../spec/engine.md) へ移した
- [llm-generate.md](llm-generate.md): Vulkan 版で、9 文字以上の読みの並べ替えに LLM の貪欲法で生成した候補を
  1 件足し、候補バーの全体変換の枠を並べ替えの結果の上位から埋め直す。[docs/spec/engine.md](../spec/engine.md) と
  [docs/spec/suggest.md](../spec/suggest.md) へ移した
- [eval-sets.md](eval-sets.md): 変換精度の評価セット (AJIMEE-Bench と、実ユーザの learning_stats.tsv の
  再生評価)。[docs/spec/engine.md](../spec/engine.md)「評価セット」へ移した
- [learning-score.md](learning-score.md): 学習を使った度合いの点数にする。確定した文節を辞書の語に分けて
  語の学習を Viterbi の単語コストの割引に、句の学習を文節の候補・読み全体の学習表記・予測に、文脈の句の学習を
  前文脈付きの予測と後続語の予測に使う (learning_stats.tsv)。[docs/spec/learning.md](../spec/learning.md)・
  [docs/spec/engine.md](../spec/engine.md)・[docs/spec/suggest.md](../spec/suggest.md) へ移した
- [forget-learning.md](forget-learning.md): 入力パネルで選んでいる候補の学習を Ctrl+Delete で消す。
  [docs/spec/learning.md](../spec/learning.md)・[docs/spec/suggest.md](../spec/suggest.md)・
  [docs/spec/keys.md](../spec/keys.md) へ移した
- [modeless.md](modeless.md): モードレス入力 (自動英字判定) の基本。設定 `modeless`・判定のルール 1〜3・
  英字モード中のキー。[docs/spec/modeless.md](../spec/modeless.md) へ移した
- [modeless-detection.md](modeless-detection.md): 判定の強化 (ルール 4〜7) と、英単語の部分だけを英字にする
  英字区間・英単語辞書・ASCIISTART。[docs/spec/modeless.md](../spec/modeless.md) へ移した
- [modeless-commit-candidate.md](modeless-commit-candidate.md): run 終了時の英字判定を書き換えではなく
  候補バーの英字候補として出す。きっかけの打鍵の Backspace で判定前に戻す。[docs/spec/modeless.md](../spec/modeless.md) へ移した
- [mixed-conversion.md](mixed-conversion.md): 英字混じり文の変換。英字区間を閉じて日本語に戻り、日本語の
  部分だけを変換する (ASCIIWORD・英字の連続の語)。[docs/spec/modeless.md](../spec/modeless.md) と
  [docs/spec/engine.md](../spec/engine.md) へ移した
- [english-case.md](english-case.md): 英単語の大文字小文字の補正 (`github` → `GitHub`) を候補として出す
  (ASCIIFORM)。[docs/spec/modeless.md](../spec/modeless.md) と [docs/spec/engine.md](../spec/engine.md) へ移した
- [dict-import.md](dict-import.md): 他の IME (MS-IME・ATOK・Mozc) の辞書テキストを単語登録ツールで取り込み、
  インポート辞書としてエンジンで引く。[docs/spec/dictionary.md](../spec/dictionary.md) へ移した
- [newline-key.md](newline-key.md): 改行キー (既定 Ctrl+J)。対象アプリで Shift+Enter を送って改行を入れる。
  [docs/spec/input.md](../spec/input.md) と [docs/spec/tools.md](../spec/tools.md) へ移した
- [no-learning-apps.md](no-learning-apps.md): 指定したアプリでは確定・採用の結果を学習に記録しない。
  [docs/spec/learning.md](../spec/learning.md) と [docs/spec/tools.md](../spec/tools.md) へ移した
- [panel-theme.md](panel-theme.md): 入力パネルのテーマ (組み込みのカプセル・縦積み・ゴーストと、
  テーマファイル)。[docs/spec/display.md](../spec/display.md) と [docs/spec/tools.md](../spec/tools.md) へ移した
- [config-tabs.md](config-tabs.md): 設定ツールのメイン画面のタブ化 (Win32 の標準コントロールの画面)。
  [docs/spec/tools.md](../spec/tools.md) へ移した
- [app-settings.md](app-settings.md): 設定ツールの「アプリ」タブを、アプリを軸にした1つの一覧とアプリごとの
  編集欄にする。[docs/spec/tools.md](../spec/tools.md) へ移した
- [config-input-display.md](config-input-display.md): 設定ツールで入力中の読みの表示先 (`input_buffer`・
  `panel_input`) を 1 つのコンボで選び、入力モード表示の文言を短くする。[docs/spec/tools.md](../spec/tools.md) へ移した
- [slint-gui.md](slint-gui.md): 設定ツールと単語登録ツールの画面を Slint (fluent スタイル) で作り直す。左のナビと
  ページのカード、変えたら即保存、キー割当・アプリの展開カード、結果とエラーの InfoBar。
  [docs/spec/tools.md](../spec/tools.md) へ移した
- [panel-preview.md](panel-preview.md): 設定ツールの「見た目」ページに、選んでいるテーマ・フォント・サイズの
  入力パネルの見本を、登録されている TSF の DLL の描画で出す。[docs/spec/display.md](../spec/display.md) と
  [docs/spec/tools.md](../spec/tools.md) へ移した
- [next-word-ngram.md](next-word-ngram.md): 日本語版ウィキペディアから数えた文節の連なりの統計 (next_word.tsv) を、
  読みなしの後続語予測で学習の後ろに足す。[docs/spec/suggest.md](../spec/suggest.md) と
  [docs/spec/runtime.md](../spec/runtime.md) へ移した
- [llm-rerank.md](llm-rerank.md): LLM (zenz 系モデル、別プロセス quicklime-llm) による N-best の並べ替え。
  [docs/spec/engine.md](../spec/engine.md) と [docs/spec/runtime.md](../spec/runtime.md) へ移した
- [dict-expansion.md](dict-expansion.md): 辞書の語彙補強 (Mozc の単漢字辞書と、mozc-ut 由来の UT 辞書)。
  [docs/spec/dictionary.md](../spec/dictionary.md) と [docs/spec/runtime.md](../spec/runtime.md) へ移した
- [engine-autostart.md](engine-autostart.md): エンジンの早期起動。最初の変換より前に起動しておき、起動中の変換は
  準備ができるまで待つ。named mutex による二重起動の防止。管理者権限で起動しても通常権限のアプリから接続できるパイプの
  アクセス権。[docs/spec/runtime.md](../spec/runtime.md) へ移した
- [third-party-notices.md](third-party-notices.md): 配る exe に静的リンクしている Rust の crate の著作権表示と
  許諾文を、インストーラのビルド時に cargo-about で `LICENSE-rust-crates.txt` に生成して同梱する。
  [docs/spec/runtime.md](../spec/runtime.md) へ移した

- [experiment-0b-llm.md](experiment-0b-llm.md): LLM による順位補正の遅延・メモリ・精度の計測の仕様
- [experiment-0b-llm-results.md](experiment-0b-llm-results.md): 上の計測の結果
- [experiment-0c-llm-gen.md](experiment-0c-llm-gen.md): LLM による候補の生成 (自由生成と、統計変換を
  ドラフトにした照合) の精度・速さ・副作用の計測の仕様
- [experiment-0c-llm-gen-results.md](experiment-0c-llm-gen-results.md): 上の計測の結果
- [docs-reorganization.md](docs-reorganization.md): ドキュメントの構成 (architecture・設計書の索引・
  basics・roadmap の分担)。[spec-docs.md](spec-docs.md) に置き換わった
- [user-docs.md](user-docs.md): 利用者向けドキュメントの分割 (README・docs/user/・docs/development.md・
  設計書の分担と、設定一覧 docs/user/settings.md)
- [spec-docs.md](spec-docs.md): 仕様書 (docs/spec/) と設計書の分離。2 層の役割・仕様書の区分・移行の進め方と、
  変更の流れ。今も守るルールは [docs/spec/README.md](../spec/README.md) へ移した
- [basics.md](basics.md): 設計書の無い基本機能の仕様と理由。全部の節を各仕様書へ移した

- [experiment-0a-append-input.md](experiment-0a-append-input.md): 追記型入力の2方式 (F1 / F2)
  の試作と比較。F1 を [append-input.md](append-input.md) で採用
- [live-conversion-commit-notes.md](live-conversion-commit-notes.md): ライブ変換の結果を途中で
  確定する手段の検討。後続の打鍵で前方の変換が変わる問題は [docs/spec/suggest.md](../spec/suggest.md)「採用」で扱う
- [single-unit-conversion-notes.md](single-unit-conversion-notes.md): 連文節変換をやめ、文節を
  見せない変換にする案の検討
- [modeless-detection-notes.md](modeless-detection-notes.md): モードレス入力の英語判定の強化案と
  Meltype の調査。採った項目は [docs/spec/modeless.md](../spec/modeless.md)
- [llm-rerank-notes.md](llm-rerank-notes.md): Zenzai・mozkey の LLM 補正の調査。並べ替えは
  [llm-rerank.md](llm-rerank.md) で実装し、フィードバック学習は [roadmap](../roadmap.md) の未着手
- [rakukan-llm-notes.md](rakukan-llm-notes.md): rakukan の LLM 変換 (生成の前後処理・jinen・運用) の調査。
  生成は [llm-generate.md](llm-generate.md) で実装
