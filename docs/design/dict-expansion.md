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

- データ: Mozc の `src/data/single_kanji/single_kanji.tsv` (BSD、既存の同梱辞書と同じ Mozc のライセンス)。
  書式は `読み\t漢字を連結した文字列` (例: `あられ\t霰`、`ひょう\t氷丙平…雹…`)。
  `#` で始まる行・空行は読み飛ばす。漢字の並びは記載順のまま候補順とする
- `variant_rule.txt` (異体字の注釈) は使わない
- ラティスには入れない。候補リストへの追記だけにする (Mozc の single_kanji_rewriter と同じ扱い)。
  ラティスに入れると「あ」→ 亜 のような単漢字が経路に割り込み、区切りと第1候補が崩れるため
- 追記位置は、通常の辞書候補 (完全一致・先頭語入れ替え・数字の助数詞入れ替え) の後、記号候補の前。
  記号候補と同じく `MAX_DICT_CANDIDATES` の枠には数えない。既に候補にある表記は追加しない
- 対象は記号候補を出している 3 箇所すべて (engine/src/convert.rs):
  - `segment_from_group` (文節候補。文節の読み全体で引く)
  - `candidates` (全文候補)
  - `convert_nbest` の縦の候補リスト (`whole(...)` で追加。`from_user` は false)
- 学習はこれまでどおり表記で効くので、選んだ単漢字は次回から先頭に来る (変更不要)
- 読み込み: `Dictionary` に記号辞書と同様の `single_kanji: HashMap<String, Vec<String>>` を持たせ、
  `load_single_kanji(path)` / `load_single_kanji_from(reader)` / `lookup_single_kanji(reading) -> &[String]`
  を足す。無くても単漢字候補なしで起動を続ける
- 探索 (main.rs、`load_symbols` と同じ方式): 辞書ディレクトリ直下の `single_kanji.tsv` を優先し、
  無ければ Mozc の配置 (dictionary_oss と並ぶ `single_kanji/single_kanji.tsv`)
- `references/mozc` は sparse checkout のため、`src/data/single_kanji` を追加取得する
  (`git -C references/mozc sparse-checkout add src/data/single_kanji`)

### 2. UT 辞書

#### 取り込む元データ

| 元データ | リポジトリ | ライセンス | 品詞 (id.def の名前) |
|---|---|---|---|
| SudachiDict | utuhiro78/mozcdic-ut-sudachidict | Apache 2.0 (一部 UniDic 由来、BSD) | 名詞,一般,\*,\*,\*,\*,\* |
| 日本郵便の郵便番号データ (地名) | utuhiro78/mozcdic-ut-place-names | 著作権を主張しない (日本郵便) | 名詞,固有名詞,地域,一般,\*,\*,\* |
| 人名 | utuhiro78/mozcdic-ut-personal-names | Apache 2.0 | 名詞,固有名詞,人名,一般,\*,\*,\* |

採らなかったもの:

- NEologd: 2020 年頃で更新が止まっており、「ぁまのじゃく」のような崩し表記が多く誤変換の元になりやすい。
  mozc-ut の既定構成でも除外されている
- jawiki・EDICT (CC BY-SA)、SKK-JISYO・alt-cannadic (GPL): 同梱時のライセンス条件が重い
- 公式のマージスクリプト (merge-ut-dictionaries の merge_dictionaries.py): コスト調整に
  Wikipedia のタイトル一覧 (CC BY-SA、数百 MB) を取得して使い、品詞もすべて名詞,一般にする。
  最新の Mozc master を取りに行くため、手元の Mozc 辞書 (固定コミット) とも食い違う

#### 取得 (scripts\fetch-ut-dict.ps1)

