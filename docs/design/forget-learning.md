# 学習の削除 (候補パネルからの Ctrl+Delete)

## 背景と目的

学習 (learning.tsv・learning_context.tsv・learning_word.tsv) は追記ログで、誤って確定した表記を
覚えると、同じ読みで確定し直すまで候補バーの先頭や予測候補に出続ける。消す手段は学習ファイルを
手で編集することしかない。

候補パネル (候補バー・読みなしの後続語予測) で選んでいる候補を Ctrl+Delete で指定し、その候補の
学習を消せるようにする。

## 決定事項

### 消す対象

- 消すのは学習だけ。辞書・ユーザ辞書 (手動登録・インポート辞書) の語は消さない。学習でない候補を
  指定したときは何も消えない
- (読み, 表記) の組を指定し、次をすべて消す
  - learning.tsv: その読みの学習表記が指定の表記のとき、その読みの学習
  - learning_context.tsv: 読みと表記が一致する学習を、前文脈によらずすべて
  - learning_word.tsv: 読みと表記が一致する複合語
- learning.tsv は読みごとに最後の表記だけを持つ。消した後に、同じ読みで以前に確定していた表記が
  先頭に戻ることはない (下記のファイルの書き直しで古い行も消えるため)
- 境界学習 (learning_boundary.tsv) は (読み, 表記) の組を持たないので触らない
- 設定 `learning` が 0 のときと、`no_learning_apps` のアプリでも消せる (記録ではないため)

### 候補の種類ごとに送る組

| 種類 | 送る (読み, 表記) |
|---|---|
| 予測候補 (読みなしの後続語予測を含む) | 候補の (読み, 表記) |
| 全体変換 | 読み全体と表記の連結の組、および文節ごとの (読み, 表記) |
| 先頭文節 | その文節の (読み, 表記) |
| 英字候補 | 送らない (学習に由来する候補ではない) |

- 全体変換で文節ごとの組も送るのは、文節ごとの学習を当てた 1-best が同じ表記を作り続けるため。
  文節の学習は表記が一致するものだけ消えるので、学習していない文節には影響しない。正しく覚えていた
  文節の学習も消えるが、次の確定で覚え直される

### プロトコル: FORGET

```
要求:   FORGET\t<読み>\x1f<表記>\t<読み>\x1f<表記>...\n
成功:   OK\t<消した学習の件数>\n
失敗:   ERR\t<メッセージ>\n   (組が無い、読み・表記の欠落)
```

- 件数は 3 つのファイルで消した項目の合計。0 なら学習でない候補だった
- 旧エンジンは「不明なコマンド」を返す。TSF 層はそのとき何もしない

### ファイルの書き直し

- 消した項目があったファイルだけを、メモリ上の内容から書き直す。learning.tsv と learning_context.tsv は
  記録順の連番の昇順で書き、読み込み後の新しさの順を保つ。learning_word.tsv は保持している順で書く
- 同じディレクトリの一時ファイルに書いてから rename で置き換える。書き込みに失敗したらファイルは
  元のまま残し、メモリ上の削除は保つ (エンジンの再起動で戻る)
- 削除を表す行 (tombstone) を追記する方式は採らない。追記ログに新しい書式が増え、旧エンジンでは
  消した学習が戻るため

### キー割当

- 割当機能 `ForgetLearning` (設定キー `key.forget_learning`、働く状態はサジェスト選択中、
  既定 `Ctrl+Delete`) を足す。keymap.md の機能の表の末尾に置く
- サジェスト選択中は、割当に一致した打鍵を常に食べる (keymap.md の規則どおり)。英字候補を選んでいる
  とき・学習でない候補のとき・エンジン未接続のときも食べて何もしない。アプリへ渡すとキャレットの
  後ろの文字が消えるため
- 設定ツールの「キー」タブに「学習の削除」として足す

### 削除後の表示

- 件数が 1 以上なら候補を作り直す。run 中は確定済みかなで PREDICT と CONVNBEST を引き直し、
  読みなしの後続語予測は NEXTWORD を引き直す。LLM の並べ替えの結果は捨てる (消した学習の候補を
  含みうるため)
- 選択は同じ番号に保つ。候補が減って番号が無くなれば末尾を選び、候補が無くなればバーを消す
  (run 中で入力行を出しているときは入力行だけ残す)
