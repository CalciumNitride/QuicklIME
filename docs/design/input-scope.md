# InputScope による素通し

## 背景と目的

数字・電話番号・メールアドレス・URL などの入力欄は、半角英数字を前提にしている。IME を通った文字は全角になったり
composition になったりして、1文字1枠の認証コード欄などで崩れる (docs/design/input-buffer-modes.md の
「フォームの計測結果」)。今は run の外の半角数字だけを素通しにしている (`digits=half`) ので、英字を含む欄は
救えず、利用者が IME をオフにするしかない。

アプリは入力欄の種類を InputScope (`GUID_PROP_INPUTSCOPE`) で IME に伝えている。Chromium は HTML の `type` と
`inputmode` を次のように対応させる (ui/base/ime/win/tsf_input_scope.cc)。

| HTML | InputScope |
|---|---|
| `inputmode="numeric"` | `IS_DIGITS` |
| `type="number"` / `inputmode="decimal"` | `IS_NUMBER` |
| `type="tel"` / `inputmode="tel"` | `IS_TELEPHONE_FULLTELEPHONENUMBER` |
| `type="email"` / `inputmode="email"` | `IS_EMAIL_SMTPEMAILADDRESS` |
| `type="url"` / `inputmode="url"` | `IS_URL` |
| `type="search"` / `inputmode="search"` | `IS_SEARCH` |
| `type="password"` | `IS_PRIVATE` |
| シークレットウィンドウ・ゲストの欄 | 上記に `IS_PRIVATE` を足す |

Chromium は入力欄を移るたびに `ITfThreadMgr::SetFocus` を呼び直す (同じ種類の欄の間では、空の文書を挟んで同じ
文書へ戻す。tsf_bridge.cc の `OnTextInputTypeChanged`)。InputScope は属性を取り出す時点の欄の種類から作るので、
フォーカスが移るたびに読めば欄ごとの値が取れる。

そこで、半角英数字を前提にした InputScope の欄では、IME がオンでもキーを食べずにアプリへ渡す。

## 決定事項

### 対象の InputScope

フォーカス中の文書の InputScope の一覧に、次のどれかが含まれていれば対象にする。

| 種類 | InputScope |
|---|---|
| 数字 | `IS_DIGITS`、`IS_NUMBER` (`IS_NUMBER_FULLWIDTH` は対象外) |
| 電話番号 | `IS_TELEPHONE_FULLTELEPHONENUMBER`、`IS_TELEPHONE_COUNTRYCODE`、`IS_TELEPHONE_AREACODE`、`IS_TELEPHONE_LOCALNUMBER` |
| メール・URL | `IS_EMAIL_SMTPEMAILADDRESS`、`IS_EMAIL_USERNAME`、`IS_URL` |
| 半角英数 | `IS_ALPHANUMERIC_HALFWIDTH` |
| パスワード・PIN | `IS_PASSWORD`、`IS_NUMERIC_PASSWORD`、`IS_NUMERIC_PIN`、`IS_ALPHANUMERIC_PIN`、`IS_ALPHANUMERIC_PIN_SET` |

- `IS_PRIVATE` は対象にしない。シークレットウィンドウでは通常の欄にも付くため。Chrome のパスワード欄は
  今どおり compartment (`GUID_COMPARTMENT_KEYBOARD_DISABLED` など) で判定する (docs/design/basics.md)
- `IS_SEARCH`・`IS_DEFAULT` など、上記以外だけの欄は対象にしない
- 定数の値は Windows SDK の InputScope.h に従う (古い SDK に無い定数は値で書く)

### 挙動

- 対象の欄では、キーを一切食べない。IME オフのとき、またはキー入力が無効な文書と同じ扱いにする
  (`IsKeyEaten` の入口で判定する)。変換キー・機能キーの割当も働かない
- 言語バー・モード表示は変えない (IME はオンのまま)。対象でない欄へ移れば、そのまま日本語入力に戻る
- フォーカス移動で run は終わっているので、対象の欄で run が残ることはない

### 取得の時点

- `OnSetFocus` (`ITfThreadMgrEventSink`) で判定を「未取得」に戻し、読み取りの edit session
  (`TF_ES_ASYNCDONTCARE | TF_ES_READ`) で InputScope を読む。同期で走らなかったときは、走った時点で結果を入れる
  (今の診断ログの `GetInputScopeEditSession` の取得方法を判定にも使う)
- キーの判定の時点で「未取得」なら、同期の読み取り (`TF_ES_SYNC | TF_ES_READ`) を試み、結果を同じ値にしまう。
  読めなければ対象外として扱う (IME を使う側に倒す)
- 後から走った非同期の結果は、そのフォーカスの世代のものだけを使う (フォーカスが移った後に届いた古い結果で
  上書きしない)

