# 読めない行の読み飛ばし

## 背景と目的

辞書・学習・ユーザ辞書・設定のファイルに UTF-8 として読めない行が 1 行でもあると、今は次のようになる。

- Mozc 辞書・UT 辞書・単漢字辞書・記号辞書・品詞 ID 表 (`line?`): 読み込み全体がエラーになる。Mozc 辞書か UT 辞書では
  システム辞書なしで起動する
- 学習・ユーザ辞書・インポート辞書・設定・英単語辞書 (`break`、設定ツールは `map_while(Result::ok)`): その行から後ろを読まない

後ろを読まないファイルでは、次の 2 つの経路でデータを失う。

- 学習の削除 (FORGET) と learning_word.tsv の書き直し、設定ツールの保存は、メモリの内容でファイル全体を書き直す。
  読まなかった後半の行がここで消える
- userdict.tsv・learning_word.tsv・learning_boundary.tsv・learning_stats.tsv は追記で保存する。読めない行より後ろに
  追記した単語登録や学習は、次の起動で読まれない

読めない行はその行だけを読み飛ばし、続きを読むようにする。書き直す学習のファイルは、書き直しで読めない行が消える前に
元のファイルを退避する。

## 決定事項

### 読めない行の扱い

- UTF-8 として読めない行 (`io::ErrorKind::InvalidData`) は、その 1 行だけを読み飛ばして続きを読む
  - `BufRead::lines()` は不正な UTF-8 の行で `Err(InvalidData)` を返すが、その行のバイト列は読み終えているので、
    次の呼び出しは次の行から続く
- それ以外の I/O エラーでは、そこで読むのをやめる (同じエラーが返り続けて無限ループになるのを防ぐ)
  - `line?` で読み込み全体をエラーにしていた関数 (dict.rs・pos.rs) は、今までどおりエラーを返す
  - `break` で止めていた関数は、今までどおり読めた分だけで続ける
- 読み飛ばした行があれば、ファイルごとに 1 回、行数をログ (標準エラー) に出す。パスが分かる呼び出し元で出す

採らなかった案:
- U+FFFD に置き換えて読む (lossy)。置き換えた読みや表記が正しい語として学習やユーザ辞書に入り、そのあとの書き直しで
  元のバイト列が失われるため

### 共通の読み方

新しいモジュール `engine/src/textfile.rs` に、行ごとに処理して読み飛ばした行数を返す関数を置く。

```rust
/// reader を 1 行ずつ f に渡す。UTF-8 として読めない行は飛ばし、その数を返す。
/// それ以外の I/O エラーではそこで止め、エラーを返す
pub fn read_lines(reader: impl BufRead, f: impl FnMut(String)) -> io::Result<usize>
```

- 各読み込み関数のループをこの関数で書き直す。ループの中の `continue` は closure の `return` にする
- 設定ツール (engine/gui) は、import.rs と同じく `#[path]` で engine/gui/src/lib.rs から取り込む

### 書き直す学習のファイルの退避

- 対象は learning_stats.tsv と learning_word.tsv (エンジンがファイル全体を書き直す学習のファイル)
- エンジンの起動時の読み込みで読み飛ばした行があったら、読み込みの直後 (書き直しより前) に、元のファイルを同じディレクトリの
  `<ファイル名>.unreadable-<YYYYMMDD-HHMMSS>` (ローカル時刻) へコピーし、退避先をログに出す。コピーに失敗したらログに出して続ける
- RELOADUSER の読み直しでは退避しない。単語登録のたびに読み直すので、ファイルが壊れているあいだ退避ファイルが増え続けるため。
  起動時に退避した時点で元の内容は残っている
- 次の書き直しで読めない行は消え、それより後の起動では退避しない
- 追記だけのファイル (userdict.tsv・learning_boundary.tsv)・インポート辞書・システム辞書は退避しない。エンジンが書き直さないので、
  読めない行はファイルに残る
- 設定ファイルは退避しない。設定ツールの保存で読めない行は消えるが、失うのは設定 1 行で、設定ツールで直せるため

### 対象外

- 連接行列 (matrix.rs) と後続語の統計の表 (next_word.tsv。predict.rs) は `read_to_string` でファイル全体を読み、
  読めなければ行列なし・統計なしで動く。どちらも配布物で利用者が編集しないので変えない
