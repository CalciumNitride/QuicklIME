# 英単語の大文字小文字の補正

## 背景と目的

モードレス入力 (docs/design/modeless.md、docs/design/modeless-detection.md) と英字混じり文の変換
(docs/design/mixed-conversion.md) では、英単語は打ったとおりの大文字小文字で出る。`github` と打つと
`github` のままで、`GitHub` にするには Shift を混ぜて打つ必要がある。

英単語辞書 (data/english-words.txt、data/english-names.txt) は小文字だけで持っており、正しい表記の
情報が無い。ユーザ辞書の英字の語も、照合用には小文字にして持っている。

本書の目的は、`github` と打ったときに `GitHub` を候補として選べるようにすることである。

## 決定事項

新しい設定は設けない。補正した表記は候補として出すだけで、打った文字列を自動では書き換えない。

### A. 正しい表記の出どころ

- data/english-names.txt を正しい表記で書く (下表)。照合は今どおり小文字にして行い、表記が小文字だけでない
  語について「小文字の形 → 表記」を持つ
- ユーザ辞書の英字の語 (短縮よみ・名詞系単語のうち表記が ASCII 英字だけのもの) は、登録した表記が
  小文字だけでなければ、その表記を正しい表記として持つ。小文字だけの登録は読みから引くためのものとみなし、
  表記として持たない (english-names.txt の表記を打ち消さない)。english-names.txt と同じ小文字の形があればユーザ辞書を優先する。
  同じ小文字の形で表記の異なる登録が複数あるときは、ユーザ辞書の並び順で先のもの
- data/english-words.txt (SCOWL) の語は補正に使わない (今どおり小文字だけ)。`may`/`May`、`bill`/`Bill` の
  ように、小文字のほうが正しい場合があるため
- 表記が ASCII 英字だけで書けない語 (`Next.js`、`Node.js`、`VS Code`、`Wi-Fi`、`Hugging Face`) は補正しない

english-names.txt の表記 (ここに無い語は小文字のまま、補正しない):

| 小文字 | 表記 | 小文字 | 表記 | 小文字 | 表記 |
|---|---|---|---|---|---|
| amazon | Amazon | intellij | IntelliJ | perl | Perl |
| ansible | Ansible | ipad | iPad | playstation | PlayStation |
| anthropic | Anthropic | ipados | iPadOS | postgres | Postgres |
| apache | Apache | iphone | iPhone | postgresql | PostgreSQL |
| bitbucket | Bitbucket | java | Java | powershell | PowerShell |
| bluetooth | Bluetooth | javascript | JavaScript | pytorch | PyTorch |
| centos | CentOS | jenkins | Jenkins | qualcomm | Qualcomm |
| chatgpt | ChatGPT | jetbrains | JetBrains | readme | README |
| claude | Claude | jira | Jira | reddit | Reddit |
| clojure | Clojure | julia | Julia | redis | Redis |
| cloudflare | Cloudflare | jupyter | Jupyter | samsung | Samsung |
| confluence | Confluence | kafka | Kafka | scala | Scala |
| copilot | Copilot | kotlin | Kotlin | sony | Sony |
| debian | Debian | kubernetes | Kubernetes | spotify | Spotify |
| deno | Deno | laravel | Laravel | sqlite | SQLite |
| django | Django | linkedin | LinkedIn | supabase | Supabase |
| docker | Docker | linux | Linux | svelte | Svelte |
| dropbox | Dropbox | lua | Lua | tensorflow | TensorFlow |
| elixir | Elixir | macos | macOS | terraform | Terraform |
| emacs | Emacs | markdown | Markdown | tiktok | TikTok |
| erlang | Erlang | matlab | MATLAB | trello | Trello |
| facebook | Facebook | microsoft | Microsoft | ubuntu | Ubuntu |
| fedora | Fedora | mongodb | MongoDB | unicode | Unicode |
| figma | Figma | mysql | MySQL | vercel | Vercel |
| firebase | Firebase | neovim | Neovim | vim | Vim |
| firefox | Firefox | netflix | Netflix | vite | Vite |
| gemini | Gemini | netlify | Netlify | vue | Vue |
| github | GitHub | nintendo | Nintendo | wezterm | WezTerm |
| gitlab | GitLab | numpy | NumPy | whatsapp | WhatsApp |
| gmail | Gmail | nuxt | Nuxt | wikipedia | Wikipedia |
| google | Google | nvidia | NVIDIA | xbox | Xbox |
| haskell | Haskell | oauth | OAuth | xcode | Xcode |
| heroku | Heroku | onedrive | OneDrive | youtube | YouTube |
| icloud | iCloud | openai | OpenAI | zapier | Zapier |
| instagram | Instagram | | | | |
| intel | Intel | | | | |

