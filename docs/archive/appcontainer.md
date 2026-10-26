# AppContainer のアプリでの入力

## 背景と目的

スタートメニューの検索欄で QuicklIME が効かない。全角/半角キーを押しても IME の状態が変わらず、日本語を入力できない。

検索欄を表示する SearchHost.exe は AppContainer で動く (トークンの `TokenIsAppContainer` が 1)。AppContainer のプロセスから
QuicklIME を使うには、次の 4 つが足りない。

1. DLL を読めない。開発版の `tsf/build/Debug/QuicklIME.dll` には `ALL APPLICATION PACKAGES` (S-1-15-2-1) の ACE が無く、
   SearchHost には DLL が読み込まれていない (`tasklist /M QuicklIME.dll`)。インストール版は Program Files から
   `ALL APPLICATION PACKAGES` と `ALL RESTRICTED APPLICATION PACKAGES` の RX を引き継ぐので読める
2. エンジンのパイプを開けない。今のパイプの記述子 `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;<user>)S:(ML;;NW;;;ME)` には
   AppContainer の ACE が無く、整合性ラベルも Medium のため。インストール版でも変換できない
3. `%APPDATA%\QuicklIME\` の config.tsv・romaji.tsv・themes を読めず、TSF 層が既定値で動く
4. AppContainer の中から `CreateProcess` したエンジン・単語登録ツール・設定ツールは同じ AppContainer で動き、辞書や学習・
   設定のファイルを読み書きできない

AppContainer のアプリ (スタートメニューの検索欄・UWP アプリなど) でも、通常のアプリと同じように変換・予測・学習が
使えるようにする。

## 決定事項

### DLL のファイルの権限

- 開発版の DLL は、ビルド後に `icacls <DLL> /grant *S-1-15-2-1:RX *S-1-15-2-2:RX` で読めるようにする (tsf/CMakeLists.txt の
  POST_BUILD)。インストール版と同じ 2 つの SID にそろえる。32bit 版のビルドも同じ CMakeLists.txt なので同じく付く
- インストール版はインストーラでは何もしない。Program Files の継承で付く。`scripts\dev-deploy.ps1 -Dll -Install` の
  `Copy-Item` も新しいファイルを作るので、コピー先の継承で付く

### パイプのアクセス権

- 記述子を `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;<user>)(A;;GA;;;AC)S:(ML;;NW;;;LW)` にする
  - `AC` (ALL APPLICATION PACKAGES) に読み書きを許し、整合性ラベルを Low にする
  - AppContainer のアクセスチェックでは、ユーザとグループの判定と AppContainer の判定の両方を通る必要がある。DACL の
    ユーザは現在のユーザの SID だけなので、ほかのユーザの AppContainer のプロセスは今までどおり開けない
  - ラベルを Low にするので、AppContainer ではない Low の整合性レベルのプロセスも、同じユーザなら開ける
- 受け入れるリスク: 同じユーザのサンドボックス化されたアプリが、予測などで学習の内容を引き出し、学習を書き込み・消し、
  LLM の並べ替えで負荷をかけられる。CorvusSKK も同じ記述子 (`(A;;GA;;;AC)` と `S:(ML;;NW;;;LW)`) で開いている。
  AppContainer にパイプを開かないと、スタートメニューの検索欄で変換できないので受け入れる
- SearchHost は LPAC (Less Privileged AppContainer) ではなく、`AC` の ACE で開ける (実機で変換と予測が動いた)

### AppContainer のクライアントに許さない要求

- エンジンは接続を受けたときに、クライアントのプロセスが AppContainer かを判定する
  - `Stream::peer_creds()` でクライアントの PID を取り、`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`・
    `OpenProcessToken(TOKEN_QUERY)`・`GetTokenInformation(TokenIsAppContainer)` で調べる。判定は接続ごとに 1 回で、
    `Session` に持つ
  - 判定できなかったとき (プロセスやトークンを開けないなど) は AppContainer ではないものとして扱う。通常権限のエンジンは、
    管理者として動くクライアントのトークンを開けない。そのため、拒否する側に倒すと管理者で起動したツールが使えなくなる。
    同じユーザの AppContainer のプロセスのトークンは、通常権限のエンジンからも開ける
- AppContainer のクライアントからの ADDWORD・RELOADUSER・RELOADCONFIG は処理せず、`ERR\tAppContainer のアプリからは使えません\n`
  を返す。どれも単語登録ツール・設定ツール用で、TSF 層は送らない
- 予測・変換・学習 (LEARN 系・FORGET を含む) は TSF 層が使うので許す

### 設定ファイルの権限

- config.tsv・romaji.tsv と themes のフォルダに、`ALL APPLICATION PACKAGES` の読み取り (FILE_GENERIC_READ) の ACE を足す
  - 今の DACL に足す (置き換えない)。同じ ACE があれば何もしない
  - themes のフォルダの ACE は、中のファイルとフォルダに継承させる (OI|CI)。中のファイルを置き換えても継承で付く
  - 学習 (learning*.tsv)・ユーザ辞書 (userdict.tsv)・インポート辞書 (imported) には付けない。入力の履歴を
    UWP アプリに丸ごと読ませないため
- 付けるのは次の 2 か所
  - エンジンの起動時。3 つのパスを、TSF 層と同じ規則で求める (`QUICKLIME_CONFIG_FILE`・`QUICKLIME_ROMAJI_FILE`・
    `QUICKLIME_THEME_DIR` を優先)。無いパスは飛ばす
  - 設定ツールが config.tsv を保存した後。保存は一時ファイルへの書き込みと rename なので、置き換えたファイルは
    親のフォルダの継承の ACL になり、足した ACE が消えるため
- 失敗しても標準エラー (設定ツールでは無視) にとどめて動作を続ける。AppContainer のアプリで既定値になるだけなので
- 手で編集したエディタがファイルを置き換えた場合や、romaji.tsv を新しく作った場合は、次にエンジンが起動するまで
  AppContainer のアプリでは既定値で動く (既知の問題として roadmap に書く)
- `%APPDATA%\QuicklIME\` のフォルダ自体には付けない。AppContainer のトークンも走査のチェックを省く特権
  (SeChangeNotifyPrivilege) を持つので、ファイルをフルパスで開ける (実機で SearchHost が config.tsv の設定で動いた)

### AppContainer の TSF 層からはプロセスを起動しない

- TSF 層は、自分のプロセスが AppContainer か (`GetTokenInformation(TokenIsAppContainer)`) を 1 回だけ調べて覚える
- AppContainer では、エンジンの自動起動 (`EngineClient::TryLaunchEngine`)、単語登録ツール (`TextService::LaunchWordRegister`)、
  設定ツール (`TextService::LaunchConfigTool`) を起動しない
  - エンジン: パイプを開いてみるだけにする。起動しようとしないので、待つ問い合わせでも待たずに失敗にする。ログオン時に
    explorer の TSF 層がエンジンを起動するので、ふつうはもう動いている。動いていなければ、エンジンにつながらないときの
    フォールバック (ひらがな 1 候補) で動く
  - ツール: 何もしない。AppContainer の中で起動しても保存できないため
- 調べられなかったときは AppContainer ではないものとして扱う (今の挙動のまま)

### 入力パネルなどのウィンドウのオーナー

検索欄の画面 (SearchHost の CoreWindow) は immersive の重なり順の層にある。オーナー無しで作った入力パネルは
`WS_EX_TOPMOST` でもデスクトップの層に置かれ、検索欄の後ろに隠れる (診断ログでは、表示は `visible=1` で候補も出ていた)。
オーナー付きのウィンドウはオーナーのすぐ上に同じ層で置かれるので、どのアプリでも文書のウィンドウをオーナーにする
(CorvusSKK も候補ウィンドウのオーナーに `ITfContextView::GetWnd` のウィンドウを渡している)。入力先のウィンドウを
閉じたときに入力パネルも消えるのも自然な挙動になる。

- 対象: 入力パネル (layered とぼかしの両方)、入力モードのポップアップ、キャレットの印
- オーナーにするウィンドウ: 文書のウィンドウの最上位 (`ITfContextView::GetWnd` → `GetAncestor(GA_ROOT)`)。ビューが
  ウィンドウを返さない文書では、スレッドのフォーカスのウィンドウ (無ければアクティブのウィンドウ) の最上位。
  `TextService::FallbackIfOutsideWindow` (tsf/src/text_service_direct.cpp) が文書のウィンドウを求める処理と同じなので、
  共通の関数にまとめる。どちらも取れなければ今までどおりオーナー無し
- オーナーが変わったとき: 表示するときに求めたオーナーが、ウィンドウを作ったときのオーナーと違えば作り直す
  (入力パネルは作ったウィンドウを使い回すことがあるため)
- オーナーが閉じられたとき: オーナー付きのウィンドウはオーナーと一緒に OS が破棄する。3 つとも `WM_DESTROY` で
  `hwnd_` を null にし (自分で破棄したときも外から破棄されたときも)、次に表示するときに作り直す
- 受け入れる挙動の変化: オーナーを最小化すると隠れる。オーナーより下には回らない

### 採らなかった案

- ウィンドウのオーナー
  - AppContainer のプロセスのときだけオーナーを付ける: 通常のアプリの挙動は変わらないが、入力先のウィンドウを閉じても
    入力パネルが残りうる。どのアプリでも同じ作りにするほうが自然
- 設定ファイルの権限
  - `%APPDATA%\QuicklIME\` に継承する ACE を付ける: 確実で単純だが、学習やユーザ辞書 (入力の履歴) もどの UWP アプリ
    からでも読めるようになる
  - TSF 層がパイプでエンジンから設定を受け取る: ファイルの権限には触れないが、プロトコルの追加と TSF 層の読み込みの
    経路の分岐が要り、大がかり
  - 何もせず既定値で動かす: キー割当・ローマ字テーブル・テーマの変更が検索欄では効かない
  - CorvusSKK のように、保護された DACL で丸ごと置き換える: 利用者が付けた権限を消してしまう
- ツールの起動
  - エンジンに起動させる (CorvusSKK の設定ツールの起動と同じ): どこからでも開けるが、プロトコルの追加と、UWP アプリから
    エンジンにプロセスを起動させられる経路ができる。スタートメニューの検索欄でツールを開く必要はまずない
- AppContainer のクライアントの要求を制限しない: 拒否する 3 つは TSF 層が使わないので、拒否しても失うものが無い

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/CMakeLists.txt | DLL の POST_BUILD で `icacls` |
| engine/src/main.rs | `pipe_sddl` の記述子。接続ごとの AppContainer の判定と `Session` への保持。ADDWORD・RELOADUSER・RELOADCONFIG の拒否。起動時の設定ファイルへの ACE の付与の呼び出し |
| engine/src/ (新しいモジュールか既存の config.rs) | ファイルとフォルダに `ALL APPLICATION PACKAGES` の読み取りの ACE を足す関数。romaji.tsv・themes のパスの求め方 |
| engine/gui/src/config.rs | `Config::save` の後に config.tsv へ ACE を足す (エンジン側の関数を共有できなければ同じものを置く) |
| tsf/src/engine_client.cpp・.h | AppContainer では `TryLaunchEngine` が起動しない。待つ問い合わせが待たずに失敗する |
| tsf/src/text_service.cpp | AppContainer では `LaunchWordRegister`・`LaunchConfigTool` が何もしない |
| tsf/src/ (globals など) | 自分のプロセスが AppContainer かの判定と保持 |
| tsf/src/suggest_panel.cpp・.h、mode_popup.cpp・.h、caret_marker.cpp・.h | オーナーを受け取って作る。オーナーが変われば作り直す。`WM_DESTROY` で `hwnd_` を null にする |
| tsf/src/text_service*.cpp | 文書のウィンドウの最上位を求める共通の関数 (`FallbackIfOutsideWindow` と共有) と、3 つのウィンドウへのオーナーの受け渡し |
| docs/spec/display.md | 入力パネル・入力モードのポップアップ・キャレットの印の節に、オーナーと破棄・作り直し (WHY は immersive の層) |
| docs/spec/runtime.md | 「パイプのアクセス権」(新しい記述子とリスク、AppContainer のクライアントに許さない要求)、「エンジンの自動起動」(AppContainer では起動しない)、「DLL の登録」か「配布物」(DLL のファイルの権限)、「採らなかった案」(Low にする案を、今回の採らなかった案に書き換える) |
| docs/spec/tools.md | 「場所と書式」か「読み込みと反映」(設定ファイルの権限)、「起動」(AppContainer のアプリからは開かない)、「保存」(保存後に ACE を足す)、「採らなかった案」 |
| docs/spec/input.md | 「ローマ字テーブル」に、AppContainer のアプリで読むための ACE (tools.md への参照) |
| docs/spec/display.md | 「テーマ」に同じく参照 |
| docs/protocol.md | ADDWORD・RELOADUSER・RELOADCONFIG に、AppContainer のクライアントへの ERR |
| docs/development.md | 「ビルド」か「注意」に、開発版の DLL に付ける権限と、SearchHost で試すときの手順 (下記「検証方法」) |
| docs/user/ (単語登録ツール・設定ツールの開き方を書いたページ) | スタートメニューの検索欄などでは単語登録ツール・設定ツールを開けないこと |
| docs/roadmap.md | 既知の問題に「手で置き換えた設定ファイルは、次にエンジンが起動するまで AppContainer のアプリで読めない」 |
| docs/design/README.md | 進行中の設計書にこの設計書を足す (実装後に archive へ移すときに消す) |

## 実装手順

1. tsf/CMakeLists.txt の POST_BUILD に `icacls` を足し、ビルドして DLL に 2 つの ACE が付くことを確かめる
2. エンジンの `pipe_sddl` を書き換え、単体テストを直す
3. エンジンに、接続ごとの AppContainer の判定と、3 つの要求の拒否を足す。拒否の判定は `handle_request` の前で
   `Session` の値を見て行い、AppContainer として扱う場合の単体テストを足す
4. ACE を足す関数を書き、エンジンの起動時 (待ち受けを始める前で、辞書の読み込みを待たせない位置) と設定ツールの保存後に
   呼ぶ。一時ファイルで ACE が足されること・2 回呼んでも増えないこと・継承の指定を単体テストで確かめる
5. TSF 層に AppContainer の判定を足し、エンジンの自動起動・ツールの起動を止める
6. 仕様書・protocol.md・development.md・docs/user・roadmap を書き換える
7. 入力パネル・入力モードのポップアップ・キャレットの印のオーナー (「入力パネルなどのウィンドウのオーナー」) を実装し、
   display.md を書き換える

## 検証方法

### 単体テスト

- `pipe_sddl` が `(A;;GA;;;AC)` を含み、`S:(ML;;NW;;;LW)` で終わる
- AppContainer のクライアントとして扱う `Session` で、ADDWORD・RELOADUSER・RELOADCONFIG が `ERR\t...` を返し、ユーザ辞書や
  設定が変わらない。SUGGEST・LEARN などはそのまま処理される
- 通常のクライアントでは 3 つの要求が今までどおり処理される
- ACE を足す関数: 一時ファイルに `S-1-15-2-1` の読み取りの ACE が付く。2 回呼んでも 1 つのまま。フォルダには継承の
  ACE が付き、後から作ったファイルに継承される。元の ACE が残っている

### 実機 (SearchHost)

SearchHost は自分で起動するプロセスではないので、環境変数での隔離が効かず、常用のエンジンと実データにつながる。
壊れたときは Win+Space で Microsoft IME に切り替え、`taskkill /IM SearchHost.exe /F` で検索欄を作り直す (自動で
起動し直す)。

1. `tasklist | grep quicklime` で古いエンジンを終了し、新しいエンジンを起動する (新しい記述子のパイプにするため)
2. `scripts\dev-deploy.ps1 -Dll` で開発版に切り替え、`taskkill /IM SearchHost.exe /F` で検索欄を作り直す
3. 次を確かめる

| # | 操作 | 期待 |
|---|---|---|
| 1 | `tasklist /M QuicklIME.dll /FI "IMAGENAME eq SearchHost.exe"` | SearchHost に読み込まれている |
| 2 | 検索欄で全角/半角キー | IME がオンになる |
| 3 | 検索欄で `nihongo` → Space | 漢字の候補が出る (エンジンにつながっている) |
| 4 | 検索欄で予測が出る読みを入力 | 入力パネルに予測が出る |
| 5 | config.tsv でキー割当や `punctuation` を既定から変え、エンジンを起動し直して検索欄で試す | 設定が効いている |
| 6 | 設定ツールで設定を変えて保存し、検索欄を作り直して試す | 設定が効いている。`icacls config.tsv` に `ALL APPLICATION PACKAGES:(R)` がある |
| 7 | themes に自作のテーマを置いて選び、検索欄で入力 | そのテーマで表示される |
| 8 | 検索欄で単語登録ツール・設定ツールを開くキーを押す | 何も起きない。検索欄は固まらない |
| 9 | エンジンを終了してから検索欄で入力する | エンジンは起動せず (`tasklist`)、ひらがなで入力できる。メモ帳で入力するとエンジンが起動する |
| 10 | メモ帳・管理者のターミナルで入力、単語登録ツールで登録、設定ツールで保存 | 今までどおり動く (登録・RELOADCONFIG が拒否されない) |
| 11 | `icacls %APPDATA%\QuicklIME\learning.tsv` | `ALL APPLICATION PACKAGES` が無い |

ウィンドウのオーナーの回帰 (通常のアプリ):

| # | 操作 | 期待 |
|---|---|---|
| 12 | メモ帳・管理者のターミナル・Chrome か Edge・Office 系で入力・変換・IME の切替 | 入力パネル・入力モードのポップアップ・キャレットの印が今までどおりの位置に見える |
| 13 | 入力パネルを出したままアプリのウィンドウを閉じる | 入力パネルも消える。別のアプリで入力すると入力パネルが出る |
| 14 | 入力パネルを出したまま最小化し、元に戻してから打つ | 最小化 (フォーカスの移動) で run が終わって入力パネルが消える。戻した時点では出ず、打つと新しい run の入力パネルが出る |
| 15 | 入力パネルを出したまま別のアプリへ切り替え、そこで入力する | 切り替え先のアプリの上に出る |
| 16 | 入力パネルのテーマのぼかしを使うテーマで 12 と 13 | 同じく見え、閉じると消える |

- 3 でつながらないときは、SearchHost が LPAC の可能性がある。パイプに `S-1-15-2-2` の ACE を足して試す
- 5〜7 で設定が効かないときは、フォルダの走査で弾かれている可能性がある。`%APPDATA%\QuicklIME\` への ACE の要否を確かめる

## 未決事項

- オーナーを付けると入力パネルが検索欄の上に出るか (推定。検証の 2〜4 で確かめる)