- TSF 層 (C++) の設定ファイルとローマ字テーブルの読み込みは変えない

## 変更対象ファイル

- engine/src/textfile.rs (新規): `read_lines`
- engine/src/main.rs: `mod textfile;`
- engine/src/dict.rs: `load_from`・`load_symbols_from`・`load_single_kanji_from`。`Dictionary::load` で読み飛ばした行数を
  ファイルごとにログに出す (パスを持つ場所で出す。単漢字・記号は main.rs の `load_single_kanji`・`load_symbols` でもよい)
- engine/src/pos.rs: `load_from`
- engine/src/learn.rs: `load_stats_from`・`import_plain`・`import_context`・`load_boundary_from`。`load_files` で
  learning_stats.tsv を読み飛ばした行があれば退避する (`load_stats_from` は読み飛ばした行数も返せるようにする)
- engine/src/userdict.rs: `load_from`・`load_imported` のループ・`load_learned_from`。起動時 (`load_default`) の
  learning_word.tsv の読み込みで読み飛ばした行があれば退避する
- engine/src/config.rs: `load_from`
- engine/src/english.rs: `load_with_names` のループ
- engine/gui/src/lib.rs: `#[path = "../../src/textfile.rs"] pub mod textfile;`
- engine/gui/src/config.rs: `Config::load`
- 仕様書
  - docs/spec/dictionary.md「システム辞書の書式」(読めない行でシステム辞書なしになる記述を、その行を読み飛ばす記述に)・
    「ユーザ辞書の書式」(後ろを読まない記述を書き換え)・「インポート辞書の読み込み」(該当する記述があれば)・単漢字辞書と
    記号辞書の節 (該当する記述があれば)
  - docs/spec/learning.md「学習ファイル」(読めない行の読み飛ばしと退避の節を足す)
  - docs/spec/tools.md「場所と書式」(読めない行を読み飛ばすこと)
  - docs/spec/modeless.md「英単語辞書」(該当する記述があれば)
- docs/roadmap.md: 「辞書・学習のファイルに UTF-8 として読めない行があるときの扱い」の項目を消す
- docs/design/README.md: 進行中の設計書の一覧にこの設計書を足す (実装後に archive へ移すときに消す)

## 実装手順

1. textfile.rs に `read_lines` とその単体テストを書く
2. 上の各読み込み関数を `read_lines` で書き直す。読み飛ばした行数を呼び出し元へ返し、ログに出す
3. learning_stats.tsv と learning_word.tsv の起動時の退避を足す
4. 設定ツールの `Config::load` を書き直す
5. 仕様書と roadmap を書き換える
6. `cargo test` (engine と engine/gui) と `cargo clippy` を通す

## 検証方法

単体テスト (不正な UTF-8 は `b"\xff\xfe"` などのバイト列で作る):

- `read_lines`: 2 行目が不正な 3 行の入力で、1 行目と 3 行目が渡され、1 が返る。不正な行が無ければ 0
- `read_lines`: 最終行が改行なしで不正なとき、それまでの行が渡され、1 が返る
- システム辞書: 1 行が不正な辞書で、その語だけが無く、前後の語は引ける (今は全体がエラー)
- learning_stats.tsv: 2 行目が不正なファイルで、1 行目と 3 行目の件が読まれ、退避ファイルが元のバイト列と同じ内容で作られる
- learning_stats.tsv: 同じ条件で FORGET したあと、本体は書き直され、退避ファイルに元のバイト列がそのまま残る
- learning_stats.tsv: 不正な行が無いとき、退避ファイルは作られない
- learning_word.tsv: 不正な行の後ろの複合語が読まれ、起動時に退避される。`reload` では退避しない
- userdict.tsv: 途中に不正な行があり、その後ろに追記した語が引ける
- インポート辞書・設定ファイル (エンジンと設定ツール)・英単語辞書: 不正な行の後ろの行が読まれる

実機 (隔離環境。`QUICKLIME_LEARN_FILE`・`QUICKLIME_USER_DICT_FILE` などを使う):

- 不正な行を入れた learning_stats.tsv でエンジンを起動し、ログに読み飛ばした行数と退避先が出る。退避ファイルができている