小文字のまま残す語: gitignore、golang、huggingface、localhost、nextjs、nginx、nodejs、pnpm、regex、
tmux、vscode、webhook、webpack、wifi、zsh

### B. プロトコル: ASCIIFORM

```
要求:   ASCIIFORM\t<文字列>\n
成功:   OK\t<表記>\n
        表記: 文字列 (小文字にして照合する) が A の正しい表記を持つ語と完全一致し、表記が文字列と
              異なるときはその表記。それ以外は空
失敗:   ERR\t<メッセージ>\n   (文字列の欠落、英字以外を含む)
```

- 例: `github` → `OK\tGitHub`、`Github` → `OK\tGitHub`、`GitHub` → `OK\t`、`apple` → `OK\t`
- TSF 層は自動起動や接続待ちをせずに送る (ASCIISTART・ASCIIWORD と同じ呼び方)。未接続・エラー・
  ASCIIFORM を知らない旧エンジンのときは補正の候補を出さない
- ASCIIWORD の応答に表記を足す案は採らない。今の TSF 層の解析が余分なフィールドを不正とみなすため、
  旧 TSF 層と新エンジンの組み合わせで英字区間を閉じる判定が働かなくなる

### C. 変換 (エンジン)

- `build_lattice` で英字の連続全体を1語にするノード (docs/design/mixed-conversion.md の B) を作るとき、
  連続が A の正しい表記を持ち、表記が読みと異なるなら、読み = 英字の連続・表記 = 正しい表記のノードを
  もう1つ足す。単語コストは `ASCII_WORD_COST` より小さい値 (`ASCII_WORD_COST - 100`) にして、
  打ったとおりのノードより上位に来るようにする。打ったとおりのノードは今どおり残す
- これにより CONVNBEST・CONVSEG・CONV などの読みを変換するすべてのコマンドで、英字を含む読みの
  補正した候補が上位に、打ったとおりの候補がその下に出る
  (`きょうはgithubでさぎょうする` → `今日はGitHubで作業する`、`今日はgithubで作業する`)
- 変換キー・候補バーの全体変換は、この結果をそのまま使う (TSF 層の変更は無い)。
  閉じた英字区間を含む run と、modeless 有効で run 全体が英字の run (読みが `github`) が対象になる
- 学習は今どおり。打ったとおりの候補を選んで確定すれば、次からそちらが上位に来る
- 英字を含む候補の `protected` (LLM の再ランクから外す) は今どおり

### D. 生ローマ字の候補 (TSF 層)

- `InsertRawCandidates` で打鍵のローマ字と大文字小文字の変種 (`CaseCycleVariants`) を入れるとき、
  打鍵のローマ字 (`Raw()`) が英字だけなら ASCIIFORM を問い合わせ、表記が返れば変種の先頭に入れる
  (`github` / `GitHub` / `Github` / `GITHUB` の順でなく、`GitHub` / `github` / `Github` / `GITHUB`)。
  既にある候補は今どおり動かさず入れない
- modeless の有効・無効によらない (`modeless 0` で `ぎてゅb` → 変換キーでも `GitHub` を選べる)
- F6〜F10 の循環 (`CaseCycleVariants` の他の呼び出し) は変えない

