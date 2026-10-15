# 英字混じり文の変換

## 背景と目的

モードレス入力 (docs/design/modeless.md、docs/design/modeless-detection.md) では、日本語に続けて
英単語を打つと英単語の部分だけが英字になる (`kyouhagithu` → `きょうはgithu`)。しかし次の2点で、
英字混じりの文 (`今日はgithubで作業する`) を普通に打って変換することができない。

- 英字区間は run が終わるまで続き、日本語に戻らない。`kyouhagithubdesagyousuru` は
  `きょうはgithubdesagyousuru` になる。途中で Space を押すと run が終わって半角スペースが入る
- 英字を含む読みは漢字に変換されない。変換キーでは全体を1文節に固定して生ローマ字の候補を出し
  (tsf/src/text_service.cpp `BuildConversionSegments`)、エンジンは英字を1文字ずつ未知語ノードにする
  (engine/src/convert.rs `build_lattice`)

本書の目的は、`kyouhagithubdesagyousuru` と続けて打つと `きょうはgithubでさぎょうする` になり、
変換キーで `今日はgithubで作業する` を選べるようにすることである。

## 決定事項

すべて設定 `modeless` が有効なときだけ働く (B のエンジンの変更を除く)。新しい設定は設けない。

### A. 英字区間を閉じて日本語に戻す

#### 閉じる判定

- 英字区間 (自動判定で入ったもの・Shift+英字で入ったものの両方) で英字を1打鍵打つたびに、
  英字区間の文字列 S と打った文字 c をエンジンに問い合わせる (新コマンド ASCIIWORD、下記)
- S+c が英単語辞書のどの語の前方一致 (完全一致を含む) にもならないとき、次を満たす k (切れ目) を探す
  - S の先頭 k 文字が英単語辞書の語と完全一致する (k は3以上)
  - 残り S[k..] + c が、既定どおりのローマ字の照合でかな (末尾の未完成のローマ字を含んでよい) として
    読める。ローマ字として読めない打鍵 (自動英字判定のルール1) が起きないこと
- 条件を満たす k が複数あるときは、残りが子音 (a i u e o 以外) で始まる k を優先し、その中で最長のものを採る。
  子音で始まる k が無ければ最長のものを採る。英単語に続く日本語はほとんどが子音で始まり
  (を・で・の・が・は・する・に)、母音1文字から始まることはまれなため
- 残りがローマ字として読めるかは、自動英字判定を止めた別の `RomajiComposer` (modeless 無効) に流し、
  素通しの英字 (ルール1 にあたる打鍵) が出ないことで調べる
- k が見つかれば、先頭 k 文字で英字区間を閉じ、S[k..] + c を日本語区間の打鍵としてローマ字で打ち直す。
  打ち直しでは S[k..] の部分は自動英字判定を止めて流し、最後の c だけを通常の打鍵と同じく判定に通す
  (ResolveAsciiRequest を含む)
- 見つからなければ今どおり英字区間が続く (辞書に無い語、一致する切れ目が無い語)
- S+c が前方一致になる間は閉じない (`apple` + `s` は `apples` で英字のまま)
- 例
  - `github` + `d` → `github` で閉じ、`d` は未完成のローマ字 (続く `e` で `で`)
  - `make` + `r` → `maker` が前方一致なので英字のまま。続く `u` で `makeru` が前方一致でなくなる。
    k=5 (`maker`) の残り `u` は母音で始まり、k=4 (`make`) の残り `ru` は子音で始まるので、
    `make` で閉じて `ru` → `る`
  - `carpet` + `w` → k=6 の残り `w` が子音で始まるので `carpet` で閉じる (`car` では閉じない)
  - `apple` + `a` → 子音で始まる k が無いので最長の `apple` で閉じ、`あ`
  - `python` + `w` → `python` で閉じ、続く `o` で `を`
  - `xyzw` + `a` → 完全一致する先頭部分が無いので英字のまま
- 閉じるのは英字区間の文字を打ったとき (ローマ字の英小文字キー) だけ。記号キー・数字・Space・
  確定キーでは今どおり
- エンジン未接続・エラー・ASCIIWORD を知らない旧エンジンのときは閉じない (今の挙動)。
  自動起動や接続待ちはしない (ASCIISTART と同じ呼び方)

#### run の構造

