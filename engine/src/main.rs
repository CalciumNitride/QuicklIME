// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod config;
mod convert;
mod datetime;
mod dict;
mod learn;
mod matrix;
mod pos;
mod predict;
mod userdict;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use interprocess::local_socket::traits::{ListenerExt, Stream as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};

use config::Config;
use dict::Dictionary;
use learn::LearningStore;
use matrix::ConnectionMatrix;
use pos::FunctionalIds;
use userdict::UserDict;

/// named pipe の既定名 (Windows では \\.\pipe\quicklime-engine になる)
const DEFAULT_PIPE_NAME: &str = "quicklime-engine";

/// パイプ名。テストで衝突しないよう環境変数 QUICKLIME_PIPE_NAME で上書きできる
fn pipe_name() -> String {
    std::env::var("QUICKLIME_PIPE_NAME").unwrap_or_else(|_| DEFAULT_PIPE_NAME.to_string())
}

/// 変換に必要なデータ一式 (全接続スレッドで共有する)
struct EngineData {
    dictionary: Dictionary,
    matrix: ConnectionMatrix,
    functional: FunctionalIds,
    /// ユーザ辞書 (ADDWORD で書き込むため Mutex で保護)
    user: Mutex<UserDict>,
    /// 学習データ (LEARN で書き込むため Mutex で保護)
    learning: Mutex<LearningStore>,
    /// 設定 (RELOADCONFIG で差し替えるため Mutex で保護)
    config: Mutex<Config>,
}

fn main() -> std::io::Result<()> {
    let pipe = pipe_name();

    // 既に別のエンジンが同じパイプで待機していれば二重起動しない
    // (TSF 側の自動起動が複数アプリから同時に走った場合の保険)
    if Stream::connect(pipe.clone().to_ns_name::<GenericNamespaced>()?).is_ok() {
        eprintln!("既にエンジンが起動しているため終了します");
        return Ok(());
    }

    // ユーザ辞書の品詞名解決に品詞ID表を使うため、先に読み込んでおく
    let functional = load_functional_ids();
    let user = UserDict::load_default(&functional);
    let data = Arc::new(EngineData {
        dictionary: load_dictionary(),
        matrix: load_matrix(),
        functional,
        user: Mutex::new(user),
        learning: Mutex::new(LearningStore::load_default()),
        config: Mutex::new(Config::load_default()),
    });

    let name = pipe.clone().to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    eprintln!("quicklime-engine: \\\\.\\pipe\\{pipe} で待機中");

    // クライアント (アプリごとの TSF DLL) を1接続=1スレッドで処理する
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let data = Arc::clone(&data);
                thread::spawn(move || handle_client(stream, &data));
            }
            Err(e) => eprintln!("接続の受け付けに失敗: {e}"),
        }
    }
    Ok(())
}

/// 辞書ディレクトリの決定。優先順:
/// 1. 環境変数 QUICKLIME_DICT_DIR
/// 2. exe と同じディレクトリの dict\ (インストール先のレイアウト。存在するときのみ)
/// 3. プロジェクトルートの references/mozc/src/data/dictionary_oss
///    (exe が engine/target/{debug,release}/ にある前提で相対解決)
fn dictionary_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QUICKLIME_DICT_DIR") {
        return Some(PathBuf::from(dir));
    }
    let exe = std::env::current_exe().ok()?;
    let bundled = exe.parent()?.join("dict");
    if bundled.is_dir() {
        return Some(bundled);
    }
    let project_root = exe.parent()?.parent()?.parent()?.parent()?;
    Some(project_root.join("references/mozc/src/data/dictionary_oss"))
}

