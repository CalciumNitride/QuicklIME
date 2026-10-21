# 設定ツールと単語登録ツールの Slint 化

設定ツール (quicklime-config.exe) と単語登録ツール (quicklime-regword.exe) の画面を、Win32 の標準
コントロールから Slint (fluent スタイル) に作り直し、Windows 11 の設定アプリに近い見た目と操作にする。

## 背景と目的

両ツールは `windows-sys` から Win32 の標準コントロール (タブ・チェックボックス・コンボ・ListView・EDIT)
を直接作っている。見た目が Windows 11 の Fluent 2 と揃わず、項目が増えるにつれタブとサブダイアログ
(キー編集・キー取り込み・実行中のアプリから選ぶ) に操作が分散している。描画の基盤を Slint に替え、
ページの分け方と操作も設定アプリ風に組み直す。

## 決定事項

### 描画の基盤: Slint

- Slint 1.16 系、fluent スタイル、既定の renderer (FemtoVG)。royalty-free ライセンスで使い、帰属表示は
  設定ツールの「QuicklIME について」ページに `AboutSlint` を置いて満たす。`AboutSlint` の中のロゴは
  幅いっぱいに伸びるので、ロゴの最小の高さ (48px) に合う幅に固定して左寄せにする
- ダークモードは Slint の fluent スタイルが OS の設定に追従するのに任せる
- UI のフォントは `Yu Gothic UI`。ナビのアイコンはシステムの `Segoe Fluent Icons` の字形で描く
- 採らなかった案
  - Direct2D での自前描画: 部品 (コンボのドロップダウン・一覧・フォーカス移動・IME の効く入力欄) と
    UIA provider をすべて自作することになり、工数のほとんどが設定ツールの本質でない部分に取られる
  - WinUI 3: 見た目は最も揃うが、設定ファイルの読み書き・キー割当の検査を C# 等へ移植して二重管理になり、
    Windows App SDK のランタイムを同梱する必要がある
  - 標準コントロールのまま化粧直し: 標準コントロールのダークモード対応が不完全で、Fluent には寄り切らない
- 最初の版で対応しないもの (roadmap に残す)
  - ハイコントラスト: Slint の fluent スタイルにハイコントラストの配色が無い
  - Mica: winit の unstable な API と透明ウィンドウの組み合わせが要り壊れやすい。背景は Fluent の単色
    (ライト・ダークそれぞれの既定の背景色) にする

### Win32 に残す処理

Slint の外で、`windows-sys` で直接行う。Slint のウィンドウの HWND は winit から取る。

- キーの取り込み: Slint のキーイベントでは Convert・NonConvert・`VK_PROCESSKEY` を確実には受けられない
  ため、取り込み中だけ HWND をサブクラス化して `WM_KEYDOWN` / `WM_SYSKEYDOWN` の仮想キーを直接読む。
  修飾キーの判定と割り当てられないキーの理由は今の `on_capture_key` と同じ。取り込み中は
  `ImmAssociateContextEx(hwnd, 0, 0)` で IME を外し、終わったら取り込みを始めた時点の状態に戻す
  (IME が外れていたなら外したまま。winit はフォーカスのある部品に応じて IME を付け外ししており、
  常に有効へ戻すとボタンにフォーカスがあるときにも IME が効いてしまう)
  - 名前の無いキーの表記は `VK_<16進>` のまま。ただし理由の表示では 0xF3・0xF4・0x19 を「半角/全角」と
    出す (`VK_F3` が F3 キーに見えるため。どれも割り当てられないキーなので設定ファイルには書かれない)
- ファイル選択 (`GetOpenFileNameW`): アプリの exe を選ぶ・他 IME の辞書をインポートする
- 実行中のアプリの列挙 (今の `visible_app_windows`)
- 二重起動の防止: 名前付き mutex で判定し、既に開いていればウィンドウタイトルで探して前面に出す
  (winit のウィンドウクラス名は自分で決められないため、今のクラス名での `FindWindowW` は使えない)

