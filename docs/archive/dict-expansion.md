# 辞書の語彙補強 (単漢字・UT 辞書)

同梱辞書 (Mozc OSS 辞書、約 129 万エントリ) に無い語を補う。単漢字 (「あられ」→ 霰 など) と、
一般語・地名・人名の語彙を追加し、インストーラで最初から同梱する。

## 背景と目的

- Mozc OSS 辞書 (`dictionary_oss/dictionary00.txt`〜`09.txt`) には単漢字だけのエントリが少ない。
  Mozc 本体は単漢字を別データ `src/data/single_kanji/single_kanji.tsv` から rewriter で補っているが、
  QuicklIME はこれを読んでいないため、「あられ」で霰が出ない
- 固有名詞・地名・人名・複合語も Mozc OSS 辞書だけでは不足している。Mozc 向けの追加辞書
  mozc-ut (utuhiro78 氏) のうち、同梱しやすいライセンスのものを取り込む

## 決定事項

### 1. 単漢字辞書

[docs/spec/dictionary.md](../spec/dictionary.md)「単漢字辞書」へ移した

### 2. UT 辞書

#### 取り込む元データ

[docs/spec/dictionary.md](../spec/dictionary.md)「元データ」へ移した

#### 取得 (scripts\fetch-ut-dict.ps1)

[docs/spec/dictionary.md](../spec/dictionary.md)「取得」へ移した

#### 生成 (engine/examples/build_ut_dict.rs)

[docs/spec/dictionary.md](../spec/dictionary.md)「生成」へ移した

#### エンジンでの読み込み

[docs/spec/dictionary.md](../spec/dictionary.md)「辞書ディレクトリ」「システム辞書の書式」「UT 辞書の読み込み」へ移した

### 3. インストーラ (installer\build.ps1)

- `references\mozc\src\data\single_kanji\single_kanji.tsv` と
  `references\mozcdic-ut\dictionary_ut.txt` を `dict\` へコピーする。どちらも無ければ throw する
  (dictionary_ut.txt が無い場合は `scripts\fetch-ut-dict.ps1` の実行を促す)
- ライセンス文書 `LICENSE-mozcdic-ut.txt` を生成して同梱する。中身は次を連結したもの:
  - 3 元データの名前・取得元リポジトリ・ライセンスの説明 (上の表の内容)
  - 地名: 日本郵便が郵便番号データの著作権を主張しない旨
  - SudachiDict の LEGAL と LICENSE-2.0.txt の全文
- installer.iss: `dict\` の追加ファイルは `dict\*` のコピーに含まれるため変更不要。
  ライセンス文書は 1 ファイルずつ列挙しているので、`LICENSE-mozcdic-ut.txt` をインストール先の直下に
  置く行を 1 行足す

### 4. ドキュメント

README.md のビルド前提 (インストーラでの導入) と同梱辞書のライセンスの節に、単漢字辞書・UT 辞書・
`scripts\fetch-ut-dict.ps1` を書き足す。

## 変更対象ファイル

- engine/src/dict.rs (単漢字辞書の読み込みと検索、`load` の extra 引数)
- engine/src/convert.rs (単漢字候補の追記 3 箇所)
- engine/src/main.rs (単漢字辞書・UT 辞書の探索と読み込み)
- engine/Cargo.toml (dev-dependencies: bzip2, unicode-normalization)
- engine/examples/build_ut_dict.rs (新規)
- engine/examples/ut_diff.rs (新規、検証用。下記)
- scripts/fetch-ut-dict.ps1 (新規)
- installer/build.ps1
- README.md

## 実装手順

1. `git -C references/mozc sparse-checkout add src/data/single_kanji` で単漢字辞書を取得する
2. dict.rs に単漢字辞書を足し、単体テストを書く (書式のパース、記載順、コメント行・空行)
3. convert.rs の 3 箇所に単漢字候補を足し、テストを書く (位置が辞書候補の後・記号の前、
   重複しない、MAX_DICT_CANDIDATES に数えない、ラティスの第1候補が変わらない)
4. main.rs に単漢字辞書の探索を足す
5. `Dictionary::load` に extra を足し、main.rs に UT 辞書の探索を足す
6. build_ut_dict.rs を書き、除外規則と品詞 ID の引き方をテスト可能な関数に分けて単体テストを書く
7. fetch-ut-dict.ps1 を書き、実行して `references\mozcdic-ut\dictionary_ut.txt` を生成する
8. ut_diff.rs で変換の差分を確認する (下記)。誤変換が目立てば `--cost-offset` で再生成し、
   採用した値を fetch-ut-dict.ps1 に固定する
9. installer\build.ps1 と README.md を更新する

## 検証方法

- `cargo test` がすべて通ること
- 検証用 example `ut_diff.rs`: 読みの一覧 (1 行 1 読み。`examples/data/llm_cases.tsv` の読みの列と、
  新規に `examples/data/ut_diff_cases.txt` に置く日常文 50〜100 件) を、UT 辞書なし・ありの 2 つの辞書で
  `convert_segments` にかけ、第1候補 (文節表記の連結) が変わったものだけを出力する。
  変化を目視で確認し、悪化が多ければコストを調整する
- 起動時間とメモリ: UT 辞書なし・ありでエンジンを隔離起動し (`QUICKLIME_PIPE_NAME` などを使う)、
  起動ログの読み込み時間と `tasklist` のメモリ使用量を比べて記録する
- 実機 (メモ帳、開発版 DLL) で試すテストケース:
  - 「あられ」→ 候補に 霰 が出る (アラレ・あられ・辞書候補の後)
  - 「ひょう」→ 表・票・豹 など辞書候補の後に 雹 などの単漢字が並び、記号候補より前にある
  - 「あ」「か」のような 1 文字の読みで、単漢字の多い候補リストでも変換・選択・確定が正常に動く
  - 単漢字を選んで確定 → 次回同じ読みで先頭に来る (学習)
  - 「わたしはがっこうにいきます」など日常文の第1候補が UT 辞書の追加前と変わらない
  - 地名 (例: 市区町村名) と人名が変換できる
  - 予測入力に UT の語が出ても、よく使う語より前に割り込まない
- インストーラ: build.ps1 で生成した setup.exe の `dict\` に single_kanji.tsv と dictionary_ut.txt、
  ルートに LICENSE-mozcdic-ut.txt が入っていること

## 未決事項

- `--cost-offset` の値 (検証の結果で決める。既定 0 から始める)
