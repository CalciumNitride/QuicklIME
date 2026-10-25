# QuicklIME

Windows 用の日本語 IME です。一般的なローマ字入力をベースにしながら、効率的な変換と候補提示により高速で快適な入力を支援します。

入力中の読みはキャレット（カーソル）付近に表示される候補バーにリアルタイムで反映され、変換結果や予測候補が並びます。候補バーからの選択や変換キーでスムーズに確定できるほか、ミスタイプを自動補正した候補の提示、確定履歴や文脈に応じた学習機能、確定後の再変換やアンドゥ、IME を切り替えずに英単語をそのまま入力できるモードレス入力、AI（かな漢字変換モデル）による文脈に応じた候補の並べ替えなど、多彩な機能を備えています。

## 設計思想

雑に打って、並んだ候補から選ぶIMEです。正確に打たなくても、最後まで打たなくても、文節を直さなくても、書きたい文字列にたどり着けることを目指しています。

- **文書が変わるのは、採用したときだけ**: IME が文書の文字列を勝手に書き換えません。自動の判定 (英字への切り替えなど) も、書き換えずに候補として出します
- **直すより選ぶ**: 入力全体をひとまとまりで変換し、予測を中心に置きます。文節の区切りを直す操作は、既定では使いません
- **雑に打っても届く**: 打ち間違いは打鍵そのものの上で測り、読みが長いほど多くのずれを許します
- **候補は種類ではなく確からしさで並べる**: 全体の変換・予測・補正の候補を、統一された基準で並べます
- **1 つのキーには 1 つの機能**: Space はスペース、Enter はアプリの Enter (改行・送信)、変換キーは変換です
- **使うほど自分に合ってくる**: どの表記をどれだけ使ったかを学習し、変換と予測の両方に反映します
- **入力は外部に送らない**: AI (LLM) はローカルでのみ動かし、入力した文字列を外部に送りません

## 主な機能

