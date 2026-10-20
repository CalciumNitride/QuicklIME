# 入力モードのアイコン

## 背景と目的

タスクバーの入力インジケータ (`GUID_LBI_INPUTMODE`) に出すアイコンの見た目を整え、入力欄でないときの
状態を出せるようにする。

これまでのアイコンは、黒で塗りつぶした正方形に白の Yu Gothic UI Bold で「あ」「A」を `DrawTextW` で
描いていた。そのため次の問題があった:

- 背景が不透明な黒い四角で、ライトテーマのタスクバーでは黒い箱になる
- 32bpp の alpha を使っておらず、ClearType で描いているので、文字の縁に色のにじみが出る
- `DT_VCENTER` は行の高さで中央に置くので、字形が上下にずれる
- 入力欄でない所 (デスクトップなど) にフォーカスがあっても「あ」「A」のままで、日本語を打てるかどうかが
  見た目から分からない

## 決定事項

### 見た目

- 状態は 3 つ: IME オンで「あ」、オフで「A」、入力欄でないときは「×」(U+00D7)。入力欄でないときは、
  オン/オフより × を優先する
- フォントは M PLUS Rounded 1c Regular を DLL に同梱して使う
  - 候補は同梱できるフォント (OFL など) から選んだ。Adobe Fonts のフォントは、アプリへの埋め込みが
    ライセンスで認められていないので使わない
  - 太さは Regular。タスクバーの時計 (Segoe UI Regular) と線の太さが揃い、なじむため。Medium は
    16px で少しはっきりするが、周りから浮く
- 背景は透明にし、文字だけを描く。文字の色はタスクバーのテーマに合わせ、ライトなら黒、ダークなら白にする
- アンチエイリアスはグレースケール (ClearType は使わない。アイコンは描いた後に別の背景へ合成されるため)
- 大きさ: アイコンの一辺は従来どおり `MulDiv(16, LOGPIXELSY, 96)` px。「あ」の字形の外接矩形
  (ink bounds) の長辺がアイコンの一辺 − 2px になる文字サイズを求め、「A」「×」も同じ文字サイズで描く。
  「×」は「あ」より小さく見えるが、状態の違いが分かれば十分なので拡大しない
- 位置: 字ごとに字形の外接矩形を、アイコンの上下左右の中央に置く。描く原点は整数 px に丸める
  (線をぼかさないため)

### フォントの同梱

- M PLUS Rounded 1c Regular (Google Fonts の配布物。SIL Open Font License 1.1、著作権表示は
  `Copyright 2016 The Rounded M+ Project Authors.`) を、「あ」「A」「×」の 3 字だけにサブセット化する。
  元のフォントは約 3.4MB あり、DLL は全プロセスにロードされるため、使う字以外は持たない
  - Reserved Font Name の指定は無いので、サブセット化してもフォント名は変えなくてよい
- サブセット化したフォントは、生成物としてリポジトリにコミットする。ビルドに Python や元のフォントを
  要らないようにするため。再生成用のスクリプトもリポジトリに置く
- サブセットのフォントは DLL のリソース (`RCDATA`) に入れる。別ファイルにすると、DLL からの探索と
  インストーラの管理が増えるため
- OFL の本文と著作権表示を `LICENSE-MPLUSRounded1c.txt` として、ほかの LICENSE と同じくインストール先に
  置き、README の同梱物の説明に足す

### 描画

- DirectWrite でリソースのフォントを読み込む。対応 OS の下限が Windows 10 1607 (10.0.14393) で、
  `IDWriteFactory5::CreateInMemoryFontFileLoader` (1703 以降) は使えないため、リソースのメモリを返す
  `IDWriteFontFileLoader` / `IDWriteFontFileStream` を自前で実装する。
  `CreateCustomFontFileReference` → `CreateFontFace` で `IDWriteFontFace` を作り、フォントコレクションは
  作らない (3 字を glyph run で描くだけなので要らない)
