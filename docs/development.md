# 開発者向けの手順

QuicklIME のビルド・インストーラの作成・開発版の動作確認の手順。利用者向けの説明は
[README.md](../README.md) と [docs/user/](user/) にある。

## 構成

| ディレクトリ | 内容 |
|---|---|
| `tsf/` | TSF テキストサービス (C++ / in-proc COM DLL) |
| `engine/` | 変換エンジン (Rust / 常駐別プロセス)。Cargo workspace のルート |
| `engine/gui/` | 設定ツール・単語登録ツール (Rust / Slint。workspace の crate `quicklime-gui`) |
| `llm/` | LLM による候補の並べ替えの子プロセス (Rust / llama.cpp。エンジンが起動する) |
| `models/` | LLM のモデル (git管理外。`scripts/fetch-models.ps1` で取得) |
| `data/` | 設定ファイルのプリセット (AZIK 風ローマ字テーブルなど) |
| `docs/` | ドキュメント。全体の構成は [docs/architecture.md](architecture.md)、機能の領域ごとの今の仕様は [仕様書の索引](spec/README.md)、実装前の設計は [設計書の索引](design/README.md)、未着手の項目と既知の問題は [docs/roadmap.md](roadmap.md)、TSF-エンジン間プロトコルは [docs/protocol.md](protocol.md) |
| `references/` | 外部のリポジトリと取得したデータ (git管理外)。中身は下記「references/ の中身」 |
| `scripts/` | 開発用スクリプト (`dev-deploy.ps1`: デバッグ反映、`cleanup-old-binaries.ps1`: 旧DLL/exeの掃除、`fetch-models.ps1`: LLM のモデルの取得、`build-llm.ps1`: LLM の子プロセスのビルド、`fetch-ajimee.ps1`: 評価セット AJIMEE-Bench の取得、`fetch-ut-dict.ps1`: UT 辞書の取得と生成、`fetch-wiki-ngram.ps1`: 日本語版ウィキペディアのダンプの取得と後続語の統計の生成、`subset-icon-font.py`: 入力モードのアイコン用フォントのサブセット化) |

### references/ の中身

