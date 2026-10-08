# 実験 0b: LLM による順位補正の計測

入力モデル v2 (docs/design/input-model-v2.md) の段階0b。段階5 (LLM による順位補正) を
採るかどうかと、その規模を決めるための計測の仕様。

## 背景と目的

段階4で入力全体の N-best 候補を出すようにすると、文脈による同音異義語の判別
(「きょうはいしゃにいった」を「今日は医者に行った」と「今日歯医者に行った」のどちらにするか) が
候補順位の質を左右する。統計変換 (連接コスト + 文脈学習) はこれが弱い。
azooKey の Zenzai が使う zenz 系のかな漢字変換モデルで補えるかを確かめる。

本実験では次の3点を確かめる。

- 速さ: 候補バーの更新に間に合う遅延で動くか
- 資源: 常駐させてよいメモリ量か
- 精度: 統計変換の 1-best より良い順位になるか

## 決定事項

### 対象のモデル

Hugging Face の Miwa-Keita 名義で公開されている GGUF 版の zenz 系モデルを使う。

| モデル | パラメータ数 | 備考 |
|---|---|---|
| zenz-v3.2-small-gguf | 95M | Q5_K_M で約 74MB |
| zenz-v3.2-xsmall-gguf | 26M | |

- モデルファイルはリポジトリに入れず、実行時に引数でパスを渡す
- ライセンスは版によって表記が違う (v3.2 の GGUF は apache-2.0、v3.1・v2 は CC-BY-SA 4.0)。
  計測結果の記録に、使ったモデルのライセンス表記も残す
- プロンプトの書式 (読み・左文脈・出力を区切る特殊トークン、読みをカタカナにするかなど) は
  モデルカードに記載が無い。azooKey の変換器 (AzooKeyKanaKanjiConverter) の実装を読んで
  確認し、確認した書式と出典 (ファイル名・コミット) を計測結果に記録する

### 推論の実行方式

- llama.cpp を Rust バインディング (llama-cpp-2 クレートなど) でライブラリとして使う。
  候補の尤度計算に全トークンのロジットが要るため、llama-server を HTTP で呼ぶ方式は採らない
- GPU は特定のベンダーに依存させない。バックエンドは CPU と Vulkan を計測する。
  CUDA は計測の対象外とする (NVIDIA 以外の環境で動かないため)
- ビルドに必要な開発環境: CMake (VS 付属のものに PATH を通す)、LLVM (bindgen が使う
  libclang)、Vulkan SDK (Vulkan バックエンドのシェーダのコンパイル)。いずれも開発機だけに
  必要で、利用者の環境には要らない

### 実装の形

- `engine/examples/llm_bench.rs` として作る。エンジン本体 (src/) と TSF 層には組み込まない
- バインディングは optional 依存にし、cargo feature `llm-bench` (CPU のみ) と
  `llm-bench-vulkan` (Vulkan も有効) で有効にする。example には `required-features` を付け、
  通常の `cargo build` / `cargo test` ではビルドしない
- 実行例: `cargo run --release --example llm_bench --features llm-bench-vulkan -- --model <gguf> --backend vulkan --cases engine/examples/data/llm_cases.tsv`

### 計測項目

すべて release ビルドで、ウォームアップを数回行ってから、各条件 30 回以上の中央値と p95 を取る。

| 項目 | 条件 |
|---|---|
| 生成 (読み + 左文脈 → 変換結果) の遅延 | 読み 5 / 10 / 20 かな、貪欲法 |
| 順位付け (N 個の候補それぞれの対数尤度) の遅延 | N = 5 / 10、読み 10 / 20 かな。共通の接頭部 (プロンプト) の KV キャッシュを使い回す形と、使い回さない形の両方 |
| モデルの読み込み時間 | |
| 常駐時・推論中のピークの RAM と VRAM | |

上記を、バックエンド (CPU / Vulkan) とモデル (small / xsmall) の組み合わせごとに測る。
CPU はスレッド数 (2 / 4 / 物理コア数) も変えて測る。

### 精度の簡易確認

- テストセット `engine/examples/data/llm_cases.tsv`: 1行1件で「左文脈<TAB>読み<TAB>候補1|候補2|...<TAB>正解」。
  30〜50 件。同音異義語・助詞と同音の漢字・文脈で決まる語を中心にする
- 初版は実装者が典型例で作り、ユーザが普段の誤変換の例を足す
- 比べるもの:
  - 統計変換の 1-best: 起動中のエンジンに読みで CONVERT/CONVSEG を投げた結果
    (エンジンを隔離して起動する。.claude/CLAUDE.md「エンジン検証の隔離」)
  - LLM: 候補群の対数尤度による順位の1位
  - LLM: 生成の結果
- 正解率をそれぞれ出し、外れた例を一覧にする

### 出力

計測結果を `docs/design/experiment-0b-llm-results.md` に表で書く。
環境 (CPU・GPU・ドライバの版)、llama.cpp の版、モデルとライセンス表記、確認したプロンプトの書式も記す。

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| engine/Cargo.toml | optional 依存と feature、example の `required-features` |
| engine/examples/llm_bench.rs | 新規 |
| engine/examples/data/llm_cases.tsv | 新規 |
| docs/design/experiment-0b-llm-results.md | 新規 (計測結果) |

## 実装手順

1. AzooKeyKanaKanjiConverter の実装から、zenz 系のプロンプト書式を確認する
2. CPU のみでモデルを読み込み、1件の生成と1件の順位付けが動く最小の形を作る
3. 計測の枠組み (反復・中央値・p95・メモリ計測) を作る
4. Vulkan バックエンドを有効にして同じ計測を行う
5. テストセットを作り、精度の比較を行う
6. 結果を docs/design/experiment-0b-llm-results.md にまとめる

## 検証方法

- 通常の `cargo build` / `cargo test` が、feature 無しで今までどおり通ること
  (依存が増えていないこと)
- `--backend vulkan` で実際に GPU が使われていること (llama.cpp の起動ログのデバイス名、VRAM の増加)

## 判断基準 (目安)

- 順位付け N=10・読み 20 かなで p95 が 50ms 以下
- 常駐による追加メモリが 300MB 以下
- 精度が統計変換の 1-best より明らかに良いこと

CPU だけで基準を満たすなら、段階5は CPU を既定とし、GPU は任意とする方向で検討する。

## 未決事項

- 配布時の Vulkan への依存の扱い。Vulkan バックエンドを静的にリンクすると、Vulkan ランタイムの
  無い環境でエンジンが起動しなくなるおそれがある (遅延ロード・バックエンドの動的ロードなど)。
  段階5で決める
