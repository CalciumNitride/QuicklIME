# QuicklIME

Windows 用の自作日本語IME。通常のローマ字入力をベースに、変換の工夫による高速入力を目指す。

## 構成

| ディレクトリ | 内容 |
|---|---|
| `tsf/` | TSF テキストサービス (C++ / in-proc COM DLL)。フェーズ1で作成 |
| `engine/` | 変換エンジン (Rust / 常駐別プロセス) |
| `docs/` | ドキュメント。開発計画は [docs/roadmap.md](docs/roadmap.md) |
| `references/` | 参考用の外部リポジトリ (git管理外)。CorvusSKK、SampleIME |

## 開発環境

- Visual Studio 2022 (C++ によるデスクトップ開発ワークロード、Windows SDK 含む)
- Rust (stable-x86_64-pc-windows-msvc)
- Windows 11

## ビルド

- エンジン: `cd engine && cargo build`
- TSF層: (フェーズ1で追記)

## 注意

IME の DLL は全アプリケーションのプロセスにロードされる。開発版の動作確認は
テスト用アプリで行い、Microsoft IME へいつでも切り替えられる状態を維持すること。
