// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod config;
mod convert;
mod datetime;
mod dict;
// 取り込み処理は単語登録ツールが使う。エンジン本体では単体テストのためだけに含める
#[cfg(test)]
mod import;
mod learn;
mod llm;
mod matrix;
mod pos;
mod predict;
mod userdict;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// LLM による並べ替えのワーカーと、接続ごとの依頼・結果
    llm: llm::LlmManager,
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
    let user = UserDict::load_default(&functional, learn::learned_word_path());
    let config = Config::load_default();
    let data = Arc::new(EngineData {
        dictionary: load_dictionary(),
        matrix: load_matrix(),
        functional,
        user: Mutex::new(user),
        learning: Mutex::new(LearningStore::load_default()),
        config: Mutex::new(config),
        llm: llm::LlmManager::new(Box::new(llm::ProcessLauncher)),
    });
    data.llm.configure(config.llm, config.llm_backend);

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

/// 接続ごとの状態 (RERANK の依頼 ID は接続ごとに振る)
struct Session {
    conn: u64,
    /// 最後に受け付けた RERANK の ID
    last_id: u64,
}

impl Session {
    fn new() -> Self {
        static NEXT_CONN: AtomicU64 = AtomicU64::new(1);
        Session { conn: NEXT_CONN.fetch_add(1, Ordering::Relaxed), last_id: 0 }
    }
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream, data: &EngineData) {
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);
    let mut session = Session::new();

    for line in reader.lines() {
        let Ok(line) = line else {
            break; // 読み取りエラー = 切断とみなす
        };
        let response = handle_line(&line, data, &mut session);
        if send.write_all(response.as_bytes()).is_err() {
            break;
        }
    }
    data.llm.disconnect(session.conn);
}

/// 接続ごとの状態を使う要求 (RERANK・RERANKGET) を処理し、それ以外は handle_request に回す
fn handle_line(line: &str, data: &EngineData, session: &mut Session) -> String {
    let mut fields = line.split('\t');
    match fields.next() {
        Some("RERANK") => {
            // RERANK\t<LLM 文脈>\t<文脈読み>\x1f<文脈表記>\t<かな> : LLM による並べ替えを依頼する。
            // LLM が無効・使えない (読み込み中を含む) ときは ID 0 (並べ替えをしない)
            let llm_context = fields.next().unwrap_or("");
            let ctx = fields.next().and_then(parse_context);
            let Some(kana) = fields.next().filter(|k| !k.is_empty()) else {
                return "ERR\tかなが空です\n".to_string();
            };
            if !data.config.lock().expect("config lock").llm {
                return "OK\t0\n".to_string();
            }
            let candidates = {
                let user = data.user.lock().expect("user lock");
                let learning = data.learning.lock().expect("learning lock");
                convert::convert_nbest(
                    kana,
                    ctx.as_ref(),
                    &data.dictionary,
                    &user,
                    &data.matrix,
                    &data.functional,
                    &learning,
                )
            };
            let job = llm::Job {
                id: session.last_id + 1,
                context: llm::preprocess_context(llm_context),
                reading: kana.to_string(),
                candidates,
            };
            if !data.llm.request(session.conn, job) {
                return "OK\t0\n".to_string();
            }
            session.last_id += 1;
            format!("OK\t{}\n", session.last_id)
        }
        Some("RERANKGET") => {
            // RERANKGET\t<ID> : 並べ替えの結果を問い合わせる
            let Some(id) = fields.next().and_then(|f| f.parse::<u64>().ok()) else {
                return "ERR\tID が不正です\n".to_string();
            };
            match data.llm.poll(session.conn, id) {
                llm::Poll::Pending => "OK\tPENDING\n".to_string(),
                llm::Poll::Done(candidates) => {
                    format!("OK\tDONE\t{}\n", format_candidates(&candidates))
                }
                llm::Poll::None => "OK\tNONE\n".to_string(),
            }
        }
        _ => handle_request(line, data),
    }
}

/// CONVSEG 応答で文節内のフィールドを区切る文字 (ASCII Unit Separator)
const FIELD_SEPARATOR: char = '\x1f';

/// CONVCTX の前文脈フィールドの最大文字数 (読み・表記それぞれ)。
/// 異常に長い文脈が送られてもビタビ復元の計算量を抑えるためのガード
const MAX_CONTEXT_CHARS: usize = 64;