### E. 開いている英字区間の候補バー (TSF 層)

- modeless が有効で、開いている英字区間があるとき (自動判定で入ったもの・Shift+英字で入ったものの両方。
  run 全体が英字の run を含む)、候補バーを作り直すたびに英字区間の文字列で ASCIIFORM を問い合わせる。
  同じ文字列への応答は覚えておき、文字列が変わるまで問い合わせ直さない
- 表記が返れば、英字区間の文字列をその表記に置き換えた run を候補バーの先頭の候補にする
  (`きょうはgithub` → `きょうはGitHub`、`github` → `GitHub`)。ほかの候補は今どおりその後ろに並ぶ。
  ほかの候補が無ければこの候補だけのバーを出す。`min_suggest_chars` によらず出す
- 候補の種類と採用の経路は英字候補 (`BarKind::Ascii`、docs/design/modeless-commit-candidate.md) と同じにする:
  `composer_` の複製の英字区間のかなと打鍵列を表記に置き換え、採用時に `composer_` をその複製に置き換えて
  文書の run を書き換える。採用後も英字区間は開いたまま
- 採用後の扱いは英字候補と同じ (Space は確定して半角スペース、確定キーは確定、Enter は確定して Enter を
  アプリへ渡す、印字キーは英字区間に打鍵を続ける)
- 英字モード中は英字候補 (run 終了時の判定) を出さないので、同じバーに両方が並ぶことは無い
- `candidate_bar 0` のときは出ない (変換キーの C・D で直す)

## 変更対象ファイル

| ファイル | 変更内容 |
|---|---|
| data/english-names.txt | 正しい表記で書き直す。先頭のコメント (英小文字で書く旨) を更新 |
| engine/src/english.rs | 表記が小文字だけでない語の「小文字の形 → 表記」を持ち、引けるようにする。テスト |
| engine/src/userdict.rs | 英字の登録語の表記を保持し、引けるようにする。テスト |
| engine/src/main.rs | ASCIIFORM の処理 (ユーザ辞書を優先)、変換への表記の引き方の受け渡し。テスト |
| engine/src/convert.rs | `build_lattice` の補正した表記のノード。テスト |
| tsf/src/engine_client.h, tsf/src/engine_client.cpp | ASCIIFORM の呼び出し (接続待ちしない) |
| tsf/src/romaji.h, tsf/src/romaji.cpp | 開いている英字区間のかなと打鍵列を同じ長さの文字列に置き換える操作 |
| tsf/src/text_service.h, tsf/src/text_service.cpp | `InsertRawCandidates` の正しい表記の追加、英字区間の補正候補の生成 (応答の保持を含む) とバーへの表示 |
| tsf/src/text_service_direct.cpp | 採用時の文書の書き換えが英字候補の経路で足りない場合の対応 |
| docs/protocol.md | ASCIIFORM の追記 |
| docs/design/mixed-conversion.md | 「英単語の大文字小文字は直さない」「別の課題とする」を本書の参照に置き換える |
| README.md | モードレス入力・英字混じりの変換の説明に補正の候補を追記 |
| docs/roadmap.md | 該当項目 (英字混じり文の変換の「未対応: 英単語の大文字小文字の補正」) の完了 |

## 実装手順

各手順の後にビルドとテストが通り、既存の動作が壊れていないことを確かめる。

1. data: english-names.txt を表記付きにする
2. engine: english.rs・userdict.rs に表記の保持と引き方。`cargo test`
3. engine: ASCIIFORM (main.rs)。隔離したエンジンにコマンドを送って結果を確かめる
4. engine: `build_lattice` の補正した表記のノード。`cargo test` に
   `きょうはgithubでさぎょう` の先頭候補が `GitHub` を含み、`github` を含む候補も出るテストを足す
5. tsf: engine_client に ASCIIFORM
6. tsf: `InsertRawCandidates` の正しい表記
7. tsf: 開いている英字区間の補正候補 (romaji の置き換え操作、バーへの表示、採用)
8. ドキュメントの更新 (protocol.md、mixed-conversion.md、README.md、roadmap.md)

