# ドキュメントファイルの扱い
## 作業中断時のハンドアウト
- `docs/handout.md` は作業を中断するときの申し送り用スクラッチファイル。
  中断時点の状況・未確認のテスト項目・注意点をここに書いておき、次回のセッションはここから読んで再開する
- ユーザが中断を指示したときに使用する
- 作業がひと区切りついて内容を commit したら、`docs/handout.md` の中身は空(テンプレートのみ)に戻す。
  ファイル自体は削除せずに残す
- git 管理下には置かない (コミットしない)。中断時点のみ有効なメモであり、履歴に残す価値はない


## メモ
- `docs/memo.md`はユーザーの開発用のメモ。指示したときのみ参照する。

## 設計書とロードマップ
- 今の仕様は `docs/spec/` の仕様書に書き、コードの変更と同じ commit で書き換える。経緯 (完了日・
  「当初は〜」「〜を廃止した」・設計書の名前による置き換えの注記) は書かない。索引は `docs/spec/README.md`
- 設計書 (`docs/design/`) は実装前の合意の置き場所。「変更対象ファイル」に書き換える仕様書と節を含め、
  実装後に内容を仕様書へ移して `docs/archive/` へ移す (同じ commit でよい)
- 設計書の要らない小さな変更は、該当する仕様書に直接書く。仕様書と設計書の分担と変更の流れは
  `docs/spec/README.md`
- `docs/roadmap.md` には未着手・見送り・既知の問題だけを書く。着手して完了したら項目を消す
  (完了の記録は残さない)

## 画面の部品の呼び名
- 画面の文言・docs・コードコメントで UI の部品を呼ぶときは、`docs/spec/display.md`
  「画面の部品の呼び名」に従う (パネルは「入力パネル」。「候補バー」はパネルの名前ではない)
- ユーザが目にする UI (設定ツール・単語登録ツールの文言、エラーや理由の表示) とユーザ向けの
  ドキュメント (README.md・docs/user/) では、内部の名称 (クラス名・関数名・設計書での呼び分け・
  開発中の略称など) を使わず、ユーザにもわかる表現で書く。設定キーやテーマのキーなど、ユーザが
  ファイルに書く値はそのまま示してよい

## 調査メモと archive
- 他の IME などの調査メモや検討段階の草案は、今後の検討で参照するあいだは `docs/` 直下に置く
  (例: `docs/rakukan-llm-notes.md`)。関係する `docs/roadmap.md` の項目からリンクする
- `docs/archive/` は使い終わってもう参照しないドキュメントの置き場所。検討が終わって使わなくなったら
  `docs/archive/` へ移動する
- 仕様書へ内容を移し終えた設計書 (実装前の合意の記録) も `docs/archive/` へ移す。今の仕様は `docs/spec/` を見る

# 設定ツール・単語登録ツールの UI
- 画面は Windows 11 の設定アプリ (WinUI 3 / Fluent 2) の部品と振る舞いに倣う。今の構成は
  `docs/spec/tools.md`「画面の構成」
- 色・文字サイズ・フォント・角丸は `engine/gui/ui/theme.slint` のトークンを使い、.slint に値を直に書かない。
  足りなければトークンを足す。余白・間隔は WinUI 3 の対応する値に合わせて各部品に直に書いてよい。
  アイコンは Segoe Fluent Icons の字形を `Icons` に足して使う
