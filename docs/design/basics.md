# 基本機能の仕様

専用の設計書が無い基本機能について、コードから読み取りにくい仕様と、その理由 (WHY) をまとめる。
利用者向けの操作の説明は [docs/user/](../user/) にあり、ここでは重複させない。
各機能の設計書は [設計書の索引](README.md) から引ける。

## 画面の部品の呼び名

[docs/spec/display.md](../spec/display.md)「画面の部品の呼び名」へ移した

## 辞書

- 同梱辞書は Mozc OSS 辞書 (`dictionary00.txt`〜`09.txt`。書式は `読み\t左文脈ID\t右文脈ID\tコスト\t表記`)。
  単漢字辞書と UT 辞書による語彙の補強は [dict-expansion.md](dict-expansion.md)
- 辞書の場所 (engine/src/main.rs `dictionary_dir`) は、環境変数 `QUICKLIME_DICT_DIR` → exe と同じ
  ディレクトリの `dict\` (インストール先のレイアウト) → 開発レイアウトの
  `references/mozc/src/data/dictionary_oss` の順に探す。読み込めなければ辞書なしで起動を続ける
- 検索は fst (有限状態トランスデューサ) の `fst::Map` 1つで行う (engine/src/dict.rs)。読みをキー、
  エントリ列の位置を値にし、次の3種類を同じ構造で引く
  - 完全一致 (候補の生成)
  - 共通接頭辞検索 (Viterbi のラティス構築。入力の各位置から1回の走査で辞書語を列挙する)
  - 前方一致 (予測)。短い読みで一致範囲が巨大になるのを避けるため、走査は 20,000 読みで打ち切る
- fst はキーの共通接頭辞・接尾辞を圧縮するため、ハッシュ表と前方一致用の読み配列を別々に持つより
  メモリが少ない
- ダブル配列のクレート (yada / crawdad) は前方一致の列挙の API が無く、予測入力に使えないため採らない

## 変換 (Viterbi と文節)

[docs/spec/engine.md](../spec/engine.md)「ラティスと Viterbi」「文節の分け方」「文節の候補 (CONVSEG・CONVCTX)」へ移した

## 数字の書き方の候補

[docs/spec/conversion.md](../spec/conversion.md)「数字の書き方の候補」へ移した

## 学習

学習の仕組みは [learning-score.md](learning-score.md)。学習データは `%APPDATA%\QuicklIME\` に置く
(`QUICKLIME_LEARN_FILE` で learning.tsv の場所を変えると、他のファイルも同じディレクトリに対で作る)。

| ファイル | 内容 | 使い道 |
|---|---|---|
| learning_stats.tsv | 語の学習・句の学習・文脈の句の学習と、それぞれの使った度合い。追記ログで、同じ件は後の行が勝つ | 変換 (Viterbi の単語コストの割引、文節の候補の並び、読み全体の学習表記) と予測 |
| learning_boundary.tsv | 文節伸縮で直した分割の文節読み (2文字以上) | 変換時の文節境界を寄せる |
| learning_word.tsv | 辞書に無い語を文節伸縮で割って確定した複合語 | 経路が作れない読みだけ1語としてラティスに載せる |

- 確定した文節は辞書の語に分け、自立語ごとに語の学習、文節について句の学習、前文脈があれば文脈の句の学習を
  1 回数える。2 文節以上の確定の読み全体は句の学習だけを数える。読みが 1 文字の語・表記が読みと同じひらがなの
  語・表記に数字を含む語は語の学習に数えない
- 使った度合いは数えるたびに 1 増え、確定の回数で古くなる (2000 文節の確定で半分)。並びは度合いだけで決まり、
  最後に選んだ表記を先頭にする規則は無い
- 前文脈 (直前に確定した文節の表記) は予測と後続語の予測にだけ使い、変換の経路には効かない
- learning_stats.tsv が無いときは、古い形式の learning.tsv (読み → 最後に確定した表記) と learning_context.tsv
  ((前文脈, 読み) → 表記) から作る。古いファイルは読むだけで書かない
- learning_word.tsv はユーザ辞書と混ぜずに別に持つ。誤学習だけを捨てられるようにするため。
  単語コストを高く (12000) して、辞書語の組み合わせで経路が作れる読みは1語にしない
  (1語にすると先頭語の入れ替え候補が消え、別の表記を選べなくなる)
- かなのままで run を終えたときは学習しない。学習は変換候補の並びにも効き、「きょう → きょう」を
  覚えるとひらがなの候補が上に来るため。英字を含む入力をそのまま終えたときは読み = 表記で学習し、
  予測の確定履歴に載せる (英単語の省入力)
- 日付・時刻の動的候補は学習しない (下記「日付・時刻の候補」)
- 設定 `learning` が 0 のときは記録しないが、既存の学習データは使い続ける
- 入力パネルで選んでいる候補の学習は Ctrl+Delete (`key.forget_learning`) で消せる。消すと
  learning_stats.tsv を書き直す ([forget-learning.md](forget-learning.md))

## 予測

[docs/spec/suggest.md](../spec/suggest.md)「SUGGEST の並び」と「旧エンジンとの互換」へ移した

## タイプミス補正

[docs/spec/engine.md](../spec/engine.md)「タイプミス補正」と [docs/spec/suggest.md](../spec/suggest.md)「ファジィ照合」へ移した

## 記号辞書と記号キー

- 記号辞書は Mozc の symbol.tsv (辞書ディレクトリ直下、無ければ Mozc の配置の `symbol/`)。
  「やじるし」→「→」のように引く。無ければ記号候補なしで動く
- 記号キー・Shift+数字キー・テンキーで run に入る文字は [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
- 記号辞書を引くときは、読みの全角英数記号を半角にしてから引き直す。記号キーは全角形で run に入るが、
  symbol.tsv の読みは半角形で載っているため (「（」を変換すると ［ 〔 などが出る)
- Shift+英字の英字モードは [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
  (モードレス入力での扱いは [モードレス入力](../user/modeless.md) と [modeless.md](modeless.md))

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

- ユーザ辞書は `%APPDATA%\QuicklIME\userdict.tsv` (`QUICKLIME_USER_DICT_FILE` で変更可)。書式は
  Mozc のユーザ辞書のエクスポートと同じ `読み\t表記\t品詞[\tコメント]` で、`#` で始まる行はコメント、
  品詞の列が無い行は短縮よみとする (engine/src/userdict.rs)
