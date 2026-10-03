# QuicklIME

Windows 用の自作日本語IME。通常のローマ字入力をベースに、変換の工夫による高速入力を目指す。

## 構成

| ディレクトリ | 内容 |
|---|---|
| `tsf/` | TSF テキストサービス (C++ / in-proc COM DLL)。フェーズ1で作成 |
| `engine/` | 変換エンジン (Rust / 常駐別プロセス) |
| `data/` | 設定ファイルのプリセット (AZIK 風ローマ字テーブルなど) |
| `docs/` | ドキュメント。開発計画は [docs/roadmap.md](docs/roadmap.md)、TSF-エンジン間プロトコルは [docs/protocol.md](docs/protocol.md) |
| `references/` | 参考用の外部リポジトリ (git管理外)。CorvusSKK、SampleIME |
| `scripts/` | 開発用スクリプト (`dev-deploy.ps1`: デバッグ反映、`cleanup-old-binaries.ps1`: 旧DLL/exeの掃除) |

## 開発環境

- Visual Studio 2022 (C++ によるデスクトップ開発ワークロード、Windows SDK 含む)
- Rust (stable-x86_64-pc-windows-msvc)
- Windows 11

## ビルド

- エンジン: `cd engine && cargo build`
  (quicklime-engine.exe 本体に加え、`src/bin/` の quicklime-config.exe /
  quicklime-regword.exe も一緒にビルドされる)
- エンジンのテスト: `cd engine && cargo test`
- エンジン単体の動作確認 (起動中のエンジンに接続して CONVERT/CONVSEG を試す):
  `cargo run --example query -- <読み> [<読み>...]`
- TSF層 (VS付属のCMakeを使用、64bit):
  ```
  cmake -S tsf -B tsf/build -G "Visual Studio 17 2022" -A x64
  cmake --build tsf/build --config Debug
  ```
  成果物: `tsf/build/Debug/QuicklIME.dll`

## インストーラでの導入

`installer\build.ps1` を実行すると `installer\output\quicklime-<版>-setup.exe` が生成される
(前提: Inno Setup 6 = `winget install -e --id JRSoftware.InnoSetup`、references/mozc の辞書)。

- 配置先は `%ProgramFiles%\QuicklIME\` (64bit DLL + エンジン・設定・単語登録の exe +
  辞書 `dict\`)。32bit アプリ用の DLL は `x86\` に入り、両方が IME として登録される
- 初回インストールは再起動不要。更新時は DLL がロード中のため再起動を求められることがある
- アンインストールしてもユーザデータ (`%APPDATA%\QuicklIME\` の設定・ユーザ辞書・学習) は残る
- 同梱辞書は Mozc (BSD ライセンス) のもの。ライセンス文書 LICENSE-mozc.txt を同梱している

登録後、Win+Space で「QuicklIME」を選択して使用する。

## 開発版 DLL の切替とインストール版の更新

`scripts\dev-deploy.ps1` でビルドから反映までをまとめて実行できる。

| コマンド | 動作 |
|---|---|
| `scripts\dev-deploy.ps1` | エンジンのみ反映 (release ビルド → 常用の Program Files へコピー) |
| `scripts\dev-deploy.ps1 -Dll` | エンジン反映に加え、TSF DLL を Debug ビルドして開発版へ切替 (要管理者権限) |
| `scripts\dev-deploy.ps1 -Dll -Install` | エンジン反映に加え、TSF DLL を Release ビルドして常用インストール版へ直接反映 (管理者権限・再起動とも不要) |
| `scripts\dev-deploy.ps1 -Restore` | DLL を開発版からインストール版へ戻す (要管理者権限) |

DLL の反映方法は用途で使い分ける。

- **`-Dll`** (開発版切替): `tsf\build\Debug\QuicklIME.dll` に regsvr32 で排他的に切り替える。
  常用インストール版には触れない。確認が終わったら `-Restore` で必ず戻すこと
- **`-Dll -Install`** (インストール版更新): 常用インストール版の DLL 自体を新しいビルドに
  差し替える。ロード中の DLL を `.old-<日時>` にリネーム退避してから同じパスに新 DLL を
  コピーするので、レジストリ再登録も PC 再起動も不要 (インストーラでの更新は
  ロード中だと `restartreplace` により再起動待ちになる)。32bit 版 (`x86\QuicklIME.dll`) も
  同じ手順で更新する (ビルドツリーは `tsf\build32`。未構成なら自動で生成する)

手動で行う場合 (開発版切替):

```
regsvr32 tsf\build\Debug\QuicklIME.dll      # 登録
regsvr32 /u tsf\build\Debug\QuicklIME.dll   # 解除
```

インストール版と開発版は同じ CLSID を共有し、後から regsvr32 (または再インストール) した
方に登録が切り替わる。開発を終えて常用へ戻すときは、インストーラを再実行するか
`regsvr32 "%ProgramFiles%\QuicklIME\QuicklIME.dll"` で戻す。

## ローマ字テーブルのカスタマイズ

`%APPDATA%\QuicklIME\romaji.tsv` (UTF-8) を置くと、既定のローマ字テーブルへ
追加・上書きされる。書式は 1行1エントリ「ローマ字<TAB>かな」。`#` 始まりの行は
コメント、かな欄が空の行は既定エントリの削除。反映は各アプリの再起動後。

