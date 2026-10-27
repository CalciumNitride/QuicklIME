# 入力パネルのフォントを GDI の面名から解決する

## 背景と目的

設定ツールのフォントの選択肢は GDI (`EnumFontFamiliesExW`) の面名で、`candidate_font` にもその面名を書く。
入力パネルは DirectWrite のシステムのフォントコレクションの `FindFamilyName` でその名前を探し、見つからなければ
黙って Yu Gothic UI で描く。

GDI は太さや幅の違うフォントを別の面名 (`A-OTF UD Shin Go Pr6N L`、`Source Han Code JP R`、`Noto Sans JP Medium`、
`Yu Gothic UI Semibold`、`Segoe UI Light` など) で列挙するが、DirectWrite ではこれらは 1 つのファミリ
(`A-OTF UD Shin Go Pr6N` など) の中の太さ・幅の違いになる。そのため、こうした面名を選ぶと入力パネルにも
設定ツールの見本にも反映されない。開発機では GDI の面名 380 個のうち 156 個が `FindFamilyName` で見つからなかった。

また、GDI のビットマップフォント・ベクタフォント (`System`、`Terminal`、`MS Sans Serif`、`Modern` など) は
DirectWrite で使えず、選んでも反映されないのに選択肢に出ている。

目的:

- 設定ツールで選べるフォントは、入力パネルと見本にその通り反映されるようにする
- 入力パネルで使えないフォントは、設定ツールの選択肢に出さない

## 決定事項

### 入力パネル (TSF 層): GDI の面名を DirectWrite のフォントに読み替える

`SuggestPanel::SetFont` で、フォントを次の順に解決する。

1. `FindFamilyName` で見つかれば、今と同じくその名前のファミリを標準の太さ・幅・斜体なしで使う
2. 見つからなければ、`EnumFontFamiliesExW` に面名を指定して (charset は `DEFAULT_CHARSET`) その面名の `LOGFONTW` を得る。
   斜体でないものがあればそれを、なければ最初のものを使う。その `LOGFONTW` を
   `IDWriteGdiInterop::CreateFontFromLOGFONT` に渡して `IDWriteFont` を得て、そのフォントのファミリ名
   (`GetFontFamily` → `GetFamilyNames`。en-us があればそれ、なければ先頭) と太さ・幅・斜体を使う
3. どちらでも得られなければ、今と同じく Yu Gothic UI を標準の太さで使う

- 書式 (候補・入力行・番号・ラベル・タグ) はすべて、解決したファミリ名と太さ・幅・斜体で作る
- 選択中の候補の太字の書式は、今と同じく太さを Bold に固定する (元の太さによらない)。幅と斜体は解決したものを使う
- 面名だけを入れた `LOGFONTW` (太さは標準) を `CreateFontFromLOGFONT` に渡す方法は採らない。開発機では
  `Noto Sans JP Thin`・`Cascadia Code ExtraLight`・`Sora Thin` などが「太さ 700 + 太字の合成」という別のフォントに
  解決された。列挙で得た `LOGFONTW` (そのフォント自身の太さ入り) を渡すと、検査した全フォントが正しい太さ・合成なしに
  解決された
- 設定ツールの見本も `SuggestPanel` で描く (tsf/src/preview.cpp) ので、同じ変更で反映される

### 設定ツール: DirectWrite で使えないフォントを選択肢から外す

`win32::font_families` の列挙で、次のどちらかに当てはまる面名だけを選択肢に入れる。

- `font_type` に `TRUETYPE_FONTTYPE` が立っている
- `NEWTEXTMETRICW` の `ntmFlags` に `NTM_PS_OPENTYPE` か `NTM_TT_OPENTYPE` が立っている
  (CFF の OpenType は `TRUETYPE_FONTTYPE` が立たず `DEVICE_FONTTYPE` で列挙されるため。UD 新ゴ・源ノ角ゴシック Code など)

- `EnumFontFamiliesExW` のコールバックの第 2 引数は、ビットマップフォントでも `NEWTEXTMETRICEXW` を先頭に持つ構造体
  (`ENUMTEXTMETRICW` など) を指すので、`ntmFlags` はどの種類でも読める