- 描画は D2D の `ID2D1DCRenderTarget` (premultiplied BGRA) を 32bpp の DIB section に bind し、
  透明でクリアしてから `DrawGlyphRun` で描く。入力パネル・キャレットの印と同じ方式
- 字形の外接矩形は `IDWriteFontFace::GetDesignGlyphMetrics` (横書きの advance・side bearing と、
  縦の top/bottom side bearing・verticalOriginY) から求める
- HICON は `CreateIconIndirect` で作る。color は 32bpp DIB で、文字色は単色なので、premultiplied の
  画素を straight alpha (RGB = 文字色、A = 被覆率) に直して渡す。mask は 0 で初期化した
  1bpp のビットマップ (32bpp の alpha があれば mask は使われないが、未初期化の内容は渡さない)
- DirectWrite・D2D の factory、自前の loader、font face、render target は `LangBarButton` が持ち、
  最初の `GetIcon` で作って使い回す。loader の登録解除と解放は `LangBarButton` のデストラクタで行う。
  DWrite の factory は `DWRITE_FACTORY_TYPE_ISOLATED` にする (自前の loader を登録するため、
  プロセス共有の factory を汚さない)
- テーマは `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize` の
  `SystemUsesLightTheme` (DWORD) を `GetIcon` のたびに読む。1 ならライト (黒の文字)、それ以外・値が
  無いときはダーク (白の文字)。Windows のテーマを切り替えても、次にアイコンを作り直すまで
  (モードの切替・フォーカスの移動) は古い色のまま。即時に追従させるには設定変更の通知を受ける隠しウィンドウが
  要り、それに見合わないため
- 失敗時: リソースのフォントを読めないときは、システムの Yu Gothic UI (Regular) で同じ描き方をする。
  D2D の初期化・描画に失敗したときは `GetIcon` が `E_FAIL` を返す (従来の GDI の描画は残さない)

### 入力欄でないときの「×」

- 判定は既存の `TextService::IsKeyboardDisabled()` (フォーカスに document manager・context が無い、
  または context の `GUID_COMPARTMENT_KEYBOARD_DISABLED` / `GUID_COMPARTMENT_EMPTYCONTEXT` が立っている)。
  CorvusSKK と同じ判定
- `ITfThreadMgrEventSink::OnSetFocus` の最後で `LangBarButton::NotifyUpdate()` を呼び、フォーカスが移るたびに
  アイコンを作り直させる。IME オン/オフの切替で作り直すのは従来どおり
- × の間は、左クリック (オン/オフの切替) では何もしない。切り替えてもアイコンが × のままで、
  状態が見えないまま変わるため。右クリックのメニュー (「設定...」「単語登録...」) は使えるままにする。
  デスクトップなどからも設定を開けるようにするため
- `GetStatus` は従来どおり 0 を返す (`TF_LBI_STATUS_DISABLED` にすると右クリックのメニューも使えなくなる)

採らなかった案:

- フォントを画像 (.ico) に描き出して同梱する案: DPI ごとの大きさを揃える必要があり、テーマの色も
  画像ごとに要る。実行時に描けば、どの DPI でも鮮明で色も選べる
