# 辞書インポート

## 背景と目的

他の IME (MS-IME / ATOK / Mozc・Google 日本語入力) からエクスポートしたユーザ辞書や、
それらの形式で配布されている辞書 (ニコニコ大百科 IME 辞書・pixiv 辞書など) を
QuicklIME に取り込めるようにする。配布辞書は数万〜数十万語あるため、大規模でも
変換速度を落とさないことを前提とする。

既存の部品:

- ユーザ辞書 `userdict.tsv` (engine/src/userdict.rs)。書式 `読み\t表記\t品詞[\tコメント]`
  (Mozc エクスポート互換)。対応品詞は 短縮よみ + 名詞系 7 種
  (名詞/固有名詞/人名/姓/名/地名/組織)。名詞系は線形走査のため数千語規模が前提
- 同梱辞書 `Dictionary` (engine/src/dict.rs)。fst 索引で完全一致・共通接頭辞・前方一致を引く
- 単語登録ツール quicklime-regword.exe (engine/src/bin/quicklime-regword.rs)。
  ADDWORD / エンジン未起動時の userdict.tsv 直接追記
- RELOADUSER (ユーザ辞書の再読込)

## 決定事項

### 保存形式と置き場所

- インポートした辞書は手動登録 (userdict.tsv) と分け、
  `%APPDATA%\QuicklIME\imported\<元ファイル名の拡張子を除いた名前>.tsv` に 1 辞書 1 ファイルで保存する
  - 環境変数 `QUICKLIME_IMPORT_DIR` でディレクトリを上書きできる (エンジン・単語登録ツール共通)
  - 中身は正規化済みの `読み\t表記\t品詞` (UTF-8、BOM なし)。品詞は QuicklIME の 8 品詞名のみ
  - 同名ファイルの再インポートは上書き (配布辞書の更新用)
  - 外したい辞書はファイルを消せばよい。一覧・削除 UI は今回作らない
- 採らなかった案: userdict.tsv への追記。手動登録と混ざって辞書単位で外せず、
  線形走査のため大規模辞書で変換が遅くなる

### エンジン側の保持と検索

- UserDict にインポート辞書を持たせる。名詞系の語は既存の `Dictionary` (fst) に載せる
  - 品詞 → 文脈 ID は既存の `noun_id_prefix` + `FunctionalIds::find_id` で解決する
    (引けなければ DEFAULT_NOUN_ID)
  - Dictionary に「解決済みの (読み, Entry) を staging へ積む」公開メソッドを足し、全ファイル読込後に finalize する
  - コストは定数 `IMPORTED_WORD_COST` (初期値 5000)。手動登録 (3000) より低優先、一般名詞並み
- インポート辞書の 短縮よみ は既存の shortcuts と同様のリストで持ち、手動登録分の後ろに並べる
  (完全一致・前方一致の扱いは手動登録の短縮よみと同じ)
- 変換・候補・予測でインポート辞書も引く:
  - ラティス構築 (convert.rs の `user.common_prefix_words` 呼び出し付近) で
    インポート辞書の `common_prefix_search` の結果もノードに載せる
  - 候補列挙 (`user.lookup_words` を使う箇所) でインポート辞書の `lookup` も含める
  - 予測 (predict.rs) では、インポート辞書の語を履歴の後に置き、同梱辞書の候補とコスト順で
    混ぜて出す (並び: 短縮よみ → 手動登録 → 履歴 → 同梱辞書+インポート辞書のコスト順)。
    同コストは同梱辞書を先にする。手動登録の直後に置くと、大規模な配布辞書で予測枠が
    インポート語だけで埋まり、履歴や一般語が出なくなるため。
    変換側でインポート語を一般名詞並みに扱うのと揃える
  - 手動登録と同じ (読み, 表記) がある場合は手動登録側を優先し、候補が重複表示されないこと
  - 具体的な API 形 (UserDict にメソッドを足すか、呼び出し側で両方引くか) は実装時に既存コードに合わせて決めてよい
- 起動時 (`UserDict::load_default`) と RELOADUSER で `imported\*.tsv` をすべて読み直す。
  RELOADUSER の応答ログにインポート辞書の件数も含める

### 形式判別と変換 (engine/src/import.rs)

純粋なパース・変換ロジックとして engine/src/import.rs に置き、エンジンの `cargo test` で
単体テストする。単語登録ツールからは `#[path = "../import.rs"] mod import;` で取り込む
(エンジン本体の main.rs には mod 宣言しなくてもよいが、テストを走らせるために
`#[cfg(test)] mod import;` 等で含めるか、本体から使う関数があれば通常の mod にする)。

文字コード判定:

1. 先頭が UTF-16LE BOM (FF FE) → UTF-16LE
2. UTF-16BE BOM (FE FF) → UTF-16BE
3. UTF-8 BOM (EF BB BF) → BOM を除いて UTF-8
4. BOM なしで UTF-8 として妥当 → UTF-8
5. それ以外 → CP932 (Shift_JIS)。単語登録ツール側で Win32 `MultiByteToWideChar(932, ...)`
   を使って変換する (新規 crate は追加しない。windows-sys に `Win32_Globalization` feature を追加)。
   import.rs のパース関数はデコード済み文字列を受け取る形にし、CP932 デコードは呼び出し側から関数で注入する

形式判別 (デコード後の先頭行):

