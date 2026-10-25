# LLM による順位補正

入力モデル v2 (docs/archive/input-model-v2.md) の段階5。段階4 (docs/archive/nbest.md) の
入力全体の N-best を、zenz 系のかな漢字変換モデルで並べ替え、結果を候補バーへ非同期に反映する。

## 背景と目的

統計変換 (連接コスト + 文脈学習) は、文脈による同音異義語の判別が弱い。段階0b の計測
(docs/archive/experiment-0b-llm-results.md) では、手作りの候補群での順位1位の正解率が、
統計 1-best の 50% に対して LLM は 84〜88% だった。段階4の N-best は、正解が上位10件に入る割合が
94% あり、並べ替えで順位を上げられる余地が大きい。

本段階の目的は次のとおり。

- N-best の上位を LLM の尤度で並べ替え、文脈に合う候補を先頭に出す
- 打鍵を止めない (推論は非同期で行い、結果が出たら候補バーを作り直す)
- LLM が使えない・無効な環境では、今の統計変換だけで同じように動く

## 決定事項

### 役割

[docs/spec/engine.md](../spec/engine.md)「LLM の並べ替え」と「LLM による候補の生成」へ移した

### プロセス構成

- llama.cpp は別プロセス `quicklime-llm.exe` で動かす。総論の「エンジンプロセス内で動かす」は
  本書の決定で置き換える
  - Vulkan 版は vulkan-1.dll に依存し、GPU ドライバの無い環境では起動できない。
    llama.cpp の異常終了も含めて、エンジン本体を巻き込まないため
  - エンジンの通常のビルドに LLVM・CMake・Vulkan SDK を要らなくするため
- CPU 版 `quicklime-llm.exe` と Vulkan 版 `quicklime-llm-vulkan.exe` の2つを作る
- `quicklime-llm` は独立したクレート `llm/` (リポジトリ直下) にする。エンジンのクレートは
  llama.cpp に依存しない。0b の example (engine/examples/llm_bench.rs と feature `llm-bench`) は
  そのまま残す
- エンジンは子プロセスとして起動し、標準入出力の行プロトコル (後述) でやり取りする。
  子プロセスとのやり取りは、エンジン内の専用スレッド (LLM ワーカー) 1本だけが行う

### バックエンドとモデル

[docs/spec/engine.md](../spec/engine.md)「モデルと尤度」へ移した (起動と切り替えは次の 1 項)

- Vulkan 版が起動できない・初期化に失敗したときは、CPU 版と xsmall に切り替えて動く
  (設定は書き換えない。エンジンを再起動するか設定を読み直すと、再び Vulkan 版から試す)

### モデルの配布

- インストーラに両方のモデルを同梱する ({app}\models\)。ライセンス表記 (モデルの apache-2.0、
  llama.cpp の MIT) も同梱する
- quicklime-llm の exe は VC++ ランタイムを動的にリンクする (llama.cpp が静的 CRT でリンクできないため)。
  再頒布可能な DLL (msvcp140・vcruntime140・vcruntime140_1・vcomp140) を exe と同じディレクトリに
  同梱する (app-local 配置)。DLL はビルド環境の Visual Studio の再頒布用フォルダ
  (VC\Redist\MSVC\<版>\x64) から取り、installer\build.ps1 が staging に入れる
