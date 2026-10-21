# rakukan の LLM 変換の調査メモ

作成日: 2026-10-21

rakukan (https://github.com/fukuyori/rakukan、v0.12.0、コミット `ec09244`) の LLM まわりの調査。
LLM による候補の生成 ([llm-generate.md](../design/llm-generate.md)) を検討したときの材料。

## rakukan の方式の概要

- karukan (https://github.com/togatoga/karukan、MIT) の変換エンジンを中核にした Rust 製の Windows IME。
  LLM が読みから候補を直接生成し、Mozc 系辞書の候補と合わせる (QuicklIME は統計変換の N-best を
  LLM で並べ替えるだけで、生成はしない)
- llama.cpp を別プロセス (engine-host) で動かす。CPU / Vulkan / CUDA の engine DLL を `gpu_backend` で選び、
  `auto` は cuda → vulkan → cpu の順に実際にロードを試す
- 候補数 1 なら greedy、2 以上なら beam search (beam 幅 = min(候補数, 3))
- 候補の並びは「学習 → ユーザ辞書 → システム辞書 → LLM」(v0.11.6。それまでは LLM を辞書より上にしていた)
- 生成 1 回の上限は 15 秒。変換 1 回はかなの区間ごとに生成を呼ぶ

## モデル: jinen

- karukan 用のかな漢字変換モデル。Hugging Face の togatogah/jinen-* (GGUF)。**ライセンスは CC BY-SA 4.0**
  (モデルカードの表記。zenz は Apache-2.0)。モデルカードの本文は空で、学習データ・ベースモデルの記載は無い
- v1 は GPT-2 系 (xsmall 26M / small 90M)、v2 は Qwen3 系 (xsmall 36M / small)。Q5_K_M と f16 がある。
  プロンプトの形式は v1 と v2 で共通
- プロンプト: `<文脈トークン>文脈<入力開始トークン>カタカナの読み<出力開始トークン>`
  (`crates/rakukan-engine/src/kanji/backend.rs` の `build_jinen_prompt`)。読みはカタカナにして渡す
- 同梱する場合は継承の条件 (同じライセンスでの再配布) を確かめる

## 生成の前後処理 (生成を入れるなら必要になるもの)

### リテラル保護 (`crates/rakukan-engine/src/digits.rs`)

- 読みを数字・英字・記号・かなの区間に分け、LLM にはかなの区間だけを渡す。数字・英字・記号は元の文字のまま
  残し、半角と全角の両方を候補にする (`2024ねん` → `2024年`。LLM が数字を書き換えない)
- 小型モデルは数字・英字を勝手に別の表記へ変えるため。QuicklIME の並べ替えでは、数字の書き方と補正の候補は
  すでに動かさない扱いにしている

### 出力の後処理 (`backend.rs`)

- ルビの除去: `健診(けんしん)` のように、括弧の中がかなだけなら括弧ごと消す。`(笑)` などは残す
- 破綻した候補の棄却: 読みの長さから決めた上限より長い候補と、同じ文字列の繰り返し (タンデム反復) を含む候補を
  捨てる。読み自身が繰り返し (`わかったわかった`) のときは繰り返しの判定をしない
- 自信度での棄却: beam の累積 log-prob をトークン数で割った平均で、最良の候補との差が大きい候補を捨て、
  最良の候補もしきい値を下回れば全部捨てる

### 文脈のエコー対策

小型モデルは、文脈に読みと同じひらがなの並びがあると、変換せずに文脈からコピーする (全ビームがそちらに寄り、
漢字の候補が消える)。2 段で防いでいる:

- 文脈に足すとき (`crates/rakukan-engine/src/lib.rs` の `is_context_echo_risk`): 句読点などを除いて全部
  ひらがなで、4 文字以上の確定文は文脈に入れない (未変換のまま確定した文)
- 生成のとき (`backend.rs` の `strip_echo_context`): 読みが 4 文字以上なら、読みの先頭 6 文字を含む
  8 文字以上のひらがなの並びがある文を、文脈から文単位で消す

QuicklIME は文脈を文書から読むので、ひらがなのまま確定した文も文脈に入る。並べ替え (採点) では害は小さいが、
生成では同じ問題が出うる。

## 運用面 (QuicklIME の quicklime-llm の監視と比べるとき)

- 推論が 3 回続けて失敗したらホストが自分で終了し、次の要求で起動し直す。5 分以内に繰り返したら LLM を
  諦めて辞書だけで動く (`crates/rakukan-engine-rpc/src/health.rs`)。GPU のドライバ更新・スリープ復帰・TDR で
  GPU デバイスだけが無効になり、推論が即失敗し続ける事例に対応したもの
- 同じ変換が 30 秒以上終わらないときも同じ扱いにする
- 未着手: beam search の KV キャッシュの再利用 (rakukan の Issue #83)

## QuicklIME で検討するときの論点

- 生成の位置付け: 統計変換の候補に足すのか、zenz の並べ替えの前に候補の数を増やすのか
- モデル: zenz (生成もできる) と jinen を `nbest_eval --llm` で比べる。jinen はライセンスの条件が違う
- 前後処理: リテラル保護・破綻した候補の棄却・エコー対策は、どのモデルでも要る