- 入力全体の変換: 入力したかな全体をまとめて変換し、文節の区切り方が異なる候補から選択できます ([入力と変換](docs/user/input.md))
- 候補バーと予測: 入力中に変換結果や予測候補を横一列に表示し、選んだ候補だけを文書に入力します ([候補バーと表示](docs/user/candidate-bar.md))
- 打ち間違いの補正: キーの押しすぎ・抜け・隣接キーの誤打・文字の入れ替えなどを自動補正した候補を表示します ([入力と変換](docs/user/input.md))
- 学習: 確定・採用した表記の使用頻度や新しさを学習し、次回以降の変換で優先的に表示します ([学習](docs/user/learning.md))
- 後置再変換・確定アンドゥ: 入力済みの文字列を再変換したり、直前の確定を取り消して入力中の読みに戻したりできます ([入力と変換](docs/user/input.md))
- 次の語の予測: 確定直後に、過去に続けて確定したことのある語と、日本語版ウィキペディアの文章でよく続く語を候補として表示します ([次の語の予測](docs/user/candidate-bar.md#次の語の予測))
- モードレス入力: 英語の入力を自動で判別し、IME をオフに切り替えることなく英字のまま入力できます ([モードレス入力](docs/user/modeless.md))
- AI による並べ替え: 文脈を考慮した AI（LLM）により、最適な候補を上位に並べ替えます。GPU (Vulkan) で動かすときは、AI が生成した候補も加えます ([AI による候補の並べ替え](docs/user/llm.md))
- 辞書インポート: 他の IME のユーザー辞書形式のファイルを取り込めます ([辞書とローマ字](docs/user/dictionary.md))
- テーマ: 入力パネルの配色やレイアウトをカスタマイズできます ([入力パネルのテーマ](docs/user/theme.md))

## 動作環境

Windows 11 で動作を確認しています（64bit アプリおよび 32bit アプリの両方に対応）。インストーラは Windows 10 バージョン 1607 以降で実行できますが、Windows 10 での動作は確認していません。

## インストール

インストーラ (`quicklime-<版>-setup.exe`) を実行し、`Win + Space` で「QuicklIME」を選択して使用します。

- 配置先: `%ProgramFiles%\QuicklIME\` (64bit 用 DLL、エンジン・設定・単語登録ツールの exe、同梱辞書 `dict\` など)。32bit アプリ用の DLL は `x86\` フォルダに配置され、両方が IME として登録されます。
- 初回インストール時は PC の再起動は不要です。更新時は DLL がプロセスに読み込まれているため、再起動を求められることがあります。

インストーラの作成手順については [docs/development.md](docs/development.md) を参照してください。

## アンインストール

アンインストールを行っても、ユーザーデータ (`%APPDATA%\QuicklIME\` 内の設定、ユーザー辞書、学習データ) は削除されず保持されます。

## 基本操作

以下は既定のキー割当による操作です。

| 操作 | キー |
|---|---|
| 読みを入力する | ローマ字で入力 |
| 変換する | 変換キー |
| 候補を選ぶ (変換中) | Space / ↑↓ (Shift+Space で前の候補)。Ctrl+1〜9 でその番号の候補で確定 |
| 確定する | Enter (確定と同時に Enter をアプリへ渡す)・無変換キー (確定のみ) |
| 候補バーから採用する | Tab・↓ で選び、無変換キー・Enter・Space で採用。Ctrl+1〜9 でその番号の候補を採用 |
| 取り消す | Esc (変換中は読みに戻す。入力中は読みを破棄) |
| 確定アンドゥ | Ctrl+Backspace |
| 単語登録 | Ctrl+F7 |
| 設定を開く | Ctrl+F12 |

詳しい操作方法は [入力と変換](docs/user/input.md)、キーのカスタマイズ方法は [キー割当](docs/user/keymap.md) を参照してください。

## 設定のしかた

設定は、設定ツール（`Ctrl + F12`、またはタスクバーの通知領域にある入力モードアイコンを右クリックして「設定...」）から変更できます。
設定項目の一覧は [設定一覧](docs/user/settings.md) を参照してください。

ユーザー設定やデータファイルは `%APPDATA%\QuicklIME\` に保存されます。

| ファイル | 内容 | 説明 |
|---|---|---|
| `config.tsv` | 設定ファイル（設定ツールから保存されます） | [設定一覧](docs/user/settings.md) |
| `romaji.tsv` | ローマ字テーブルの追加・上書き設定 | [辞書とローマ字](docs/user/dictionary.md#ローマ字テーブルのカスタマイズ) |
| `userdict.tsv` | ユーザー辞書（単語登録ツールで登録した単語） | [辞書とローマ字](docs/user/dictionary.md) |
| `imported\` | インポートした辞書ファイル | [辞書とローマ字](docs/user/dictionary.md#辞書インポート) |
| `themes\` | カスタムテーマファイル | [入力パネルのテーマ](docs/user/theme.md) |
| `learning_stats.tsv`・`learning_boundary.tsv`・`learning_word.tsv` | 学習データ | [学習](docs/user/learning.md) |

## ドキュメント

利用者向けドキュメント:

- [入力と変換](docs/user/input.md)
- [学習](docs/user/learning.md)
- [候補バーと表示](docs/user/candidate-bar.md)
- [入力パネルのテーマ](docs/user/theme.md)
- [キー割当](docs/user/keymap.md)
- [アプリごとの設定](docs/user/app-settings.md)
- [モードレス入力](docs/user/modeless.md)
- [AI による候補の並べ替え](docs/user/llm.md)
- [辞書とローマ字](docs/user/dictionary.md)
- [設定一覧](docs/user/settings.md)

開発者向けドキュメント:

- [開発者向けの手順](docs/development.md) (構成・ビルド・インストーラのビルド・開発版 DLL の切替)
- [アーキテクチャ](docs/architecture.md)
- [仕様書の索引](docs/spec/README.md) (機能の領域ごとの今の仕様)
- [設計書の索引](docs/design/README.md)
- [ロードマップ](docs/roadmap.md) (未着手の項目と既知の問題)
- [TSF-エンジン間プロトコル](docs/protocol.md)

## 同梱物とライセンス

QuicklIME 本体のソースコードは MIT License です (`LICENSE`。インストール先には `LICENSE.txt` として同梱しています)。同梱している辞書・モデル・フォントなどは、以下のそれぞれのライセンスに従います。

- 同梱辞書 (`dict\`) および単漢字辞書 (`dict\single_kanji.tsv`) は Mozc (BSD ライセンス) のデータを使用しています。ライセンス文書 `LICENSE-mozc.txt` を同梱しています。
- UT 辞書 (`dict\dictionary_ut.txt`) は、Mozc 向けの追加辞書 mozc-ut (utuhiro78 氏) のうち SudachiDict (一般語。Apache License 2.0、一部 UniDic 由来で BSD ライセンス)、日本郵便の郵便番号データ (地名)、人名データ (Apache License 2.0) から生成したものです。元データの説明および SudachiDict の LEGAL・LICENSE-2.0.txt の全文をまとめた `LICENSE-mozcdic-ut.txt` を同梱しています。
- 次の語の予測の統計 (`dict\next_word.tsv`) は、日本語版ウィキペディアの本文 (CC BY-SA 4.0) から文節の連なりの回数を数えて生成したもので、CC BY-SA 4.0 に従います。出典・取得元・ダンプの日付を記した `LICENSE-wikipedia.txt` を同梱しています。
- モードレス入力用の英単語辞書 (`dict\english-words.txt`) は SCOWL (English Speller Database, https://github.com/en-wl/wordlist ) から生成したものです。著作権表示と許諾文 `LICENSE-SCOWL.txt` を同梱しています。固有名詞・技術用語を収録した `dict\english-names.txt` は独自に作成したものです。
- LLM 推論プロセス (quicklime-llm.exe / quicklime-llm-vulkan.exe) およびモデル (`models\` 配下の zenz-v3.2-xsmall / zenz-v3.2-small) を同梱しています。ライセンス文書として `LICENSE-llama.cpp.txt` (MIT) および `LICENSE-zenz.txt` (Apache License 2.0) を同梱しています。
- タスクバーの入力モードアイコンは、M PLUS Rounded 1c Regular (SIL Open Font License 1.1) をサブセット化して DLL のリソースに埋め込んで描画しています。著作権表示と許諾文 `LICENSE-MPLUSRounded1c.txt` を同梱しています。
- 設定ツール (quicklime-config.exe) と単語登録ツール (quicklime-regword.exe) の画面は、GUI ツールキット Slint (https://slint.dev/) を royalty-free ライセンスで使用しています。帰属表示は設定ツールの「QuicklIME について」ページに表示しています。
- エンジン (quicklime-engine.exe)・設定ツール (quicklime-config.exe)・単語登録ツール (quicklime-regword.exe)・LLM 推論プロセス (quicklime-llm.exe / quicklime-llm-vulkan.exe) は、Rust の crate (MIT・Apache-2.0・BSD・Unicode-3.0 など) を静的リンクしています。それらの著作権表示と許諾文をまとめた `LICENSE-rust-crates.txt` を同梱しています。

同梱データの取得・生成手順については [docs/development.md](docs/development.md) を参照してください。