/// CONVCTX の前文脈フィールド「読み\x1f表記」を解釈する。
/// どちらかが空なら前文脈なし (None) として扱う
fn parse_context(field: &str) -> Option<convert::Context> {
    let (reading, surface) = field.split_once(FIELD_SEPARATOR)?;
    if reading.is_empty() || surface.is_empty() {
        return None;
    }
    // 長すぎる場合は末尾を残して切り詰める (文脈IDの復元に効くのは末尾)
    let tail = |s: &str| -> String {
        let chars: Vec<char> = s.chars().collect();
        chars[chars.len().saturating_sub(MAX_CONTEXT_CHARS)..].iter().collect()
    };
    Some(convert::Context { reading: tail(reading), surface: tail(surface) })
}

/// CONVSEG / CONVCTX 共通の文節変換応答を作る。
/// lengths_field は文節長 (カンマ区切り、文節伸縮時の境界固定用)、無ければ通常の文節分割
fn segments_response(
    kana: &str,
    lengths_field: Option<&str>,
    ctx: Option<&convert::Context>,
    data: &EngineData,
) -> String {
    let user = data.user.lock().expect("user lock");
    let learning = data.learning.lock().expect("learning lock");
    let segments = if let Some(lengths_field) = lengths_field {
        let lengths: Vec<usize> =
            lengths_field.split(',').filter_map(|t| t.parse().ok()).collect();
        let segments = convert::convert_segments_fixed(
            kana,
            &lengths,
            ctx,
            &data.dictionary,
            &user,
            &data.matrix,
            &data.functional,
            &learning,
        );
        if segments.is_empty() {
            return "ERR\t文節長が不正です\n".to_string();
        }
        segments
    } else {
        convert::convert_segments(
            kana,
            ctx,
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

/// CONVNBEST 応答で候補内の文節を区切る文字 (ASCII Record Separator)
const SEGMENT_SEPARATOR: char = '\x1e';

/// CONVNBEST の応答を作る。候補ごとに文節の「読み\x1f表記」を \x1e でつなぐ
fn nbest_response(kana: &str, ctx: Option<&convert::Context>, data: &EngineData) -> String {
    let user = data.user.lock().expect("user lock");
    let learning = data.learning.lock().expect("learning lock");
    let candidates = convert::convert_nbest(
        kana,
        ctx,
        &data.dictionary,
        &user,
        &data.matrix,
        &data.functional,
        &learning,
    );
    format!("OK\t{}\n", format_candidates(&candidates))
}

/// 入力全体の候補を CONVNBEST の応答の形 (候補はタブ区切り、文節は \x1e 区切り) にする
fn format_candidates(candidates: &[convert::SentenceCandidate]) -> String {
    candidates
        .iter()
        .map(|c| {
            c.segments
                .iter()
                .map(|(reading, surface)| format!("{reading}{FIELD_SEPARATOR}{surface}"))
                .collect::<Vec<_>>()
                .join(&SEGMENT_SEPARATOR.to_string())
        })
        .collect::<Vec<_>>()
        .join("\t")
}

/// 1行の要求を解釈して1行の応答を作る
fn handle_request(line: &str, data: &EngineData) -> String {
    // 日付・時刻の動的候補用の現在日時 (CONVSYM の候補生成と LEARN の除外判定に使う)
    let now = chrono::Local::now().naive_local();

    let mut fields = line.split('\t');
    match fields.next() {
        Some("CONVERT") => match fields.next() {
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let candidates = convert::candidates(
                    kana, &data.dictionary, &user, &data.matrix, &data.functional);
                format!("OK\t{}\n", candidates.join("\t"))
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVSEG") => match fields.next() {
            Some(kana) if !kana.is_empty() => segments_response(kana, fields.next(), None, data),
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("CONVCTX") => {
            // CONVCTX\t<文脈読み>\x1f<文脈表記>\t<かな>[\t<文節長>] :
            // 前文脈 (直前確定の文節) 付きの文節変換。応答は CONVSEG と同一形式
            let ctx = fields.next().and_then(parse_context);
            match fields.next() {
                Some(kana) if !kana.is_empty() => {
                    segments_response(kana, fields.next(), ctx.as_ref(), data)
                }
                _ => "ERR\tかなが空です\n".to_string(),
            }
        }
        Some("CONVNBEST") => {
            // CONVNBEST\t<文脈読み>\x1f<文脈表記>\t<かな> : 入力全体の候補を返す
            let ctx = fields.next().and_then(parse_context);
            match fields.next() {
                Some(kana) if !kana.is_empty() => nbest_response(kana, ctx.as_ref(), data),
                _ => "ERR\tかなが空です\n".to_string(),
            }
        }
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
            // (短縮よみ → 名詞系 → インポート辞書の名詞系) のみを返す。該当なしは候補ゼロの OK
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let mut candidates: Vec<String> =
                    user.lookup_shortcuts(kana).into_iter().map(String::from).collect();
                for word in user.lookup_words(kana) {
                    if !candidates.iter().any(|s| *s == word.surface) {
                        candidates.push(word.surface.clone());
                    }
                }
                for entry in user.imported_words(kana) {
                    if !candidates.iter().any(|s| *s == entry.surface) {
                        candidates.push(entry.surface.clone());
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
        Some("LEARN2") => {
            // LEARN2\t読み\x1f表記\x1f文脈\t... : 前文脈付きの確定学習。
            // 文脈 (直前文節の表記) が空の文節は従来の学習のみ記録する
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for entry in fields {
                let mut parts = entry.splitn(3, FIELD_SEPARATOR);
                let (Some(reading), Some(surface)) = (parts.next(), parts.next()) else {
                    continue;
                };
                if reading.is_empty() || surface.is_empty() {
                    continue;
                }
                let context = parts.next().unwrap_or("");
                // 日付・時刻の動的候補は時間が経つと古くなるため学習しない (LEARN と同様)
                if !datetime::candidates_at(reading, now).iter().any(|c| c == surface) {
                    learning.record(reading, surface);
                    if !context.is_empty() {
                        learning.record_ctx(context, reading, surface);
                    }
                }
                count += 1;
            }
            if count > 0 {
                "OK\n".to_string()
            } else {
                "ERR\t記録する内容がありません\n".to_string()
            }
        }
        Some("LEARNSEG") => {
            // LEARNSEG\t読み\t読み... : 文節境界の学習。人が文節伸縮で分割を直して
            // 確定したときだけ送られるため、届いた文節読みはそのまま境界として信頼する
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for reading in fields {
                if reading.is_empty() {
                    continue;
                }
                learning.record_boundary(reading);
                count += 1;
            }
            if count > 0 {
                "OK\n".to_string()
            } else {
                "ERR\t記録する内容がありません\n".to_string()
            }
        }
        Some("LEARNWORD") => {
            // LEARNWORD\t読み\x1f表記\t... : 分割して確定した複合語を1語として学習する。
            // 辞書に無い語を文節伸縮で割って入力したときだけ送られるため、そのまま
            // ユーザ登録語と同じ重みでラティスに載せる
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let mut user = data.user.lock().expect("user lock");
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for pair in fields {
                let Some((reading, surface)) = pair.split_once(FIELD_SEPARATOR) else {
                    continue;
                };
                // 日付・時刻の動的候補は時間が経つと古くなるため学習しない (LEARN と同様)
                if !datetime::candidates_at(reading, now).iter().any(|c| c == surface) {
                    // ラティスへ載せるのは辞書語で経路が作れない読みだけ (コストが高いため)。
                    // 経路が作れる読みは「読み → 表記」の学習で候補の先頭に来る
                    user.learn_word(reading, surface, &data.functional);
                    learning.record(reading, surface);
                }
                count += 1;
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
            // ユーザ辞書ファイルとインポート辞書を読み直す (手動編集・インポートの反映用)
            let mut user = data.user.lock().expect("user lock");
            user.reload(&data.functional);
            eprintln!(
                "ユーザ辞書を再読込しました: 短縮よみ {} 件・単語 {} 件・複合語 {} 件・インポート辞書 {} 件",
                user.shortcut_count(),
                user.word_count(),
                user.learned_count(),
                user.imported_count()
            );
            "OK\n".to_string()
        }
        Some("RELOADCONFIG") => {
            // 設定ファイルを読み直す (設定ツールの保存時に呼ばれる)
            let config = Config::load_default();
            *data.config.lock().expect("config lock") = config;
            // LLM の子プロセスは、llm 系の設定が変わったときに起動・終了する
            data.llm.configure(config.llm, config.llm_backend);
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
            llm: test_llm(),
        }
    }

    /// テスト用の LLM ワーカー (偽の子プロセス。設定で有効にするまで起動しない)
    fn test_llm() -> llm::LlmManager {
        llm::tests::fake_manager(llm::tests::FakeBehavior::Normal, std::time::Duration::ZERO).0
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
            llm: test_llm(),
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
    fn convctxはconvsegと同一形式の応答を返す() {
        // 前文脈が候補順に影響しない入力では CONVSEG と同じ応答になる
        let data = sample_data();
        let expected = handle_request("CONVSEG\tきょうは", &data);
        assert_eq!(handle_request("CONVCTX\tはれ\x1f晴れ\tきょうは", &data), expected);
        // 前文脈フィールドが空 (読み・表記なし) でも通常変換として動く
        assert_eq!(handle_request("CONVCTX\t\tきょうは", &data), expected);
        // 文節長指定も CONVSEG と同様に使える
        let expected = handle_request("CONVSEG\tはれは\t2,1", &data);
        assert_eq!(handle_request("CONVCTX\tきょう\x1f今日\tはれは\t2,1", &data), expected);
    }

    #[test]
    fn learn2の文脈学習がconvctxで最優先になる() {
        let data = sample_data();
        // 文脈「晴れ」付きで「京は」を確定 → その後、文脈なしで「キョウハ」を確定
        // (LEARN2 は文脈なし学習も併せて更新するため、この順で両者が分かれる)
        assert_eq!(handle_request("LEARN2\tきょうは\x1f京は\x1f晴れ", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうは\x1fキョウハ", &data), "OK\n");

        // 文脈が一致すれば文脈学習の「京は」が最優先
        let response = handle_request("CONVCTX\tはれ\x1f晴れ\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1f京は\x1fキョウハ\x1f"), "{response}");
        // 文脈なしなら文脈なし学習の「キョウハ」が先頭
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1fキョウハ\x1f"), "{response}");
    }

    #[test]
    fn learn2は文脈なしでも従来の学習になる() {
        let data = sample_data();
        // 文脈フィールドが空 (読み\x1f表記\x1f) は LEARN と同じ扱い
        assert_eq!(handle_request("LEARN2\tきょうは\x1fキョウハ\x1f", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1fキョウハ\x1f"), "{response}");
        // 内容が無ければエラー
        assert!(handle_request("LEARN2", &data).starts_with("ERR\t"));
        assert!(handle_request("LEARN2\t読みだけ", &data).starts_with("ERR\t"));
    }

    #[test]
    fn convctxのかなが空ならエラー() {
        let data = sample_data();
        assert!(handle_request("CONVCTX\tはれ\x1f晴れ\t", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVCTX\tはれ\x1f晴れ", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVCTX", &data).starts_with("ERR\t"));
    }

    #[test]
    fn convnbest要求に文節つきの入力全体の候補を返す() {
        // 候補はタブ区切り、候補内の文節は RS (\x1e) 区切りで「読み\x1f表記」
        let data = sample_data();
        let response = handle_request("CONVNBEST\t\tきょうははれ", &data);
        assert!(
            response.starts_with("OK\tきょうは\x1f今日は\x1eはれ\x1f晴れ\t"),
            "{response}"
        );
        assert!(
            response.ends_with("\tきょうははれ\x1fキョウハハレ\tきょうははれ\x1fきょうははれ\n"),
            "{response}"
        );
        // 前文脈が候補順に影響しない入力では、前文脈ありでも同じ応答になる
        assert_eq!(handle_request("CONVNBEST\tはれ\x1f晴れ\tきょうははれ", &data), response);
    }

    #[test]
    fn convnbestのかなが空ならエラー() {
        let data = sample_data();
        assert!(handle_request("CONVNBEST\t\t", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVNBEST\tはれ\x1f晴れ", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVNBEST", &data).starts_with("ERR\t"));
    }

    #[test]
    fn 読み全体の学習がconvnbestの先頭とpredictに出る() {
        let data = sample_data();
        // 文節ごとの学習と、読み全体 → 候補の表記 (文脈表記は空) を1回の LEARN2 で送る
        let request = "LEARN2\tきょうは\x1f京は\x1f\tはれ\x1f晴れ\x1f京は\t\
                       きょうははれ\x1f京は晴れ\x1f";
        assert_eq!(handle_request(request, &data), "OK\n");
        let response = handle_request("CONVNBEST\t\tきょうははれ", &data);
        assert!(response.starts_with("OK\tきょうははれ\x1f京は晴れ\t"), "{response}");
        let response = handle_request("PREDICT\tきょうは", &data);
        assert!(response.contains("きょうははれ\x1f京は晴れ"), "{response}");
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
    fn convuserにインポート辞書の語も入る() {
        let data = sample_data();
        load_user(&data, "きょう\t匡\t名\n");
        data.user.lock().unwrap().load_imported_from(
            ["きょう\t匡\t人名\nきょう\t杏\t名\nきょう\t(^^)\t短縮よみ\n".as_bytes()],
            &data.functional,
        );
        // 短縮よみ → 手動登録の名詞系 → インポート辞書の名詞系。手動登録と同じ語は1回だけ
        assert_eq!(handle_request("CONVUSER\tきょう", &data), "OK\t(^^)\t匡\t杏\n");
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
    fn learnsegで文節境界を記録する() {
        let data = sample_data();
        assert_eq!(handle_request("LEARNSEG\tきょうは\tはれ", &data), "OK\n");
        let learning = data.learning.lock().unwrap();
        assert!(learning.is_boundary("きょうは"));
        assert!(learning.is_boundary("はれ"));
    }

    #[test]
    fn learnwordで複合語を記録する() {
        let data = sample_data();
        let request = format!("LEARNWORD\tけいしょうか{FIELD_SEPARATOR}形象化");
        assert_eq!(handle_request(&request, &data), "OK\n");
        let user = data.user.lock().unwrap();
        assert_eq!(user.lookup_words("けいしょうか")[0].surface, "形象化");
    }

    #[test]
    fn learnwordの内容が空ならエラー() {
        let data = empty_data();
        assert!(handle_request("LEARNWORD", &data).starts_with("ERR\t"));
        assert!(handle_request("LEARNWORD\t読みだけ", &data).starts_with("ERR\t"));
    }

    #[test]
    fn 学習無効ならlearnwordは記録しない() {
        let data = empty_data();
        data.config.lock().unwrap().learning = false;
        let request = format!("LEARNWORD\tけいしょうか{FIELD_SEPARATOR}形象化");
        assert_eq!(handle_request(&request, &data), "OK\n");
        assert!(data.user.lock().unwrap().lookup_words("けいしょうか").is_empty());
    }

    #[test]
    fn learnsegの内容が空ならエラー() {
        let data = empty_data();
        assert!(handle_request("LEARNSEG", &data).starts_with("ERR\t"));
        assert!(handle_request("LEARNSEG\t", &data).starts_with("ERR\t"));
    }

    #[test]
    fn 学習無効ならlearnsegは記録しない() {
        let data = empty_data();
        data.config.lock().unwrap().learning = false;
        assert_eq!(handle_request("LEARNSEG\tきょうは", &data), "OK\n");
        assert!(!data.learning.lock().unwrap().is_boundary("きょうは"));
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
    fn llm無効ならrerankはid0を返す() {
        let data = sample_data();
        let mut session = Session::new();
        assert_eq!(handle_line("RERANK\t\t\tきょうははれ", &data, &mut session), "OK\t0\n");
        assert_eq!(handle_line("RERANKGET\t0", &data, &mut session), "OK\tNONE\n");
        assert!(handle_line("RERANK\t\t\t", &data, &mut session).starts_with("ERR\t"));
        assert!(handle_line("RERANKGET\tabc", &data, &mut session).starts_with("ERR\t"));
    }

    #[test]
    fn rerankの結果はconvnbestと同じ形式で返る() {
        let data = sample_data();
        data.config.lock().unwrap().llm = true;
        data.llm.configure(true, config::LlmBackend::Cpu);
        llm::tests::wait_ready(&data.llm);
        let mut session = Session::new();
        // 読み込み済みなので ID を振る (接続ごとに 1 から)
        assert_eq!(handle_line("RERANK\t熱が出て\t\tきょうははれ", &data, &mut session), "OK\t1\n");
        let started = Instant::now();
        let response = loop {
            let response = handle_line("RERANKGET\t1", &data, &mut session);
            if response != "OK\tPENDING\n" {
                break response;
            }
            assert!(started.elapsed().as_secs() < 5, "DONE にならない");
            thread::sleep(std::time::Duration::from_millis(5));
        };
        // 偽の子プロセスは後ろの候補ほど高い尤度を返すので、並びが変わる (候補の集合は同じ)
        let nbest = handle_request("CONVNBEST\t\tきょうははれ", &data);
        let body = response.strip_prefix("OK\tDONE\t").expect("DONE の応答");
        let mut got: Vec<&str> = body.trim_end().split('\t').collect();
        let mut expected: Vec<&str> =
            nbest.strip_prefix("OK\t").unwrap().trim_end().split('\t').collect();
        assert_ne!(got, expected);
        got.sort();
        expected.sort();
        assert_eq!(got, expected);
        // 古い ID・別の接続の ID は NONE
        assert_eq!(handle_line("RERANK\t\t\tきょうは", &data, &mut session), "OK\t2\n");
        assert_eq!(handle_line("RERANKGET\t1", &data, &mut session), "OK\tNONE\n");
        let mut other = Session::new();
        assert_eq!(handle_line("RERANKGET\t2", &data, &mut other), "OK\tNONE\n");
    }

    #[test]
    fn reloadconfigはokを返す() {
        assert_eq!(handle_request("RELOADCONFIG", &sample_data()), "OK\n");
    }
}