- 品詞は短縮よみと名詞系7種 (名詞・固有名詞・人名・姓・名・地名・組織)。それ以外の品詞の行は読み飛ばす
  (活用の展開には対応しない)
  - 短縮よみ: 連接の情報を持たない定型文 (メールアドレスなど)。ラティスには載せず、文節候補の経路表記の
    直後と予測の先頭に入れる
  - 名詞系: id.def から文脈 ID を引き、通常の辞書語としてラティスに載せる (文中でも変換される)。
    単語コストは 3000 (同じ読みの一般語より上に来やすい中位の値。並びは学習で調整される)
- 登録 (ADDWORD) はファイルへの追記とメモリへの反映を一緒に行い、エンジンの再起動なしに使える。
  手で編集したファイルは RELOADUSER で読み直す
- 単語登録ツール quicklime-regword.exe (engine/gui/src/bin/quicklime-regword.rs) は、入力なしの状態の
  `key.register_word` (既定 Ctrl+F7) か、言語バーのメニューから起動する。選択中の文字列 (1行目だけ) を
  単語欄の初期値にする (変換できなかった語を選択してそのまま登録する用途)。よみのカタカナはひらがなに直す。
  エンジンに接続できないときは userdict.tsv へ直接追記し、エンジンの次の起動で反映される
- 他の IME の辞書の取り込みは [dict-import.md](dict-import.md)

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
