# 文節の連なりの統計による後続語の予測

## 背景と目的

- 読みなしの後続語予測 (NEXTWORD、[next-word-prediction.md](../archive/next-word-prediction.md)) の材料は文脈の句の学習だけで、
  学習の無い前文脈では何も出ない。使い始めや、初めて確定した語の後では働かない
- Mozc 辞書には語の連接の統計が無く、zenz は読みなしの生成に向かない。公開されている日本語の n-gram
  (Google 日本語 N-gram など) は配布の条件が厳しく同梱できない
- 日本語 Wikipedia の本文から文節の連なりを数えた表を作って同梱し、学習の無い前文脈でも NEXTWORD が
  候補を出すようにする

## 決定事項

### A. コーパス

[docs/spec/suggest.md](../spec/suggest.md)「コーパス」へ移した

### B. 文節への分け方

[docs/spec/suggest.md](../spec/suggest.md)「文節への分け方」へ移した

### C. 表

[docs/spec/suggest.md](../spec/suggest.md)「表」へ移した

### D. 生成

[docs/spec/suggest.md](../spec/suggest.md)「生成」へ移した

### E. エンジン

[docs/spec/suggest.md](../spec/suggest.md)「候補」(後続語の予測) と「エンジンでの読み込み」へ移した

### F. 同梱

- `installer/build.ps1` で `next_word.tsv` を `dict\` に入れる。無ければ UT 辞書と同じくビルドを止める
- 出典とライセンスを書いた `LICENSE-wikipedia.txt` を同梱する: 日本語版ウィキペディアの本文から文節の連なりの
  回数を数えて生成したこと、取得元、ダンプの日付、CC BY-SA 4.0 (ライセンスの URL)。README.md の
  「同梱物とライセンス」にも 1 行足す

### 範囲

[docs/spec/suggest.md](../spec/suggest.md)「候補」(後続語の予測) へ移した

### 採らなかった案

[docs/spec/suggest.md](../spec/suggest.md)「コーパス」「候補」(後続語の予測)「エンジンでの読み込み」へ移した

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| scripts/fetch-wiki-ngram.ps1 | 新規。ダンプの取得、manifest.tsv、生成ツールの実行 |
| engine/examples/build_next_word.rs | 新規。逆変換による文節の連なりの集計と next_word.tsv の書き出し (単体テスト付き) |
| engine/Cargo.toml | `serde_json` を `[dev-dependencies]` に足す。`build_next_word` の `[[example]]` (test = true) |
| engine/src/reverse.rs | 付属語かどうかも返す逆変換の版 |
| engine/src/predict.rs | 統計の表 (型・読み込み・引き当て)、`next_words` に表を足す |
| engine/src/main.rs | next_word.tsv の場所の決定、別スレッドの読み込み、NEXTWORD で表を渡す |
| engine/examples/learning_eval.rs | NEXTWORD の評価 (下記) と `--next-word` |
| installer/build.ps1 | next_word.tsv と LICENSE-wikipedia.txt の同梱 |
| docs/protocol.md | NEXTWORD の説明に統計の表を足す |
| docs/archive/next-word-prediction.md | 予測の材料に本書へのリンクを足す |
| docs/design/eval-sets.md | NEXTWORD の評価を足す |
| docs/development.md | next_word.tsv の生成の手順 |
| README.md | 読みなしの予測の説明、同梱物とライセンス |
| docs/design/README.md | 本書の 1 行 |
| docs/roadmap.md | 本項目を消し、前文脈つきの予測への統計の利用を未着手に足す |

## 実装手順

1. reverse.rs: 付属語かどうかも返す版 (単体テスト付き)
2. build_next_word.rs と Cargo.toml: 区切り・文節へのまとめ・数え方・表の書き出し (単体テストは小さな辞書と
   手書きの本文で行う)
3. fetch-wiki-ngram.ps1: 取得と manifest.tsv。1 分割で生成し、所要時間と表の大きさを確かめる
4. 全分割 (または時間の許す分割) で生成し、表の大きさと読み込みの時間から未決事項を決める
5. predict.rs・main.rs: 表の読み込みと NEXTWORD (単体テスト付き)
6. learning_eval.rs: NEXTWORD の評価。統計あり・なしで測る
7. installer/build.ps1 と LICENSE-wikipedia.txt
8. ドキュメントの更新

## 検証方法

### 単体テスト

- 付属語かどうかも返す逆変換の版で、語の列が今の `reverse` と同じになり、助詞が付属語になる
- 生成: 記号・英字で区切った連続をまたいで組を数えない。付属語が前の語にまとまる。数字を含む組と、
  付属語だけの前・次の文節と、未知語を含む前・次の文節を数えない。最小回数と上位件数で切り詰める。文脈の 16 文字の切り詰め
- NEXTWORD: 学習 → 文節の表 → バックオフの表の順に並び、表記の重複が除かれ、`max_predictions` で切れる
- 表が無い (読み込み前・ファイルなし) とき、今と同じ結果になる
- `suggest 0` で候補ゼロ。`learning 0` でも表の候補が出る
- 前文脈を `split_words` で分けられないときは、バックオフの表を引かない

### 評価 (learning_eval)

- `--next-word <tsv>` で表を読み込む。`q` の行 (文脈の句の学習) の表記が NEXTWORD の 9 位以内・1 位に
  入る割合を、次の 4 つの組で出す。学習ありのモードの既出・初出の分け方は今の予測の評価と同じ
  - 学習なし・表なし (常に 0%。基準)
  - 学習なし・表あり
  - 学習あり・表なし
  - 学習あり・表あり

### 実機テスト (メモ帳、隔離したエンジン)

学習ファイルは `QUICKLIME_LEARN_FILE` で空のファイルに隔離し、エンジンは `QUICKLIME_PIPE_NAME` で隔離する。
表は `references/jawiki/next_word.tsv` を使う (開発版のエンジンが自動で見つける)。

| # | 操作 | 期待結果 |
|---|---|---|
| 1 | 「今日は」を確定キーで採用 | 学習が無くても、読みなしの予測に統計の候補 (天気など) が出る |
| 2 | 1 の続きで Ctrl+↓ → 確定キー | 候補が入り、続けてその後の予測が出る |
| 3 | 「今日も」を採用 (文節の表に無い場合) | バックオフの表で「今日」の後の候補が出る |
| 4 | 「今日は」の後に「いい天気」を数回採用してから、また「今日は」を採用 | 「いい天気」が先頭に出る (学習が先) |
| 5 | エンジンの起動直後 (表の読み込み前) に 1 の操作 | 入力が止まらず、学習の候補だけ出る (無ければ出ない) |
| 6 | `QUICKLIME_DICT_DIR` に next_word.tsv の無い辞書一式を指定して 1 の操作 | 予測は出ず、入力は止まらない |
| 7 | インストーラで入れた版で 1 の操作 | 統計の候補が出る (dict\next_word.tsv が読まれる) |

## 未決事項

- なし