- 閉じた英字区間の文字は、今の英字区間と同じく「ASCII 文字1文字 = かな1文字、打鍵列も同じ1文字」
  として `kana_` / `raw_` に残す。英字モード (`asciiMode_`) を抜けて日本語区間として入力を続ける
- `RomajiComposer` に「閉じた位置」(最後に閉じた英字区間の終わりの `kana_` の位置) を1つ持たせる。
  閉じた英字区間が無ければ 0
- 1つの run に英字区間が複数できてよい (`きょうは|github|で|python|をつかう`)。開いている英字区間は
  今どおり最大1つ (`asciiStart_`) で、閉じた位置より後ろにだけできる
- 閉じた位置より前は、新しい英字区間の判定の対象外にする
  - ASCIISTART に送る要素 (`AsciiRequestElements`) と境界の候補は、閉じた位置より後ろだけにする。
    閉じた位置より後ろで一致しなければ、「run 全体を英字にする」扱いは「閉じた位置から後ろを英字にする」
    に読み替える
  - ルール6 (run 全体が対象) は、閉じた英字区間のある run には適用しない (区間分割した run と同じ)
  - ルール7 の全体一致・末尾一致、ルール4 の「打鍵列が5文字以上」も、閉じた位置より後ろの打鍵列で数える
  - ルール4・5 の綴りの有無 (`HasWeakAsciiSpelling`) も、閉じた位置より後ろの打鍵列だけを見る
    (閉じた `github` の `thu` で、閉じた後の打鍵ごとにルール5 の問い合わせが出続けないようにする)
- Backspace で閉じた位置まで戻った (閉じた位置より後ろのかなと未完成のローマ字が空になった) ときは、
  英字区間を開き直す (英字モードに戻り、開始位置は閉じた英字区間の先頭)。閉じた位置は1つ前の
  閉じた英字区間の終わりに戻す。そのため閉じた英字区間の (開始位置, 終わり) を積んで持つ
- `RemoveFront`・`PrependKana` は、閉じた英字区間の位置も今の英字区間の開始位置と同じくずらす。
  先頭から取り除かれた閉じた英字区間は忘れる
- `Raw()`・`Commit()`・`Display()` は今どおり (閉じた英字区間は ASCII 文字のかなとして出る)
- F6〜F10 は今どおり run 全体を `Raw()` / かなから作る
- 範囲の伸縮で積む読みの状態 (docs/design/conversion-range.md) は `RomajiComposer` 全体なので、
  閉じた英字区間の情報も合わせて戻る

採らなかった案: 閉じる専用のキーを設ける案。英字混じりの文を打つたびにキーを挟む必要があり、
モードレスの目的 (切替を意識しない) に合わない。

#### プロトコル: ASCIIWORD

```
要求:   ASCIIWORD\t<文字列>\n
成功:   OK\t<前方一致>\t<長さ1>,<長さ2>,...\n
        前方一致: 文字列全体が英単語の前方一致 (完全一致を含む) なら 1、でなければ 0
        長さ: 文字列の先頭部分のうち、英単語と完全一致するものの長さ (3以上、昇順)。無ければ空
失敗:   ERR\t<メッセージ>\n   (文字列の欠落、英字以外を含む)
```

- 文字列は S + c (英大文字は小文字にして照合する)。TSF 層は「前方一致が 0」のときに、長さの一覧のうち
  S の長さ以下のものを切れ目の候補とし、上記「閉じる判定」の優先順で選ぶ
- 英単語辞書は ASCIISTART と同じ (一般語・固有名詞・ユーザ辞書の英字の語)。前方一致には ASCIISTART の
  長さの制限 (前方一致は4文字以上) を当てない
- 辞書が無いときは `OK\t1\t` (前方一致あり = 閉じない) を返す

### B. エンジンで英字の連続を1語として扱う

- `build_lattice` で、読みの中の ASCII 英字 (`[a-zA-Z]`) の連続ごとに、その連続全体を読み = 表記とする
  ノードを1つ足す。品詞は未知語ノードと同じ名詞 (`DEFAULT_NOUN_ID`)、単語コストは未知語ノードより十分小さい
  一般的な名詞程度の値にする
- 英字の連続の途中で始まる・終わるノード (英字の1文字の未知語ノード、連続の一部だけに一致する辞書語) は
  作らない。英単語が途中で割れないようにするため。連続全体にちょうど一致する辞書語・ユーザ辞書語は残す