### 設定

| キー | 値 | 既定 | 内容 |
|---|---|---|---|
| `input_scope_passthrough` | `1` / `0` | `1` | 1 なら対象の InputScope の欄でキーを食べない |

設定ツールの「入力」タブにチェックボックス「数字・電話番号・メールアドレス・URL などの入力欄では日本語入力を使わない」を置く。

### 診断ログ

`debug_log` を設定しているときは、フォーカス先の InputScope の一覧 (今の出力) に加えて、素通しにするかどうかの
判定を出す。

## 採らなかった案

- 対象の欄で IME のモードを半角英数に切り替え、言語バーにも出す案 (MS-IME の挙動に近い): QuicklIME には入力モードが
  無く (オン / オフだけ)、オフに切り替えると欄を出たときに戻す処理が要る。キーを食べないだけで目的を果たせる
- 数字系だけを対象にする案: メールアドレス・URL の欄でも、かなに化けて利用者が IME をオフにする手間が残る
- InputScope の指定が無い英数字の欄 (`inputmode` なしの認証コード欄) を推測で見分ける案: 判断材料が無く、
  誤判定すると通常の欄で日本語が打てなくなる。IME をオフにして入力してもらう

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| tsf/src/config.h, tsf/src/config.cpp | `input_scope_passthrough` の読み込み |
| tsf/src/edit_session.h, tsf/src/edit_session.cpp | `GetInputScopeEditSession` が InputScope の値の一覧も返すようにする |
| tsf/src/text_service.h, tsf/src/text_service.cpp | フォーカスの世代と判定結果の保持、`OnSetFocus` での取得、`IsKeyEaten` の入口での判定、診断ログ |
| engine/src/bin/quicklime-config.rs | 「入力」タブのチェックボックスと `input_scope_passthrough` の読み書き |
| README.md | 設定と挙動の説明 |
| docs/design/README.md | 本書を足す |
| docs/roadmap.md | 「InputScope の指定が無い英数字の欄では IME をオフにする必要がある」を既知の問題に足す |

## 実装手順

1. config と設定ツールに `input_scope_passthrough` を足す
2. `GetInputScopeEditSession` から値の一覧を取り出せるようにし、対象かどうかを判定する関数を作る
3. `OnSetFocus` での非同期取得とフォーカスの世代、キー判定時の同期取得
4. `IsKeyEaten` の入口で素通しにする。診断ログを足す
5. README.md・docs を直す

docs/design/input-buffer-modes.md とは独立に実装できる。両方を進めるときは input-buffer-modes.md を先に終える
(同じ text_service.cpp の入口を触るため)。

## 検証方法

ビルドと反映は docs/design/input-buffer-modes.md と同じ。検証用の HTML (次の欄を並べ、各欄の keydown・input と
value を記録するもの) を Chrome で開いて確かめる。`debug_log` で InputScope と判定を確認する。

### テストケース

| # | 欄 | 操作 | 期待 |
|---|---|---|---|
| 1 | `inputmode="numeric"` の1文字1枠 | `1a2b3c` | すべて半角のまま素のキーとして入る (keyCode 229 が出ない、composition が出ない) |
| 2 | `type="tel"` | `090-1234` | 半角で入る |
| 3 | `type="email"` | `foo@example.com` | 半角で入り、候補パネルが出ない |
| 4 | `type="url"` | `https://` | 半角で入る |
| 5 | `type="number"` | `12` | 半角で入る |
| 6 | 指定なしの欄 | `kyouha` | 日本語入力が働く |
| 7 | `type="search"` | `kyouha` | 日本語入力が働く |
| 8 | シークレットウィンドウの指定なしの欄 | `kyouha` | 日本語入力が働く (`IS_PRIVATE` だけでは素通しにしない) |
| 9 | `type="password"` | `abc` | 今どおり IME が働かない (compartment の判定) |
| 10 | 3 の欄 → Tab → 6 の欄 | `kyouha` | 6 の欄では日本語入力に戻る |
| 11 | 6 の欄 → 3 の欄をクリック | 続けて打鍵 | 3 の欄では素通しになる |
| 12 | 同じ `type="email"` の欄を2つ並べて行き来 | 打鍵 | どちらも素通し (同じ種類の欄の間の移動でも判定し直す) |
| 13 | 3 の欄 | `input_scope_passthrough 0` | 日本語入力が働く (今の挙動) |
| 14 | メモ帳 | `kyouha` | 日本語入力が働く (InputScope が既定) |
| 15 | 設定ツール | 「入力」タブのチェックボックスを切り替えて保存 | config.tsv の `input_scope_passthrough` が書き換わる |