/// 辞書を読み込む。失敗しても空の辞書で起動を続行する (カタカナ/ひらがな候補のみになる)
fn load_dictionary() -> Dictionary {
    let Some(dir) = dictionary_dir() else {
        eprintln!("辞書ディレクトリを特定できません。辞書なしで起動します");
        return Dictionary::empty();
    };
    let started = Instant::now();
    let mut dict = match Dictionary::load(&dir) {
        Ok(dict) => {
            eprintln!(
                "辞書を読み込みました: {} エントリ ({:.1}秒) [{}]",
                dict.entry_count(),
                started.elapsed().as_secs_f64(),
                dir.display()
            );
            dict
        }
        Err(e) => {
            eprintln!("辞書の読み込みに失敗しました ({e})。辞書なしで起動します");
            Dictionary::empty()
        }
    };
    load_symbols(&mut dict, &dir);
    dict
}

/// 記号辞書 (Mozc の symbol.tsv) を読み込む。無くても記号候補なしで続行する。
/// 辞書ディレクトリ直下の symbol.tsv を優先し、無ければ Mozc の配置
/// (dictionary_oss と並ぶ symbol/) から探す
fn load_symbols(dict: &mut Dictionary, dictionary_dir: &Path) {
    let candidates = [
        dictionary_dir.join("symbol.tsv"),
        dictionary_dir.parent().map(|p| p.join("symbol/symbol.tsv")).unwrap_or_default(),
    ];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        eprintln!("記号辞書 (symbol.tsv) がありません。記号候補なしで動作します");
        return;
    };
    match dict.load_symbols(path) {
        Ok(()) => eprintln!(
            "記号辞書を読み込みました: {} エントリ [{}]",
            dict.symbol_count(),
            path.display()
        ),
        Err(e) => eprintln!("記号辞書の読み込みに失敗しました ({e})。記号候補なしで動作します"),
    }
}

/// 連接行列を読み込む。失敗しても連接コスト0で起動を続行する
fn load_matrix() -> ConnectionMatrix {
    let Some(dir) = dictionary_dir() else {
        return ConnectionMatrix::empty();
    };
    let path = dir.join("connection_single_column.txt");
    if !path.exists() {
        eprintln!("連接行列がありません ({}), 連接コスト0で動作します", path.display());
        return ConnectionMatrix::empty();
    }
    let started = Instant::now();
    match ConnectionMatrix::load(&path) {
        Ok(matrix) => {
            eprintln!("連接行列を読み込みました ({:.1}秒)", started.elapsed().as_secs_f64());
            matrix
        }
        Err(e) => {
            eprintln!("連接行列の読み込みに失敗しました ({e})。連接コスト0で動作します");
            ConnectionMatrix::empty()
        }
    }
}

/// 品詞ID表 (id.def) を読み込む。無くても単語=文節として動作を続行する
fn load_functional_ids() -> FunctionalIds {
    let Some(dir) = dictionary_dir() else {
        return FunctionalIds::empty();
    };
    let path = dir.join("id.def");
    match FunctionalIds::load(&path) {
        Ok(ids) => ids,
        Err(e) => {
            eprintln!("品詞ID表の読み込みに失敗しました ({e})。単語単位の文節になります");
            FunctionalIds::empty()
        }
    }
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream, data: &EngineData) {
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);

    for line in reader.lines() {
        let Ok(line) = line else {
            break; // 読み取りエラー = 切断とみなす
        };
        let response = handle_request(&line, data);
        if send.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
}

/// CONVSEG 応答で文節内のフィールドを区切る文字 (ASCII Unit Separator)
const FIELD_SEPARATOR: char = '\x1f';