- 開発機ではこの判定で外れるのはビットマップフォント 7 個 (`System`・`Terminal`・`FixedSys`・`Courier`・`MS Serif`・
  `MS Sans Serif`・`Small Fonts`) とベクタフォント 3 個 (`Modern`・`Roman`・`Script`) だけで、どれも
  `CreateFontFromLOGFONT` で解決できないものだった
- 今の設定値が列挙に無ければ足す今の規則はそのまま (外したフォントを手で書いていても、選び直さずに保存して値が変わらない)

### 採らなかった案

- 設定ツールの選択肢を DirectWrite のファミリ名で作る: 太さ違い (UD 新ゴ L など) を選べなくなり、
  すでに `candidate_font` に保存してある GDI の面名とも合わなくなる
- 選択中の候補の太字を「元の太さから一段太く」する: 今の見た目 (Bold 固定) で十分なため

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| tsf/src/suggest_panel.cpp | フォントの解決 (上記 1〜3) を足し、`SetFont` で解決したファミリ名・太さ・幅・斜体で書式を作る。`CreateFormat` に幅・斜体の引数を足す |
| tsf/src/suggest_panel.h | 必要なら宣言を足す (解決の関数は cpp の無名名前空間でよい) |
| engine/gui/src/win32.rs | `font_families` でビットマップ・ベクタのフォントを外す |
| docs/spec/display.md | 「入力パネル」の節のフォントの行 (「フォントは設定 `candidate_font` を…見つからなければ Yu Gothic UI」) を、上記の解決の順と、面名だけの `LOGFONTW` を使わない理由に書き換える |
| docs/spec/tools.md | 「入力パネルのテーマとフォント」のフォントの選択肢の行に、DirectWrite で使えないフォント (ビットマップ・ベクタ) を外すこととその判定を足す |

docs/user/settings.md の `candidate_font` の説明 (「入力パネルのフォント名」) は変えない。

## 実装手順

1. tsf/src/suggest_panel.cpp にフォントの解決を足し、`SetFont` をそれで書式を作るように変える
2. engine/gui/src/win32.rs の `font_families` に除外の判定を足す
3. 仕様書 (display.md・tools.md) を書き換える
4. TSF 層 (Debug / Release) と設定ツールをビルドする

## 検証方法

隔離した設定 (`QUICKLIME_CONFIG_FILE` など) で、メモ帳と設定ツールで確かめる。

| # | 操作 | 期待 |
|---|---|---|
| 1 | 設定ツールで「フォント」に `A-OTF UD Shin Go Pr6N L` を選ぶ | 見本が UD 新ゴ L で描かれる |
| 2 | 1 のまま保存し、メモ帳で入力する | 入力パネルの候補・入力行・番号が UD 新ゴ L。選択中の候補は太字 |
| 3 | `Source Han Code JP R` / `Source Han Code JP EL` を選ぶ | 見本が源ノ角ゴシック Code の該当する太さ (EL は細い) |
| 4 | `Noto Sans JP Thin` を選ぶ | 細い字で描かれる (太字にならない) |
| 5 | `Yu Gothic UI Semibold`、`Segoe UI Light` を選ぶ | それぞれの太さで描かれる |
| 6 | `Bahnschrift SemiBold Condensed` を選ぶ | 幅の狭い字で描かれる |
| 7 | `Meiryo`、`BIZ UDPGothic` など、今も効くフォントを選ぶ | 今と同じに描かれる |
| 8 | 設定ツールのフォントの選択肢を見る | `System`・`Terminal`・`MS Sans Serif`・`Modern` などが無い。UD 新ゴ・源ノ角ゴシック Code は有る |
| 9 | config.tsv に `candidate_font	System` と手で書いて設定ツールを開く | 選択肢の末尾に `System` が出て選ばれている。入力パネルは Yu Gothic UI |
| 10 | config.tsv に存在しない面名を書く | 入力パネルは Yu Gothic UI |
| 11 | スタートメニューの検索欄 (AppContainer) で、1 のフォントで入力する | UD 新ゴ L で描かれる。描けなくても Yu Gothic UI で表示され、入力は壊れない |
