# 基本機能の仕様

専用の設計書が無い基本機能について、コードから読み取りにくい仕様と、その理由 (WHY) をまとめる。
利用者向けの操作の説明は [docs/user/](../user/) にあり、ここでは重複させない。
各機能の設計書は [設計書の索引](../design/README.md) から引ける。

## 画面の部品の呼び名

[docs/spec/display.md](../spec/display.md)「画面の部品の呼び名」へ移した

## 辞書

[docs/spec/dictionary.md](../spec/dictionary.md)「辞書の種類」「辞書ディレクトリ」「システム辞書」へ移した

## 変換 (Viterbi と文節)

[docs/spec/engine.md](../spec/engine.md)「ラティスと Viterbi」「文節の分け方」「文節の候補 (CONVSEG・CONVCTX)」へ移した

## 数字の書き方の候補

[docs/spec/conversion.md](../spec/conversion.md)「数字の書き方の候補」へ移した

## 学習

[docs/spec/learning.md](../spec/learning.md) へ移した

## 予測

[docs/spec/suggest.md](../spec/suggest.md)「SUGGEST の並び」と「旧エンジンとの互換」へ移した

## タイプミス補正

[docs/spec/engine.md](../spec/engine.md)「タイプミス補正」と [docs/spec/suggest.md](../spec/suggest.md)「ファジィ照合」へ移した

## 記号辞書と記号キー

- 記号辞書は [docs/spec/dictionary.md](../spec/dictionary.md)「記号辞書」へ移した
- 記号キー・Shift+数字キー・テンキーで run に入る文字は [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
- Shift+英字の英字モードは [docs/spec/input.md](../spec/input.md)「印字キー」へ移した
  (モードレス入力での扱いは [モードレス入力](../user/modeless.md) と [docs/spec/modeless.md](../spec/modeless.md)「英字区間」)

## 対記号の両側同期

[docs/spec/conversion.md](../spec/conversion.md)「対記号の両側同期」へ移した

## 生ローマ字の候補

[docs/spec/conversion.md](../spec/conversion.md)「生ローマ字の候補」へ移した

## ファンクションキー変換

[docs/spec/conversion.md](../spec/conversion.md)「ファンクションキー変換 (F4〜F10)」へ移した

## 日付・時刻の候補

[docs/spec/engine.md](../spec/engine.md)「日付・時刻の候補」へ移した

## 確定アンドゥ

[docs/spec/input.md](../spec/input.md)「確定アンドゥ」へ移した

## 読める文書の判定

[docs/spec/input.md](../spec/input.md)「読める文書の判定」へ移した

## ローマ字テーブル

[docs/spec/input.md](../spec/input.md)「ローマ字テーブル」へ移した

## ユーザ辞書と単語登録ツール

[docs/spec/dictionary.md](../spec/dictionary.md)「ユーザ辞書」「インポート辞書」「単語登録ツール」へ移した

## 設定ファイルと設定ツール

[docs/spec/tools.md](../spec/tools.md)「設定ファイル」と「設定ツール」へ移した

## 言語バー

[docs/spec/display.md](../spec/display.md)「入力モードのアイコン」と「言語バー」へ移した

## エンジンの自動起動

[docs/spec/runtime.md](../spec/runtime.md)「エンジンの自動起動」「二重起動の防止」へ移した

## インストーラ

[docs/spec/runtime.md](../spec/runtime.md)「インストーラ」「CRT のリンク」へ移した

## パスワード欄などキー入力が無効な文書

[docs/spec/input.md](../spec/input.md)「キー入力が無効な文書」へ移した

## 横並びのテーマでのサジェスト選択の ←→

[docs/spec/suggest.md](../spec/suggest.md)「選択」へ移した