## 検証方法

開発版 DLL に切り替え (scripts\dev-deploy.ps1 -Dll)、テスト用アプリで行う。config.tsv は
`QUICKLIME_CONFIG_FILE` で隔離したものを使い、`modeless 1`・`candidate_bar 1`・`debug_log` を設定する。
エンジンを release・debug の両方でビルドし直し、古いエンジンプロセスが残っていないことを確かめる。
ユーザ辞書は `QUICKLIME_USER_DICT_FILE` で隔離したものを使う。

| # | 環境 | 操作 | 期待 |
|---|---|---|---|
| 1 | 隔離したエンジン | `ASCIIFORM\tgithub` / `Github` / `GitHub` / `apple` / `localhost` | `OK\tGitHub` / `OK\tGitHub` / `OK\t` / `OK\t` / `OK\t` |
| 2 | 隔離したエンジン | ユーザ辞書に表記 `Github` を登録して `ASCIIFORM\tgithub` | `OK\tGithub` (ユーザ辞書を優先) |
| 3 | 隔離したエンジン | `ASCIIFORM\tgit-hub` | `ERR` |
| 4 | メモ帳 | `kyouhagithubdesagyousuru` | 文書は打ったとおり `きょうはgithubでさぎょうする` (自動では直さない) |
| 5 | メモ帳 | 4 に続けて変換キー | 先頭候補が `今日はGitHubで作業する`。下位に `今日はgithubで作業する` がある |
| 6 | メモ帳 | 4 の途中 (`kyouhagithubde`) の候補バー | 全体変換に `今日はGitHubで` 系が出る |
| 7 | メモ帳 | `kyouhagithub` の候補バー | 先頭に `きょうはGitHub` が出る |
| 8 | メモ帳 | 7 → Tab → Space | `きょうはGitHub ` (半角スペース) |
| 9 | メモ帳 | 7 → Tab → `s` | `きょうはGitHubs` (英字区間に続く) |
| 10 | メモ帳 | `github` → 変換キー | 先頭候補が `GitHub` |
| 11 | メモ帳 | `github` の候補バー → Tab → 確定キー | `GitHub` |
| 12 | メモ帳 | `modeless 0` で `github` → 変換キー | 生ローマ字の候補の先頭が `GitHub`、続いて `github` `Github` `GITHUB` |
| 13 | メモ帳 | 5 で `今日はgithubで作業する` を選んで確定 → 同じ入力 → 変換キー | 学習により `github` の候補が先頭 |
| 14 | メモ帳 | `kyouhaapple` の候補バー | 補正の候補は出ない (SCOWL の語) |
| 15 | メモ帳 | Shift+`G` `itHub` の候補バー | 補正の候補は出ない (表記と同じ) |
| 16 | メモ帳 | `kyouhamacos` の候補バー / 変換キー | `きょうはmacOS` が出る / `今日はmacOS` |
| 17 | メモ帳 | エンジンを終了して 7・12 | 補正の候補は出ず、今どおり |
| 18 | WezTerm | 7・8・9 と同じ操作 | メモ帳と同じ。採用で日本語区間が消されない |
| 19 | Chrome の textarea | 5・8 と同じ操作 | メモ帳と同じ |
| 20 | メモ帳 | `candidate_bar 0` で `kyouhagithub` → 変換キー | バーは出ない。変換キーの候補に `今日はGitHub` が出る |

## 未決事項

- SCOWL にもある語で、技術文脈では大文字で書くことが多い語 (`python` → `Python`、`rust`、`go`、`swift`、
  `ruby`) を english-names.txt に表記付きで足すか。一般語としての意味もあるため本書では足さず、常用で見る
- 略語 (`api` → `API`、`html` → `HTML`) は Shift で打つ前提で辞書に入れていない。必要なら別の課題とする
- 補正した表記のノードのコストの差 (100) が、学習や接続コストとの兼ね合いで足りるかは常用で見る