- 「×」を線で描く・拡大する案: 違いが分かれば十分なので、フォントの字をそのまま使う
- × のときに `TF_LBI_STATUS_DISABLED` を返す案 (CorvusSKK の方式): 右クリックのメニューも使えなくなる

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/Font/MPLUSRounded1c-Regular-icon.ttf (新規) | サブセット化したフォント (あ・A・× の 3 字) |
| tsf/src/Font/LICENSE-MPLUSRounded1c.txt (新規) | OFL 1.1 の本文と著作権表示 |
| scripts/subset-icon-font.py (新規) | 元のフォントのパスを引数に取り、fontTools でサブセットを作る |
| tsf/src/resource.h, tsf/src/QuicklIME.rc | サブセットのフォントを `RCDATA` として足す |
| tsf/src/lang_bar.h, tsf/src/lang_bar.cpp | アイコンの描画を DirectWrite / D2D に置き換え、× の状態とテーマの色、× の間の左クリックの無視 |
| tsf/src/text_service.cpp | `OnSetFocus` (ThreadMgr) で `NotifyUpdate()` を呼ぶ |
| installer/build.ps1, installer/installer.iss | `LICENSE-MPLUSRounded1c.txt` をステージングしてインストール先に置く |
| README.md | 同梱物の説明に M PLUS Rounded 1c と許諾文を足す |
| docs/design/basics.md | 「言語バー」の節を、3 状態のアイコンと × の間のクリックの扱いに直し、この設計書へリンクする |
| docs/design/README.md | この設計書を 1 行足す |

## 実装手順

1. scripts/subset-icon-font.py を書き、Google Fonts の `ofl/mplusrounded1c/MPLUSRounded1c-Regular.ttf` から
   「あ」「A」「×」だけのサブセットを作って tsf/src/Font/ に置く。OFL の本文と著作権表示を
   LICENSE-MPLUSRounded1c.txt に書く
2. リソースにフォントを足す
3. lang_bar.cpp にリソースのメモリ上の font file loader / stream を実装し、`LangBarButton` に
   factory・font face・render target を持たせる
4. `CreateModeIcon` を、3 状態 (あ・A・×)、テーマの色、字形の外接矩形での大きさと中央寄せで描くものに
   置き換える。リソースのフォントを読めないときの Yu Gothic UI への切替も入れる
5. `OnClick` で、`IsKeyboardDisabled()` のときは左クリックを無視する
6. `TextService::OnSetFocus` (ThreadMgr) の最後で `NotifyUpdate()` を呼ぶ
7. インストーラ・README・docs を更新する

## 検証方法

x64 (tsf/build) と x86 (tsf/build32) の両方でビルドが通ること。実機は `scripts\dev-deploy.ps1 -Dll` で
開発版に切り替え、`taskkill /IM Notepad.exe /F` してからメモ帳を開いて確かめる。

| # | 操作 | 期待 |
|---|---|---|
| 1 | メモ帳の本文で IME をオン/オフ | 「あ」「A」が切り替わる。背景は透明で、字がアイコンの中央にある |
| 2 | タスクバーのテーマをダーク/ライトに切り替え、メモ帳でモードを切り替える | ダークで白、ライトで黒の文字になる。ライトで黒い四角が出ない |
| 3 | デスクトップをクリックする | 「×」になる |
| 4 | メモ帳に戻る | 直前のオン/オフの状態 (「あ」か「A」) に戻る |
| 5 | × の状態でアイコンを左クリック | 何も起きない。メモ帳に戻ったときのオン/オフも変わっていない |
| 6 | × の状態でアイコンを右クリック | 「設定...」「単語登録...」のメニューが出て、それぞれ起動できる |
| 7 | 表示スケールを 100% / 125% / 150% にして 1 を見る | どの倍率でも字がにじまず、アイコンからはみ出さない |
| 8 | エクスプローラ・Chrome・Edge・VS Code などで、入力欄とそれ以外をクリックする | 入力欄で「あ」「A」、入力欄以外で × になるか確かめ、結果を記録する (× にならないアプリがあっても不具合とはしない。下の未決事項) |
| 9 | インストーラをビルドしてインストールする | インストール先に LICENSE-MPLUSRounded1c.txt がある |

## 未決事項

- アプリによっては、入力欄でない所でも `EMPTYCONTEXT` を立てず、× にならないことがある。検証 8 の結果を
  見て、拾えないアプリへの対応 (しないことも含む) を決める
- フォーカスを移さずに context の `KEYBOARD_DISABLED` だけが変わった場合は、アイコンを作り直さない。
  実害が見つかったら compartment の変更通知を受けることを検討する
