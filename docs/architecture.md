# QuicklIME のアーキテクチャ

## プロジェクト概要

- **QuicklIME**: Windows 用の自作日本語IME
- **目的**: 常用に耐える実用IMEを作ること（学習目的ではない）
- **入力方式**: 通常のローマ字入力。変換を工夫して様々なシーンでの高速入力を目指す

機能ごとの設計は [docs/design/README.md](design/README.md) (設計書の索引)、未着手の項目と既知の問題は
[docs/roadmap.md](roadmap.md) にある。

## アーキテクチャ

Mozc / Google日本語入力と同じ「薄いクライアント + 別プロセスエンジン」構成を採用する。

| 層 | 言語 | 形態 | 役割 |
|---|---|---|---|
| TSF層 | C++ | in-proc COM DLL | Windows に IME として登録、キー捕捉、ローマ字→かな変換、文書への入力と composition 管理、候補描画 |
| 変換エンジン | Rust | 常駐別プロセス | かな漢字変換、予測、学習 |
| LLM (任意) | Rust (llama.cpp) | エンジンの子プロセス | 入力全体の候補の並べ替え (zenz 系モデル) |
| 設定・単語登録ツール | Rust | 別実行ファイル | 設定画面、ユーザ辞書管理 |

- TSF 層とエンジンは named pipe 上の行ベース TSV でやり取りする。プロトコルは
  [docs/protocol.md](protocol.md)
- エンジンに接続できないときも、TSF 層だけでかなの入力を続けられる (変換候補はかなのみになる)

### この構成を選んだ理由

- TSF のテキストサービスは in-proc COM DLL であり、全アプリのプロセスにロードされるため、軽量・堅牢な C++ 以外の選択肢が実質ない
- エンジンを別プロセスにすることで:
  - 学習データを全アプリで共有できる
  - エンジンがクラッシュしてもホストアプリを巻き込まない
  - 32bit/64bit 問題がエンジン側では消える（DLL は両アーキテクチャ分必要）
  - エンジンの言語を自由に選べる → 品質重視で Rust を選択
- 検討した代替案: 全部C++（メモリ安全性とイテレーション速度で見送り）、C#エンジン（品質重視のため見送り）

## 参考リンク

- Mozc: https://github.com/google/mozc
- CorvusSKK: https://github.com/nathancorvussolis/corvusskk
- Windows-classic-samples (SampleIME): https://github.com/microsoft/Windows-classic-samples
- ime-rs (SampleIME の Rust 移植例): https://github.com/microsoft/ime-rs
- Mozc の履歴予測実装: https://github.com/google/mozc/blob/master/src/prediction/user_history_predictor.cc
- Zenzai (ニューラルかな漢字変換) 解説: https://knowledge.sakura.ad.jp/42901/
- AZIK 総合解説書 (拡張ローマ字): http://www1.vecceed.ne.jp/~bemu/azik/azikinfo.htm
