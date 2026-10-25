# Rust の crate のライセンス表示

## 背景と目的

- インストーラで配る Rust の exe には、依存する crate が静的リンクされている
  - quicklime-engine.exe: 約 10 個 (MIT/Apache-2.0、0BSD、Unlicense)
  - quicklime-config.exe・quicklime-regword.exe: Slint を含めて約 250 個。大半は MIT/Apache-2.0 で、ほかに
    Unicode-3.0 (ICU 系)、BSD-2-Clause、BSD-3-Clause、Zlib、BSL-1.0 (clipboard-win・error-code)、ISC、
    CC0-1.0、Slint のライセンス
  - quicklime-llm.exe・quicklime-llm-vulkan.exe: 約 8 個 (MIT/Apache-2.0) と llama.cpp
- MIT・BSD・Unicode-3.0・BSL-1.0 などは、バイナリで配るときにも著作権表示と許諾文の同梱を求める。今同梱している
  のは llama.cpp (`LICENSE-llama.cpp.txt`) と Slint の帰属表示 (設定ツールの「QuicklIME について」) だけで、
  ほかの crate の表示が無い
- 配る exe にリンクされる crate の著作権表示と許諾文を、インストーラのビルドのたびに自動で生成して同梱する
- TSF の DLL は Windows SDK のほかに C/C++ のライブラリをリンクしていないので対象外 (アイコンのフォントは
  `LICENSE-MPLUSRounded1c.txt` で表示済み)

## 決定事項

- cargo-about で生成する。リンクされる crate を target と features まで解決して拾い、crate に入っている
  LICENSE ファイルから著作権表示と全文を集める
- 生成はインストーラのビルド時に `installer/build.ps1` で行い、生成物はリポジトリに入れない。依存を更新しても
  表示が古くならない。cargo-about はビルド環境の前提にする (`cargo install --locked --features cli cargo-about`。
  0.9 から `cli` feature を指定しないと exe が入らない)
- 対象は engine の workspace (quicklime-engine・quicklime-gui) と llm (quicklime-llm) の 2 つ。target は
  `x86_64-pc-windows-msvc` に限り、build-dependencies・dev-dependencies は含めない (バイナリに入らない)
  - proc-macro とその依存 (syn・quote・serde_derive・i-slint-compiler など) もバイナリには入らないが、cargo-about
    には除く設定が無いので一覧に含めたまま出す。余分な表示が増えるだけで害は無い (engine は 265 件中 56 件、
    llm は 16 件中 7 件。ライセンスは MIT・Unicode-3.0・0BSD だけ)
- 出力はテキストのファイル 1 つ、`LICENSE-rust-crates.txt` (UTF-8、BOM なし)。既存の `LICENSE-*.txt` と同じく
  インストール先の直下に置き、メモ帳で読めるようにする。2 つの対象の結果を 1 つのファイルにまとめる
  - 中身はライセンスの種類ごとに、そのライセンスを使う crate の名前と版の一覧と、許諾文の全文を並べる
- 自前の crate (quicklime-engine・quicklime-gui・quicklime-llm) は一覧に出さない。本体は `LICENSE.txt` で
  表示している。3 つの Cargo.toml に `publish = false` を足し、cargo-about の `private.ignore = true` で除く
- 受け入れるライセンスを設定ファイルに列挙する。列挙に無いライセンスだけの crate が依存に入ったら生成を失敗させ、
  インストーラのビルドを止める。GPL などが紛れ込んだときに気づくため
  - Slint の crate は `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` で、
    royalty-free を選ぶ。GPL-3.0 は受け入れる一覧に入れない。royalty-free の全文は cargo-about が自動では
    見つけられないので、`about.toml` の clarify で crate の中の `LICENSES/LicenseRef-Slint-Royalty-free-2.0.md` を
    指定する
  - cargo-about 0.9.2 は `LicenseRef-*` の全文を拾う条件が逆になっていて、ファイルのライセンスを
    `LicenseRef-Slint-Royalty-free-2.0` にした clarify を拾わない。同じファイルをダミーのライセンス名
    (`LicenseRef-Slint-Royalty-free-2.0-text`) でも指定し、直った版でも今の版でも全文が 1 回だけ入るようにする
  - checksum が合わない (Slint の更新で文面が変わった) などで clarify が使われなくても、cargo-about は警告だけで
    生成に成功する。build.ps1 で生成後に全文の見出しの行があるかを確かめ、無ければ止める
  - 複数のライセンスから選べる crate は、受け入れる一覧の先にあるものが選ばれる。一覧は MIT を先頭にする