| 形式 | 判別 | 行の形式 |
|---|---|---|
| MS-IME | `!Microsoft IME Dictionary Tool` で始まる | `!` 始まりはヘッダ。本体は `読み\t語句\t品詞[\t...]` |
| ATOK | `!!ATOK_TANGO_TEXT_HEADER_1` で始まる | `!` 始まりはヘッダ。本体は `読み\t語句\t品詞[\t...]` |
| Mozc / Google | 上記以外 | `#` 始まりはコメント。`読み\t表記\t品詞[\tコメント]` |

各行の処理:

- 列不足・読みや表記が空 → 不正な行として数える
- 読みのカタカナはひらがなに変換する (regword の `to_hiragana` を import.rs へ移して共用)
- 品詞を下表で QuicklIME の品詞に対応づけ、対応外は「未対応の品詞」として数えてスキップ
- 品詞末尾の `$` と `*` は除去してから判定する (この順で1つずつ)。ATOK が自動登録語と
  手動登録語を区別するために付ける印で、Mozc の辞書取り込みも同じ扱いをしている

品詞対応 (判定は完全一致の表を優先し、次に規則を上から順に適用):

| 元の品詞 | QuicklIME |
|---|---|
| 人名, 固有人他, 固有人名(姓名) | 人名 |
| 姓, 固有人姓 | 姓 |
| 名, 固有人名 | 名 |
| 地名, 固有地名, 地名その他 (MS-IME) | 地名 |
| 組織, 固有組織, 社名 (MS-IME) | 組織 |
| 固有名詞, 固有一般, 固有商品, 物品 (MS-IME) | 固有名詞 |
| 短縮よみ, 短縮読み, 顔文字, 記号 | 短縮よみ |
| (規則) 「固有」で始まる、または「固有名詞」を含む (ことえりの「その他の固有名詞」等) | 固有名詞 |
| (規則) 「名詞」を含む (名詞, 普通名詞, さ変名詞, ざ変名詞, 名詞サ変, 名詞ザ変, 形動名詞, 名詞形動, 名サ形動, さ変形動名詞, 副詞的名詞, 代名詞 等) | 名詞 |
| 上記以外 (動詞・形容詞・副詞・数・サジェストのみ・品詞なし・その他自立語・単漢字 等) | スキップ |

品詞名は Mozc の他 IME 品詞対応表 (references/mozc/src/data/rules/third_party_pos_map.def の
MS-IME・ATOK 節) で確認した。

重複:

- ファイル内の (読み, 表記, 品詞) 重複、および userdict.tsv・他のインポート済みファイル
  (上書き対象の同名ファイル自身は除く) に既にある (読み, 表記) はスキップして「重複」として数える

結果は件数集計 (取り込み / 重複 / 未対応の品詞 / 不正な行) を返す。

### UI (quicklime-regword.exe)

- ダイアログに「インポート...」ボタンを追加する (登録・キャンセルとは別、左寄せ等レイアウトは既存に合わせる)
- 押下 → `GetOpenFileNameW` (フィルタ: テキスト *.txt;*.tsv / すべて *.*) → 読込・変換 →
  `imported\<名前>.tsv` へ書き出し → エンジンへ `RELOADUSER` 送信 → 結果をメッセージボックスで表示
  - 例: 「辞書をインポートしました: 123,456 語 (重複 120 / 未対応の品詞 3,400 / 不正な行 2)」
  - エンジン未接続時は保存のみ行い「反映は次回のエンジン起動時」と添える
  - 取り込み 0 語の場合はファイルを作らず、その旨を表示する
- コマンドラインからの起動 (`quicklime-regword.exe --import <file>`) は今回作らない

## 変更対象ファイル

- engine/src/import.rs (新規): 文字コード判定・形式判別・品詞対応・行パース・重複除去・集計
- engine/src/userdict.rs: インポート辞書の保持 (Dictionary + 短縮よみ)、読込、reload、検索 API
- engine/src/dict.rs: 解決済みエントリを積む公開メソッド
- engine/src/convert.rs, engine/src/predict.rs: インポート辞書を引く
- engine/src/main.rs: mod 宣言、RELOADUSER のログ
- engine/src/bin/quicklime-regword.rs: インポートボタン、ファイル選択、CP932 デコード、保存、RELOADUSER 送信
- engine/Cargo.toml: windows-sys feature 追加 (`Win32_UI_Controls_Dialogs`, `Win32_Globalization`)
- docs/protocol.md: RELOADUSER がインポート辞書も読み直す旨
- README.md: 辞書インポートの使い方・保存先・対応形式
- docs/roadmap.md: 該当項目の追記

## 実装手順

1. import.rs: デコード (BOM/UTF-8 判定、CP932 は注入) と形式判別・品詞対応・パースを実装し単体テスト
2. dict.rs / userdict.rs: インポート辞書の読込・fst 構築・reload、検索 API とテスト
3. convert.rs / predict.rs: ラティス・候補・予測への組み込みとテスト
4. regword: ボタン・ファイル選択・保存・RELOADUSER
5. ドキュメント更新

## 検証方法

- `cargo test` (import.rs の各形式・文字コード・品詞対応・重複、userdict の読込と reload、
  convert/predict でインポート語が出ること、手動登録との重複が出ないこと)
- 大規模辞書での性能: 数十万語のインポートファイルで起動時間と変換応答時間を計測する
  (`QUICKLIME_IMPORT_DIR` と `QUICKLIME_PIPE_NAME` 等で実ユーザ環境から隔離)
- 実機: 各形式のサンプルファイル (MS-IME UTF-16LE / MS-IME Shift_JIS / ATOK / Mozc) を
  単語登録ツールから取り込み、メモ帳で変換・予測に出ることを確認

## 未決事項

- IMPORTED_WORD_COST の値は実機テストで調整する
