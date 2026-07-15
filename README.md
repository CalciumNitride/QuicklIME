# QuicklIME

Windows 用の自作日本語IME。通常のローマ字入力をベースに、変換の工夫による高速入力を目指す。

## 構成

| ディレクトリ | 内容 |
|---|---|
| `tsf/` | TSF テキストサービス (C++ / in-proc COM DLL)。フェーズ1で作成 |
| `engine/` | 変換エンジン (Rust / 常駐別プロセス) |
| `data/` | 設定ファイルのプリセット (AZIK 風ローマ字テーブルなど) |
| `docs/` | ドキュメント。開発計画は [docs/roadmap.md](docs/roadmap.md) |
| `references/` | 参考用の外部リポジトリ (git管理外)。CorvusSKK、SampleIME |

## 開発環境

- Visual Studio 2022 (C++ によるデスクトップ開発ワークロード、Windows SDK 含む)
- Rust (stable-x86_64-pc-windows-msvc)
- Windows 11

## ビルド

- エンジン: `cd engine && cargo build`
- TSF層 (VS付属のCMakeを使用):
  ```
  cmake -S tsf -B tsf/build -G "Visual Studio 17 2022" -A x64
  cmake --build tsf/build --config Debug
  ```
  成果物: `tsf/build/Debug/QuicklIME.dll`

## IME の登録と解除 (要管理者権限)

```
regsvr32 tsf\build\Debug\QuicklIME.dll      # 登録
regsvr32 /u tsf\build\Debug\QuicklIME.dll   # 解除
```

登録後、Win+Space で「QuicklIME」を選択して使用する。

## ローマ字テーブルのカスタマイズ

`%APPDATA%\QuicklIME\romaji.tsv` (UTF-8) を置くと、既定のローマ字テーブルへ
追加・上書きされる。書式は 1行1エントリ「ローマ字<TAB>かな」。`#` 始まりの行は
コメント、かな欄が空の行は既定エントリの削除。反映は各アプリの再起動後。

AZIK 風拡張 (撥音拡張「かん」=kz、二重母音拡張「こう」=kp など) のプリセットを
[data/romaji-azik.tsv](data/romaji-azik.tsv) に用意している。使う場合はこれを
`%APPDATA%\QuicklIME\romaji.tsv` へコピーする。

```powershell
# PowerShell の場合 (%APPDATA% は展開されないので $env:APPDATA を使う)
copy data\romaji-azik.tsv $env:APPDATA\QuicklIME\romaji.tsv
```

## 注意

IME の DLL は全アプリケーションのプロセスにロードされる。開発版の動作確認は
テスト用アプリで行い、Microsoft IME へいつでも切り替えられる状態を維持すること。