- 数字は対象外 (今の数字の結合の扱いのまま)
- これにより CONVNBEST・CONVSEG・CONV など、読みを変換するすべてのコマンドで英字混じりの読みが
  日本語の部分だけ変換される。この変更は `modeless` によらない (英字を含む読みは TSF 層が1文節固定で
  送っているので、modeless 無効時の結果は変わらない)
- 英字を含む候補に付く `protected` (LLM の再ランクから外す。convert.rs `convert_nbest`) は今どおりとする

### C. 変換キー・候補バー

- modeless が有効で、run が英字区間 (開いている・閉じたものの両方) を含み、run 全体が英字ではない
  (日本語区間がある) とき、変換キーは英字を含む読みでも1文節に固定せず、英字を含まない run と同じ経路
  (文節 UI 無効なら CONVNBEST の入力全体の候補、有効なら CONVSEG) で変換する。生ローマ字の候補
  (`InsertRawCandidates`) は入れない。run 全体を英字にしたいときは F10 を使う
- 上記以外 (modeless 無効、run 全体が英字の `apple`、素通しの英字だけを含む `あっpぇ`) は今どおり
  1文節に固定し、生ローマ字の候補を入れる
- 候補バーの読み (`BarReading`) は今どおり: 開いている英字区間があればその開始位置まで、無ければ確定済みの
  かな全体。閉じた英字区間は日本語区間の一部として読みに含まれ、B により英字のまま変換される。
  開いている英字区間は今どおり全体変換の表記の後ろに付けて見せる
- 開いている英字区間が無い run (閉じた英字区間だけを含む) は区間分割した run として扱わない
  (予測候補も出す。今の `SplitAsciiRun` の判定のまま)
- 英単語の大文字小文字の補正 (`github` → `GitHub`) は docs/design/english-case.md

### 学習

- 変換して確定したときは、英字を含む読みでも通常の学習を送る (文節ごとの読み・表記)
- 変換せずに確定したときは、今の英字を含む入力と同じ扱い (読み = 表記)

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| engine/src/english.rs | 文字列の前方一致と、先頭部分の完全一致の長さの一覧 |
| engine/src/main.rs | ASCIIWORD の処理とテスト |
| engine/src/convert.rs | `build_lattice` の英字の連続ノード、英字の途中で始まる・終わるノードの除外、テスト |
| tsf/src/engine_client.h, tsf/src/engine_client.cpp | ASCIIWORD の呼び出し (ASCIISTART と同じく接続待ちしない) |
| tsf/src/romaji.h, tsf/src/romaji.cpp | 閉じた英字区間の積み、英字区間を閉じる操作 (先頭 k 文字で閉じて残りを打ち直す)、残りがローマ字として読めるかの判定、Backspace での開き直し、閉じた位置より後ろだけを対象にする判定 (ASCIISTART の要素・ルール4・6・7)、`RemoveFront`・`PrependKana` のずらし |
| tsf/src/text_service.h, tsf/src/text_service.cpp | 英字区間の打鍵ごとの ASCIIWORD の問い合わせと閉じる処理、`BuildConversionSegments` の英字混じり run の通常変換 |
| tsf/src/text_service_direct.cpp | `PushDirectKey` の英字モード中の打鍵で閉じる判定を通す。閉じたときの文書の書き換え (打ち直した部分がかなになる) |
| docs/protocol.md | ASCIIWORD の追記 |
| docs/design/modeless-detection.md | 「英字区間の後に日本語区間へは戻らない」を本書の参照に置き換える |
| README.md | モードレス入力の説明に英字区間を閉じる動作と英字混じりの変換を追記 |
| docs/roadmap.md | 該当項目の追加と完了 |

## 実装手順

各手順の後にビルドとテストが通り、既存の動作が壊れていないことを確かめる。

1. engine: B (英字の連続ノード)。`cargo test` に英字混じりの読みの変換テストを足す
   (`きょうはgithubでさぎょう` で `github` が割れずに1文節以内に残る、英字だけの読みも変換できる)
2. engine: ASCIIWORD (english.rs の照合と main.rs の処理、テスト)。隔離したエンジンにコマンドを送って
   結果を確かめる
