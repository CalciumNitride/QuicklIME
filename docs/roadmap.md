# QuicklIME ロードマップ

未着手の項目と、見送り・既知の問題の一覧。実装済みの機能は [設計書の索引](design/README.md)、
全体の構成は [architecture.md](architecture.md) にある。

## 未着手

- 確定済みの任意の選択範囲の再変換 (TSF の ITfFnReconversion)。今の後置再変換の対象は、かなだけの
  選択範囲・キャレット直前のかな・採用した文字列の記憶 ([adopted-reconversion.md](design/adopted-reconversion.md))
  の範囲だけ
- 打ち間違えやすいキーの組の学習。補正の候補を選んだ記録から、その人の打ち間違えやすいキーの組を学ぶ
  ([unified-suggest.md](design/unified-suggest.md))
- 同音異義語の学習の割引の上限の例外。学習の度合いが最も大きい語が勝つようにするか
  ([learning-score.md](design/learning-score.md))
- 前文脈による変換の使い分け (「服を|着る」「紙を|切る」)。今は前文脈を予測と後続語の予測にだけ使う
  ([learning-score.md](design/learning-score.md))
- LLM による候補の生成 (読みに沿うよう制約したデコード) と、補正の候補の LLM での採点
  ([unified-suggest.md](design/unified-suggest.md))
- 子音だけ・途中までの省略した打鍵 (`yrsk` → よろしく) の照合 ([unified-suggest.md](design/unified-suggest.md))
- 一般の語の連なりの統計による後続語の予測 ([unified-suggest.md](design/unified-suggest.md))
- 候補バーの数字キーでの採用。今の数字キーは選ぶだけで、採用はしない
- LLM の並べ替えの結果で、候補バーの全体変換の位置を結果の上位から埋め直す。今はバーにある全体変換の中で
  入れ替えるだけで、全体変換の 3 枠の外にあった候補は LLM が上位にしても出ない
  ([unified-suggest.md](design/unified-suggest.md) の G)
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
- 設定ツールと単語登録ツールのハイコントラスト対応。Slint の fluent スタイルにハイコントラストの配色が無い
  ([slint-gui.md](design/slint-gui.md))
- 設定ツールと単語登録ツールの背景の Mica。winit の unstable な API と透明ウィンドウの組み合わせが要り
  壊れやすいため、今は Fluent の単色 ([slint-gui.md](design/slint-gui.md))

## 見送り・既知の問題

- 初めて打つ句の予測が、学習した句に押し出されやすい。再生評価 ([eval-sets.md](design/eval-sets.md)) の初出の
  `p` の行で、予測が 9 位以内に入る割合は学習なし 4.0%・学習あり 2.2% (266 行) と下がり、予測の上位 9 件の多くを
  学習した句が占める。全体では学習で予測が上がり (11.6% → 26.6%)、件数も少ないので点数の調整は見送り
  ([learning-score.md](design/learning-score.md) の D)
- ずれのある予測 (候補バーの「補正」の予測) に、UT 辞書の不自然な表記の語 (「おやすまないと → おやスマナイト」)
  が出ることがある。辞書の質の問題 ([unified-suggest.md](design/unified-suggest.md))
- マビノギで Ctrl 併用のキー割当 (Ctrl+Space の変換など) が効かない (対応は見送り)。回避策は Convert キーを使うこと
  - ゲームは OnTestKeyDown を呼び、IME が食べると答えても OnKeyDown を呼ばない (Ctrl なしのキーでは呼ぶ)。
    key-up では OnTestKeyUp だけが呼ばれ、OnKeyUp は来ない
  - composition 中に Ctrl 併用のキーを受けると、ゲームが選択中の候補のまま composition を確定させる
    (OnTestKeyDown と OnTestKeyUp の間に起き、IME からは止められない)
  - 試した対策: OnKeyDown が来ないまま OnTestKeyUp が来たら機能を実行する方式。変換の開始はできても、
    候補選択中の候補送りはできなかった
  - 入力パネル類の位置は [candidate-position-fallback.md](design/candidate-position-fallback.md) で扱う
- InputScope の指定が無い英数字の欄 (`inputmode` の無い認証コード欄など) では IME をオフにする必要がある。
  素通しは InputScope で欄の種類が分かるときだけ ([input-scope.md](design/input-scope.md))
