# 実行と配布

QuicklIME のプロセスの起動と配布物の仕様。プロセスの構成、TSF 層とエンジンの間のパイプ (名前・接続・アクセス権)、
エンジンの自動起動と二重起動の防止、LLM の子プロセスの起動と監視、TSF の DLL の登録、インストール先の構成と
インストーラ、第三者の著作権表示を扱う。

- 利用者向けの説明は [README.md](../../README.md) の「インストール」「アンインストール」「同梱物とライセンス」、
  ビルドとインストーラの作成の手順は [docs/development.md](../development.md) にある
- パイプの上の要求と応答の形式は [docs/protocol.md](../protocol.md)、エンジンと quicklime-llm の間の行の形式と試しの推論は
  [docs/protocol.md](../protocol.md) の「エンジン - quicklime-llm 間」。本書は形式を重複させず、プロセスの起動・終了と
  監視を書く
- LLM の並べ替えと生成の規則、`llm_backend` ごとの子プロセスとモデル、依頼と結果の扱いは
  [engine.md](engine.md)「LLM の並べ替え」「モデルと尤度」「依頼と結果」「LLM による候補の生成」
- 設定ツールの起動と二重起動の防止は [tools.md](tools.md)「起動」、単語登録ツールの起動は
  [dictionary.md](dictionary.md)「単語登録ツール」。本書は 2 つのツールの exe の探し方を書く
- 辞書ディレクトリと各辞書のファイルの探し方は [dictionary.md](dictionary.md)「辞書ディレクトリ」、英単語辞書は
  [modeless.md](modeless.md)「英単語辞書」、後続語の統計は [suggest.md](suggest.md)「エンジンでの読み込み」。
  本書はインストール先にどのファイルを置くかを書く
- 実装は tsf/src/engine_client.cpp (パイプへの接続・エンジンの自動起動・exe の探し方)、tsf/src/text_service.cpp
  (`ActivateEx` での起動、`LaunchWordRegister`・`LaunchConfigTool`)、tsf/src/globals.cpp (自分のプロセスが AppContainer か)、
  tsf/src/registry.cpp と tsf/src/dll_main.cpp (DLL の登録)、tsf/CMakeLists.txt (開発版の DLL のファイルの権限)、
  engine/src/main.rs (パイプの待ち受け・二重起動の防止・パイプのアクセス権・AppContainer のクライアントの判定)、engine/src/llm.rs
  (LLM の子プロセスの起動と監視)、llm/src/lib.rs (quicklime-llm)、installer/installer.iss (インストーラ)、
  installer/build.ps1 (インストーラのビルド)、installer/about.toml と installer/about.hbs (Rust の crate の表示)、
  scripts/fetch-models.ps1 (モデルの取得)、scripts/build-llm.ps1 (LLM の exe のビルド)

## プロセスの構成

| プロセス | 起動するもの | 数 |
|---|---|---|
| TSF の DLL (QuicklIME.dll。64bit と 32bit) | TSF がアプリのプロセスに読み込む | 入力に使うアプリごと |
| エンジン (quicklime-engine.exe) | TSF 層 (下記「エンジンの自動起動」) | パイプ名ごとに 1 つ |
| LLM の子プロセス (quicklime-llm.exe・quicklime-llm-vulkan.exe) | エンジン (下記「LLM の子プロセス」) | エンジンごとに多くて 1 つ |
| 設定ツール (quicklime-config.exe) | TSF 層 ([tools.md](tools.md)「起動」)・スタートメニューのショートカット | 1 つ |
| 単語登録ツール (quicklime-regword.exe) | TSF 層 ([dictionary.md](dictionary.md)「単語登録ツール」) | 制限なし |

- エンジンは 1 つのパイプで全部のアプリの TSF 層からの接続を受け、接続ごとに 1 スレッドで応答する。学習とユーザ辞書を
  全アプリで共有するため
- エンジンは、起動の直後の二重起動の確かめ (下記「二重起動の防止」) で終わるときを除き、自分からは終了しない。終わるのは
  プロセスを終了されたとき (タスクマネージャー・インストーラ・サインアウト)
- エンジンと quicklime-llm は動作のログを標準エラーにだけ出し、ファイルには書かない。エンジンは子プロセスの標準エラーを
  捨てる。TSF 層の診断ログは [tools.md](tools.md)「診断ログ」

## パイプ

### パイプ名