- モデルファイルはリポジトリに入れない。取得スクリプト `scripts\fetch-models.ps1` が、0b で記録した
  リビジョンのファイルを `models\` (リポジトリ直下、git 管理外) に取得し、サイズとハッシュを確かめる
- エンジンのモデルの探し方: 環境変数 `QUICKLIME_LLM_MODEL_DIR` → exe と同じディレクトリの
  `models\` → 開発レイアウト (リポジトリ直下の `models\`)。llm の exe も同じ規則
  (exe と同じディレクトリ → 開発レイアウトの `llm\target\release\`) で探す。release を優先するのは
  FindExePath と同じ

### 左文脈

[docs/spec/engine.md](../spec/engine.md)「左文脈」へ移した

### 並べ替えの規則

[docs/spec/engine.md](../spec/engine.md)「並べ替える依頼」と「並べ替えの規則」へ移した

### 非同期の流れ

[docs/spec/engine.md](../spec/engine.md)「依頼と結果」と [docs/spec/suggest.md](../spec/suggest.md)「LLM の並べ替えの結果の反映」へ移した

### プロトコル (TSF 層 - エンジン)

[docs/protocol.md](../protocol.md)「RERANK / RERANKGET」と [docs/spec/engine.md](../spec/engine.md)「依頼と結果」へ移した

### プロトコル (エンジン - quicklime-llm)

標準入出力で1行1要求、UTF-8、タブ区切り。

```
起動時: quicklime-llm.exe --model <gguf> --threads <N>
        → 読み込みと試しの推論に成功したら READY\n、失敗したら ERR\t<メッセージ>\n を出して終了
要求:   SCORE\t<文脈>\t<読み>\t<候補1>\t<候補2>...\n   (文脈・読みは前処理済み、読みはカタカナ)
成功:   OK\t<対数尤度1>\t<対数尤度2>...\n
失敗:   ERR\t<メッセージ>\n

