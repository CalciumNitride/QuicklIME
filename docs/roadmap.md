# QuicklIME ロードマップ

未着手の項目と、見送り・既知の問題の一覧。実装済みの機能は [設計書の索引](design/README.md)、
全体の構成は [architecture.md](architecture.md) にある。

## 未着手

- 確定済みの任意の選択範囲の再変換 (TSF の ITfFnReconversion)。今の後置再変換の対象は、かなだけの
  選択範囲・キャレット直前のかな・採用した文字列の記憶 ([adopted-reconversion.md](design/adopted-reconversion.md))
  の範囲だけ
- 文節 UI (CONVSEG / CONVCTX) へのタイプミス補正。今の変換の補正は入力全体の候補 (CONVNBEST・RERANK)
  だけ ([typo-conversion.md](design/typo-conversion.md))
- 学習の改良。今は読みごとに最後に確定した表記を先頭にする単純な方式 (頻度ベースなど)
- モードレス入力の修正の学習。英字区間の境界を F10 / F6 で直したことを次の判定に使う
  ([modeless-detection.md](design/modeless-detection.md))
- run が無いときのキャレット直前のかなの変換を、候補バーの対象にする
  ([conversion-range.md](design/conversion-range.md))
- LLM の右文脈の利用。範囲より後ろの文字列を同音異義語の判別に使う。zenz のプロンプトの形式に右の文脈が無く、
  効果を実験で確かめる必要がある ([conversion-range.md](design/conversion-range.md))
- LLM の並べ替えのフィードバック学習。採用・却下を、生の文脈を保存せずに記録して候補コストに反映する
  ([llm-rerank-notes.md](archive/llm-rerank-notes.md))
- (保留・後で検討) LLM の並べ替えの段階式の省略。統計変換の 1 位と 2 位のコスト差 (読み 1 字あたり) が
  しきい値を超えるときは RERANK を送らず、LLM の呼び出しと遅延を減らす。しきい値は `nbest_eval --llm` で
  正解率を落とさない値を測って決める。shunti IME (Japanese-IME-for-HarmonyOS-next) は同じ方式で、
  正解率をほぼ保ったまま普段の入力でのモデルの呼び出しを約 4 割に減らしている
- (保留・後で検討) 変換精度の評価セットの拡充。今の `engine/examples/data/llm_cases.tsv` は手作りの
  小さなセットのため、AJIMEE-Bench (CC BY-SA 3.0。ローマ字で打てない字を含む問題は除く) と、
  学習データの確定を今のエンジンで変換し直して確定した表記の順位を採点する再生評価を足す。
  再生評価の入力と結果は実ユーザの文を含むので git に入れない。しきい値などの調整に使ったセットと
  評価だけに使うセットは分ける

## 見送り・既知の問題

- マビノギで Ctrl 併用のキー割当 (Ctrl+Space の変換など) が効かない (対応は見送り)。回避策は Convert キーを使うこと
  - ゲームは OnTestKeyDown を呼び、IME が食べると答えても OnKeyDown を呼ばない (Ctrl なしのキーでは呼ぶ)。
    key-up では OnTestKeyUp だけが呼ばれ、OnKeyUp は来ない
  - composition 中に Ctrl 併用のキーを受けると、ゲームが選択中の候補のまま composition を確定させる
    (OnTestKeyDown と OnTestKeyUp の間に起き、IME からは止められない)
  - 試した対策: OnKeyDown が来ないまま OnTestKeyUp が来たら機能を実行する方式。変換の開始はできても、
    候補選択中の候補送りはできなかった (擬似 Backspace の目印も Ctrl+F24 として届くため、目印も同じく
    key-up で処理する必要があった)
  - 候補ウィンドウ類の位置は [candidate-position-fallback.md](design/candidate-position-fallback.md) で扱う
- InputScope の指定が無い英数字の欄 (`inputmode` の無い認証コード欄など) では IME をオフにする必要がある。
  素通しは InputScope で欄の種類が分かるときだけ ([input-scope.md](design/input-scope.md))