3. tsf: engine_client に ASCIIWORD
4. tsf: romaji の閉じた英字区間 (閉じる操作・Backspace での開き直し・位置のずらし・判定の対象の制限)
5. tsf: text_service / text_service_direct で打鍵ごとに閉じる判定を通し、文書を書き換える
   (読める文書・追記のみの文書)
6. tsf: 変換キーの英字混じり run の通常変換
7. ドキュメントの更新 (protocol.md、modeless-detection.md、README.md、roadmap.md)

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。config.tsv は
`QUICKLIME_CONFIG_FILE` で隔離したものを使い、`modeless 1` と `debug_log` を設定する。
エンジンを release・debug の両方でビルドし直し、古いエンジンプロセスが残っていないことを確かめる。

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | メモ帳 | `kyouhagithubdesagyousuru` | `きょうはgithubでさぎょうする` |
| 2 | メモ帳 | 1 に続けて変換キー | 候補に `今日はgithubで作業する` がある。生ローマ字の候補 (`kyouhagithub…`) は出ない |
| 3 | メモ帳 | 1 の途中 (`kyouhagithubde`) の候補バー | 全体変換に `今日はgithubで` 系が出る |
| 4 | メモ帳 | `pythonwotukau` | `pythonをつかう` |
| 5 | メモ帳 | Shift+`M` `akeru` | `Makeる` (`Maker` の間は英字のまま。`makeru` は `まける` と読めるので英字区間に入らない) |
| 5a | メモ帳 | `carpetwo` | `carpetを` |
| 5b | メモ帳 | `kyouhagithubdesagyou` (debug_log) | `github` を閉じた後の `desagyou` の打鍵で、ルール5 による ASCIISTART の問い合わせがログに出ない |
| 6 | メモ帳 | `apples`、`hello world` | 今どおり英字 (`apples`、`hello world`) |
| 7 | メモ帳 | `kyouhagithubdepythonwotukau` | `きょうはgithubでpythonをつかう` (英字区間が2つ) |
| 7a | メモ帳 | `kyouhagithubdepythonwo` を1打鍵ずつ | `でpyt` の間は小窓に `pyt` が出て文書は `きょうはgithubで`。`pyth` で `きょうはgithubでpyth` になり、`github` 以降が英字にならない。最後は `きょうはgithubでpythonを` |
| 7b | メモ帳 | `korehapython` | `これはpython` (`pyt` で run 全体が英字にならない) |
| 8 | メモ帳 | `githubd` → Backspace → `s` | `githubs` (英字区間を開き直す) |
| 9 | メモ帳 | `korehaxyzwdesu` | 今どおり `これはxyzwdesu` (閉じられる切れ目が無い) |
| 10 | メモ帳 | Shift+`G` `ithubde` | `Githubで` |
| 11 | メモ帳 | 1 の後 F10 | `kyouhagithubdesagyousuru` |
| 12 | メモ帳 | `githubde` → 変換キー → ← | 範囲の伸縮は今どおり英字を含む run ではしない (確定してアプリへ渡す) |
| 13 | メモ帳 | `modeless 0` で `apple` → 変換キー | 今どおり `あっpぇ` から apple / Apple / APPLE を選べる |
| 14 | メモ帳 | `apple` → 変換キー (modeless 1) | 今どおり生ローマ字の候補が出る (run 全体が英字) |
| 15 | メモ帳 | エンジンを終了して 1 | 閉じずに今どおり `きょうはgithubdesagyousuru` |
| 16 | WezTerm | 1・4・8 と同じ操作 | メモ帳と同じ。閉じたときに日本語区間・閉じた英字区間が消されない |
| 17 | Chrome の textarea | 1・2・7 と同じ操作 | メモ帳と同じ |
| 18 | 隔離したエンジン | `ASCIIWORD\tgithubd` | `OK\t0\t6` |
| 19 | 隔離したエンジン | `ASCIIWORD\tmaker` | `OK\t1\t4,5` |

## 未決事項

- 完全一致とみなす語の最小の長さ (3文字)。`go` `is` などの短い語で閉じないことの不便さを常用で見る
- 辞書の語で始まる辞書に無い語 (`githubber` など) で誤って閉じる頻度
- 英字混じりの候補が LLM の再ランクから外れる (`protected`) こと。zenz に英字混じりの読みを渡す効果は
  別に検証する
- 英単語の大文字小文字の補正 (`github` → `GitHub`) は docs/design/english-case.md