| ディレクトリ | 内容 | 使うもの |
|---|---|---|
| `references/mozc/` | Mozc (https://github.com/google/mozc) の sparse checkout。取得の手順は下記「同梱データの取得・生成」 | 開発版のエンジンの辞書ディレクトリ、インストーラの同梱、UT 辞書と後続語の統計の生成、`engine/examples/` の評価 |
| `references/mozcdic-ut/` | UT 辞書の元データとライセンス文書、生成した `dictionary_ut.txt` (`scripts\fetch-ut-dict.ps1`) | 開発版のエンジン、インストーラの同梱、評価 |
| `references/jawiki/` | 日本語版ウィキペディアのダンプの分割と `manifest.tsv`、生成した `next_word.tsv` (`scripts\fetch-wiki-ngram.ps1`) | 開発版のエンジン、インストーラの同梱、`learning_eval` |
| `references/AJIMEE-Bench/` | 評価セットの JSON と変換した `ajimee_cases.tsv` (`scripts\fetch-ajimee.ps1`) | `nbest_eval`・`llm_gen_eval` |
| `references/wordlist/` | SCOWL (https://github.com/en-wl/wordlist) | `data\english-words.txt` の生成 (手順はファイル先頭のコメント) |
| `references/corvusskk/` | CorvusSKK (https://github.com/nathancorvussolis/corvusskk) | TSF の実装を読む参考 |
| `references/Windows-classic-samples/` | Windows-classic-samples (https://github.com/microsoft/Windows-classic-samples) の `Samples/IME` (SampleIME) | TSF の実装を読む参考 |

## 開発環境

- Visual Studio 2022 (C++ によるデスクトップ開発ワークロード、Windows SDK 含む)
- Rust (stable-x86_64-pc-windows-msvc)
- Windows 11
- LLM の子プロセス (`llm/`) のビルドのみ: LLVM (`C:\Program Files\LLVM`)、Vulkan SDK
  (既定は `C:\VulkanSDK\1.4.363.0`)

## ビルド

- エンジンと設定ツール・単語登録ツール: `cd engine && cargo build`
  (`engine/` は Cargo workspace で、エンジン本体 (package `quicklime-engine`) と `engine/gui/`
  (package `quicklime-gui`) の両方を既定のビルド対象にしている。quicklime-engine.exe・
  quicklime-config.exe・quicklime-regword.exe はどれも `engine/target/` に出る)
  - 両ツールの画面は `engine/gui/ui/*.slint` に Slint (fluent スタイル) で書き、`engine/gui/build.rs` の
    `slint-build` で exe に組み込む。設定ファイルの読み書き・キー割当・アプリ設定・単語登録と辞書の
    インポートの処理は `engine/gui/src/` の lib にあり、エンジンと共有する `engine/src/import.rs` は
    `#[path]` で参照する。エンジン本体の依存に Slint は入らない
  - workspace では `windows-sys` などの feature がエンジン本体と gui でまとめられるため、gui の依存を
    変えたときやエンジン単体 (`-p quicklime-engine`) のビルドと行き来したときも、`engine/` での release
    ビルドで `quicklime-engine.exe` がリンクし直される。常用中のエンジンが `engine/target/release/`
    から動いているとロックされて失敗するので、先にエンジンを止めるか、exe をリネームして退避しておく
- エンジンと両ツールのテスト: `cd engine && cargo test` (workspace の全 crate)
- エンジン単体の動作確認 (起動中のエンジンに接続して CONVERT/CONVSEG を試す):
  `cargo run --example query -- <読み> [<読み>...]`
- TSF層 (VS付属のCMakeを使用、64bit):
  ```
  cmake -S tsf -B tsf/build -G "Visual Studio 17 2022" -A x64
  cmake --build tsf/build --config Debug
  ```
  成果物: `tsf/build/Debug/QuicklIME.dll`。ビルドの後に `icacls` で ALL APPLICATION PACKAGES (S-1-15-2-1) と
  ALL RESTRICTED APPLICATION PACKAGES (S-1-15-2-2) に読み取りと実行を許す (tsf/CMakeLists.txt の POST_BUILD)。
  スタートメニューの検索欄などの AppContainer のアプリが開発版の DLL を読み込めるようにするため
  ([runtime.md](spec/runtime.md)「DLL のファイルの権限」)
- LLM の子プロセス: `scripts\fetch-models.ps1` でモデルを `models\` に取得し (サイズと
  SHA-256 を確かめる)、`scripts\build-llm.ps1` で CPU 版 `quicklime-llm.exe` と Vulkan 版
  `quicklime-llm-vulkan.exe` を release ビルドする (成果物は `llm\target\release\`)。
  llama.cpp のビルドに要る環境変数 (LIBCLANG_PATH、CMake・Ninja、VULKAN_SDK、vcvars64) と、
  長いパスを避ける `subst` はスクリプトの中で設定する。エンジンの通常のビルドには要らない
- LLM の並べ替えの計測 (エンジンを隔離して起動し、llm_cases.tsv の左文脈で RERANK を送る):
  `cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv --llm cpu`
  (`--llm vulkan` で Vulkan 版)。`--grid` を足すと、読みの文字数の下限と統計変換の 1 位と 2 位の
  コスト差 (読み 1 文字あたり) の上限の格子で、条件を満たす件だけ並べ替えたときの正解率を表で出す
  (全件を並べ替えて測るため、エンジンに `QUICKLIME_RERANK_MIN_CHARS=0` を渡す)。
  `--no-cases` は読み・正解・候補を含む件ごとの表を出さない (実ユーザのデータを測るとき用)

## 変換精度と学習の評価

評価セットの分担と測る項目は [engine.md](spec/engine.md)「評価セット」にある。どちらの結果も git に入れない。

- `engine/examples/data/llm_cases.tsv`: N-best と LLM の並べ替えの定数の調整に使う手作りのセット
- AJIMEE-Bench: 評価だけに使うセット (定数の調整には使わない)。データは CC BY-SA 3.0 のため
  リポジトリに入れず、`scripts\fetch-ajimee.ps1` でコミット固定の JSON を `references\AJIMEE-Bench\` に
  取得し (サイズと SHA-256 を確かめる)、`ajimee_cases.tsv` (llm_cases.tsv と同じ形式。正解は `|` 区切りで
  複数) に変換する。読みに 『』… を含む問題は除く。`nbest_eval` は隔離したエンジンを起動するので、先に
  `cargo build --release` でエンジンの exe を最新にしておく
  ```
  scripts\fetch-ajimee.ps1
  cd engine
  cargo build --release
  cargo run --release --example nbest_eval -- --cases ../references/AJIMEE-Bench/ajimee_cases.tsv
  cargo run --release --example nbest_eval -- --cases ../references/AJIMEE-Bench/ajimee_cases.tsv --llm cpu
  ```
- LLM による候補の生成の計測 (実験 0c。[experiment-0c-llm-gen.md](archive/experiment-0c-llm-gen.md)):
  `llm_gen_eval` は辞書と llama.cpp をプロセス内で使い、並べ替えだけ・自由生成 (貪欲法・beam)・統計変換を
  ドラフトにした照合を、テストセットの左文脈つき・学習が空で比べる。エンジンと quicklime-llm は起動しない。
  llama.cpp を含むので、ビルドには quicklime-llm と同じ環境変数 (LIBCLANG_PATH、CMake・Ninja、vcvars64。
  Vulkan 版は VULKAN_SDK・`CMAKE_GENERATOR=Ninja` と、長いパスを避ける `subst` のドライブを `--target-dir` に
  する) が要る。手順は [experiment-0c-llm-gen-results.md](archive/experiment-0c-llm-gen-results.md) の「再現手順」
  ```
  cd engine
  cargo build --release --example llm_gen_eval --features llm-bench
  target\release\examples\llm_gen_eval --model ..\models\zenz-v3.2-xsmall-Q5_K_M.gguf --backend cpu --threads 4 --cases ..\references\AJIMEE-Bench\ajimee_cases.tsv
  ```
  `--features llm-bench-vulkan` でビルドしたものは `--backend vulkan` で測る。`--methods` で方式を絞り
  (rerank,a-greedy,a-beam,c,a+c)、`--limit` で照合の評価の回数の上限 (既定 10) を変え、`--no-cases` で件ごとの表を
  出さない
- 学習の再生評価: 実ユーザの learning_stats.tsv を `t` (最後に使った通し番号) の順に分け、学習が空のときの
  変換と、前の件を取り込んだ学習での後ろの件の変換・予測を測る。件の数の 8 割の位置にある件の `t` 以下を前、
  それより大きい件を後ろとし (同じ `t` の件は分けない)、後ろの件のうち `d - 1` が 0.05 以上の件は `d - 1` の
  度合いで前の学習にも入れる。測る行は既出・初出に分けても出す。ファイルは読むだけで、学習は
  メモリ上にだけ作る。結果は件数と割合だけを出す
  ```
  cd engine
  cargo run --release --example learning_eval -- --stats <learning_stats.tsv> [--weights "half=5000"]...
  ```
  `--next-word ..\references\jawiki\next_word.tsv` を付けると、読みなしの後続語予測 (NEXTWORD) を学習と後続語の
  統計のあり・なしの 4 つの組で測る

## インストーラのビルド

`installer\build.ps1` を実行すると `installer\output\quicklime-<版>-setup.exe` が生成される
(前提: Inno Setup 6 = `winget install -e --id JRSoftware.InnoSetup`、references/mozc の Mozc 辞書・
連接行列・品詞 ID 表・単漢字辞書・記号辞書と LICENSE (下記「同梱データの取得・生成」の手順で取得)、
`scripts\fetch-ut-dict.ps1` で生成した UT 辞書 (`references\mozcdic-ut\dictionary_ut.txt`)、
`scripts\fetch-wiki-ngram.ps1` で生成した後続語の統計 (`references\jawiki\next_word.tsv` と `manifest.tsv`)、
`scripts\build-llm.ps1` でビルドした LLM の exe、`scripts\fetch-models.ps1` で取得したモデル、
Rust の crate のライセンス表示 `LICENSE-rust-crates.txt` を生成する cargo-about =
`cargo install --locked --features cli cargo-about`)。

`installer\build.ps1` は 64bit の `tsf\build` をビルドするだけで構成はしないので、先に上記「ビルド」の
TSF 層の手順 (`cmake -S tsf -B tsf/build ...`) で構成しておく (32bit の `tsf\build32` は未構成なら自動で構成する)。
CMake と VC++ ランタイムは Visual Studio 2022 Community の既定のインストール先
(`C:\Program Files\Microsoft Visual Studio\2022\Community`) から取るので、ほかのエディションや場所では動かない。

同梱物とそのライセンスは [README.md](../README.md) の「同梱物とライセンス」にある。

## 同梱データの取得・生成

- Mozc のデータは `references\mozc\` に、固定したコミットを sparse checkout で取得する
  (blob は checkout するパスの分だけ取る)
  ```
  git clone --filter=blob:none --no-checkout https://github.com/google/mozc references/mozc
  git -C references/mozc sparse-checkout set --cone src/data/dictionary_oss src/data/single_kanji src/data/symbol
  git -C references/mozc checkout 0651fbcae495ff98f6ae17bb59e2bee0e3a79661
  ```
  使うファイルは次のもの。インストーラはどれも `dict\` 直下へコピーし、開発版のエンジンは辞書ディレクトリ
  `references\mozc\src\data\dictionary_oss\` (と、それと並ぶ `single_kanji\`・`symbol\`) から読む
  ([dictionary.md](spec/dictionary.md)「辞書ディレクトリ」)
  - Mozc 辞書 (`dict\dictionary00.txt`〜`dictionary09.txt`)、連接行列 (`dict\connection_single_column.txt`)、
    品詞 ID 表 (`dict\id.def`): `src/data/dictionary_oss/` のもの。UT 辞書と後続語の統計の生成にも使う
  - 記号辞書 (`dict\symbol.tsv`): `src/data/symbol/symbol.tsv`
  - Mozc の LICENSE (リポジトリ直下。cone の sparse checkout は直下のファイルを常に含む): インストーラが
    `LICENSE-mozc.txt` として同梱する
- 単漢字辞書 (`dict\single_kanji.tsv`) は Mozc の `src/data/single_kanji/` のもので、候補リストの
  辞書候補の後ろに単漢字を足すほか、逆変換で辞書の語で読めない漢字の読み ([engine.md](spec/engine.md)「逆変換 (REVERSE)」の「格子の語」) と、
  後続語の統計の生成 (`engine/examples/build_next_word.rs` が本文を逆変換するとき。[suggest.md](spec/suggest.md)
  「文節への分け方」) に使う
- UT 辞書 (`dict\dictionary_ut.txt`): `scripts\fetch-ut-dict.ps1` が元データをリビジョン固定で
  `references\mozcdic-ut\` に取得し (サイズと SHA-256 を確かめる)、`engine/examples/build_ut_dict.rs`
  で、Mozc 辞書と重複する語などを除いて生成する
- 後続語の統計 (`dict\next_word.tsv`): 日本語版ウィキペディアの CirrusSearch のダンプから、
  `engine/examples/build_next_word.rs` で文節の連なりの回数を数えて生成する
  ([suggest.md](spec/suggest.md)「文節の連なりの統計」)。ダンプの置き場所
  (https://dumps.wikimedia.org/other/cirrus_search_index/) は週ごとに作られ直近 4 週だけが残るので、日付は
  そこで今あるものを選ぶ。1 分割 (約 650MB) の取得に約 30 分、4 分割の生成に約 15 分かかる
  ```
  scripts\fetch-wiki-ngram.ps1 -Date <日付> -Parts (0..3)
  ```
  分割を `references\jawiki\` に取得して名前・サイズ・SHA-256 を `manifest.tsv` に書き (取得済みでサイズが一致する
  分割は取り直さない)、`references\jawiki\next_word.tsv` を生成する。開発版のエンジンは辞書ディレクトリに
  next_word.tsv が無ければこれを読む。インストーラは manifest.tsv のダンプの日付を出典の表示
  (`LICENSE-wikipedia.txt`) に書く
- モードレス入力の英単語辞書 (`dict\english-words.txt`) は SCOWL (English Speller Database、
  https://github.com/en-wl/wordlist) から生成したもの。生成の手順と条件はファイル先頭のコメントにある
- タスクバーの入力モードのアイコンは、M PLUS Rounded 1c Regular を「あ」「A」「×」の 3 字に
  サブセット化したもの (`tsf\src\Font\MPLUSRounded1c-Regular-icon.ttf`) を DLL のリソースに入れて描く。
  サブセットは `scripts\subset-icon-font.py` で Google Fonts の
  `ofl/mplusrounded1c/MPLUSRounded1c-Regular.ttf` から作る
- LLM の子プロセスは VC++ ランタイムを動的にリンクするため、再頒布可能な DLL (msvcp140・vcruntime140・
  vcruntime140_1・vcomp140) を exe と同じディレクトリに置く (ビルド環境の Visual Studio の
  `VC\Redist\MSVC\<版>\x64` から取る)

## 開発版 DLL の切替とインストール版の更新

`scripts\dev-deploy.ps1` でビルドから反映までをまとめて実行できる。

| コマンド | 動作 |
|---|---|
| `scripts\dev-deploy.ps1` | エンジンのみ反映 (release ビルド → 常用の Program Files へコピー。Program Files への書き込みが通常権限で成功する環境なら管理者権限は不要) |
| `scripts\dev-deploy.ps1 -Dll` | エンジン反映に加え、TSF DLL を Debug ビルドして開発版へ切替 (要管理者権限) |
| `scripts\dev-deploy.ps1 -Dll -Install` | エンジン反映に加え、TSF DLL を Release ビルドして常用インストール版へ直接反映 (再起動は不要。Program Files への書き込みが通常権限で成功する環境なら管理者権限は不要) |
| `scripts\dev-deploy.ps1 -Restore` | DLL を開発版からインストール版へ戻す (要管理者権限) |

DLL の反映方法は用途で使い分ける。

- **`-Dll`** (開発版切替): `tsf\build\Debug\QuicklIME.dll` に regsvr32 で排他的に切り替える。
  常用インストール版には触れない。確認が終わったら `-Restore` で必ず戻すこと
- **`-Dll -Install`** (インストール版更新): 常用インストール版の DLL 自体を新しいビルドに
  差し替える。ロード中の DLL を `.old-<日時>` にリネーム退避してから同じパスに新 DLL を
  コピーするので、レジストリ再登録も PC 再起動も不要 (インストーラでの更新は
  ロード中だと `restartreplace` により再起動待ちになる)。32bit 版 (`x86\QuicklIME.dll`) も
  同じ手順で更新する (ビルドツリーは `tsf\build32`。未構成なら自動で生成する)

手動で行う場合 (開発版切替):

```
regsvr32 tsf\build\Debug\QuicklIME.dll      # 登録
regsvr32 /u tsf\build\Debug\QuicklIME.dll   # 解除
```

インストール版と開発版は同じ CLSID を共有し、後から regsvr32 (または再インストール) した
方に登録が切り替わる。開発を終えて常用へ戻すときは、インストーラを再実行するか
`regsvr32 "%ProgramFiles%\QuicklIME\QuicklIME.dll"` で戻す。

## 注意

IME の DLL は全アプリケーションのプロセスにロードされる。開発版の動作確認は
テスト用アプリで行い、Microsoft IME へいつでも切り替えられる状態を維持すること。

### スタートメニューの検索欄 (AppContainer のアプリ) で試す

検索欄を表示する SearchHost.exe は自分で起動するプロセスではないので、環境変数での隔離が効かず、常用のエンジンと
実データにつながる。壊れたときは Win+Space で Microsoft IME に切り替え、`taskkill /IM SearchHost.exe /F` で検索欄を
作り直す (自動で起動し直す)。

1. `tasklist | grep quicklime` で古いエンジンを終了し、新しいエンジンを起動する (パイプの記述子はエンジンが作るため)
2. `scripts\dev-deploy.ps1 -Dll` で開発版に切り替え、`taskkill /IM SearchHost.exe /F` で検索欄を作り直す
3. `tasklist /M QuicklIME.dll /FI "IMAGENAME eq SearchHost.exe"` で、SearchHost に DLL が読み込まれたことを確かめてから
   検索欄で入力する

- AppContainer の TSF 層はエンジンを起動しない ([runtime.md](spec/runtime.md)「AppContainer のアプリ」)。エンジンが
  動いていなければ、メモ帳などで入力して起動しておく
- 設定のファイルの読み取りの権限は `icacls %APPDATA%\QuicklIME\config.tsv` で確かめる
  (`ALL APPLICATION PACKAGES:(R)` があること。[tools.md](spec/tools.md)「AppContainer のアプリから読むための権限」)

## 検証用の環境変数

| 環境変数 | 内容 |
|---|---|
| `QUICKLIME_IMPORT_DIR` | 取り込んだ辞書の保存先ディレクトリ (既定は `%APPDATA%\QuicklIME\imported\`) を変更する (エンジン・単語登録ツール共通。動作検証時の隔離用)。[dictionary.md](spec/dictionary.md)「インポート辞書」 |
| `QUICKLIME_LLM_MODEL_DIR` | エンジンが LLM のモデルを探すディレクトリ。設定するとほかの場所より先に探す。探す順は [runtime.md](spec/runtime.md)「exe とモデルの探し方」 |
| `QUICKLIME_RERANK_MIN_CHARS` | エンジンが並べ替える読みの最小文字数 (0 以上の整数。既定は 9、不正な値なら既定値)。起動時に 1 回だけ読む。計測用。[engine.md](spec/engine.md)「並べ替える依頼」 |
