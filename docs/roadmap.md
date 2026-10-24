# QuicklIME ロードマップ

未着手の項目と、見送り・既知の問題の一覧。実装済みの機能は [設計書の索引](design/README.md)、
全体の構成は [architecture.md](architecture.md) にある。

## 未着手

- 打ち間違えやすいキーの組の学習。補正の候補を選んだ記録から、その人の打ち間違えやすいキーの組を学ぶ
  ([unified-suggest.md](design/unified-suggest.md))
- 補正の候補の LLM での採点 ([unified-suggest.md](design/unified-suggest.md))
- 子音だけ・途中までの省略した打鍵 (`yrsk` → よろしく) の照合 ([unified-suggest.md](design/unified-suggest.md))
- モードレス入力の修正の学習。英字区間の境界を F10 / F6 で直したことを次の判定に使う
  ([modeless-detection.md](design/modeless-detection.md))
- LLM の右文脈の利用。範囲より後ろの文字列を同音異義語の判別に使う。zenz のプロンプトの形式に右の文脈が無く、
  効果を実験で確かめる必要がある ([conversion-range.md](design/conversion-range.md))
- LLM の並べ替えのフィードバック学習。採用・却下を、生の文脈を保存せずに記録して候補コストに反映する
  ([llm-rerank-notes.md](archive/llm-rerank-notes.md))
- 設定ツールと単語登録ツールのハイコントラスト対応。Slint の fluent スタイルにハイコントラストの配色が無い
  ([slint-gui.md](design/slint-gui.md))
- 設定ツールと単語登録ツールの背景の Mica。winit の unstable な API と透明ウィンドウの組み合わせが要り
  壊れやすいため、今は Fluent の単色 ([slint-gui.md](design/slint-gui.md))

## 見送り・既知の問題

- アプリ・システムからの再変換の依頼の入口 (TSF の ITfFnReconversion) は見送り。Word の右クリックの再変換や、
  サクラエディタ・秀丸エディタなど IMM32 アプリの再変換コマンド (CUAS 経由) の依頼を受ける入口だが、
  使う場面が無いため。今の後置再変換は、自分で変換キー・F6〜F10 を押したときだけ働く
  ([selection-reconversion.md](design/selection-reconversion.md))
- 初めて打つ句の予測が、学習した句に押し出されやすい。再生評価 ([eval-sets.md](design/eval-sets.md)) の初出の
  `p` の行で、予測が 9 位以内に入る割合は学習なし 4.0%・学習あり 2.2% (266 行) と下がり、予測の上位 9 件の多くを
  学習した句が占める。全体では学習で予測が上がり (11.6% → 26.6%)、件数も少ないので点数の調整は見送り
  ([learning-score.md](design/learning-score.md) の D)
- 同音異義語の学習の割引の上限の例外 (学習の度合いが最も大きい語が勝つようにする) は見送り。再生評価
  ([eval-sets.md](design/eval-sets.md)) の学習ありのモードで、同音で 1 位を外した行はすべて正解に学習が無く、
  例外で直る「どちらも学習あり」「正解の方が多く使われている」の行が 0 行だったため
  ([learning-score.md](design/learning-score.md))
- 前文脈による変換の使い分け (「服を|着る」「紙を|切る」) は見送り。今は前文脈を予測と後続語の予測にだけ使う。
  語の単位の文脈の割引は評価で変換の 1 位の割合を下げ、再生評価の学習ありのモードで 1 位は 93.6% あり、
  残る外れも分類に薄く散らばっているため ([learning-score.md](design/learning-score.md))
- 前文脈つきの予測 (PREDICT・SUGGEST) への文節の連なりの統計の利用は見送り。今は統計を読みなしの後続語予測
  (NEXTWORD) にだけ使う。再生評価の学習ありのモードの後ろの `q` の行で、正解の表記が統計の表 (文節の表か
  バックオフの表) のどこかに入るのは既出 983 行のうち 111 行、初出 280 行のうち 1 行で、伸ばしたい初出の句には
  順位の付け方によらず効かない。初出の行のうち 110 行は前文脈の自立語がバックオフの表にあっても正解が無く、
  表の大きさより Wikipedia の書き言葉と打つ文の内容の違いが効いている ([next-word-ngram.md](design/next-word-ngram.md))
- 短い文節の表記ゆれ (こと / 事、できます / 出来ます) を、読み全体の学習表記の固定で取り違える。同じ読みで両方の表記を
  使うと、決め手が無い。再生評価の学習ありのモードでは、固定した 322 行のうち固定で当てた行が 13 行、外した行が
  12 行とほぼ同数で、度合いや読みの長さでも分かれない ([eval-sets.md](design/eval-sets.md))
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
- 逆変換で、数字の後の助数詞の複合語の、単位の後ろの部分の読みを誤ることがある (10キロ位 → きろい、
  5パーセント増 → ぱーせんとます)。数字の後の読みの表は語の先頭だけを見る
  ([selection-reconversion.md](design/selection-reconversion.md))
- 後続語の予測の統計に逆変換の読み誤り (何の → なにの、一日 → ひといち) が入る。採用するとその読みで学習される
  ([next-word-ngram.md](design/next-word-ngram.md))
- InputScope の指定が無い英数字の欄 (`inputmode` の無い認証コード欄など) では IME をオフにする必要がある。
  素通しは InputScope で欄の種類が分かるときだけ ([input-scope.md](design/input-scope.md))
- 変換の区切りの誤り。再生評価 ([eval-sets.md](design/eval-sets.md)) の学習が空のモード (8066 行) で、1 位を外した
  行のうち区切りが 209 行、同じ区間 (区切りは同じで、自立語と付属語の別が違う) が 120 行ある
  - 区切りの中心は、境界の位置のずれ (79 行) と、正解では複数の語の部分を Mozc 辞書の長い 1 語にまとめる誤り (69 行)
  - 区切りの 209 行のうち 79 行は、正解が辞書に無いカタカナ語で、正解の表記を辞書のカタカナの断片に分けたために
    区切りに数えたもの。長い 1 語にまとめる誤りの残り 30 行の多くは、長い語の表記ゆれ (手をつける → 手を付ける、
    ちょうど良い → 丁度いい) と固有名詞で、連接や長い語のコストで直す対象は少ない
  - 学習ありでも、語の学習で覚えた表記の部品 (手・つける) が、別の表記の複合語 (手を付ける) に負けることがある
  - 同じ区間の多くは、正解の語が語の学習に数えない語 (読みが 1 文字の語・ひらがなのままの語) か、学習の無い語で、
    学習でも直りにくい
