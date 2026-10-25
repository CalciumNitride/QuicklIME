# 基本機能の仕様

専用の設計書が無い基本機能について、コードから読み取りにくい仕様と、その理由 (WHY) をまとめる。
利用者向けの操作の説明は [docs/user/](../user/) にあり、ここでは重複させない。
各機能の設計書は [設計書の索引](README.md) から引ける。

## 画面の部品の呼び名

[docs/spec/display.md](../spec/display.md)「画面の部品の呼び名」へ移した

## 辞書

[docs/spec/dictionary.md](../spec/dictionary.md)「辞書の種類」「辞書ディレクトリ」「システム辞書」へ移した

## 変換 (Viterbi と文節)

[docs/spec/engine.md](../spec/engine.md)「ラティスと Viterbi」「文節の分け方」「文節の候補 (CONVSEG・CONVCTX)」へ移した

## 数字の書き方の候補

[docs/spec/conversion.md](../spec/conversion.md)「数字の書き方の候補」へ移した

## 学習

[docs/spec/learning.md](../spec/learning.md) へ移した

## 予測

[docs/spec/suggest.md](../spec/suggest.md)「SUGGEST の並び」と「旧エンジンとの互換」へ移した

## タイプミス補正

[docs/spec/engine.md](../spec/engine.md)「タイプミス補正」と [docs/spec/suggest.md](../spec/suggest.md)「ファジィ照合」へ移した

## 記号辞書と記号キー

