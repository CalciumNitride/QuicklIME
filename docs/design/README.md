# 設計書の索引

機能ごとの設計書の一覧。各設計書は実装前に合意した設計で、後の設計書で一部または全部が置き換えられた
ものには、置き換えた設計書を併記する。全体の構成は [docs/architecture.md](../architecture.md)、
未着手の項目と既知の問題は [docs/roadmap.md](../roadmap.md) にある。

- [basics.md](basics.md): 設計書の無い基本機能 (辞書・Viterbi・学習・予測・タイプミス補正・記号・
  ファンクションキー変換・日付時刻の候補・確定アンドゥ・ローマ字テーブル・ユーザ辞書と単語登録・
  設定ファイルと設定ツール・言語バー・エンジンの自動起動・インストーラ・パスワード欄) の仕様と理由
- [docs/protocol.md](../protocol.md): TSF 層とエンジンの間のプロトコル
- [docs/theme.md](../theme.md): 候補パネルのテーマファイルのリファレンス

## 入力方式

- [input-model-v2.md](input-model-v2.md): 入力モデル v2 の総論 (文書を自動で書き換えない・1キー1機能
  などの原則と段階の区切り)。各段階の詳細は下記の各設計書、LLM のプロセス構成の詳細は
  [llm-rerank.md](llm-rerank.md)
- [direct-input.md](direct-input.md): 直接入力方式。打鍵した文字を文書に直接入れ、run を覚えて変換キーで
  後置変換する。composition 方式とフォールバックは [append-input.md](append-input.md) で廃止。かなの
  追記の方法と文書の分類は [append-input.md](append-input.md)、パネル保持入力は
  [panel-buffer.md](panel-buffer.md)
- [append-input.md](append-input.md): 追記型入力。未完成のローマ字を小窓に出し、かなを文書に追記する。
  読める文書と追記のみの文書の扱い
- [keymap.md](keymap.md): キー割当を状態別の表にし、Enter をアプリへ渡すキーに分け、確定キーを設ける
- [panel-buffer.md](panel-buffer.md): パネル保持入力 (設定 `input_buffer panel`)。入力中の読みを文書に
  書かず候補パネルの入力行だけに持ち、採用・確定で挿入する
- [newline-key.md](newline-key.md): 改行キー (既定 Ctrl+J)。対象アプリで Shift+Enter を送って改行を入れる。
  設定ツールの「アプリ」タブは [app-settings.md](app-settings.md) で置き換え
- [ime-start-state.md](ime-start-state.md): アプリの起動時の IME の状態 (オン・オフ・自動) をアプリごとに決める
- [no-learning-apps.md](no-learning-apps.md): 指定したアプリでは確定・採用の結果を学習に記録しない
- [forget-learning.md](forget-learning.md): 候補パネルで選んでいる候補の学習を Ctrl+Delete で消す

## 候補の表示

- [candidate-bar.md](candidate-bar.md): 候補バー。入力中に変換結果と予測候補を横一列に出し、採用時だけ
  文書を書き換える。バーの全体変換の件数と先頭文節の取り方は [nbest.md](nbest.md) で、見た目・表示位置・
  入力行は [suggest-panel.md](suggest-panel.md) で置き換え
- [suggest-panel.md](suggest-panel.md): サジェストパネル。候補バーの見た目の刷新 (カプセル)、画面中央への
  表示、入力行、背景のぼかし。見た目のカスタマイズは [panel-theme.md](panel-theme.md)、縦の候補一覧は
  [conversion-panel.md](conversion-panel.md) で置き換え
- [panel-theme.md](panel-theme.md): 候補パネルのテーマ (組み込みのカプセル・縦積み・ゴーストと、
  テーマファイル)
- [next-word-prediction.md](next-word-prediction.md): 読みなしの後続語予測。確定の直後に次の語を候補バーに
  出し、予測候補の順位にも前文脈を使う
- [conversion-panel.md](conversion-panel.md): 変換キーの候補一覧をサジェストパネルで出す (テーマ・表示位置・
  変換中の入力行)

## 変換

- [nbest.md](nbest.md): 入力全体の N-best。候補バーと変換キーの候補に使い、文節 UI を設定 `segment_ui`
  のオプションにする
- [llm-rerank.md](llm-rerank.md): LLM (zenz 系モデル、別プロセス quicklime-llm) による N-best の並べ替え
- [conversion-range.md](conversion-range.md): run が無いときのキャレット直前のかなの変換と、候補選択中の
  ←→ による変換の範囲の伸縮
- [adopted-reconversion.md](adopted-reconversion.md): 採用・確定した文字列を読みとともに覚え、漢字を含む
  部分も変換キーで変換し直す

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

- [config-tabs.md](config-tabs.md): 設定ツールのメイン画面のタブ化
- [app-settings.md](app-settings.md): 設定ツールの「アプリ」タブを、アプリを軸にした1つの一覧とアプリごとの
  編集欄にする (改行キー・起動時の IME の状態)

## エンジン

- [engine-autostart.md](engine-autostart.md): エンジンの早期起動。最初の変換より前に起動しておき、起動中の変換は
  準備ができるまで待つ。named mutex による二重起動の防止

## 互換

- [candidate-position-fallback.md](candidate-position-fallback.md): 入力位置がウィンドウの外を指すアプリ
  (マビノギなど) で、候補ウィンドウ類をウィンドウ中央に出し、モニタの作業領域に収める

## 実験

- [experiment-0b-llm.md](experiment-0b-llm.md): LLM による順位補正の遅延・メモリ・精度の計測の仕様
- [experiment-0b-llm-results.md](experiment-0b-llm-results.md): 上の計測の結果

## ドキュメント

- [docs-reorganization.md](docs-reorganization.md): ドキュメントの構成 (architecture・設計書の索引・
  basics・roadmap の分担)

## 採らなかった案・検討段階の草案 (docs/archive/)

- [experiment-0a-append-input.md](../archive/experiment-0a-append-input.md): 追記型入力の2方式 (F1 / F2)
  の試作と比較。F1 を [append-input.md](append-input.md) で採用
- [live-conversion-commit-notes.md](../archive/live-conversion-commit-notes.md): ライブ変換の結果を途中で
  確定する手段の検討。後続の打鍵で前方の変換が変わる問題は [candidate-bar.md](candidate-bar.md) の採用で扱う
- [single-unit-conversion-notes.md](../archive/single-unit-conversion-notes.md): 連文節変換をやめ、文節を
  見せない変換にする案の検討
- [modeless-detection-notes.md](../archive/modeless-detection-notes.md): モードレス入力の英語判定の強化案と
  Meltype の調査。採った項目は [modeless-detection.md](modeless-detection.md)
- [llm-rerank-notes.md](../archive/llm-rerank-notes.md): Zenzai・mozkey の LLM 補正の調査。並べ替えは
  [llm-rerank.md](llm-rerank.md) で実装し、フィードバック学習は [roadmap](../roadmap.md) の未着手