### crate の構成

- `engine/` を Cargo workspace にし、新しい crate `engine/gui/` (package `quicklime-gui`) を足す。
  `quicklime-config` と `quicklime-regword` の 2 つの bin をこの crate に移す
- `default-members` に `engine` 本体と `gui` の両方を入れ、`engine/` で `cargo build --release` すると
  両方がビルドされるようにする
  - target が `engine/target/` のままなので、TSF 層の `FindExePath`・`installer/build.ps1`・
    `scripts/dev-deploy.ps1` の exe の場所を変えずに済む
  - エンジン本体の依存に Slint が入らない
- `.slint` は `engine/gui/ui/` に置き、`slint-build` で `build.rs` からコンパイルする。exe のアイコンは
  今と同じく `winresource` で埋め込む
- 設定ファイルの読み書き・キー割当の parse と検査・アプリ設定の merge と split は、今の
  `quicklime-config.rs` から画面の処理を外した Rust のコードとして `gui` crate に移し、既存のテストも移す。
  `import.rs` はエンジン本体と共有のまま `#[path]` で参照する
- `edit_keys.rs` (EDIT に Ctrl+A・Ctrl+Backspace を足す処理) は使う側がなくなるので消す

### 保存: 変えたら即保存

- [保存] [キャンセル] はなくす。値を変えるたびに config.tsv を書き、`RELOADCONFIG` を送る
  - 書き込みは 300 ms まとめてから行う (コンボを続けて切り替えたときに、書き込みと LLM の子プロセスの
    起動・終了を繰り返さないため)。ウィンドウを閉じるときに未書き込みの変更があれば書く
  - 書き込みの内容と規則 (全キーを書く、`debug_log` は値があるときだけ、`panel_input` は「入力パネルだけに
    出す」なら読み込んだ値のまま) は今の `Config::save` と同じ
  - 書き込みに失敗したら、ウィンドウ上部に InfoBar (エラー) を出す。次の変更で書けたら消す
- 診断ログのパスの入力欄は、打鍵ごとではなく Enter かフォーカスが外れたときに確定して保存する
- キー割当の衝突は、追加の時点で弾く (衝突するキーは追加せず、そのカードに理由を出す)。保存の時点で
  まとめて検査する今の方式はなくなる
  - 読み込んだ config.tsv に元から衝突があれば、キー割当のページ上部に警告の InfoBar を出す
    (保存は止めない。TSF 層は今と同じく先に一覧にある機能を優先する)

### 設定ツールの画面

左にナビ、右にページ。各項目は Windows 11 の設定アプリのカード (見出し・説明・右端のコントロール) で並べる。
ウィンドウは大きさを変えられ、最小の大きさを持つ。最後に開いたページは覚えず、起動時は「変換」を出す。

| ページ | 項目 (設定キー) |
|---|---|
| 変換 | 学習する (`learning`)、タイプミス補正 (`typo_correction`)、文節単位で変換する (`segment_ui`)、AI で候補を並べ替える (`llm`。展開すると実行環境 `llm_backend`) |
| 候補バー | 候補バーを表示する (`candidate_bar`)、入力中に予測を表示する (`suggest`)、確定の後に次の語を予測する (`next_word`)、件数 (`max_predictions`)、表示を始める文字数 (`min_suggest_chars`)、表示位置 (`candidate_position`) |
| 入力 | 入力中の読み (`input_buffer`・`panel_input`)、スペース (`space`)、句読点 (`punctuation`)、数字 (`digits`)、モードレス入力 (`modeless`)、数字やメールアドレスの欄では日本語入力を使わない (`input_scope_passthrough`) |
| 見た目 | 入力パネルのテーマ (`panel_theme`。横に [テーマのフォルダを開く])、フォント (`candidate_font`)、サイズ (`candidate_font_size`)、オン/オフの切り替え時に「あ」「A」を表示する (`mode_popup`) |
| キー割当 | 後述 |
| アプリ | 後述 |
| 詳細 | 診断ログ (`debug_log`。オン/オフと出力先のパス) |
| QuicklIME について | 版、`AboutSlint`、同梱ライセンスの一覧 (README の「同梱物とライセンス」と同じ内容) |