- 記号辞書は [docs/spec/dictionary.md](../spec/dictionary.md)「記号辞書」へ移した
- 記号キー・Shift+数字キー・テンキーで run に入る文字は [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
- Shift+英字の英字モードは [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
  (モードレス入力での扱いは [モードレス入力](../user/modeless.md) と [docs/spec/modeless.md](../spec/modeless.md)「英字区間」)

## 対記号の両側同期

[docs/spec/conversion.md](../spec/conversion.md)「対記号の両側同期」へ移した

## 生ローマ字の候補

[docs/spec/conversion.md](../spec/conversion.md)「生ローマ字の候補」へ移した

## ファンクションキー変換

[docs/spec/conversion.md](../spec/conversion.md)「ファンクションキー変換 (F4〜F10)」へ移した

## 日付・時刻の候補

[docs/spec/engine.md](../spec/engine.md)「日付・時刻の候補」へ移した

## 確定アンドゥ

[docs/spec/input.md](../spec/input.md)「確定アンドゥ」へ移した

## 読める文書の判定

[docs/spec/input.md](../spec/input.md)「読める文書の判定」へ移した

## ローマ字テーブル

[docs/spec/input.md](../spec/input.md)「ローマ字テーブル」へ移した

## ユーザ辞書と単語登録ツール

[docs/spec/dictionary.md](../spec/dictionary.md)「ユーザ辞書」「インポート辞書」「単語登録ツール」へ移した

## 設定ファイルと設定ツール

- 設定ファイルは `%APPDATA%\QuicklIME\config.tsv` (`QUICKLIME_CONFIG_FILE` で変更可)。`キー\t値` の TSV で、
  `#` で始まる行はコメント。エンジンと TSF 層が同じファイルを読み、知らないキーは無視し、読めない値は
  そのキーだけ既定値にする
- エンジンは起動時と RELOADCONFIG の受信時に読む。TSF 層はフォーカスの切替と IME のオン/オフの切替で
  ファイルの更新時刻を確かめ、変わっていれば読み直す (アプリの再起動は要らない)。設定ツールで保存して
  アプリに戻る操作自体がフォーカスの切替なので、実質すぐ反映される
- 設定ツール quicklime-config.exe (engine/gui/src/bin/quicklime-config.rs) は、値を変えるたびに
  (300 ms まとめて) 全キーをコメント付きで書き出し、エンジンへ RELOADCONFIG を送る。書き込みの途中を
  TSF 層・エンジンが読まないよう、一時ファイル (`config.tsv.tmp`) に書いてから置き換える。起動は入力なしの状態の
  `key.open_config` (既定 Ctrl+F12) か言語バーのメニューから。ページの構成と画面は [slint-gui.md](slint-gui.md)。
  「詳細」ページで診断ログの出力先 (`debug_log`。空なら出さない) を設定する
- 機能ページに説明の無い基本の設定を読む側 (値と既定は [設定一覧](../user/settings.md)):

  | キー | 読む側 |
  |---|---|
  | `learning` | エンジン |
  | `suggest` | エンジン |
  | `typo_correction` | エンジン |
  | `max_predictions` | エンジン・TSF 層 |
  | `space` | TSF 層 |
  | `punctuation` | TSF 層 |
  | `digits` | TSF 層 |
  | `candidate_font` / `candidate_font_size` | TSF 層 |
  | `debug_log` | TSF 層 |

- キー割当 (`key.*`) は [docs/spec/keys.md](../spec/keys.md)「設定ファイルの書式」へ移した

## 言語バー

[docs/spec/display.md](../spec/display.md)「入力モードのアイコン」と「言語バー」へ移した

## エンジンの自動起動

設計は [engine-autostart.md](engine-autostart.md)。

- TSF 層はアクティブ化 (`ActivateEx`) のときにエンジンのパイプへ接続してみて、開けなければエンジンの exe を
  探して起動する (コンソールなし)。起動の完了は待たない (tsf/src/engine_client.cpp)。ログオン時は explorer
  などでも TSF がアクティブ化されるので、ログオンと同時にエンジンが立ち上がる
- 候補バー・英字判定のように打鍵ごとに呼ぶ問い合わせでも、パイプを開けなければエンジンを起動だけする。
  接続は待たず、その打鍵の問い合わせは失敗として扱う (打鍵を止めないため)
- 明示的な変換などの待ってよい問い合わせでは、最後にエンジンを起動できてから 5 秒以内なら新たには起動せず、
  起動から 5 秒経つまでパイプが開くのを待つ (100 ms ごと)。それ以外なら起動を試みて 5 秒まで待つ。
  エンジンは辞書を読み終えてからパイプを作り、PC の起動直後は数秒かかるため
- 起動の試行はどの経路でも 10 秒に1回まで。exe が見つからないなどで起動できなかった試行は待つ対象にしない
- exe の探索 (`FindExePath`) は DLL と同じディレクトリ → 親ディレクトリ (インストール先の `x86\` の
  32bit DLL から 64bit の exe を使う) → 開発レイアウトの `engine\target\release\` → `engine\target\debug\`
  の順。設定ツール・単語登録ツールも同じ探索で起動する
- エンジンは起動の直後、辞書を読む前に named mutex `Local\quicklime-engine-<パイプ名>` (パイプ名の `\` は
  `_` に置き換える) を作り、すでにあれば終了する。辞書の読み込み中はパイプが無いので、多くのプロセスから
  同時に起動されてもパイプへの接続だけでは見分けられないため。パイプ名ごとに分かれるので、
  `QUICKLIME_PIPE_NAME` で隔離したエンジンは常用のエンジンと並んで動ける
- あわせて、同じ名前のパイプへ接続できたら既に動いているとみなして終了する
- 自動起動したエンジンは TSF 層のプロセスの環境変数を引き継ぐ (`QUICKLIME_PIPE_NAME` などの隔離設定が効く)

## インストーラ

利用者向けの説明は [README.md](../../README.md) の「インストール」、インストーラのビルドの手順は [development.md](../development.md) にある。Inno Setup (installer/installer.iss) で作り、
installer/build.ps1 がバイナリ・辞書・ライセンスを集めてから ISCC を呼ぶ。

- 64bit DLL を `%ProgramFiles%\QuicklIME\` に、32bit DLL を `x86\` に置き、それぞれ regserver で登録する
  (32bit 側は Wow6432Node)。登録の中身は DLL の DllRegisterServer に任せる
- 更新時は先にエンジン・設定ツール・単語登録ツール・LLM の子プロセスを taskkill で止める。ロード中の DLL は
  restartreplace で再起動時に置き換える (初回のインストールは未ロードなので再起動は要らない)
- アンインストールでも `%APPDATA%\QuicklIME\` (設定・ユーザ辞書・学習) には触れない
- TSF の DLL は CRT を静的リンクする (/MT、tsf/CMakeLists.txt)。DLL は全アプリのプロセスにロードされ、
  exe を基準にする DLL 探索では DLL の隣に置いた msvcp140.dll などが見つからないため。エンジン・ツールの
  release ビルドも build.ps1 で crt-static にする。LLM の子プロセスは CRT を動的にリンクし、再頒布可能な
  DLL を同梱する

## パスワード欄などキー入力が無効な文書

[docs/spec/input.md](../spec/input.md)「キー入力が無効な文書」へ移した

## 横並びのテーマでのサジェスト選択の ←→

[docs/spec/suggest.md](../spec/suggest.md)「選択」へ移した