AZIK 風拡張 (撥音拡張「かん」=kz、二重母音拡張「こう」=kp など) のプリセットを
[data/romaji-azik.tsv](data/romaji-azik.tsv) に用意している (インストール版では
`%ProgramFiles%\QuicklIME\presets\` にも同梱)。使う場合はこれを
`%APPDATA%\QuicklIME\romaji.tsv` へコピーする。

```powershell
# PowerShell の場合 (%APPDATA% は展開されないので $env:APPDATA を使う)
copy data\romaji-azik.tsv $env:APPDATA\QuicklIME\romaji.tsv
```

## 入力方式 (input_style)

設定 `input_style` (設定ツールの「入力方式」、`%APPDATA%\QuicklIME\config.tsv`) で
打鍵した文字の入れ方を選べる。既定は `composition`。

| 値 | 挙動 |
|---|---|
| `composition` | 打鍵した文字を下線付きの未確定文字列として保持し、Space (または変換キー) で変換、Enter で確定する従来の方式 |
| `direct` | 打鍵した文字を未確定文字列を使わずに文書へ直接入れる方式。IME は自分が入れた文字列とその読み (run) を覚えておき、打鍵ごとにキャレット直前の文字列を置き換える。変換は変換キーで後から行う (後置変換)。未確定文字列の挙動が不安定な Web フォーム (文字数固定・自動で次欄へ移動する欄など) 向け |

変換キーは設定 `key.convert` (設定ツールの「変換キー」) で `Convert` (JIS 配列の変換キー、既定)
または `Ctrl+Space` (US 配列向け) を選べる。`composition` 方式では Space による変換も
そのまま使える。

`direct` のキー操作 (IME オン時):

- 英字・記号・数字: run を始める / run の読みに追加して文書の文字列を置き換える
  (下線は付かない)。Backspace は読みの末尾を1かな削る (空になれば run 終了)
- 変換キー: run 中は変換開始。候補選択中は次候補 (Shift+変換キー で前候補)。
  変換中 (候補選択中) だけは run を未確定文字列 (composition) に切り替え、下線と
  現在文節の強調を表示する。キー操作は `composition` と同じ (Space も次候補、
  ↑↓・数字 1〜9・PgUp/PgDn・←→・Shift+←→・F4〜F10 も同じ)。run 中の F4〜F10 も
  同様に未確定文字列に切り替えて働く。未確定文字列にできない文書では、現在の文節を
  選択状態にして強調する
- 候補選択中の Enter: 確定して run を終える (改行は入らない)。
  Esc・Backspace: 変換前の表示 (かな、ライブ変換 ON ならライブ表示) に戻し、下線の無い
  run に戻る。英字などの印字キー: 確定して新しい run を始める
- 後置再変換: run が無いときに、ひらがな・カタカナ・ー だけの文字列を選択して変換キーを
  押すと、それを読みとして候補選択に入る (漢字を含む選択では何も起きない)
- サジェスト: 2文字以上のかなで候補ウィンドウに出る。Tab / ↑↓ で選ぶと文書の文字列が
  候補に置き換わり、Enter で確定、Esc で選択解除。未選択のまま Enter を押すとアプリで改行
- ライブ変換 ON: 打鍵ごとに文書の文字列が変換結果に置き換わる。変換キーで候補選択に入れる
- Space: 常にスペース (設定 `space` に従う)。run 中は run を終えてからスペースを入れる
- Enter・Esc・矢印・Tab・Home/End・PgUp/PgDn (候補選択・サジェスト選択以外): run を終える
  (Esc 以外はアプリに渡す。Esc は文字を消さず run を忘れるだけ)
- Ctrl+Backspace (確定アンドゥ): run 終了後に、直前の run を読みのかな表示に戻して再開する
  (変換結果・ライブ変換結果もかなに戻る)
- run の終了時は `composition` の確定と同じく学習 (候補選択・ライブ変換の文節ごと) と
  文脈の更新を行う
- フォーカス移動・IME オフ・キャレット移動 (置換時の不一致) でも run は終わる

文書の読み取り・置換ができないアプリ (CUAS 経由の古いアプリ、ターミナルなど) では、
その文書に限って自動的に `composition` 方式で動く (最初の1文字が文書に残ることがある)。

## モードレス入力 (modeless)

設定 `modeless` (設定ツールの「モードレス入力」、既定 OFF) を ON にすると、英語の打鍵を
IME が自動で見分けてアルファベットのまま入れる。半角/全角キーで IME をオフにせずに
日本語と英語を混ぜて打てる (IME のオン/オフ自体はそのまま残る)。`composition` /
`direct` のどちらの入力方式でも働く。

判定は打鍵ごとに行い、次のいずれかが成立した時点でその composition / run を英字モードへ
移す。移るときはそれまでの打鍵列をそのまま文字として並べ直す (`あっp` → `appl`)。

- ローマ字として成立しない英小文字が現れた (英語特有の子音連続。`apple` の `pl`、`str`、`th`)
- 促音の直後に小書き母音が来た (`hello` の `へっ` + `ぉ`)
- 無変換のまま Space / Enter で終えるとき、未変換ローマ字として `n` 以外の英小文字が
  1文字残っている (`わんt` を Space で終えると `want`)。この判定は表示も英字へ直せる
  場面だけで働き、フォーカス移動・IME オフでの終了では `わんt` のまま確定される

英字モードに入った composition / run は日本語には戻らない。Backspace で全部消せば解除され、
次の入力は日本語から始まる。

`modeless` が ON のときの英字モード中 (自動判定・Shift+英字のどちらで入った場合も) のキー:

- Space: 設定 `space` が全角でも半角スペースにし、`composition` では変換せずに
  半角スペースを付けて確定、`direct` では半角スペースを入れて run を終える
- 変換キー: 候補選択に入る (生ローマ字候補で `apple` / `Apple` / `APPLE` を選べる)。
  英字モード中の変換は Space ではなく変換キーで行う
- 記号は半角、Enter は `composition` なら確定・`direct` ならアプリへ渡して run 終了

`modeless` が OFF のときは、Shift+英字で入った英字モードも従来どおり (Space は
`composition` なら変換、`direct` なら設定 `space` の幅のスペース)。

`modeless` が ON のとき、composition / run が無いときの Space は、直前に確定した
文字列が ASCII 英数字だけなら半角、それ以外は設定 `space` に従う。

自動では判定しないもの (日本語を優先する):

- ローマ字として成立する英単語 (`sake`、`name`、`date`、`pen` など)。
  変換キーの生ローマ字候補か、Shift+頭文字での英字モードで入れる
- 大文字始まり (Shift+英字で明示的に英字モードに入るため判定は不要)

打ち途中の日本語を確定したときは誤判定しうる (`かk` を Space で終えると `kak`)。

## 注意

IME の DLL は全アプリケーションのプロセスにロードされる。開発版の動作確認は
テスト用アプリで行い、Microsoft IME へいつでも切り替えられる状態を維持すること。
