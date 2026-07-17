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

## インストーラでの導入

`installer\build.ps1` を実行すると `installer\output\quicklime-<版>-setup.exe` が生成される
(前提: Inno Setup 6 = `winget install -e --id JRSoftware.InnoSetup`、references/mozc の辞書)。

- 配置先は `%ProgramFiles%\QuicklIME\` (64bit DLL + エンジン・設定・単語登録の exe +
  辞書 `dict\`)。32bit アプリ用の DLL は `x86\` に入り、両方が IME として登録される
- 初回インストールは再起動不要。更新時は DLL がロード中のため再起動を求められることがある
- アンインストールしてもユーザデータ (`%APPDATA%\QuicklIME\` の設定・ユーザ辞書・学習) は残る
- 同梱辞書は Mozc (BSD ライセンス) のもの。ライセンス文書 LICENSE-mozc.txt を同梱している

登録後、Win+Space で「QuicklIME」を選択して使用する。

## 開発版の登録と解除 (要管理者権限)

```
regsvr32 tsf\build\Debug\QuicklIME.dll      # 登録
regsvr32 /u tsf\build\Debug\QuicklIME.dll   # 解除
```

インストール版と開発版は同じ CLSID を共有し、後から regsvr32 (または再インストール) した
方に登録が切り替わる。開発を終えて常用へ戻すときは、インストーラを再実行するか
`regsvr32 "%ProgramFiles%\QuicklIME\QuicklIME.dll"` で戻す。

## ローマ字テーブルのカスタマイズ

`%APPDATA%\QuicklIME\romaji.tsv` (UTF-8) を置くと、既定のローマ字テーブルへ
追加・上書きされる。書式は 1行1エントリ「ローマ字<TAB>かな」。`#` 始まりの行は
コメント、かな欄が空の行は既定エントリの削除。反映は各アプリの再起動後。

AZIK 風拡張 (撥音拡張「かん」=kz、二重母音拡張「こう」=kp など) のプリセットを
[data/romaji-azik.tsv](data/romaji-azik.tsv) に用意している (インストール版では
`%ProgramFiles%\QuicklIME\presets\` にも同梱)。使う場合はこれを
`%APPDATA%\QuicklIME\romaji.tsv` へコピーする。

```powershell
# PowerShell の場合 (%APPDATA% は展開されないので $env:APPDATA を使う)
copy data\romaji-azik.tsv $env:APPDATA\QuicklIME\romaji.tsv
```

## 注意

IME の DLL は全アプリケーションのプロセスにロードされる。開発版の動作確認は
テスト用アプリで行い、Microsoft IME へいつでも切り替えられる状態を維持すること。