/// 1行の要求を解釈して1行の応答を作る
fn handle_request(line: &str, data: &EngineData) -> String {
    // 日付・時刻の動的候補用の現在日時 (CONVSYM の候補生成と LEARN の除外判定に使う)
    let now = chrono::Local::now().naive_local();

    let mut fields = line.split('\t');
    match fields.next() {
        Some("CONVERT") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let candidates = convert::candidates(kana, &data.dictionary, &user, &data.matrix);
                format!("OK\t{}\n", candidates.join("\t"))
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVSEG") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let learning = data.learning.lock().expect("learning lock");
                // 3番目のフィールドは文節長 (カンマ区切り、文節伸縮時の境界固定用)
                let segments = if let Some(lengths_field) = fields.next() {
                    let lengths: Vec<usize> =
                        lengths_field.split(',').filter_map(|t| t.parse().ok()).collect();
                    let segments = convert::convert_segments_fixed(
                        kana,
                        &lengths,
                        &data.dictionary,
                        &user,
                        &data.matrix,
                        &learning,
                    );
                    if segments.is_empty() {
                        return "ERR\t文節長が不正です\n".to_string();
                    }
                    segments
                } else {
                    convert::convert_segments(
                        kana,
                        &data.dictionary,
                        &user,
                        &data.matrix,
                        &data.functional,
                        &learning,
                    )
                };
                let body = segments
                    .iter()
                    .map(|s| {
                        let mut fields = vec![s.reading.as_str()];
                        fields.extend(s.candidates.iter().map(String::as_str));
                        fields.join(&FIELD_SEPARATOR.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join("\t");
                format!("OK\t{body}\n")
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVSYM") => match fields.next() {
            // 特殊変換 (F4 用): 記号辞書の候補と日付・時刻の動的候補を返す。
            // 通常語は含めない。該当なしは候補ゼロの OK
            Some(kana) if !kana.is_empty() => {
                let mut candidates: Vec<String> =
                    data.dictionary.lookup_symbols(kana).to_vec();
                candidates.extend(datetime::candidates_at(kana, now));
                if candidates.is_empty() {
                    "OK\n".to_string()
                } else {
                    format!("OK\t{}\n", candidates.join("\t"))
                }
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVUSER") => match fields.next() {
            // ユーザ辞書変換 (F5 用): 読みに完全一致するユーザ登録語
            // (短縮よみ → 名詞系、記載順) のみを返す。該当なしは候補ゼロの OK
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let mut candidates: Vec<String> =
                    user.lookup_shortcuts(kana).into_iter().map(String::from).collect();
                for word in user.lookup_words(kana) {
                    if !candidates.iter().any(|s| *s == word.surface) {
                        candidates.push(word.surface.clone());
                    }
                }
                if candidates.is_empty() {
                    "OK\n".to_string()
                } else {
                    format!("OK\t{}\n", candidates.join("\t"))
                }
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("PREDICT") => match fields.next() {
            // 予測入力: 読みの前方一致でユーザ辞書・履歴・辞書から候補を返す。
            // 最小文字数未満・該当なし・サジェスト無効時は候補ゼロの OK (エラーにしない)
            Some(kana) if !kana.is_empty() => {
                let cfg = *data.config.lock().expect("config lock");
                if !cfg.suggest {
                    return "OK\n".to_string();
                }
                let user = data.user.lock().expect("user lock");
                let learning = data.learning.lock().expect("learning lock");
                let candidates =
                    predict::predict(kana, &data.dictionary, &user, &learning, &cfg);
                if candidates.is_empty() {
                    "OK\n".to_string()
                } else {
                    let body = candidates
                        .iter()
                        .map(|(reading, surface)| {
                            format!("{reading}{FIELD_SEPARATOR}{surface}")
                        })
                        .collect::<Vec<_>>()
                        .join("\t");
                    format!("OK\t{body}\n")
                }
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("LEARN") => {
            // LEARN\t読み\x1f表記\t読み\x1f表記... : 文節ごとの確定結果を記録する。
            // 学習が無効なら記録せず OK を返す (既存の学習データは使い続ける)
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for pair in fields {
                if let Some((reading, surface)) = pair.split_once(FIELD_SEPARATOR) {
                    // 日付・時刻の動的候補は時間が経つと古くなるため学習しない
                    // (学習すると翌日以降も昨日の日付が先頭に来てしまう)
                    if !datetime::candidates_at(reading, now).iter().any(|c| c == surface) {
                        learning.record(reading, surface);
                    }
                    count += 1;
                }
            }
            if count > 0 {
                "OK\n".to_string()
            } else {
                "ERR\t記録する内容がありません\n".to_string()
            }
        }
        Some("ADDWORD") => {
            // ADDWORD\t読み\t表記\t品詞 : ユーザ辞書へ1件登録する
            // (userdict.tsv へ追記し、メモリへ即時反映する)
            let (Some(reading), Some(surface), Some(pos)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return "ERR\t引数が足りません (読み・表記・品詞)\n".to_string();
            };
            let mut user = data.user.lock().expect("user lock");
            match user.add(reading, surface, pos, &data.functional) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERR\t{e}\n"),
            }
        }
        Some("RELOADUSER") => {
            // ユーザ辞書ファイルを読み直す (手動編集の反映用)
            let mut user = data.user.lock().expect("user lock");
            user.reload(&data.functional);
            eprintln!(
                "ユーザ辞書を再読込しました: 短縮よみ {} 件・単語 {} 件",
                user.shortcut_count(),
                user.word_count()
            );
            "OK\n".to_string()
        }
        Some("RELOADCONFIG") => {
            // 設定ファイルを読み直す (設定ツールの保存時に呼ばれる)
            *data.config.lock().expect("config lock") = Config::load_default();
            eprintln!("設定を再読込しました");
            "OK\n".to_string()
        }
        _ => "ERR\t不明なコマンドです\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_data() -> EngineData {
        EngineData {
            dictionary: Dictionary::empty(),
            matrix: ConnectionMatrix::empty(),
            functional: FunctionalIds::empty(),
            user: Mutex::new(UserDict::empty()),
            learning: Mutex::new(LearningStore::in_memory()),
            config: Mutex::new(Config::default()),
        }
    }

    fn sample_data() -> EngineData {
        let mut dictionary = Dictionary::empty();
        dictionary
            .load_from(
                "きょう\t1\t1\t2000\t今日\nは\t2\t2\t500\tは\nはれ\t1\t1\t3000\t晴れ\n"
                    .as_bytes(),
            )
            .unwrap();
        dictionary.finalize();
        let functional = FunctionalIds::load_from("1 名詞,一般\n2 助詞,係助詞\n".as_bytes()).unwrap();
        EngineData {
            dictionary,
            matrix: ConnectionMatrix::empty(),
            functional,
            user: Mutex::new(UserDict::empty()),
            learning: Mutex::new(LearningStore::in_memory()),
            config: Mutex::new(Config::default()),
        }
    }

    /// data のユーザ辞書に TSV の内容を読み込む
    fn load_user(data: &EngineData, tsv: &str) {
        data.user
            .lock()
            .unwrap()
            .load_from(tsv.as_bytes(), &data.functional);
    }

    #[test]
    fn convert要求に候補を返す() {
        assert_eq!(
            handle_request("CONVERT\tにほん", &empty_data()),
            "OK\tニホン\tにほん\n"
        );
    }

    #[test]
    fn かなが空ならエラー() {
        let data = empty_data();
        assert!(handle_request("CONVERT\t", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVERT", &data).starts_with("ERR\t"));
    }

    #[test]
    fn 不明なコマンドはエラー() {
        assert!(handle_request("FOO\tbar", &empty_data()).starts_with("ERR\t"));
    }

    #[test]
    fn convseg要求に文節列を返す() {
        // 文節はタブ区切り、文節内は US (\x1f) 区切りで「読み 候補1 候補2...」
        let response = handle_request("CONVSEG\tきょうは", &sample_data());
        assert_eq!(
            response,
            "OK\tきょうは\x1f今日は\x1fキョウハ\x1fきょうは\n"
        );
    }

    #[test]
    fn convsegで文節長を指定できる() {
        // 「はれ / は」に固定 (通常の文節分割なら「はれは」1文節)
        let response = handle_request("CONVSEG\tはれは\t2,1", &sample_data());
        assert_eq!(
            response,
            "OK\tはれ\x1f晴れ\x1fハレ\x1fはれ\tは\x1fは\x1fハ\n"
        );

        // 長さの合計が合わない場合はエラー
        assert!(handle_request("CONVSEG\tはれは\t9,9", &sample_data()).starts_with("ERR\t"));
    }

    #[test]
    fn convsymに日付候補が入る() {
        // 「きょう」の特殊変換には現在日付の候補 (2026/07/15 形式など) が入る
        let now = chrono::Local::now().naive_local();
        let expected = datetime::candidates_at("きょう", now);
        let response = handle_request("CONVSYM\tきょう", &sample_data());
        assert!(response.starts_with("OK\t"));
        for candidate in &expected {
            assert!(response.contains(candidate.as_str()), "{candidate} が候補に無い: {response}");
        }
        // 通常変換 (CONVSEG) には日付候補は入らない
        let response = handle_request("CONVSEG\tきょう", &sample_data());
        assert!(!response.contains(&expected[0]), "CONVSEG に日付候補が入っている: {response}");
    }

    #[test]
    fn convsymで記号と日付候補が両方出る() {
        // 記号辞書に「きょう」の読みを持つエントリがあれば、記号 → 日付の順に並ぶ
        let mut data = sample_data();
        data.dictionary
            .load_symbols_from("記号\t↑\tきょう\t上矢印 (テスト用の読み)\n".as_bytes())
            .unwrap();
        let now = chrono::Local::now().naive_local();
        let date = datetime::candidates_at("きょう", now)[0].clone();
        let response = handle_request("CONVSYM\tきょう", &data);
        assert!(response.starts_with("OK\t↑\t"), "記号が先頭に無い: {response}");
        assert!(response.contains(&date), "日付候補が無い: {response}");
    }

    #[test]
    fn 日付候補は学習されない() {
        let data = sample_data();
        let now = chrono::Local::now().naive_local();
        let date = datetime::candidates_at("きょう", now)[0].clone();

        // 日付候補の確定は学習に記録されず、候補順は変わらない
        assert_eq!(handle_request(&format!("LEARN\tきょう\x1f{date}"), &data), "OK\n");
        let response = handle_request("CONVSEG\tきょう", &data);
        assert!(response.starts_with("OK\tきょう\x1f今日\x1f"));

        // 通常の表記は今まで通り学習される
        assert_eq!(handle_request("LEARN\tきょう\x1fキョウ", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょう", &data);
        assert!(response.starts_with("OK\tきょう\x1fキョウ\x1f今日\x1f"));
    }

    #[test]
    fn convsym要求に記号候補のみを返す() {
        let mut data = empty_data();
        data.dictionary
            .load_symbols_from(
                "記号\t→\tやじるし みぎ\t右矢印\n記号\t←\tやじるし\t左矢印\n".as_bytes(),
            )
            .unwrap();
        assert_eq!(handle_request("CONVSYM\tやじるし", &data), "OK\t→\t←\n");
        // 記号辞書に無い読みは候補ゼロの OK (エラーにしない)
        assert_eq!(handle_request("CONVSYM\tにほん", &data), "OK\n");
        assert!(handle_request("CONVSYM\t", &data).starts_with("ERR\t"));
    }

    #[test]
    fn learnで記録した表記が次のconvsegで先頭に来る() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN\tきょうは\x1fキョウハ", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert_eq!(
            response,
            "OK\tきょうは\x1fキョウハ\x1f今日は\x1fきょうは\n"
        );
    }

    #[test]
    fn learnの内容が空ならエラー() {
        assert!(handle_request("LEARN", &empty_data()).starts_with("ERR\t"));
        assert!(handle_request("LEARN\t読みだけ", &empty_data()).starts_with("ERR\t"));
    }

    #[test]
    fn convuser要求にユーザ登録語のみを返す() {
        let data = sample_data();
        load_user(
            &data,
            "きょう\tmail@example.com\t短縮よみ\nきょう\tsecond@example.jp\t短縮よみ\nきょう\t匡\t名\n",
        );
        // sample_data の辞書語 (今日など) は含めず、短縮よみ → 名詞系を記載順で返す
        assert_eq!(
            handle_request("CONVUSER\tきょう", &data),
            "OK\tmail@example.com\tsecond@example.jp\t匡\n"
        );
        assert_eq!(handle_request("CONVUSER\tそんざいしない", &data), "OK\n");
        assert!(handle_request("CONVUSER\t", &data).starts_with("ERR\t"));
    }

    #[test]
    fn addwordで登録した語がすぐ変換に出る() {
        let data = sample_data();
        // 登録前は変換されない (未知語としてそのまま)
        let before = handle_request("CONVSEG\tかんべは", &data);
        assert!(!before.contains("神戸"), "登録前から神戸が出ている: {before}");

        assert_eq!(handle_request("ADDWORD\tかんべ\t神戸\t姓", &data), "OK\n");
        let response = handle_request("CONVSEG\tかんべは", &data);
        assert!(response.contains("神戸は"), "神戸は が候補に無い: {response}");
        // 予測にも出る
        let response = handle_request("PREDICT\tかん", &data);
        assert!(response.contains("神戸"), "予測に神戸が無い: {response}");
    }

    #[test]
    fn addwordの検証エラー() {
        let data = sample_data();
        assert!(handle_request("ADDWORD\tよみ\t表記", &data).starts_with("ERR\t"));
        assert!(handle_request("ADDWORD\tよみ\t表記\t動詞", &data).starts_with("ERR\t"));
        assert!(handle_request("ADDWORD\t\t表記\t名詞", &data).starts_with("ERR\t"));
        // 重複はエラー
        assert_eq!(handle_request("ADDWORD\tよみ\t表記\t名詞", &data), "OK\n");
        assert!(handle_request("ADDWORD\tよみ\t表記\t名詞", &data).starts_with("ERR\t"));
    }

    #[test]
    fn reloaduserはメモリ上のみでもokを返す() {
        // パスなし (テスト用) のユーザ辞書では何もせず OK
        assert_eq!(handle_request("RELOADUSER", &sample_data()), "OK\n");
    }

    #[test]
    fn predict要求に前方一致の候補を返す() {
        // sample_data の辞書には「きょう」(今日) がある
        let response = handle_request("PREDICT\tきょ", &sample_data());
        assert_eq!(response, "OK\tきょう\x1f今日\n");
    }

    #[test]
    fn predictはlearn後に履歴が先頭に来る() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        let response = handle_request("PREDICT\tきょ", &data);
        assert_eq!(response, "OK\tきょうしつ\x1f教室\tきょう\x1f今日\n");
    }

    #[test]
    fn predictは2文字未満と該当なしで候補ゼロのok() {
        let data = sample_data();
        assert_eq!(handle_request("PREDICT\tき", &data), "OK\n");
        assert_eq!(handle_request("PREDICT\tそんざいしない", &data), "OK\n");
        assert!(handle_request("PREDICT\t", &data).starts_with("ERR\t"));
        assert!(handle_request("PREDICT", &data).starts_with("ERR\t"));
    }

    #[test]
    fn サジェスト無効ならpredictは候補ゼロのok() {
        let data = sample_data();
        data.config.lock().unwrap().suggest = false;
        assert_eq!(handle_request("PREDICT\tきょ", &data), "OK\n");
    }

    #[test]
    fn 学習無効ならlearnは記録しない() {
        let data = sample_data();
        data.config.lock().unwrap().learning = false;
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        // 記録されていないので履歴候補は出ない
        assert_eq!(handle_request("PREDICT\tきょ", &data), "OK\tきょう\x1f今日\n");
    }

    #[test]
    fn reloadconfigはokを返す() {
        assert_eq!(handle_request("RELOADCONFIG", &sample_data()), "OK\n");
    }
}