要求:   GENERATE\t<文脈>\t<読み>\n   (文脈・読みは SCORE と同じ。貪欲法で表記を生成する)
成功:   OK\t<表記>\n   (EOG トークンで止まらず上限に達した・UTF-8 として正しくないときは表記が空)
失敗:   ERR\t<メッセージ>\n
```

- GENERATE は Vulkan 版で動いているときに、SCORE の応答の後に送る ([docs/spec/engine.md](../spec/engine.md)「生成」)。
  応答待ちと異常の判定は SCORE と同じ

- エンジンは READY を受け取るまで RERANK に OK\t0 を返す (読み込み中は統計の並び)
- quicklime-llm は READY を出す前に、試しの SCORE を3通り (候補1件の短い入力、`RERANK_TOP` 件の
  短い入力、`RERANK_TOP` 件で各候補が長く、合計のトークン数が上限近くになる入力) 行う
  - Vulkan 版は、ドライバのシェーダキャッシュが無い状態だと、処理の大きさの違う形を初めて使うたびに
    数秒〜十数秒かかる (計測で 6.8 秒・16 秒)。その時間をなるべく依頼の応答待ちに持ち込まないため。
    READY を待つ時間には上限を設けない (その間は統計の並びで動き、設定の読み直しで中断できる)
- 応答待ちと異常の判定は分ける。試しの推論で覆えなかった形の初回の遅さで子プロセスを終了させると、
  キャッシュが残らず同じ失敗を繰り返すため
  - 1要求の応答が `LLM_TIMEOUT_MS` (2000ms) を超えたら、その依頼は NONE にする
    (TSF 層には統計の並びのまま)。子プロセスは終了させず、応答を待ち続ける。
    応答が遅れて届いたら捨て、次の依頼から通常に戻る
  - `LLM_TIMEOUT_MS` を超えて応答を待っている間に来た依頼は、子プロセスへ送らずに NONE にする
    (応答が届いた後の依頼から、また子プロセスへ送る)
  - 1要求の応答が `LLM_HANG_MS` (60000ms) を超えた・子プロセスが終了した・応答が不正なときは、
    子プロセスを終了して、`LLM_RESTART_COOLDOWN_MS` (10s) の後に起動し直す。
    続けて3回失敗したら、設定を読み直すまで起動しない
- 子プロセスはエンジンの起動時 (`llm` が有効なとき) と、RELOADCONFIG で `llm` 系の設定が
  変わったときに起動・終了する。エンジンが終了したら子プロセスも終了させる
  (標準入力が閉じたら子プロセスは終了する。加えてジョブオブジェクトで巻き込む)
- 子プロセスのウィンドウは出さない (CREATE_NO_WINDOW)

### 設定

[docs/spec/engine.md](../spec/engine.md)「設定」へ移した (設定ツールの画面は次の 1 項)

- 設定ツールに「AI で候補を並べ替える」チェックボックスと、バックエンドの選択
  (CPU / GPU (Vulkan)) を足す

### ビルド

- `scripts\build-llm.ps1` で CPU 版と Vulkan 版をビルドする。0b の再現手順の環境変数
  (LIBCLANG_PATH、CMake・Ninja の PATH、VULKAN_SDK、CMAKE_GENERATOR=Ninja、vcvars64) と、
  長いパスを避ける `subst` をスクリプトにまとめる
- インストーラのビルド (installer\build.ps1) は、quicklime-llm の2つの exe とモデル・ライセンスを
  staging に入れる。llm の exe が無ければビルドを止める

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| llm/ (新規クレート) | quicklime-llm (モデルの読み込み、SCORE、1バッチ評価、プロンプトの組み立て)。feature `vulkan` で Vulkan 版 |
| engine/src/llm.rs (新規) | LLM ワーカー (子プロセスの起動・監視・再起動、依頼の置き換え、文脈の前処理、並べ替えの規則) |
| engine/src/convert.rs | N-best の候補に保護の印 (学習・ユーザ辞書語・英字) を付ける |
| engine/src/main.rs | RERANK・RERANKGET、接続ごとの依頼と結果、起動時と RELOADCONFIG での子プロセスの管理 |
| engine/src/config.rs | `llm`・`llm_backend` |
| engine/src/bin/quicklime-config.rs | チェックボックスとバックエンドの選択 |
| engine/examples/nbest_eval.rs | `--llm` で並べ替え後の順位を計測する (左文脈を使う) |
| tsf/src/engine_client.h, tsf/src/engine_client.cpp | `Rerank`・`RerankGet` (live の呼び出し・旧エンジン判定) |
| tsf/src/text_service.h, tsf/src/text_service.cpp | 左文脈 (run 開始時の読み取り、追記のみの文書の履歴、部分採用の後)、タイマーと結果の反映、変換キーでの結果の利用 |
| tsf/src/text_service_direct.cpp | run 開始時の左文脈の読み取り、追記のみの文書での履歴の更新と破棄 |
| tsf/src/edit_session.h, tsf/src/edit_session.cpp | run の前の文字列を読む同期 edit session |
| tsf/src/config.h, tsf/src/config.cpp | `llm` |
| scripts/fetch-models.ps1, scripts/build-llm.ps1 (新規) | モデルの取得、llm のビルド |
| installer/build.ps1, installer/installer.iss | llm の exe・モデル・ライセンスの同梱 |
| .gitignore | `/models/` |
| docs/protocol.md | RERANK・RERANKGET |
| docs/archive/input-model-v2.md | LLM の決定事項 (別プロセス、構成) と段階表 (本書の作成時に済ませる) |
| README.md | LLM の説明、設定、ビルド手順 |

## 実装手順

各手順の後にビルドとテストが通り、既存の動作が壊れていないことを確かめる。

1. モデルの取得スクリプトと `.gitignore`
2. llm クレート: 読み込み・SCORE・1バッチ評価。llm_cases.tsv の候補群で 0b と同じ順位1位になることを確かめる
3. ビルドスクリプト (CPU 版・Vulkan 版)
4. エンジン: 候補の保護の印、LLM ワーカー、並べ替えの規則、RERANK・RERANKGET、設定。単体テスト
   (子プロセスは差し替えられる形にし、テストでは偽の子プロセスか関数で代用する)
5. エンジン: `nbest_eval --llm` で計測し、定数を調整する
6. TSF: 左文脈の取得 (読める文書・追記のみの文書・部分採用の後)
7. TSF: RERANK・タイマー・結果の反映・変換キーでの利用
8. 設定ツール
9. インストーラ
10. ドキュメントの更新 (protocol.md、README.md)

## 検証方法

### 自動テスト

- 並べ替えの規則: 保護する候補 (学習・ユーザ辞書語・英字) の位置が動かない。保護しない候補だけが
  尤度順に詰め直される。`RERANK_TOP` より後ろは動かない
- 文脈の前処理: 先頭の空白、40 文字への切り詰め、半角スペース → U+3000
- 依頼の置き換え: 同じ接続の待ち中の依頼が新しいものに置き換わる。古い ID は NONE
- 子プロセスの異常: 応答なし・終了・不正な応答で再起動し、続けて3回で止まる。READY 前は OK\t0
- RERANK・RERANKGET の応答形式、`llm` 0 で OK\t0

### 計測 (`nbest_eval --llm`)

- llm_cases.tsv の左文脈と読みで、並べ替え後に正解が1位・上位3件に入る割合を、
  並べ替え前 (段階4: 1位 25/50、上位3件 43/50) と比べる。CPU + xsmall と Vulkan + small の両方
- `RERANK_MIN_CHARS` のしきい値は `--grid` で、AJIMEE-Bench と実ユーザの句から作ったセット
  (リポジトリに入れない) の 1 位の割合を見て決める
- RERANK から DONE までの時間 (中央値・p95)。目安は 0b の判断基準と同じく p95 50ms 以下
- 定数 (`RERANK_TOP` など) を変えたら本書の記述も合わせる

### 実機テスト

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。
config.tsv は `QUICKLIME_CONFIG_FILE` で隔離したものを使い、`debug_log` を設定する。
エンジンは release もビルドし直し、テスト前に古いエンジンと quicklime-llm のプロセスを終了する。

| # | 環境 | 設定・操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | `llm 1`。「熱が出て」と確定した後に `kyouhaisyaniitta` | 統計の並びのバーが出た後、短い間で並べ替えたバーに変わる (文脈に合う候補が先頭) |
| 2 | メモ帳 | 「歯が痛くて」と確定した後に 1 と同じ読み | 1 と違う候補が先頭に来る (「今日歯医者に行った」など) |
| 3 | メモ帳 | 前の行に文章を書いて改行し、行頭から入力 | 前の行は文脈に使われない (debug_log で文脈の長さが 0 と分かる。文脈そのものはログに出ない) |
| 4 | メモ帳 | 入力中に Tab でバーの候補を選び、そのまま待つ | 選んだ候補が動かない (並べ替えを捨てる) |
| 5 | メモ帳 | 素早く打ち続ける | 打鍵が遅れない。バーは打鍵に追従し、古い読みの並べ替えが出ない |
| 6 | メモ帳 | 並べ替えが出た後に変換キー | 縦の候補リストも並べ替えた順になる |
| 7 | メモ帳 | 読み全体を学習した読み (nbest.md のテスト3) | 学習した表記が先頭のまま動かない |
| 8 | メモ帳 | ユーザ辞書の語を含む入力、英字を含む入力 | その候補の位置が並べ替えで変わらない |
| 9 | WezTerm | 「熱が出て」を確定した後に 1 と同じ読み | IME の確定履歴が文脈に使われ、1 と同じ並びになる。Enter の後は文脈なし |
| 10 | Chrome の textarea | 1・2 と同じ操作 | 1・2 と同じ |
| 11 | メモ帳 | `llm_backend vulkan` | GPU 版が起動し、並べ替えが出る (タスクマネージャーで quicklime-llm-vulkan.exe) |
| 12 | メモ帳 | `llm_backend vulkan` で、Vulkan 版の exe を退避して起動できなくする | CPU 版に切り替わって並べ替えが出る |
| 13 | メモ帳 | 入力中に quicklime-llm をタスクマネージャーで終了 | 入力は止まらず、統計の並びで動く。しばらくして再起動し、並べ替えが戻る |
| 14 | メモ帳 | `llm 0` | quicklime-llm が起動せず、段階4と同じ動き |
| 15 | 設定ツール | チェックボックスとバックエンドを切り替えて保存 | config.tsv に反映され、子プロセスが起動・終了・切り替わる |
| 16 | インストーラ | インストールして `llm 1` | 同梱のモデルで並べ替えが出る。アンインストールで exe・モデルが消える |

## 未決事項

- 統計の並びから並べ替えた並びへ変わるときのバーの見え方 (チラつき・並びが変わることへの違和感)。
  実機テストで気になれば対処を検討する