- 件数が 0 なら表示を変えない
- 文書は書き換えない。確定アンドゥの記憶もそのまま

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| engine/src/learn.rs | `LearningStore::forget(reading, surface) -> usize` (map・ctx_map・ctx_index から消す) と、learning.tsv・learning_context.tsv の書き直し。単体テスト |
| engine/src/userdict.rs | 学習した複合語を (読み, 表記) で消して learning_word.tsv を書き直す関数。単体テスト |
| engine/src/main.rs | `FORGET` の処理。パイプ経由の単体テスト |
| engine/src/bin/quicklime-config.rs | `KEY_ITEMS` に `key.forget_learning` (サジェスト選択中、`Ctrl+Delete`) |
| tsf/src/config.h / config.cpp | `KeyFunc::ForgetLearning`、設定キー名、働く状態、既定の割当 |
| tsf/src/engine_client.h / engine_client.cpp | `Forget(pairs, &count)`。自動起動・接続待ちはしない |
| tsf/src/text_service.h / text_service.cpp / text_service_direct.cpp | サジェスト選択中の `ForgetLearning` の処理 (送る組の組み立て・作り直し・選択の復元) |
| docs/protocol.md | FORGET の節 |
| docs/design/keymap.md | 機能の表に `ForgetLearning` |
| docs/design/basics.md | 「学習」の節に削除の手段を1行 |
| README.md | キー割当の表と候補バーの操作の説明 |
| docs/design/README.md | 本書を索引に足す |

## 実装手順

1. エンジン: `LearningStore::forget` とファイルの書き直し、`UserDict` の複合語の削除、`FORGET` の処理と単体テスト
2. 設定ツール: `KEY_ITEMS` の追加
3. TSF 層: `KeyFunc::ForgetLearning` の設定、`EngineClient::Forget`、サジェスト選択中の処理
4. ビルド (TSF の Debug、エンジンの `cargo build`) と `cargo test`
5. ドキュメント

## 検証方法

- `cargo test` (forget の単体テスト、FORGET の要求・応答、書き直したファイルを読み込み直して同じ状態に
  なること)
- 隔離環境 (`QUICKLIME_PIPE_NAME`・`QUICKLIME_LEARN_FILE`・`QUICKLIME_USER_DICT_FILE`・
  `QUICKLIME_CONFIG_FILE`) のメモ帳で下記を確認する

### テストケース

| # | 操作 | 期待する結果 |
|---|---|---|
| 1 | `kyousi` → 「教師」を採用して確定 → `kyou` → ↓で「教師」を選ぶ → Ctrl+Delete | 「教師」が予測候補から消えるか、辞書の順位に下がる。learning.tsv から `きょうし\t教師` の行が消える |
| 2 | 1 の後にエンジンを再起動して `kyou` | 「教師」が学習の位置に戻らない |
| 3 | 「今日は」の後に「晴れ」を確定 → 確定キーで run を終える → 読みなしの予測で「晴れ」を選ぶ → Ctrl+Delete | 「晴れ」が後続語の予測から消える。learning_context.tsv の「晴れ」の行がすべて消える |
| 4 | 辞書にだけある予測候補を選んで Ctrl+Delete | 表示が変わらず、文書の文字も消えない |
| 5 | 全体変換の先頭 (読み全体の学習表記) を選んで Ctrl+Delete | その表記が先頭から外れる。読み全体の組と、表記が一致した文節の学習が消える |
| 6 | 先頭文節の候補 (「…」付き) を選んで Ctrl+Delete | その文節の学習が消え、候補が作り直される |
| 7 | 英字候補を選んで Ctrl+Delete | 何も起きず、文書の文字も消えない |
| 8 | 末尾の候補を選んで Ctrl+Delete (候補が減る) | 作り直した候補の末尾が選ばれる |
| 9 | 文節伸縮で割って確定した複合語 (learning_word.tsv) を予測で選んで Ctrl+Delete | learning_word.tsv から消え、その読みが1語の候補にならない |
| 10 | 設定 `learning 0` で 1 を行う | 削除はできる |
| 11 | 候補を選ばずに (run 中) Ctrl+Delete | 既存どおり run を終えてアプリへ渡す |
| 12 | `key.forget_learning Ctrl+Shift+Delete` に変える | Ctrl+Shift+Delete で消え、Ctrl+Delete は run を終えてアプリへ渡す |
| 13 | 学習ファイルを読み取り専用にして削除 | 表示からは消え、エンジンの再起動で戻る (エンジンのログに失敗が出る) |