- オン/オフの項目は Switch (Slint の ToggleSwitch)、選択肢は ComboBox、数値の選択肢も ComboBox にする
  (値の範囲は今と同じ)
- 展開カード (ExpanderCard) は、見出しの行に Switch を置く設定と値を持つ。「AI で候補を並べ替える」は
  見出しの行の Switch で `llm` を切り替え、展開すると実行環境を出す (Slint の `@children` は 1 か所にしか
  置けないため、見出しの行に任意の部品を差し込む口は作らない)
- 展開カードの見出しは、UIA では Invoke のできるボタンとして出し、読み上げの説明に「展開」「折りたたみ」の
  状態を付ける (Slint 1.16 + AccessKit では `accessible-expandable` を付けても UIA に ExpandCollapse も
  Invoke も出ず、ナレーターが展開状態を読まず操作もできないことを確認した)。見出しの行の Switch は
  別の項目として UIA に出る
- 文言は今の表記を基にし、ページの名前で分かる部分は省く。「サジェスト」は「予測」に揃える
  (状態の表示名の「サジェスト選択中」も「予測選択中」にする。設定キーは変えない)。状態の表示名の
  「run 中」は内部の呼び名で利用者に伝わらないため「入力中」にする (docs/user/keymap.md の表記と揃う)。ウィンドウタイトルは
  「QuicklIME の設定」
- エラーはすべてウィンドウ上部の InfoBar で知らせ、メッセージボックスは使わない
  (テーマのフォルダを作れない・開けないときも含む)
- 旧ツールの Esc で閉じる・Enter で保存・Ctrl+Tab でのタブ移動・アプリの [既定に戻す] は引き継がない
  (即時保存で保存の操作が無くなり、Windows 11 の設定アプリにも無い操作のため)

#### キー割当のページ

- 機能ごとに 1 枚の展開カード。閉じた状態は機能名・働く状態・割り当てたキーのチップ。
  「状態ごとに変える」がオンの機能は、チップの代わりに「状態ごとに設定」と出す
- 展開すると
  - キーのチップ (× で削除) と [+ キーを追加]。押すとカードの中が「キーを押してください」になり、
    取り込んだキーを足す。割り当てられないキー・衝突するキーは足さずに理由を出す。Esc か [キャンセル]
    で取り込みをやめる (取り込み中は全キーを受けるので、Tab で [キャンセル] に移れない。Esc は元から
    割り当てられないキーなので、やめる操作に使っても失うものがない)
  - 「状態ごとに変える」のチェック。オンにすると、その機能が働く状態ごとに同じ形のキーの行を出す
    (今の `overrides`)。オフに戻すと状態ごとの割当を捨てる
  - [既定に戻す]
- ページ上部に [すべて既定に戻す]
- キー編集ダイアログとキー取り込みダイアログはなくす

#### アプリのページ

- アプリごとに 1 枚の展開カード。閉じた状態は exe 名と、既定から変えた設定の要約
- 展開すると 改行キーで Shift+Enter を送る (`newline_apps`)、起動時の IME (自動・オン・オフ。
  `ime_start_apps`)、このアプリでは学習しない (`no_learning_apps`)、[一覧から外す]
- ページ上部に [実行中のアプリから追加] (押すとポップアップで実行中のアプリの一覧を出し、選ぶと足す)
  と [exe を選ぶ] (ファイル選択)
- 実行中のアプリから選ぶダイアログはなくす

### 単語登録ツールの画面

- 小さい固定サイズのウィンドウ。単語・よみ (LineEdit。見出しは欄の上)、品詞 (ComboBox)、下部の左に
  [インポート...]、右に [登録] (accent) [キャンセル]
