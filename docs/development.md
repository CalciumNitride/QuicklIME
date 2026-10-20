# 開発者向けの手順

QuicklIME のビルド・インストーラの作成・開発版の動作確認の手順。利用者向けの説明は
[README.md](../README.md) と [docs/user/](user/) にある。

## 構成

| ディレクトリ | 内容 |
|---|---|
| `tsf/` | TSF テキストサービス (C++ / in-proc COM DLL) |
| `engine/` | 変換エンジン (Rust / 常駐別プロセス) |
| `llm/` | LLM による候補の並べ替えの子プロセス (Rust / llama.cpp。エンジンが起動する) |
| `models/` | LLM のモデル (git管理外。`scripts/fetch-models.ps1` で取得) |
| `data/` | 設定ファイルのプリセット (AZIK 風ローマ字テーブルなど) |
| `docs/` | ドキュメント。全体の構成は [docs/architecture.md](architecture.md)、機能ごとの設計書は [設計書の索引](design/README.md)、未着手の項目と既知の問題は [docs/roadmap.md](roadmap.md)、TSF-エンジン間プロトコルは [docs/protocol.md](protocol.md) |
| `references/` | 参考用の外部リポジトリ (git管理外)。CorvusSKK、SampleIME |
| `scripts/` | 開発用スクリプト (`dev-deploy.ps1`: デバッグ反映、`cleanup-old-binaries.ps1`: 旧DLL/exeの掃除、`fetch-models.ps1`: LLM のモデルの取得、`build-llm.ps1`: LLM の子プロセスのビルド) |

## 開発環境

- Visual Studio 2022 (C++ によるデスクトップ開発ワークロード、Windows SDK 含む)
- Rust (stable-x86_64-pc-windows-msvc)
- Windows 11
- LLM の子プロセス (`llm/`) のビルドのみ: LLVM (`C:\Program Files\LLVM`)、Vulkan SDK
  (既定は `C:\VulkanSDK\1.4.363.0`)

## ビルド

- エンジン: `cd engine && cargo build`
  (quicklime-engine.exe 本体に加え、`src/bin/` の quicklime-config.exe /
  quicklime-regword.exe も一緒にビルドされる)
- エンジンのテスト: `cd engine && cargo test`
- エンジン単体の動作確認 (起動中のエンジンに接続して CONVERT/CONVSEG を試す):
  `cargo run --example query -- <読み> [<読み>...]`
- TSF層 (VS付属のCMakeを使用、64bit):
  ```
  cmake -S tsf -B tsf/build -G "Visual Studio 17 2022" -A x64
  cmake --build tsf/build --config Debug
  ```
  成果物: `tsf/build/Debug/QuicklIME.dll`
- LLM の子プロセス: `scripts\fetch-models.ps1` でモデルを `models\` に取得し (サイズと
  SHA-256 を確かめる)、`scripts\build-llm.ps1` で CPU 版 `quicklime-llm.exe` と Vulkan 版
  `quicklime-llm-vulkan.exe` を release ビルドする (成果物は `llm\target\release\`)。
  llama.cpp のビルドに要る環境変数 (LIBCLANG_PATH、CMake・Ninja、VULKAN_SDK、vcvars64) と、
  長いパスを避ける `subst` はスクリプトの中で設定する。エンジンの通常のビルドには要らない
- LLM の並べ替えの計測 (エンジンを隔離して起動し、llm_cases.tsv の左文脈で RERANK を送る):
  `cargo run --release --example nbest_eval -- --cases examples/data/llm_cases.tsv --llm cpu`
  (`--llm vulkan` で Vulkan 版)

## インストーラのビルド

`installer\build.ps1` を実行すると `installer\output\quicklime-<版>-setup.exe` が生成される
(前提: Inno Setup 6 = `winget install -e --id JRSoftware.InnoSetup`、references/mozc の辞書と
単漢字辞書 (`git -C references/mozc sparse-checkout add src/data/single_kanji` で取得)、
`scripts\fetch-ut-dict.ps1` で生成した UT 辞書 (`references\mozcdic-ut\dictionary_ut.txt`)、
`scripts\build-llm.ps1` でビルドした LLM の exe、`scripts\fetch-models.ps1` で取得したモデル)。

同梱物とそのライセンスは [README.md](../README.md) の「同梱物とライセンス」にある。

## 同梱データの取得・生成

- 単漢字辞書 (`dict\single_kanji.tsv`) は Mozc の `src/data/single_kanji/` のもので、候補リストの
  辞書候補の後ろに単漢字を足すのに使う
- UT 辞書 (`dict\dictionary_ut.txt`): `scripts\fetch-ut-dict.ps1` が元データをリビジョン固定で
  `references\mozcdic-ut\` に取得し (サイズと SHA-256 を確かめる)、`engine/examples/build_ut_dict.rs`
  で、Mozc 辞書と重複する語などを除いて生成する
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
| `scripts\dev-deploy.ps1` | エンジンのみ反映 (release ビルド → 常用の Program Files へコピー) |
| `scripts\dev-deploy.ps1 -Dll` | エンジン反映に加え、TSF DLL を Debug ビルドして開発版へ切替 (要管理者権限) |
| `scripts\dev-deploy.ps1 -Dll -Install` | エンジン反映に加え、TSF DLL を Release ビルドして常用インストール版へ直接反映 (管理者権限・再起動とも不要) |
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

## 検証用の環境変数

| 環境変数 | 内容 |
|---|---|
| `QUICKLIME_IMPORT_DIR` | 取り込んだ辞書の保存先ディレクトリ (既定は `%APPDATA%\QuicklIME\imported\`) を変更する (エンジン・単語登録ツール共通。動作検証時の隔離用)。[dict-import.md](design/dict-import.md) |
| `QUICKLIME_LLM_MODEL_DIR` | エンジンが LLM のモデルを探すディレクトリ。設定するとほかの場所より先に探す。探す順は [llm-rerank.md](design/llm-rerank.md) |
