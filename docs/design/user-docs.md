# 利用者向けドキュメントの分割

## 背景と目的

README.md が 750 行を超え、次の情報が 1 つのファイルに混ざっている。

1. 利用者が最初に読む情報 (概要・導入・基本操作)
2. 機能ごとの操作の詳細 (入力方式・候補バー・キー割当・モードレス入力など)
3. 開発者向けの手順 (構成・ビルド・インストーラの作り方・開発版 DLL の切替)
4. 実装の内部の挙動 (LLM の子プロセスの再起動の方針、モードレス入力の判定の細部など)

また、config.tsv の設定キーは各節に散らばっていて、一覧になっていない。

README を「利用者が最初に読むページ」に絞り、機能の詳細は docs/user/ に分け、
開発者向けの手順は docs/development.md に、内部の挙動は設計書に置く。

## 決定事項

### 置き場所

| 情報 | 置き場所 |
|---|---|
| 概要・導入・基本操作・設定のしかた・ドキュメントの一覧・同梱物とライセンス | README.md |
| 機能ごとの操作の詳細 | docs/user/*.md (新設) |
| 設定キーの一覧 | docs/user/settings.md (新設) |
| テーマファイルのリファレンス | docs/user/theme.md (docs/theme.md から移す) |
| 構成・開発環境・ビルド・インストーラのビルド・開発版 DLL の切替・検証用の環境変数 | docs/development.md (新設) |
| 実装の内部の挙動 | 既存の設計書 (無いものは docs/design/basics.md) |

- docs/theme.md は利用者向けのリファレンスなので docs/user/ へ移す。`git mv` で移し、本文は変えない
- 内部の挙動は、移す先の設計書に同じ内容があることを確かめてから README から削る。
  設計書に無い・設計書と食い違う内容は、コードで今の挙動を確かめて設計書 (無ければ basics.md) に書いてから削る
- 既存の設計書の「変更対象ファイル」「実装手順」にある README.md・docs/theme.md への言及は、
  実装前の合意の記録なので書き換えない (docs-reorganization.md と同じ方針)
- 本文の文言は原則として README のものをそのまま移す。分割に伴う見出し・節をまたぐ参照
  (「下記『候補バー』」など) の書き換えと、利用者向けから外す内部の挙動の削除だけを行う

### README.md に残す内容

上から次の順に並べる。

1. 概要 (今の冒頭の 1 文に、何ができる IME かを 1 段落足す)
2. 主な機能: 1 機能 1 行で、docs/user/ の該当ページへリンクする
   (入力全体の変換、候補バーと予測、打ち間違いの補正、学習、後置再変換・確定アンドゥ、
   次の語の予測、モードレス入力、AI による並べ替え、辞書インポート、テーマ)
3. 動作環境: Windows 11。64bit アプリと 32bit アプリの両方で使える
4. インストール: インストーラ (`quicklime-<版>-setup.exe`) を実行 → Win+Space で「QuicklIME」を選ぶ。
   配置先 `%ProgramFiles%\QuicklIME\`、初回は再起動不要、更新時は再起動を求められることがある
5. アンインストール: `%APPDATA%\QuicklIME\` のユーザデータ (設定・ユーザ辞書・学習) は残る
6. 基本操作の早見表 (表 1 つ): 打つ → 変換キー → Space / ↑↓ で選ぶ → Enter・無変換で確定、
   Tab / 数字で候補バーから採用、Esc で取り消し、Ctrl+Backspace で確定アンドゥ、
   Ctrl+F7 で単語登録、Ctrl+F12 で設定。詳細は docs/user/input.md と docs/user/keymap.md へリンク
7. 設定のしかた: 設定ツール (Ctrl+F12 か言語バーのメニュー) と、`%APPDATA%\QuicklIME\` に置くファイルの表
   (config.tsv・romaji.tsv・userdict.tsv・imported\・themes\・学習ファイル)。キーの一覧は settings.md へリンク
8. ドキュメント: docs/user/ の各ページと、開発者向け (docs/development.md・docs/architecture.md・
   設計書の索引・docs/roadmap.md・docs/protocol.md) へのリンク
9. 同梱物とライセンス: 今の「インストーラでの導入」の同梱物の箇条書きから、何を同梱し、どのライセンスで、
   どの LICENSE ファイルが付くかだけを残す (Mozc 辞書・単漢字辞書・UT 辞書・SCOWL と english-names.txt・
   llama.cpp・zenz・M PLUS Rounded 1c)。取得・生成の手順は docs/development.md へ移す

### docs/user/ のページ

| ファイル | 内容 | README の出どころ |
|---|---|---|
| input.md | 入力と変換: run、印字キー・Backspace、変換キーと候補選択中のキー、変換の範囲の伸縮、文節単位の変換 (`segment_ui`)、確定キー、Enter / Space / Esc / 矢印 / Alt 併用、Ctrl+J (改行キー)、補正の候補 (`typo_correction`)、数字の書き方の候補、後置再変換、確定アンドゥ、読める文書、入力中の読みの表示 (`input_buffer`)、数字・メールアドレスなどの入力欄 (`input_scope_passthrough`) | 「入力方式」(学習の項を除く)、「読める文書」「入力中の読みの表示」「数字・メールアドレスなどの入力欄」 |
| learning.md | 学習: 確定・採用で学習すること、学習の効き方 (度合い・ほかの文への波及・古い学習)、読み全体の学習、学習の削除 (Ctrl+Delete)、学習しないアプリ、`learning` 設定 | 「入力方式」の学習の 2 項、「候補バー」の学習の削除、「アプリごとの設定」の学習の停止 |
| candidate-bar.md | 候補バーと表示: 候補の種類と並び、ページ送り、選択と採用、採用範囲の縮め、次の語の予測、見た目 (theme.md へリンク)、表示位置、入力行、ぼかし、入力モードの一時表示 (`mode_popup`) | 「候補バー」(学習の削除を除く)、「次の語の予測」「入力モードの一時表示」 |
| theme.md | 今の docs/theme.md | docs/theme.md |
| keymap.md | キー割当: 機能・設定キー・状態・既定の表、書式、キーの表記、割り当てられないキー、重なったときの優先 | 「キー割当」 |
| app-settings.md | アプリごとの設定: 設定ツールの「アプリ」タブ、`newline_apps`・`ime_start_apps`・`no_learning_apps` | 「アプリごとの設定」 |
| modeless.md | モードレス入力: 有効化、打鍵中に英字になる場合、英字候補、英字にならない場合、英字混じりの文、大文字小文字の補正、区間分割した run の候補バー、英字モード中のキー、直し方 (F10・F6・Backspace) | 「モードレス入力」 |
| llm.md | AI による候補の並べ替え: 有効化、並べ替える対象と動かさない候補、候補バーと変換キーでの見え方、文脈の扱い (メモリ上だけで使う)、`llm_backend` の選び方、使えないときも統計変換で入力できること | 「AI による候補の並べ替え」 |
| dictionary.md | 辞書とローマ字: 単語登録ツール、辞書インポート (形式・品詞の対応・重複・保存先・外し方・優先度)、ローマ字テーブルのカスタマイズと AZIK プリセット | 「辞書インポート」「ローマ字テーブルのカスタマイズ」 |
| settings.md | 設定一覧 (下記) | 新しく書く |

- 学習の項を input.md から learning.md へ移すので、input.md からは learning.md へリンクする
- 「アプリごとの設定」の学習の停止は learning.md に本文を置き、app-settings.md からはリンクする
  (`no_learning_apps` の説明を 2 か所に書かない)
- 節をまたぐ参照 (「下記『候補バー』」「上記『アプリごとの設定』」など) は、別ページになるものは相対リンクに直す

### docs/user/settings.md (設定一覧)

config.tsv のキーを 1 つの表にまとめる。列は「キー」「設定ツールでの表記 (タブ)」「値」「既定」「説明のページ」。

- 対象: 設定ツール (engine/src/bin/quicklime-config.rs) が書き出すキーと、TSF 層 (tsf/src/config.cpp)・
  エンジン (engine/src/config.rs) が読むキーの全部。値と既定はコードから取る
- `key.*` は 1 行にまとめて keymap.md へリンクする
- 機能ページに説明の無い基本の設定 (`space`・`punctuation`・`digits`・`candidate_font`・`candidate_font_size`
  など、今は docs/design/basics.md の「README で説明していない基本の設定」の表にあるもの) は、
  settings.md の表の中で説明を完結させる
- `debug_log` は開発用として表に載せ、診断用であることだけを書く
- 冒頭に、config.tsv の場所 (`%APPDATA%\QuicklIME\config.tsv`)、書式 (`キー<TAB>値`)、設定ツールで保存すると
  全キーが書き出されること、反映のタイミング (フォーカス移動・IME のオン/オフで読み直す) を書く
- settings.md には「読む側」(エンジン / TSF 層) の列を載せない。これは開発者向けの情報なので、
  docs/design/basics.md の上記の表に「キー」「読む側」の 2 列だけを残す

### docs/development.md

README から次を移す。本文はそのまま移す。

- 構成 (ディレクトリの表)
- 開発環境
- ビルド (エンジン・TSF 層・LLM の子プロセス・LLM の並べ替えの計測)
- インストーラのビルド: `installer\build.ps1` と前提 (Inno Setup・references/mozc・単漢字辞書・UT 辞書・LLM の exe・モデル)
- 同梱データの取得・生成の手順: UT 辞書 (`fetch-ut-dict.ps1`・`build_ut_dict.rs`)、英単語辞書の生成元、
  アイコン用フォントのサブセット (`subset-icon-font.py`)、VC++ ランタイムの DLL の取り方
- 開発版 DLL の切替とインストール版の更新 (dev-deploy.ps1・regsvr32)
- 注意 (DLL は全プロセスにロードされる)
- 検証用の環境変数 (`QUICKLIME_IMPORT_DIR`・`QUICKLIME_LLM_MODEL_DIR`)。README 中の説明を移す

### 利用者向けから外す内部の挙動

| 内容 | 移す先 | 確認結果 |
|---|---|---|
| LLM の子プロセスの時間 (2 秒・60 秒・10 秒) と、続けて 3 回で止まること | docs/design/llm-rerank.md | 記載あり (`LLM_TIMEOUT_MS` 2000ms・`LLM_HANG_MS` 60000ms・`LLM_RESTART_COOLDOWN_MS` 10s・3 回)。README から削るだけ |
| LLM の exe とモデルを探す順 | docs/design/llm-rerank.md | 記載あり (`QUICKLIME_LLM_MODEL_DIR` → 同じディレクトリ → 開発レイアウト)。README から削るだけ |
| モードレス入力のルール番号と判定の細部 (4 文字までの保留、3 文字以上 / 4 文字以上の一致、ルール 7 の語リストの作り方など) | docs/design/modeless.md (ルール 1〜3)・docs/design/modeless-detection.md (ルール 4〜7・保留・英単語の始まり) | 記載あり。利用者向けには例と結果 (英字になる / ならない / 直し方) だけを残す。ルール番号は使わない |
| 後置再変換の記憶の内部 (127 文字、文書の末尾との照合、途切れる条件) | docs/archive/adopted-reconversion.md | 記載あり。利用者向けには「直前に IME で入れた文字列を打ち直さずに直せる」「スペース・改行を挟むと使えない」「フォーカス移動・IME オフで忘れる」程度を残す |
| 読める文書の判定の時点 (次の run の始め・変換キー・確定アンドゥで確かめる、フォーカス移動で未判定に戻す) | docs/design/basics.md | append-input.md は「2 文字目以降の追記時に読み戻して分類する」と書いており README と食い違う。実装時にコードで今の挙動を確かめて basics.md に書き、利用者向けには「働く文書・働かない文書の例 (メモ帳・Word・Chrome / ターミナル)」と、読めない文書では何が使えないかだけを残す |
| 検証用の環境変数 (`QUICKLIME_IMPORT_DIR`・`QUICKLIME_LLM_MODEL_DIR`) | docs/development.md | dict-import.md・llm-rerank.md に記載あり。利用者向けからは外し、development.md に一覧として移す |

- 上の「残す」内容は新しく書く文言になる。README の該当箇所の文言から、内部の数値と条件を除いて短くする

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| README.md | 上記「README.md に残す内容」に書き直す |
| docs/user/input.md ほか docs/user/ の 9 ファイル | 新設 (theme.md は移動) |
| docs/theme.md → docs/user/theme.md | `git mv`。本文は変えない |
| docs/development.md | 新設 |
| docs/design/basics.md | README の節への参照 (「README の『キー割当』」など 7 か所) を docs/user/ の該当ページへのリンクに直す。冒頭の「利用者向けの操作の説明は README.md にあり」を docs/user/ に直す。「README で説明していない基本の設定」の表は「キー」「読む側」の 2 列に減らし、値と既定は settings.md へのリンクにする。読める文書の判定の時点を足す |
| docs/design/README.md | docs/theme.md へのリンクを docs/user/theme.md に直す。本書を「ドキュメントの構成」の近くに 1 行足す |
| docs/architecture.md | README・docs/theme.md への参照があれば直す |
| .claude/CLAUDE.md | 「詳細は README.md 『開発版 DLL の切替とインストール版の更新』」を docs/development.md に直す |
| docs/design/llm-rerank.md・modeless.md・modeless-detection.md・adopted-reconversion.md | 変更しない (確認済み) |

## 実装手順

1. docs/development.md を作り、README の開発者向けの節を移す
2. `git mv docs/theme.md docs/user/theme.md`
3. docs/user/ の各ページを作り、README の該当節を移す。節をまたぐ参照をリンクに直す
4. 内部の挙動を利用者向けから外す。読める文書の判定の時点はコード (tsf/src) で確かめて basics.md に書く
5. settings.md を、設定ツール・TSF 層・エンジンのコードから作る
6. README.md を書き直す
7. basics.md・docs/design/README.md・architecture.md・.claude/CLAUDE.md の参照を直す
8. リンク切れを確かめる

## 検証方法

- README の各節の内容が、新しい README・docs/user/・docs/development.md・設計書のどこかにあること
  (分割前の README の節の一覧と、移した先の対応表を作って突き合わせる)。外した内部の挙動は
  上の表の移す先にあること
- 相対リンクがすべて実在のファイル・見出しを指すこと (`docs/user/` からは `../../` など階層に注意)
- README.md・docs/theme.md への参照が、設計書の「変更対象ファイル」「実装手順」の中 (書き換えない記録) を除いて残っていないこと
  (`grep -rn 'README\|theme\.md' docs .claude --include=*.md`)
- settings.md のキーの集合が、quicklime-config.rs・tsf/src/config.cpp・engine/src/config.rs が扱うキーの集合と一致すること。
  既定値がコードと一致すること
- docs/ の .md に絵文字が無いこと

## 未決事項

- なし