- 起動引数・初期フォーカス (単語が埋まっていればよみ)・Enter で登録・Esc で閉じる、は今と同じ
- 結果とエラーはメッセージボックスではなくウィンドウ内の InfoBar に出す
  - 登録できたら、成功の InfoBar を出してから閉じる (今のメッセージボックスの代わり。閉じるまでの
    時間は 1.5 秒)。補足 (エンジンに届かずファイルに追記した等) がある場合は閉じずに InfoBar を残す
  - 入力が空・登録の失敗・インポートの結果は InfoBar に出して開いたままにする

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| engine/Cargo.toml | `[workspace]` (members・default-members) を足す |
| engine/gui/Cargo.toml, build.rs | 新しい crate。slint・slint-build・windows-sys・winresource |
| engine/gui/src/bin/quicklime-config.rs | Slint の設定ツール |
| engine/gui/src/bin/quicklime-regword.rs | Slint の単語登録ツール |
| engine/gui/src/ (config の読み書き・キー割当・アプリ設定・Win32 の補助) | 今の quicklime-config.rs から移すロジックとテスト、キー取り込み・ファイル選択・アプリの列挙・二重起動の防止 |
| engine/gui/ui/*.slint | 画面 |
| engine/src/bin/quicklime-config.rs, quicklime-regword.rs, engine/src/edit_keys.rs | 削除 |
| installer/build.ps1, scripts/dev-deploy.ps1 | プロセスの終了・コピーの対象が変わらないかを確かめ、必要なら直す |
| README.md (同梱物とライセンス) | Slint の利用を足す |
| docs/design/basics.md (単語登録ツール・設定ファイルと設定ツール) | ソースのパス、保存のしかた (即時保存)、ページの構成の参照先、診断ログのページ名を新しい画面に合わせる |
| docs/user/settings.md, docs/user/*.md | 設定ツールでの表記・ページ名・操作を新しい画面に合わせる |
| docs/development.md | crate の構成とビルド。workspace では windows-sys の feature がまとまるため、`engine/` での release ビルドは `quicklime-engine.exe` もリンクし直す (常用中はロックされるので退避が要る) |
| docs/design/README.md | この設計書を足し、config-tabs.md・app-settings.md の画面の部分は置き換えと記す |
| docs/roadmap.md | 「設定ツールの見た目の作り直し」を消し、ハイコントラストと Mica を未着手に残す |

## 実装手順

1. 試作 (この段で問題があれば設計に戻す)
   - workspace と `gui` crate を作り、LineEdit・ComboBox・ToggleSwitch を置いただけの Slint のウィンドウを出す
   - 確かめること: QuicklIME と Microsoft IME での LineEdit への日本語入力 (未確定文字列の表示・入力パネルの
     位置・確定)、LineEdit の Ctrl+A・Ctrl+Backspace、日本語の文字の描画品質、HWND のサブクラス化での
     Convert・NonConvert・F キー・Ctrl 併用の取り込み、ダークモードへの追従、release の exe のサイズと起動時間
   - キー割当・アプリのページを、ロジックにつながないダミーデータの `.slint` でモックし、`slint-viewer` で
     画面を確認する。モックは本実装でそのまま使い、確認で変わった点はこの設計書に反映してから手順 2 へ進む
2. ロジックの移動: 設定の読み書き・キー割当・アプリ設定とテストを `gui` crate へ移す (画面なしで `cargo test` が通る)
3. 設定ツールのナビとページの枠、即時保存
4. 変換・候補バー・入力・見た目・詳細・QuicklIME についてのページ
5. キー割当のページ (キーの取り込みを含む)
6. アプリのページ (実行中のアプリの列挙・ファイル選択を含む)
7. 単語登録ツール
8. 旧 bin と `edit_keys.rs` の削除、インストーラ・スクリプト・ドキュメントの更新。試作の bin
   (quicklime-gui-proto) と `proto.slint`・`mock.slint` も消す
9. exe のサイズ: 試作の release で約 13.7 MB (今の設定ツールは 0.46 MB)。strip・lto と、使わない
   software renderer を feature から外すことで縮め、効果を測って採用する。workspace の release profile は
   エンジン本体にも効くので、エンジンの速度・サイズが悪化しないことも確かめる

## 検証方法

`QUICKLIME_CONFIG_FILE`・`QUICKLIME_PIPE_NAME`・`QUICKLIME_THEME_DIR`・`QUICKLIME_USER_DICT_FILE` で隔離して
起動する。ロジックは `cargo test` (移したテストがすべて通ること)。

### テストケース

| # | 操作 | 期待 |
|---|---|---|
| 1 | 設定ツールを起動する | 「変換」ページが出て、config.tsv の値が全ページに読み込まれている |
| 2 | 設定ツールを 2 回起動する | 2 つ目は開かず、1 つ目が前面に出る |
| 3 | 各ページで値を 1 つ変える | 300 ms 程度で config.tsv が書き換わり、隔離エンジンに RELOADCONFIG が届く |
| 4 | 「AI で候補を並べ替える」を素早く数回切り替える | 書き込みが最後の値で 1 回にまとまる |
| 5 | 値を変えてすぐウィンドウを閉じる | 変更が config.tsv に書かれている |
| 6 | config.tsv を読み取り専用にして値を変える | エラーの InfoBar が出る。戻して変えると消える |
| 7 | 入力中の読みを「入力パネルだけに出す」にし、手で `panel_input 1` を書いた config.tsv で別の項目を変える | `panel_input 1` が保たれる |
| 8 | キー割当で Convert・NonConvert・F4・Ctrl+Shift+F7 を追加する | チップが増え、config.tsv に書かれる |
| 9 | キー割当で Alt+A・A・半角/全角を押す | 足されず、今と同じ理由が出る |
| 9a | 取り込み中に Esc を押す | 何も足さずに取り込みをやめる |
| 18a | ナレーターで展開カードの見出しにフォーカスし、ナレーターキー + Enter | 展開状態が読まれ、開閉できる |
| 10 | 別の機能と同じ状態で使っているキーを追加する | 足されず、衝突する機能名が出る |
| 11 | 「状態ごとに変える」をオンにして状態別のキーを変え、オフに戻す | オンの間は状態別に書かれ、オフで状態別の割当が消える |
| 12 | 衝突のある config.tsv で起動する | キー割当のページに警告が出る |
| 13 | アプリのページで実行中のアプリから追加・exe を選んで追加・設定の変更・一覧から外す | `newline_apps`・`ime_start_apps`・`no_learning_apps` が今と同じ形で書かれる |
| 14 | 見た目のページで [テーマのフォルダを開く] | テーマのフォルダが開く (無ければ作る) |
| 15 | OS をダークモードにして両ツールを起動する | ダークの配色で出る |
| 16 | 表示倍率 100%・150% で両ツールを起動する | 項目が切れない |
| 17 | Tab・Shift+Tab・矢印・Space で設定ツールを操作する | マウスなしで全項目を変えられる |
| 18 | ナレーターで設定ツールを読む | 各カードの見出しとコントロールの状態が読まれる |
| 19 | テキストを選択して Ctrl+F7 | 単語登録ツールが単語を埋めた状態で開き、よみにフォーカスがある |
| 20 | 単語登録ツールでよみを QuicklIME で入力して Enter | 未確定文字列と入力パネルが欄の位置に出る。登録の InfoBar が出て閉じ、隔離エンジンに語が入る |
| 21 | 単語登録ツールで単語を空にして [登録] | InfoBar で入力を促し、開いたまま |
| 22 | 隔離エンジンを止めて単語登録する | ユーザ辞書に追記され、補足の InfoBar が残る |
| 23 | 単語登録ツールで MS-IME の辞書をインポートする | 結果の件数が InfoBar に出る |
| 24 | インストーラをビルドしてインストールする | 両ツールが動き、Ctrl+F12・Ctrl+F7 で開く |