- `\\.\pipe\quicklime-engine`。環境変数 `QUICKLIME_PIPE_NAME` があれば `\\.\pipe\` の後ろをその値にする (エンジン・TSF 層・
  設定ツール・単語登録ツールで同じ)
- TSF 層はプロセスで最初に接続するときに 1 度だけ環境変数を読み、以後は変えない。`DllMain` の中で環境変数を読まないため。
  値が空のときは既定の名前を使う
- 環境変数で名前を変えるのは、動作の検証で常用のエンジンから隔離するため。パイプ名ごとに二重起動の防止も分かれるので、
  隔離したエンジンは常用のエンジンと並んで動く (下記「二重起動の防止」)

### 接続

- TSF 層は、テキストサービスのインスタンスごとに 1 本の接続を持ち、開いたまま使い続ける。送れない・応答を読めないときは
  接続を閉じる
- 開くときは 1 回だけ開いてみる。全部のインスタンスが使用中 (`ERROR_PIPE_BUSY`) のときだけ、200 ms まで待って 1 回開き直す
- 待ってよい問い合わせ (下記「起動する問い合わせ」) は、送れなかったら 1 回だけ接続し直して送り直す。エンジンが再起動した
  直後は古い接続が切れているため
- 応答の読み取りに時間の上限は無い ([docs/protocol.md](../protocol.md) の「設計メモ」)。1 行の応答が 64KiB を超えたら
  接続を閉じて失敗にする
- 接続を閉じたときは、旧エンジンと判定した記録のうち接続ごとのもの (RERANK・SUGGEST・`kinds` を知らない) を消す。次の接続の
  相手のエンジンが入れ替わっているかもしれないため

### パイプのアクセス権

管理者として動くアプリ (管理者で起動したターミナルなど) の TSF 層がエンジンを起動すると、エンジンも管理者権限で動く。
セキュリティ記述子を指定しないと、昇格した token の既定の DACL と High の整合性レベルでパイプができ、通常権限のアプリが
読み書きで開けない。また、AppContainer で動くアプリ (スタートメニューの検索欄の SearchHost.exe・UWP アプリなど) の TSF 層は、
AppContainer の ACE と Low 以下の整合性ラベルが無いパイプを開けない。そのため、エンジンはどの権限で起動されても次の記述子を
付けてパイプを作る。

```
D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;<現在のユーザの SID>)(A;;GA;;;AC)S:(ML;;NW;;;LW)
```

- DACL は SYSTEM・Administrators・現在のユーザと、ALL APPLICATION PACKAGES (`AC`) に許す。パイプ名はセッションをまたいで
  共通なので、ほかのユーザには開かない。昇格した token もユーザの SID は同じなので、管理者のアプリと通常権限のアプリの
  両方から接続できる
- AppContainer のアクセスチェックは、ユーザとグループの判定と AppContainer の判定の両方を通る必要がある。DACL のユーザは
  現在のユーザの SID だけなので、ほかのユーザの AppContainer のプロセスは開けない
- 現在のユーザを表す SDDL の別名は無いので、プロセスの token のユーザの SID を文字列にして埋め込む
- 整合性ラベルは Low にする。AppContainer のプロセスの整合性レベルは Low のため。AppContainer ではない Low の整合性レベルの
  プロセスも、同じユーザなら開ける。自分の整合性レベル以下へ下げるのに特権は要らない
- 受け入れるリスク: 同じユーザのサンドボックス化されたアプリが、予測などで学習の内容を引き出し、学習を書き込み・消し、
  LLM の並べ替えで負荷をかけられる。AppContainer に開かないとスタートメニューの検索欄で変換できないので受け入れる。
  CorvusSKK も同じ記述子 (`(A;;GA;;;AC)` と `S:(ML;;NW;;;LW)`) でパイプを作る
- SID や記述子を作れなかったときは、標準エラーに出して既定の記述子でパイプを作る (エンジンが起動しないよりよい)
- 通常権限のプロセスが管理者のエンジンに要求を送れるが、要求が触るのはユーザのプロファイルの辞書・学習・設定 (通常権限で
  直接書き換えられるもの) だけで、LLM の exe はエンジンの exe の場所から引く。利用者が指定できる実行パスは無いので、
  権限の昇格にはならない

### AppContainer のクライアントに許さない要求

- エンジンは接続を受けたときに 1 回、クライアントのプロセスが AppContainer かを調べ、その接続のあいだ覚えておく。
  パイプのクライアントの PID から `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`・`OpenProcessToken(TOKEN_QUERY)`・
  `GetTokenInformation(TokenIsAppContainer)` で調べる
- 調べられなかったとき (プロセスやトークンを開けないなど) は、AppContainer ではないものとして扱う。通常権限のエンジンは
  管理者として動くクライアントのトークンを開けないので、拒否する側に倒すと管理者で起動したツールが使えなくなる。同じ
  ユーザの AppContainer のプロセスのトークンは、通常権限のエンジンからも開ける
- AppContainer のクライアントからの ADDWORD・RELOADUSER・RELOADCONFIG は処理せず、ERR を返す ([docs/protocol.md](../protocol.md))。
  どれも単語登録ツール・設定ツール用で、TSF 層は送らない
- 予測・変換・学習 (LEARN 系・FORGET を含む) は TSF 層が使うので許す

## エンジンの自動起動

TSF 層は、エンジンのパイプを開けないときにエンジンを起動する。スタートアップへの登録はしない。

- AppContainer で動くアプリの TSF 層は、エンジンを起動しない (下記「AppContainer のアプリ」)

### 起動する問い合わせ

| 起動の仕方 | 問い合わせ |
|---|---|
| 開けなければ起動だけする (起動を待たず、その問い合わせは失敗) | アクティブ化 (`ActivateEx`)、SUGGEST、打鍵ごとの CONVNBEST、ASCIISTART・ASCIIWORD・ASCIIFORM、RERANK、PREDICT・NEXTWORD |
| 開けなければ起動して待つ | CONVSEG・CONVCTX、変換キーなどの CONVNBEST、CONVSYM、CONVUSER、学習 (LEARN・LEARN2・LEARNSEG・LEARNWORD) |
| 起動しない (開いてみるだけ) | REVERSE、FORGET、RERANKGET (接続済みのときだけ送る) |

- アクティブ化で起動しておくのは、最初の変換より前にエンジンを立ち上げておくため。ログオン時は explorer などのプロセスでも
  TSF がアクティブ化されるので、実際にはログオンと同時にエンジンが立ち上がる
- 打鍵ごとの問い合わせで待たないのは、打鍵を止めないため
- REVERSE は変換キーを食べるかの判定で呼ぶので待たない ([conversion.md](conversion.md)「漢字を含む選択の再変換」)。
  FORGET は [learning.md](learning.md)「学習の削除 (FORGET)」

### 起動の間隔と待ち

- 起動の試行は、TSF 層のインスタンスごとに 10 秒に 1 回まで (どの経路でも共通)。exe が見つからない・起動できなかった
  試行も数える。起動の失敗を繰り返さないため
- 待つ問い合わせでは、パイプを開けないとき次のようにする
  - 最後にエンジンのプロセスを起動できてから 5 秒以内なら、新たには起動せず、その時刻から 5 秒経つまで 100 ms ごとに
    開いてみる。exe が見つからないなどで起動できなかった試行は、待つ対象にしない
  - それ以外なら起動を試み、起動できたら 5 秒まで待つ。10 秒の間隔の内側で起動できないときは、待たずに失敗する
- エンジンは辞書を読み終えてからパイプを作る。起動からパイプができるまで、ファイルがキャッシュに乗った状態の release 版で
  約 2 秒かかり、PC の起動直後はもっと長い。5 秒は PC の起動直後の遅い読み込みでも間に合わせるための値。アクティブ化で
  起動しておくので、実際に 5 秒待つことはまれ

### 起動のしかた

- `CreateProcessW` で、コンソールのウィンドウを出さずに (`CREATE_NO_WINDOW`) 起動する。ハンドルは引き継がない
- 環境変数はアプリのプロセスのものを引き継ぐ。`QUICKLIME_PIPE_NAME` などで隔離したアプリが起動したエンジンは、同じパイプ名・
  隔離したデータで動く
- 作業ディレクトリもアプリのプロセスのものを引き継ぐ。環境変数に相対パスを書くと、エンジンでは起動したアプリの
  作業ディレクトリから解決される

### AppContainer のアプリ

- TSF 層は、自分のプロセスが AppContainer か (`GetTokenInformation(TokenIsAppContainer)`) を最初に要るときに 1 回だけ調べて
  覚える。調べられなかったときは AppContainer ではないものとして扱う
- AppContainer では、パイプを開いてみるだけでエンジンを起動しない。上記「起動する問い合わせ」の起動する問い合わせも
  起動せず、待つ問い合わせも待たずに失敗する
- AppContainer の中から `CreateProcess` したエンジンは同じ AppContainer で動き、辞書や学習・設定のファイルを読み書き
  できないため。ログオン時に explorer の TSF 層がエンジンを起動するので、ふつうはもう動いている。動いていなければ、
  エンジンにつながらないときのフォールバック (ひらがな 1 候補) で動く
- 同じ理由で、単語登録ツール・設定ツールも起動しない (`TextService::LaunchWordRegister`・`LaunchConfigTool` が何もしない。
  設定ツールは [tools.md](tools.md)「起動」)

### exe の探し方

エンジン・設定ツール・単語登録ツールの exe は、DLL のディレクトリを基準に次の順で探し、最初にあったものを使う
(`EngineClient::FindExePath`)。見つからなければ起動しない。

1. DLL と同じディレクトリ (インストール先の 64bit の DLL)
2. 親のディレクトリ (インストール先の `x86\` の 32bit の DLL から、インストール先の直下の 64bit の exe を使う)
3. 開発レイアウトの `engine\target\release\` (DLL が `tsf\build\<構成>\` などにある前提でリポジトリの直下を決める)
4. 開発レイアウトの `engine\target\debug\`

- 開発レイアウトでは release を debug より先に見る。debug だけをビルドし直しても、release に古い exe があればそちらが
  起動する
- 32bit のアプリにも 64bit のエンジンを使う。エンジンを別のプロセスにしているので、エンジンは 64bit だけを作る

## 二重起動の防止

- エンジンは起動の直後、辞書を読む前に、名前付き mutex `Local\quicklime-engine-<パイプ名>` を作る (パイプ名の `\` は
  `_` に置き換える。mutex の名前に `\` は使えないため)。すでにあれば、ほかのエンジンが起動中か動作中なので何もせずに終了する。
  mutex はプロセスの終了まで持ち続ける (ハンドルを閉じず、終了時に OS が解放する)
- 続けて、同じ名前のパイプへ接続できたら、既に動いているとみなして終了する
- mutex が要るのは、辞書の読み込み中はパイプが無く、パイプへの接続だけでは見分けられないため。ログオン時のように多くの
  プロセスから同時に起動されると、接続の確かめだけでは複数のエンジンが辞書を読んでしまう
- 管理者のエンジンが作った mutex は通常権限のプロセスからは開けず、mutex を作れない。そのときは起動を続け、次のパイプへの
  接続の確かめで終了する (パイプのアクセス権で、通常権限からも接続できる)
- mutex はセッションの名前空間 (`Local\`) に作る

## LLM の子プロセス

llama.cpp はエンジンの中ではなく別のプロセス (quicklime-llm) で動かす。

- Vulkan 版は vulkan-1.dll に依存し、GPU のドライバの無い環境では起動できない。llama.cpp の異常終了も含めて、エンジンを
  巻き込まないため
- エンジンの通常のビルドに LLVM・CMake・Vulkan SDK を要らなくするため。llm/ はエンジンとは別の crate で、エンジンの
  crate は llama.cpp に依存しない (計測の example の feature `llm-bench` を除く)
- CPU 版 quicklime-llm.exe と Vulkan 版 quicklime-llm-vulkan.exe は、同じ crate の別の exe (Vulkan 版は feature `vulkan`)

### 起動と終了

- 子プロセスとのやり取りは、エンジンの中の専用のスレッド (LLM ワーカー) 1 本だけが行う
- エンジンの起動時と RELOADCONFIG で設定を渡す。`llm` が 1 なら子プロセスを起動し、0 なら終了する。`llm`・`llm_backend` が
  変わったとき、と、Vulkan 版から CPU 版へ切り替えていたとき (下記) は、今の子プロセスを終了して起動し直す
- 子プロセスを止めるときは、プロセスを終了させる。待っていた依頼は失敗 (RERANKGET は NONE) にする
- 起動の引数は `--model <モデルのパス> --threads <N>`。N は [engine.md](engine.md)「モデルと尤度」の CPU のスレッド数
  (論理コア数を取れなければ 4)
- ウィンドウは出さない (`CREATE_NO_WINDOW`)。標準入出力をパイプにし、標準エラーは捨てる
- 子プロセスはジョブオブジェクト (ハンドルを閉じたら中のプロセスを終了させる設定) に入れる。エンジンが異常終了しても
  子プロセスを残さないため。正常なときは、標準入力が閉じたら子プロセスは自分で終了する
- READY を待つ時間に上限は無い。その間は RERANK に ID 0 を返し、統計の並びで動く。設定の読み直しで子プロセスを起動し直す
  ことになったとき (上記) は待つのをやめる。
  Vulkan 版はシェーダキャッシュの無い初回の読み込みと試しの推論に長くかかるため ([docs/protocol.md](../protocol.md) の
  「エンジン - quicklime-llm 間」)

### exe とモデルの探し方

| ファイル | 探す順 |
|---|---|
| quicklime-llm の exe | エンジンの exe と同じディレクトリ → 開発レイアウトの `llm\target\release\` |
| モデルのディレクトリ | 環境変数 `QUICKLIME_LLM_MODEL_DIR` → エンジンの exe と同じディレクトリの `models\` (あるときだけ) → 開発レイアウトのリポジトリ直下の `models\` |

- 開発レイアウトのリポジトリ直下は、エンジンの exe が `engine\target\{debug,release}\` にある前提で 3 つ上のディレクトリと
  する。インストール先のように 3 つさかのぼれない場所では見ない
- モデルのファイル名は `llm_backend` で決まる ([engine.md](engine.md)「モデルと尤度」)。`QUICKLIME_LLM_MODEL_DIR` は
  ディレクトリがあるかを確かめない (無ければ子プロセスがモデルを読めずに失敗する)
- exe が無い・モデルを読めないときは、起動の失敗として扱う (下記)

### 応答待ちと異常

- 1 要求の応答が 2 秒 (`LLM_TIMEOUT_MS`) を超えたら、その依頼は NONE にする (TSF 層は統計の並びのまま)。子プロセスは
  終了させず、応答を待ち続ける。遅れて届いた応答は捨て、次の依頼から通常に戻る。待っている間に来た依頼は子プロセスへ
  送らずに NONE にする
- 次のときは子プロセスの異常として終了させ、10 秒 (`LLM_RESTART_COOLDOWN_MS`) 後に起動し直す
  - 1 要求の応答が 60 秒 (`LLM_HANG_MS`) を超えた
  - 子プロセスが終了した・送れなかった
  - 応答が不正 (OK・ERR で始まらない、尤度の数が候補の数と合わないなど)
  - CPU 版を起動できなかった (exe が無い、READY でなく ERR などを返した、READY の前に終了した)
- 異常が続けて 3 回 (`MAX_CONSECUTIVE_FAILURES`) になったら、設定を読み直すまで起動しない。依頼が 1 件でも成功すると
  数え直す。設定の読み直しでも 0 に戻し、待ちの 10 秒も捨てる
- ERR の応答 (入力が長すぎるなど) は異常としない。子プロセスは正常で、その依頼だけ NONE にする
  ([engine.md](engine.md)「生成」の、SCORE の ERR の後に生成した候補を入れられたときを除く)
- 応答待ちの上限と異常の判定を分けるのは、Vulkan 版で試しの推論で覆えなかった処理の形の初回が十数秒かかることがあり、
  その途中で終了させるとシェーダキャッシュが残らず、同じ失敗を繰り返すため

### Vulkan 版から CPU 版への切り替え

- `llm_backend vulkan` で Vulkan 版を起動できなかったとき (exe が無い、Vulkan のデバイスが無い、モデルを読めない、READY の
  前に終了したなど) は、待たずに CPU 版と xsmall のモデルで起動し直す。異常の回数には数えない
- quicklime-llm-vulkan.exe は、llama.cpp の Vulkan のデバイスが無ければ ERR で終了する。デバイスが無くても読み込み自体は
  CPU で通ってしまい、CPU 版へ譲れないため
- 切り替えは設定ファイルを書き換えず、設定を読み直すまで続ける。エンジンを起動し直すか設定を読み直すと、Vulkan 版から
  試し直す
- 動いていた Vulkan 版の異常は、ほかの異常と同じく 10 秒後に Vulkan 版で起動し直す

## DLL の登録

`DllRegisterServer` が次を登録し、どれかに失敗したら登録したものを消して失敗を返す。`DllUnregisterServer` は逆に消す。

- COM のクラス: `HKEY_CLASSES_ROOT\CLSID\{QuicklIME の CLSID}`。`InprocServer32` の既定値に DLL 自身のパス、
  `ThreadingModel` は `Apartment`
- 入力プロファイル: 日本語 (0x0411)、説明 `QuicklIME`、アイコンは DLL の最初のアイコン、既定で有効
- TSF のカテゴリ: キーボードの入力方式、表示属性の提供、immersive (UWP) のアプリでの動作、入力インジケーターでの表示

- 64bit と 32bit の DLL は同じ CLSID で、COM のクラスはそれぞれのレジストリの側 (32bit は Wow6432Node) に登録される
- 開発版の DLL とインストール版の DLL も同じ CLSID なので、後から登録した方に切り替わる (両方が同時に有効にはならない)。
  どちらかで登録を消すと、CLSID ごと消える。切り替えの手順は [docs/development.md](../development.md)「開発版 DLL の切替と
  インストール版の更新」
- DLL は読み込まれたらプロセスから外さない (`DllCanUnloadNow` が常に `S_FALSE`)。登録したウィンドウクラスを解除せず
  タイマーも使うので、外した後に届いたメッセージでアプリが落ちうるため

### DLL のファイルの権限

AppContainer で動くアプリが DLL を読み込むには、DLL のファイルに AppContainer 向けの読み取りと実行の ACE が要る。

- 開発版の DLL は、ビルドの後に `icacls <DLL> /grant *S-1-15-2-1:RX *S-1-15-2-2:RX` で、ALL APPLICATION PACKAGES と
  ALL RESTRICTED APPLICATION PACKAGES に読み取りと実行を許す (tsf/CMakeLists.txt の POST_BUILD。32bit 版のビルドも同じ)
- インストール版は、Program Files から同じ 2 つの ACE を継承するので、インストーラでは何もしない。
  `scripts\dev-deploy.ps1 -Dll -Install` のコピーも新しいファイルを作るので、コピー先の継承で付く

## 配布物

### インストール先の構成

インストール先は `%ProgramFiles%\QuicklIME\`。

| 場所 | 中身 |
|---|---|
| 直下 | 64bit の QuicklIME.dll、quicklime-engine.exe・quicklime-config.exe・quicklime-regword.exe・quicklime-llm.exe・quicklime-llm-vulkan.exe、VC++ ランタイム (msvcp140.dll・vcruntime140.dll・vcruntime140_1.dll・vcomp140.dll)、ライセンス文書 (下記「同梱するライセンス文書」) |
| `x86\` | 32bit の QuicklIME.dll |
| `dict\` | Mozc 辞書 (`dictionary00.txt`〜`dictionary09.txt`)・連接行列 (`connection_single_column.txt`)・品詞 ID 表 (`id.def`)・記号辞書 (`symbol.tsv`)・単漢字辞書 (`single_kanji.tsv`)・UT 辞書 (`dictionary_ut.txt`)・後続語の統計 (`next_word.tsv`)・英単語辞書 (`english-words.txt`・`english-names.txt`) |
| `presets\` | AZIK 風のローマ字テーブル (`romaji-azik.tsv`。[input.md](input.md)「ローマ字テーブル」) |
| `models\` | LLM のモデル (`zenz-v3.2-xsmall-Q5_K_M.gguf`・`zenz-v3.2-small-Q5_K_M.gguf`) |

- 利用者のデータ (設定・ユーザ辞書・学習・テーマ) は `%APPDATA%\QuicklIME\` にあり、インストーラは触れない
- `dict\` は、エンジンの辞書ディレクトリの 2 番目の候補 (exe と同じディレクトリの `dict\`) にあたる。辞書ディレクトリに
  置くファイルと、ほかの場所からも探すファイルは [dictionary.md](dictionary.md)「辞書ディレクトリ」

### CRT のリンク

- TSF の DLL は CRT を静的にリンクする (/MT、tsf/CMakeLists.txt)。DLL は全アプリのプロセスに読み込まれ、exe を基準にする
  DLL の探索では、DLL の隣に置いた msvcp140.dll などが見つからないため
- エンジン・設定ツール・単語登録ツールは、インストーラのビルドで `-C target-feature=+crt-static` を付けて CRT を静的に
  リンクする
- quicklime-llm の exe は VC++ ランタイムを動的にリンクする。llama.cpp を静的な CRT でリンクできないため。再頒布できる
  DLL (OpenMP の vcomp140.dll を含む) を exe と同じディレクトリに置く (app-local の配置)

### インストーラ

Inno Setup 6 (installer/installer.iss) で作る。

- 管理者の権限で動き、64bit の Windows (x64 互換) だけに入れる。最低の版は Windows 10 1607 (10.0.14393)。言語は日本語だけ
- スタートメニューに「QuicklIME 設定」(quicklime-config.exe) のショートカットを作る。プログラムグループの選択の画面は出さない
- 64bit と 32bit の DLL は `regserver` で登録する。登録の中身は DLL の `DllRegisterServer` に任せ (上記「DLL の登録」)、
  アンインストールでは Inno Setup が対で `DllUnregisterServer` を呼ぶ
- インストールの前とアンインストールの前に、`taskkill /F` でエンジン・設定ツール・単語登録ツール・quicklime-llm.exe・
  quicklime-llm-vulkan.exe を止める (プロセスが無くても続ける)。更新のときに exe のロックを外すため
- DLL・exe・VC++ ランタイムは、ロードされていれば再起動のときに置き換える (`restartreplace`)。アプリが読み込んでいる DLL は
  止められないので、更新は PC の再起動まで反映されないことがある。初回のインストールは読み込まれていないので再起動は要らず、
  常に再起動を求める設定にはしない
- `dict\`・`presets\`・`models\` とライセンス文書は、再起動での置き換えをしない。エンジンと子プロセスを止めた後は
  ロックされないため
- 動いているアプリを閉じる・開き直す機能 (`CloseApplications`・`RestartApplications`) は使わない
- アンインストールでも `%APPDATA%\QuicklIME\` (設定・ユーザ辞書・学習) は消さない
- インストーラの版 (`MyAppVersion`) は installer.iss に書いた値で、crate の版 (設定ツールの「QuicklIME について」に出す版) とは
  別に持つ

### インストーラのビルド

installer/build.ps1 が、バイナリ・辞書・ライセンスを `installer\staging\` に集めてから ISCC を呼び、
`installer\output\quicklime-<版>-setup.exe` を作る。手順と前提の導入は [docs/development.md](../development.md)
「インストーラのビルド」。

1. 前提を確かめる。次のどれかが無ければ、取得・生成の手段を示して止める: Mozc 辞書のディレクトリと Mozc の LICENSE、
   単漢字辞書、UT 辞書と SudachiDict のライセンス文書 2 つ、後続語の統計と manifest.tsv (ダンプの日付が 1 つに決まること)、
   VS 付属の CMake、quicklime-llm の exe 2 つ、モデル 2 つ、VC++ の再頒布用の DLL、ISCC、cargo-about
2. エンジン・設定ツール・単語登録ツールの release ビルド (CRT を静的にリンク)。ビルドの前に、exe のロックを外すため
   動いているエンジン・設定ツール・単語登録ツールを止める
3. TSF の DLL の 64bit と 32bit の Release ビルド。32bit のビルドツリー (`tsf\build32`) が無ければ構成する
4. staging を作り直し、上記「インストール先の構成」のファイルを集める。ライセンス文書を生成する (下記「第三者の著作権表示」)
5. ISCC で installer.iss をビルドする

- quicklime-llm の exe はビルドしない。scripts/build-llm.ps1 でビルドしたものがあるかだけを確かめる
- VC++ の再頒布用の DLL は、ビルド環境の Visual Studio の `VC\Redist\MSVC\<版>\x64\` のうち、版の最も新しいフォルダから取る
  (`Microsoft.VC*.CRT` と `Microsoft.VC*.OpenMP`)
- 辞書・モデル・ダンプなどの取得物と生成物はリポジトリに入れない (references/・models/ は git 管理外)。取得と生成の手順は
  [docs/development.md](../development.md)「同梱データの取得・生成」

### LLM のモデルの取得

- scripts/fetch-models.ps1 が、Hugging Face のリポジトリ (Miwa-Keita/zenz-v3.2-xsmall-gguf・zenz-v3.2-small-gguf) から、
  リビジョンを固定した `ggml-model-Q5_K_M.gguf` を、リポジトリ直下の `models\` (git 管理外) に取得する。リビジョンは
  LLM の計測 ([experiment-0b-llm-results.md](../archive/experiment-0b-llm-results.md)) で使ったもの
- 取得元のファイル名はどちらも同じなので、保存名 (`zenz-v3.2-xsmall-Q5_K_M.gguf`・`zenz-v3.2-small-Q5_K_M.gguf`) で区別する
- サイズと SHA-256 を確かめる。取得済みで一致するファイルは取り直さない。一致しない取得物は消して止まる。途中で失敗した
  ファイルを正しい名前で残さないよう、一時ファイルに取ってから置き換える

## 第三者の著作権表示

### 同梱するライセンス文書

インストール先の直下に置く。どれもメモ帳で読めるテキスト。

| ファイル | 元 | 中身 |
|---|---|---|
| `LICENSE.txt` | リポジトリの `LICENSE` | QuicklIME 本体 (MIT) |
| `LICENSE-mozc.txt` | `references\mozc\LICENSE` | Mozc の辞書・単漢字辞書・記号辞書 |
| `LICENSE-mozcdic-ut.txt` | build.ps1 が生成 | UT 辞書の元データ 3 つの名前・取得元・ライセンスの説明 ([dictionary.md](dictionary.md)「元データ」)、日本郵便が郵便番号データの著作権を主張しない旨、SudachiDict の `LEGAL` と `LICENSE-2.0.txt` の全文 |
| `LICENSE-wikipedia.txt` | build.ps1 が生成 | 後続語の統計を日本語版ウィキペディアの本文から数えて生成したこと、出典、取得元、ダンプの日付と使ったファイル (manifest.tsv から)、CC BY-SA 4.0 |
| `LICENSE-SCOWL.txt` | `data\LICENSE-SCOWL.txt` | 英単語辞書 ([modeless.md](modeless.md)「英単語辞書」) |
| `LICENSE-llama.cpp.txt` | `llm\licenses\` | llama.cpp (MIT) |
| `LICENSE-zenz.txt` | `llm\licenses\` | zenz のモデル (Apache License 2.0) |
| `LICENSE-MPLUSRounded1c.txt` | `tsf\src\Font\` | 入力モードのアイコンのフォント ([display.md](display.md)「入力モードのアイコン」) |
| `LICENSE-rust-crates.txt` | build.ps1 が cargo-about で生成 | exe に静的にリンクしている Rust の crate (下記) |

- 生成するファイルは UTF-8 (BOM なし)
- Slint の帰属表示は設定ツールの「QuicklIME について」で出す ([tools.md](tools.md)「QuicklIME について」)
- 同梱物とライセンスの一覧は、README.md の「同梱物とライセンス」と設定ツールの「QuicklIME について」に同じ内容で持つ
- TSF の DLL は Windows SDK のほかに C/C++ のライブラリをリンクしていないので、crate の表示の対象にしない (アイコンの
  フォントは `LICENSE-MPLUSRounded1c.txt`)

### Rust の crate の表示

配る exe には依存の crate が静的にリンクされている (設定ツール・単語登録ツールは Slint を含めて数百、quicklime-llm は
llama.cpp を含む)。MIT・BSD・Unicode-3.0・BSL-1.0 などは、バイナリで配るときにも著作権表示と
許諾文の同梱を求めるので、インストーラのビルドのたびに cargo-about で生成する。

- 対象は engine の workspace (quicklime-engine・quicklime-gui。`--workspace`) と llm (quicklime-llm) の 2 つ。Cargo.lock が
  別なので別々に生成し、exe の名前の見出しを付けて 1 つのファイルにつなげる
- 設定は installer/about.toml、テンプレートは installer/about.hbs。target は `x86_64-pc-windows-msvc` だけにし、
  build-dependencies・dev-dependencies は含めない (バイナリに入らない)
- 中身はライセンスの種類ごとに、そのライセンスを使う crate の名前と版の一覧と、許諾文の全文
- 自前の crate (quicklime-engine・quicklime-gui・quicklime-llm) は出さない (`publish = false` と `private.ignore`)。本体は
  `LICENSE.txt` で表示している
- proc-macro とその依存 (syn・quote・serde_derive・i-slint-compiler など) もバイナリには入らないが、一覧に含めたまま出す。
  cargo-about には除く設定が無く、余分な表示が増えるだけで害が無いため (ライセンスは MIT・Unicode-3.0・0BSD だけ)
- 受け入れるライセンスを about.toml に並べる。並びに無いライセンスだけの crate が依存に入ったとき、ライセンスを決められない
  crate があるとき (`--fail`) は生成に失敗し、インストーラのビルドを止める。GPL などが紛れ込んだときに気づくため
  - 複数のライセンスから選べる crate は、並びの先にあるものが選ばれる。並びは MIT を先頭にする
  - Slint の crate (`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`) は royalty-free を選ぶ。
    GPL-3.0 は並びに入れない
  - royalty-free の全文は cargo-about が自動では見つけないので、about.toml の clarify で crate の中の
    `LICENSES/LicenseRef-Slint-Royalty-free-2.0.md` を指定する (cargo-about 0.9.2 の不具合への対処は about.toml の注記)
  - checksum が合わないなどで clarify が使われなくても cargo-about は警告だけで成功するので、build.ps1 が生成した中身に
    royalty-free の全文の見出しがあるかを確かめ、無ければ止める
- cargo-about はビルド環境の前提にする (`cargo install --locked --features cli cargo-about`。0.9 からは `cli` の feature を
  指定しないと exe が入らない)

## 環境変数

| 環境変数 | 読む側 | 働き |
|---|---|---|
| `QUICKLIME_PIPE_NAME` | エンジン・TSF 層・設定ツール・単語登録ツール | パイプの名前 (上記「パイプ名」)。二重起動の防止の mutex の名前にも使う |
| `QUICKLIME_LLM_MODEL_DIR` | エンジン | LLM のモデルのディレクトリ (上記「exe とモデルの探し方」) |

- どちらも動作の検証で、実ユーザのデータや常用のエンジンから隔離するためのもの。辞書・学習・ユーザ辞書・設定などの
  ファイルの環境変数は、それぞれの仕様書 ([dictionary.md](dictionary.md)「環境変数」など)

## 採らなかった案

- エンジンの起動
  - 待ち時間を延ばすだけ: 最初の変換が毎回数秒止まる
  - パイプを先に作り、辞書の読み込み中の要求を待たせる: 打鍵ごとの問い合わせまで止まってしまう。読み込み中の応答を
    別に設けると、プロトコルと TSF 層の両方に手が入る
  - 辞書の読み込みを速くする (バイナリのキャッシュなど): 効果は大きいが大がかりなので、別の課題にした
- パイプのアクセス権
  - AppContainer に開かない (`AC` の ACE を付けず、整合性ラベルを Medium にする): スタートメニューの検索欄などの
    AppContainer のアプリで変換できない
  - AppContainer のクライアントにも全部の要求を許す: 拒否する ADDWORD・RELOADUSER・RELOADCONFIG は TSF 層が使わないので、
    許しても得るものが無い
  - DACL に対話ユーザ (`IU`) や認証済みユーザ (`AU`) を使う: 同じ PC のほかのユーザにも開く
- Rust の crate の表示
  - 生成物をリポジトリに入れる: ビルドにツールが要らないが、依存を更新するたびに作り直す必要があり、古くなりうる
  - HTML で出す: cargo-about の既定はこちらだが、ほかの `LICENSE-*.txt` と形式が揃わない
  - 手で一覧を作る: 数百件あり、依存の更新に追いつけない
  - proc-macro 側の crate を除く: cargo-about を JSON で出し、`cargo tree -e normal,no-proc-macro` で求めた crate に絞って
    テキストを PowerShell で組むことになり、テンプレートが使えず build.ps1 が長くなる