scripts\fetch-models.ps1 と同じ作り。リビジョンを固定し、サイズと SHA-256 を確かめ、
`references\mozcdic-ut\` (git 管理外) に保存する。取得済みで一致するファイルは取り直さない。
取得後に生成 (下記) まで実行する。

| ファイル | 取得元 URL (`https://github.com/<repo>/raw/<commit>/<file>`) | サイズ | SHA-256 |
|---|---|---|---|
| mozcdic-ut-sudachidict.txt.bz2 | utuhiro78/mozcdic-ut-sudachidict @ c686771bada1d59e9b105c81b29e6ac1a239cb54 | 11472867 | d2cb4b2b16d513e257d26d1c33673ebda254a1d5aa0c3c0c9bc94f932619fbdc |
| mozcdic-ut-place-names.txt.bz2 | utuhiro78/mozcdic-ut-place-names @ 98cf8e9d7731695a52e5bc72ed633933c71d1913 | 1743054 | 71a3e28e8acead3793739ec3466826b01adedc168354957661ec57c18688a0a2 |
| mozcdic-ut-personal-names.txt.bz2 | utuhiro78/mozcdic-ut-personal-names @ b557dd4c1548cc7c2164d5f8b14170c40cbb6f81 | 729168 | 610e2e52fad15df0edac854960a05451a7e9a62f3df09c3f065db9e1ab6f51f4 |
| SudachiDict-LEGAL | WorksApplications/SudachiDict @ 3e49051e71011cac7d74df779e80ef1dab818e56 の `LEGAL` | (実装時に確定) | (実装時に確定) |
| SudachiDict-LICENSE-2.0.txt | 同上の `LICENSE-2.0.txt` | (実装時に確定) | (実装時に確定) |

ライセンス文書 2 件のサイズと SHA-256 は、実装時に上記コミットから取得して計算した値をスクリプトに固定する。

#### 生成 (engine/examples/build_ut_dict.rs)

`cargo run --release --example build_ut_dict -- --mozc <dictionary_oss> --ut <references\mozcdic-ut> --out <出力>`。
bz2 の展開には `bzip2` クレートを dev-dependency で使う (エンジン本体の依存は増やさない)。

- 入力の各行 `読み\tID\tID\tコスト\t表記` (ID は全行 0000) を読む
- 表記を NFKC で正規化し、`~` を `〜` に置き換える (mozc-ut と同じ。正規化は `unicode-normalization`
  クレートを dev-dependency で使う)
- 除外する行:
  - 表記が 1 文字、または 26 文字以上
  - 表記が ASCII だけ (英単語はモードレス入力の英単語辞書が受け持つ)
  - (読み, 正規化した表記) が Mozc 辞書 (dictionary00〜09、表記は同じ正規化をかけて比べる) にある
  - 元データ間で (読み, 正規化した表記) が重複するもの (表の上の元データを優先し、2 件目以降を捨てる)
- 品詞 ID は `--mozc` の id.def から上表の品詞名で引く (数値を固定しない)。左右 ID は同じ値
- コストは元データの値をそのまま使う (sudachidict は大半 8999、地名 9000、人名 8000)。
  検証で誤変換が増えた場合は、全エントリに一律の値を足すオプション `--cost-offset <N>` (既定 0) で調整する。
  i16 に収まらない値はクリップする
- 出力は `dictionary_ut.txt` (Mozc 形式、UTF-8、BOM なし、読みでソート)。件数は約 112 万になる見込み
- 処理の最後に元データごとの読み込み件数・除外件数 (理由別)・出力件数を表示する

#### エンジンでの読み込み

- `Dictionary::load(dir)` を `Dictionary::load(dir, extra: &[PathBuf])` にし、dictionary00〜09 の後に
  `extra` を同じ `load_from` で積んでから `finalize` する。存在しない extra は読み飛ばす。
  同じ読みの中は挿入順が保たれ、候補順は `exact_candidates` のコストソートで決まる
- 探索 (main.rs、`english_dir` と同じ方式):
  1. 辞書ディレクトリ直下の `dictionary_ut.txt` (インストール先のレイアウト)
  2. 無ければプロジェクトルートの `references/mozcdic-ut/dictionary_ut.txt`
     (exe が engine/target/{debug,release}/ にある前提で相対解決)。
     ただし `QUICKLIME_DICT_DIR` を指定したときは 1 だけを見る。この環境変数は「この辞書一式で動かす」
     指定であり、tests/pipe.rs などの隔離起動に本物の UT 辞書が混ざらないようにするため
- 起動ログの「辞書を読み込みました: N エントリ」に UT 辞書の有無が分かるよう、読み込んだ extra のパスも出す
- UT 辞書が無くても今までどおり起動する
- 予測入力 (`predict_prefix`) にも UT のエントリが入る。コストが高いので後ろに並ぶ想定で、特別扱いはしない

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