- llama.cpp は `LICENSE-llama.cpp.txt` で表示しているので今のまま残す。llama-cpp-2・llama-cpp-sys-2 の crate
  自体の表示は `LICENSE-rust-crates.txt` に入る
- 設定ファイル (`about.toml`) とテンプレート (`about.hbs`) は `installer/` に置く。インストーラのビルドにだけ使う

### 採らなかった案

- 生成物をリポジトリに入れる: ビルドにツールが要らないが、依存を更新するたびに作り直す必要があり、古くなりうる
- HTML で出す: cargo-about の既定はこちらだが、ほかの `LICENSE-*.txt` と形式が揃わない
- 手で一覧を作る: 250 件あり、依存の更新に追いつけない
- proc-macro 側の crate を除く: cargo-about を JSON で出し、`cargo tree -e normal,no-proc-macro` で求めた crate に
  絞ってテキストを PowerShell で組むことになり、テンプレートが使えず build.ps1 が長くなる

## 変更対象ファイル

| ファイル | 変更 |
|---|---|
| installer/about.toml (新規) | 受け入れるライセンス・target・private.ignore・Slint の扱い |
| installer/about.hbs (新規) | テキストのテンプレート |
| installer/build.ps1 | cargo-about の有無の確認、2 つの対象の生成と連結、staging への出力 |
| installer/installer.iss | `LICENSE-rust-crates.txt` を `[Files]` に足す |
| engine/Cargo.toml・engine/gui/Cargo.toml・llm/Cargo.toml | `publish = false` |
| README.md | 「同梱物とライセンス」に 1 項目足す |
| engine/gui/ui/pages/about-page.slint | ライセンスの一覧に README と同じ項目を足す |
| docs/development.md | インストーラのビルドの前提に cargo-about を足す |
| docs/design/README.md | この設計書を索引に足す (節「配布」を設ける) |

## 実装手順

1. `cargo install --locked --features cli cargo-about` で導入する
2. `installer/about.toml` と `installer/about.hbs` を作る。テンプレートは、先頭にファイルの説明 (QuicklIME の
   exe に静的リンクしている Rust の crate の著作権表示と許諾文であること) を置き、ライセンスごとに名前・使う
   crate の一覧・全文を出す
3. 3 つの Cargo.toml に `publish = false` を足す
4. engine と llm のそれぞれで生成できることを確かめる。受け入れる一覧で足りないライセンスや、LICENSE ファイルを
   見つけられない crate があれば、`about.toml` の clarify などで補う。Slint の royalty-free の全文が出力に
   入るようにする
5. `installer/build.ps1`: 前提の確認の並びに cargo-about の有無の確認を足す (無ければ導入のコマンドを示して止める)。
   既存の LICENSE のコピーの並びで、2 つの対象の結果を連結して `$staging\LICENSE-rust-crates.txt` に書く。
   生成に失敗したら止める
6. `installer/installer.iss` の `[Files]` の LICENSE の並びに `LICENSE-rust-crates.txt` を足す
7. README の「同梱物とライセンス」と about-page.slint の一覧に、次の趣旨の項目を足す: エンジン・設定ツール・
   単語登録ツール・LLM 推論プロセスの exe は、Rust の crate (MIT・Apache-2.0・BSD・Unicode-3.0 など) を
   静的リンクしている。著作権表示と許諾文を `LICENSE-rust-crates.txt` にまとめて同梱している
8. development.md の「インストーラのビルド」の前提に cargo-about を、docs/design/README.md にこの設計書を足す

## 検証方法

- build.ps1 の生成の部分だけを取り出して実行し、`LICENSE-rust-crates.txt` ができる。次を確かめる
  - Slint の crate が royalty-free の項に入り、GPL-3.0 の項が無い
  - Unicode-3.0 (icu_* など)、BSL-1.0 (clipboard-win)、BSD-3-Clause (tiny-skia など) の項があり、全文が入っている
  - Slint の royalty-free の全文が入っている
  - llama-cpp-2 など llm の crate が入っている
  - quicklime-engine・quicklime-gui・quicklime-llm が入っていない
  - Windows 以外の target にだけ要る crate (例: wayland・x11 系) が入っていない
- `about.toml` の受け入れる一覧から一時的に 1 つ (例: BSL-1.0) を外すと生成が失敗し、build.ps1 が止まる
- cargo-about が無い環境を `Get-Command` の結果で装うなどして、導入のコマンドを示して止まることを確かめる
- `cargo build` (engine と gui) が通る
- インストーラ全体のビルドとインストール後のファイルの確認はユーザが行う: インストール先に
  `LICENSE-rust-crates.txt` があり、メモ帳で読める。設定ツールの「QuicklIME について」に項目が出る