- `engine/gui/ui/components/` に無い部品は、WinUI 3 の対応するコントロールの既定の見た目
  (寸法・色・状態ごとの変化) と操作を、次のソースで確かめてから作る。記憶で値を決めない。
  対応するものが無い独自の部品は作る前に相談する
  - 色・寸法: microsoft/microsoft-ui-xaml の `controls/dev/CommonStyles/` にある
    `Common_themeresources_any.xaml` と、コントロールごとの `*_themeresources.xaml`
  - 設定アプリのカード: CommunityToolkit/Windows の
    `components/SettingsControls/src/SettingsCard/SettingsCard.xaml`
  - 人が見比べる資料: Windows Design Kit for Figma
    (https://learn.microsoft.com/windows/apps/design/downloads)
- 入力パネル (TSF 層の描画) は対象外。テーマで見た目を変える作りのため

# テストケース
- 機能実装時や変更時などテストが必要なときは、テストケースを例示する

# 開発上の注意点

## 安全なテスト環境
- **自分のマシンで直接デバッグしない**。IME の DLL は全アプリにロードされるため、
  バグるとテキスト入力全体が壊れたり、DLL がロードされたままで更新できなくなったりする
  - テストはメモ帳などの小さなホストアプリで行う
  - Microsoft IME をいつでも切り替えられる状態を維持する
- TSF の公式ドキュメントは薄い。動いている実装 (references/ の CorvusSKK と Mozc の `src/win32/tip`) を読むのが最良の資料

## 古いプロセス・古いバイナリの罠
- 起動しっぱなしの古いエンジンプロセスが同名パイプへの接続を受けてしまい、新しいコードの
  テスト結果が古い挙動になることがある。エンジンのテスト前は `tasklist | grep quicklime` で
  旧プロセスの有無を確認して終了しておく
- **TSF の exe 探索 (FindExePath) は release を debug より優先する**。debug だけを
  ビルドし直しても、`engine/target/release/` に古い exe が残っていればそちらが
  自動起動される (エンジン・単語登録ツール共通)。debug でテストするときは release 側の
  古い exe に注意し、必要なら release も一緒にビルドし直す
- Windows 11 のメモ帳はウィンドウを閉じてもプロセスが残り、新しいウィンドウも同じプロセス
  (= 古い DLL をロードしたまま) で開く。TSF 層のテスト前は `taskkill /IM Notepad.exe /F` で
  プロセスごと終了してから開き直す
- 再ビルド時に DLL や exe がプロセスにロックされて失敗する場合は、ロード中でもリネームは
  可能なので `mv QuicklIME.dll QuicklIME.dll.old` のように退避してからビルドし直す。
  古いバイナリを掴んだプロセスは再起動するまで旧版のまま動く点に注意
  (テストは新しく起動したアプリ・プロセスで行う)

## エンジン検証の隔離
- エンジン単体の動作検証は、環境変数で実ユーザのデータ・常用エンジンから隔離して行う:
  `QUICKLIME_PIPE_NAME` (パイプ名。エンジン・TSF 層・単語登録/設定ツール共通)、`QUICKLIME_DICT_DIR` (辞書)、
  `QUICKLIME_LEARN_FILE` (学習)、`QUICKLIME_USER_DICT_FILE` (ユーザ辞書)、
  `QUICKLIME_CONFIG_FILE` (設定。エンジンと TSF 層の両方が読む)、
  `QUICKLIME_ROMAJI_FILE` (ローマ字テーブル。TSF 層のみ)、
  `QUICKLIME_THEME_DIR` (入力パネルのテーマフォルダ。TSF 層と設定ツール)
- TSF 層の実機確認も隔離できる: 上記の環境変数 (`QUICKLIME_PIPE_NAME` と
  `QUICKLIME_LEARN_FILE` / `QUICKLIME_USER_DICT_FILE` / `QUICKLIME_CONFIG_FILE` など) を設定した
  PowerShell から、`taskkill /IM Notepad.exe /F` で既存のメモ帳を終了してからメモ帳を起動すると、
  そのメモ帳だけが隔離エンジンにつながる
  - TSF 層が自動起動するエンジンや、メモ帳から開いた単語登録・設定ツールも環境変数を引き継ぐので、
    同じパイプ名・隔離データで動く。他のアプリは常用エンジンのまま

## 開発版 DLL とインストール版 DLL の排他切替
- `tsf/build/Debug/QuicklIME.dll` (開発版) と `%ProgramFiles%\QuicklIME\QuicklIME.dll`
  (インストール版) は同じ CLSID で登録されている。DLL 自身のロード元パスを
  `InprocServer32` に書き込む実装 (tsf/src/registry.cpp) のため、後から
  `regsvr32` した方に排他的に切り替わる (両方が同時に有効にはならない)
- 切替は新規に起動したプロセスにしか効かない。ロード済みのプロセスは切替前の DLL を
  使い続ける (Windows 11 のメモ帳は `taskkill /IM Notepad.exe /F` で終了してから開き直す)
- インストーラでの更新は、DLL がロード中だと `restartreplace` により PC 再起動まで
  反映が遅延する。ロード中でもファイルのリネームは可能なので、常用インストール版の
  DLL を直接 `.old-<日時>` へ退避 → 新 DLL を同じパスにコピーすれば、レジストリ再登録も
  再起動も不要で反映できる (InprocServer32 のパス自体は変わらないため)
- `scripts\dev-deploy.ps1` で上記をまとめて行える: `-Dll` は開発版へ regsvr32 切替、
  `-Dll -Install` は常用インストール版を直接更新、`-Restore` はインストール版へ戻す。
  詳細は docs/development.md 「開発版 DLL の切替とインストール版の更新」
