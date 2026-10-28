// QuicklIME 変換エンジン
//
// named pipe (\\.\pipe\quicklime-engine) で待ち受け、TSF 層からの
// 変換要求に候補リストを返す常駐サーバ。
// プロトコルの詳細は docs/protocol.md を参照。

mod config;
mod convert;
mod datetime;
mod dict;
mod english;
// 取り込み処理は単語登録ツールが使う。エンジン本体では単体テストのためだけに含める
#[cfg(test)]
mod import;
mod learn;
mod llm;
mod matrix;
mod number;
mod package_acl;
mod pos;
mod predict;
mod reverse;
mod suggest;
mod typo;
mod userdict;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Instant;

use interprocess::local_socket::traits::{ListenerExt, Stream as _, StreamCommon as _};
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, ToNsName};
use interprocess::os::windows::local_socket::ListenerOptionsExt;

use config::Config;
use dict::{Dictionary, SurfaceIndex};
use english::EnglishDict;
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
    /// dictionary の逆引きの索引。起動を待たせないよう待ち受けを始めてから別スレッドで作るので、
    /// できるまでは空 (spawn_surface_index_build)
    surface_index: OnceLock<SurfaceIndex>,
    /// 後続語の統計の表 (next_word.tsv)。起動を待たせないよう待ち受けを始めてから別スレッドで
    /// 読み込むので、読み終えるまでとファイルが無いときは空 (spawn_next_word_load)
    next_word: OnceLock<predict::NextWordTable>,
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
    /// 英単語辞書 (ASCIISTART 用。ユーザ辞書の英字の語は user が持つ。
    /// 大文字小文字の補正に使う表記は user にも写してある)
    english: EnglishDict,
    /// これより短い読みの RERANK は並べ替えない (起動時に決まる)
    rerank_min_chars: usize,
}

/// 二重起動の防止に使う named mutex の名前。パイプ名ごとに分け、隔離したエンジンが
/// 常用のエンジンと並んで動けるようにする (mutex の名前に `\` は使えない)
fn engine_mutex_name(pipe: &str) -> String {
    format!("Local\\quicklime-engine-{}", pipe.replace('\\', "_"))
}

/// 同じパイプ名の named mutex を作り、ほかのエンジンがすでに作っていれば false を返す。
/// 辞書の読み込み中はパイプが無いので、パイプへの接続だけでは同時に起動されたエンジンを
/// 見分けられない。mutex はプロセスの終了まで持ち続ける (ハンドルは閉じず、OS が解放する)。
/// mutex を作れなかったときは、パイプへの接続の判定だけに任せて起動を続ける
fn acquire_engine_mutex(pipe: &str) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = engine_mutex_name(pipe)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        handle.is_null() || GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// パイプのセキュリティ記述子の SDDL。管理者で起動されたエンジンのパイプにも通常権限のアプリが
/// 接続できるよう、DACL を SYSTEM・Administrators・現在のユーザに限る。AppContainer のアプリ
/// (スタートメニューの検索欄など) の TSF 層も開けるよう、ALL APPLICATION PACKAGES (AC) を足して
/// 整合性ラベルを Low にする。AppContainer のアクセスチェックはユーザの判定も通る必要があるので、
/// ほかのユーザの AppContainer のプロセスは開けない。現在のユーザを表す SDDL の別名は無いので SID を埋め込む
fn pipe_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{user_sid})(A;;GA;;;AC)S:(ML;;NW;;;LW)")
}

/// 現在のプロセスの token のユーザの SID を文字列 (`S-1-...`) で返す。
/// 昇格した token でもユーザの SID は通常権限のときと同じ
fn current_user_sid() -> std::io::Result<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = (|| {
            let mut size = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
            if size == 0 {
                return Err(std::io::Error::last_os_error());
            }
            // TOKEN_USER はポインタを含むので、u8 ではなく u64 の配列で確保して整列を保つ
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            if GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), size, &mut size) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            let mut wide: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut wide) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let sid = widestring::U16CStr::from_ptr_str(wide).to_string();
            LocalFree(wide.cast());
            sid.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })();
        CloseHandle(token);
        result
    }
}

/// プロセス pid が AppContainer で動いているか。判定できなければ false。通常権限のエンジンは
/// 管理者で動くクライアントのトークンを開けないので、拒否する側に倒すと管理者で起動したツールが
/// 使えなくなる (同じユーザの AppContainer のプロセスのトークンは開ける)
fn is_app_container_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenIsAppContainer};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut token: HANDLE = std::ptr::null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0;
        CloseHandle(process);
        if !opened {
            return false;
        }
        let mut value = 0u32;
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenIsAppContainer,
            (&raw mut value).cast(),
            size_of::<u32>() as u32,
            &mut size,
        ) != 0;
        CloseHandle(token);
        ok && value != 0
    }
}

/// パイプに付けるセキュリティ記述子 (pipe_sddl) を作る
fn pipe_security_descriptor()
-> std::io::Result<interprocess::os::windows::security_descriptor::SecurityDescriptor> {
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    let sddl = widestring::U16CString::from_str(pipe_sddl(&current_user_sid()?))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    SecurityDescriptor::deserialize(&sddl)
}

fn main() -> std::io::Result<()> {
    let pipe = pipe_name();

    if !acquire_engine_mutex(&pipe) {
        eprintln!("ほかのエンジンが起動中か動作中のため終了します");
        return Ok(());
    }

    // 既に別のエンジンが同じパイプで待機していれば二重起動しない
    // (TSF 側の自動起動が複数アプリから同時に走った場合の保険)
    if Stream::connect(pipe.clone().to_ns_name::<GenericNamespaced>()?).is_ok() {
        eprintln!("既にエンジンが起動しているため終了します");
        return Ok(());
    }

    spawn_grant_app_packages_read();

    // ユーザ辞書の品詞名解決に品詞ID表を使うため、先に読み込んでおく
    let functional = load_functional_ids();
    let english = load_english();
    let mut user = UserDict::load_default(&functional, learn::learned_word_path());
    user.set_name_forms(english.forms().clone());
    let config = Config::load_default();
    let dictionary = load_dictionary();
    let matrix = load_matrix();
    // 学習の取り込みと読み込みで、文節を辞書の語に分けるため辞書の後に読む
    let learning = LearningStore::load_default(&|reading, surface| {
        convert::split_words(reading, surface, &dictionary, &user, &matrix, &functional)
    });
    let data = Arc::new(EngineData {
        dictionary,
        surface_index: OnceLock::new(),
        next_word: OnceLock::new(),
        matrix,
        functional,
        user: Mutex::new(user),
        learning: Mutex::new(learning),
        config: Mutex::new(config),
        llm: llm::LlmManager::new(Box::new(llm::ProcessLauncher)),
        english,
        rerank_min_chars: llm::rerank_min_chars(),
    });
    data.llm.configure(config.llm, config.llm_backend);

    let name = pipe.clone().to_ns_name::<GenericNamespaced>()?;
    let mut options = ListenerOptions::new().name(name);
    // 記述子を作れなくても、エンジンが起動しないよりは既定の記述子でパイプを作るほうがよい
    match pipe_security_descriptor() {
        Ok(sd) => options = options.security_descriptor(sd),
        Err(e) => eprintln!("パイプのアクセス権を作れなかったため既定の記述子で作ります: {e}"),
    }
    let listener = options.create_sync()?;
    eprintln!("quicklime-engine: \\\\.\\pipe\\{pipe} で待機中");
    spawn_surface_index_build(&data);
    if let Some(path) = dictionary_dir().and_then(|dir| next_word_path(&dir)) {
        spawn_next_word_load(&data, path);
    }

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

/// AppContainer で動く TSF 層が設定ファイルを読めるよう、config.tsv・romaji.tsv・themes に
/// ALL APPLICATION PACKAGES の読み取りの ACE を足す。学習やユーザ辞書 (入力の履歴) には足さない。
/// 足せなくても AppContainer のアプリが既定値で動くだけなので、エラーを出して続ける
fn spawn_grant_app_packages_read() -> thread::JoinHandle<()> {
    thread::spawn(|| {
        for path in config::tsf_read_paths() {
            if !path.exists() {
                continue;
            }
            if let Err(e) = package_acl::grant_app_packages_read(&path) {
                eprintln!(
                    "AppContainer のアプリに読み取りを許せませんでした ({e}) [{}]",
                    path.display()
                );
            }
        }
    })
}

/// 辞書の逆引きの索引を別スレッドで作り、data.surface_index に入れる
fn spawn_surface_index_build(data: &Arc<EngineData>) -> thread::JoinHandle<()> {
    let data = Arc::clone(data);
    thread::spawn(move || {
        let started = Instant::now();
        let index = SurfaceIndex::build(&data.dictionary);
        eprintln!(
            "逆引きの索引を作りました: {} エントリ ({:.1}秒)",
            index.len(),
            started.elapsed().as_secs_f64()
        );
        let _ = data.surface_index.set(index);
    })
}

/// 後続語の統計の表を別スレッドで読み込み、data.next_word に入れる
fn spawn_next_word_load(data: &Arc<EngineData>, path: PathBuf) -> thread::JoinHandle<()> {
    let data = Arc::clone(data);
    thread::spawn(move || {
        let started = Instant::now();
        match predict::NextWordTable::load(&path) {
            Ok(table) => {
                eprintln!(
                    "後続語の統計を読み込みました: {} 件 ({:.2}秒) [{}]",
                    table.len(),
                    started.elapsed().as_secs_f64(),
                    path.display()
                );
                let _ = data.next_word.set(table);
            }
            Err(e) => eprintln!("後続語の統計の読み込みに失敗しました ({e})。統計なしで動作します"),
        }
    })
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
    let extra: Vec<PathBuf> = ut_dictionary_path(&dir).into_iter().collect();
    let started = Instant::now();
    let mut dict = match Dictionary::load(&dir, &extra) {
        Ok(dict) => {
            let extra_list: String =
                extra.iter().map(|p| format!(" + [{}]", p.display())).collect();
            eprintln!(
                "辞書を読み込みました: {} エントリ ({:.1}秒) [{}]{extra_list}",
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
    load_single_kanji(&mut dict, &dir);
    dict
}

/// UT 辞書 (dictionary_ut.txt) の場所。辞書ディレクトリ直下 (インストール先のレイアウト) を
/// 優先し、無ければプロジェクトルートの references/mozcdic-ut/
/// (exe が engine/target/{debug,release}/ にある前提で相対解決)。どちらにも無ければ None。
/// QUICKLIME_DICT_DIR の指定は「この辞書一式で動かす」意味なので、そのときはプロジェクトルートを
/// 見ない (隔離起動のテストに本物の UT 辞書が混ざると候補も読み込み時間も変わる)
fn ut_dictionary_path(dictionary_dir: &Path) -> Option<PathBuf> {
    let bundled = dictionary_dir.join("dictionary_ut.txt");
    if bundled.is_file() {
        return Some(bundled);
    }
    if std::env::var_os("QUICKLIME_DICT_DIR").is_some() {
        eprintln!("UT 辞書 (dictionary_ut.txt) がありません。Mozc 辞書だけで動作します");
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let project_root = exe.parent()?.parent()?.parent()?.parent()?;
    let dev = project_root.join("references/mozcdic-ut/dictionary_ut.txt");
    if dev.is_file() {
        return Some(dev);
    }
    eprintln!("UT 辞書 (dictionary_ut.txt) がありません。Mozc 辞書だけで動作します");
    None
}

/// 後続語の統計の表 (next_word.tsv) の場所。UT 辞書と同じく、辞書ディレクトリ直下を優先し、無ければ
/// プロジェクトルートの references/jawiki/ (exe が engine/target/{debug,release}/ にある前提で相対解決)。
/// QUICKLIME_DICT_DIR を指定したときはプロジェクトルートを見ない。どちらにも無ければ None
fn next_word_path(dictionary_dir: &Path) -> Option<PathBuf> {
    let bundled = dictionary_dir.join("next_word.tsv");
    if bundled.is_file() {
        return Some(bundled);
    }
    if std::env::var_os("QUICKLIME_DICT_DIR").is_none() {
        let exe = std::env::current_exe().ok()?;
        let project_root = exe.parent()?.parent()?.parent()?.parent()?;
        let dev = project_root.join("references/jawiki/next_word.tsv");
        if dev.is_file() {
            return Some(dev);
        }
    }
    eprintln!("後続語の統計 (next_word.tsv) がありません。統計なしで動作します");
    None
}

/// 単漢字辞書 (Mozc の single_kanji.tsv) を読み込む。無くても単漢字候補なしで続行する。
/// 辞書ディレクトリ直下の single_kanji.tsv を優先し、無ければ Mozc の配置
/// (dictionary_oss と並ぶ single_kanji/) から探す
fn load_single_kanji(dict: &mut Dictionary, dictionary_dir: &Path) {
    let candidates = [
        dictionary_dir.join("single_kanji.tsv"),
        dictionary_dir.parent().map(|p| p.join("single_kanji/single_kanji.tsv")).unwrap_or_default(),
    ];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        eprintln!("単漢字辞書 (single_kanji.tsv) がありません。単漢字候補なしで動作します");
        return;
    };
    match dict.load_single_kanji(path) {
        Ok(()) => eprintln!(
            "単漢字辞書を読み込みました: {} エントリ [{}]",
            dict.single_kanji_count(),
            path.display()
        ),
        Err(e) => {
            eprintln!("単漢字辞書の読み込みに失敗しました ({e})。単漢字候補なしで動作します")
        }
    }
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

/// 英単語辞書のディレクトリ。exe と同じディレクトリの dict\ に英単語辞書があればそこ
/// (インストール先のレイアウト)、無ければプロジェクトルートの data/
/// (exe が engine/target/{debug,release}/ にある前提で相対解決)。
/// QUICKLIME_DICT_DIR は Mozc 辞書の置き場所を指すので見ない
fn english_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bundled = exe.parent()?.join("dict");
    if bundled.join(english::WORD_FILES[0]).is_file() {
        return Some(bundled);
    }
    let project_root = exe.parent()?.parent()?.parent()?.parent()?;
    Some(project_root.join("data"))
}

/// 英単語辞書を読み込む。無くても起動は続ける (ASCIISTART はユーザ辞書の英字の語だけで照合し、
/// 一致が無ければ NONE を返す。ASCIIWORD は前方一致ありを返す)
fn load_english() -> EnglishDict {
    let Some(dir) = english_dir() else {
        eprintln!("英単語辞書のディレクトリを特定できません。英単語辞書なしで起動します");
        return EnglishDict::empty();
    };
    let (dict, count) = EnglishDict::load_dir(&dir);
    if count == 0 {
        eprintln!("英単語辞書がありません ({})。英単語辞書なしで起動します", dir.display());
    } else {
        eprintln!("英単語辞書を読み込みました: {count} 語 [{}]", dir.display());
    }
    dict
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
    /// クライアントが AppContainer のプロセスか (is_app_container_process)
    app_container: bool,
    /// 最後に受け付けた RERANK の ID
    last_id: u64,
    /// 最後に受け付けた RERANK の要求に kinds があったか (RERANKGET の DONE の候補に種類を付けるか)
    last_kinds: bool,
}

impl Session {
    fn new() -> Self {
        static NEXT_CONN: AtomicU64 = AtomicU64::new(1);
        Session {
            conn: NEXT_CONN.fetch_add(1, Ordering::Relaxed),
            app_container: false,
            last_id: 0,
            last_kinds: false,
        }
    }
}

/// 1つのクライアント接続を処理する。切断されるまで要求に応答し続ける
fn handle_client(stream: Stream, data: &EngineData) {
    let app_container =
        stream.peer_creds().ok().and_then(|creds| creds.pid()).is_some_and(is_app_container_process);
    let (recv, mut send) = stream.split();
    let reader = BufReader::new(recv);
    let mut session = Session { app_container, ..Session::new() };

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
    let command = fields.next();
    // 単語登録ツール・設定ツール用の要求で、TSF 層は送らない。サンドボックス化されたアプリに
    // ユーザ辞書や設定の読み直しをさせない
    if session.app_container && matches!(command, Some("ADDWORD" | "RELOADUSER" | "RELOADCONFIG")) {
        return "ERR\tAppContainer のアプリからは使えません\n".to_string();
    }
    match command {
        Some("RERANK") => {
            // RERANK\t<LLM 文脈>\t<文脈読み>\x1f<文脈表記>\t<かな>[\tkinds[\t<打鍵>]] : LLM による
            // 並べ替えを依頼する。LLM が無効・使えない (読み込み中を含む)・読みが短いときは ID 0 (並べ替えをしない)
            let llm_context = fields.next().unwrap_or("");
            let ctx = fields.next().and_then(parse_context);
            let Some(kana) = fields.next().filter(|k| !k.is_empty()) else {
                return "ERR\tかなが空です\n".to_string();
            };
            let (kinds, keys) = match parse_kinds_and_keys(&mut fields, kana) {
                Ok(parsed) => parsed,
                Err(e) => return format!("ERR\t{e}\n"),
            };
            let cfg = *data.config.lock().expect("config lock");
            if !cfg.llm || kana.chars().count() < data.rerank_min_chars {
                return "OK\t0\n".to_string();
            }
            let candidates = {
                let user = data.user.lock().expect("user lock");
                let learning = data.learning.lock().expect("learning lock");
                convert::convert_nbest_rerank(
                    kana,
                    ctx.as_ref(),
                    &data.dictionary,
                    &user,
                    &data.matrix,
                    &data.functional,
                    &learning,
                    typo_source(cfg.typo_correction, keys.as_ref()),
                    &typo::TUNING,
                )
                .candidates
            };
            let job = llm::Job {
                id: session.last_id + 1,
                context: llm::preprocess_context(llm_context),
                reading: kana.to_string(),
                candidates,
                segment_context: ctx,
            };
            if !data.llm.request(session.conn, job) {
                return "OK\t0\n".to_string();
            }
            session.last_id += 1;
            session.last_kinds = kinds;
            format!("OK\t{}\n", session.last_id)
        }
        Some("RERANKGET") => {
            // RERANKGET\t<ID> : 並べ替えの結果を問い合わせる
            let Some(id) = fields.next().and_then(|f| f.parse::<u64>().ok()) else {
                return "ERR\tID が不正です\n".to_string();
            };
            match data.llm.poll(session.conn, id) {
                llm::Poll::Pending => "OK\tPENDING\n".to_string(),
                // 結果が返るのは最後に受け付けた ID だけなので、種類の有無はその要求に従う
                llm::Poll::Done(mut reranked) => {
                    decide_generated_segments(&mut reranked, data);
                    format!("OK\tDONE\t{}\n", format_candidates(&reranked.candidates, session.last_kinds))
                }
                llm::Poll::None => "OK\tNONE\n".to_string(),
            }
        }
        _ => handle_request(line, data),
    }
}

/// 生成して新しく入れた候補の文節を、依頼の前文脈と読みの完全一致の制約つき Viterbi の経路で決める。
/// 経路が無ければ (数の書き方など、辞書の語でたどれない表記) 読み全体の 1 文節のまま
fn decide_generated_segments(reranked: &mut llm::Reranked, data: &EngineData) {
    let Some(index) = reranked.generated else {
        return;
    };
    let surface = reranked.candidates[index].surface();
    let user = data.user.lock().expect("user lock");
    let learning = data.learning.lock().expect("learning lock");
    if let Some(segments) = convert::constrained_path(
        &reranked.reading,
        reranked.segment_context.as_ref(),
        &data.dictionary,
        &user,
        &data.matrix,
        &data.functional,
        &learning,
        surface.as_bytes(),
        true,
    ) {
        reranked.candidates[index].segments = segments;
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
/// lengths_field は文節長 (カンマ区切り、文節伸縮時の境界固定用)、無ければ通常の文節分割。
/// lengths_field が kinds なら文節長の指定なしで、補正の表記 (docs/spec/engine.md「文節 UI の補正」) を
/// 足して種類を付ける。文節長の後ろの kinds は読まない (文節長を指定した変換には補正を入れない)
fn segments_response(
    kana: &str,
    lengths_field: Option<&str>,
    ctx: Option<&convert::Context>,
    data: &EngineData,
) -> String {
    let (lengths_field, kinds) = match lengths_field {
        Some(KINDS_FIELD) => (None, true),
        other => (other, false),
    };
    let typo = kinds && data.config.lock().expect("config lock").typo_correction;
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
            typo,
        )
    };
    let body = segments
        .iter()
        .map(|s| {
            let mut fields = vec![s.reading.clone()];
            fields.extend(s.candidates.iter().map(|candidate| {
                if s.typo.as_ref() == Some(candidate) {
                    format!("{candidate}{KIND_SEPARATOR}typo")
                } else {
                    candidate.clone()
                }
            }));
            fields.join(&FIELD_SEPARATOR.to_string())
        })
        .collect::<Vec<_>>()
        .join("\t");
    format!("OK\t{body}\n")
}

/// CONVNBEST 応答で候補内の文節を区切る文字 (ASCII Record Separator)
const SEGMENT_SEPARATOR: char = '\x1e';

/// CONVNBEST・CONVSEG 応答で候補の末尾に種類を付ける区切り (ASCII Group Separator)
const KIND_SEPARATOR: char = '\x1d';

/// CONVNBEST・RERANK・CONVSEG・CONVCTX の要求の末尾に付けると、応答の候補に種類が付く
const KINDS_FIELD: &str = "kinds";

/// CONVNBEST・RERANK の要求の、かなの後ろの kinds と、その後ろの省略できる打鍵のフィールドを読む。
/// 打鍵の要素の数がかなの文字数と合わなければ Err
fn parse_kinds_and_keys<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    kana: &str,
) -> Result<(bool, Option<typo::Keys>), String> {
    if fields.next() != Some(KINDS_FIELD) {
        return Ok((false, None));
    }
    let keys = fields.next().map(|field| typo::Keys::parse(kana, field)).transpose()?;
    Ok((true, keys))
}

/// 入力全体の候補の補正の元。打鍵が無ければかなを戻した近似
fn typo_source(typo: bool, keys: Option<&typo::Keys>) -> convert::TypoSource<'_> {
    match (typo, keys) {
        (false, _) => convert::TypoSource::Off,
        (true, Some(keys)) => convert::TypoSource::Keys(keys),
        (true, None) => convert::TypoSource::Kana,
    }
}

/// CONVNBEST の応答を作る。候補ごとに文節の「読み\x1f表記」を \x1e でつなぐ。
/// kinds なら補正の候補の末尾に種類を付ける。keys は打鍵 (無ければかなを戻した近似で補正する)
fn nbest_response(
    kana: &str,
    ctx: Option<&convert::Context>,
    kinds: bool,
    keys: Option<&typo::Keys>,
    data: &EngineData,
) -> String {
    let typo = data.config.lock().expect("config lock").typo_correction;
    let user = data.user.lock().expect("user lock");
    let learning = data.learning.lock().expect("learning lock");
    let candidates = convert::convert_nbest_scored(
        kana,
        ctx,
        &data.dictionary,
        &user,
        &data.matrix,
        &data.functional,
        &learning,
        typo_source(typo, keys),
        &typo::TUNING,
    )
    .candidates;
    format!("OK\t{}\n", format_candidates(&candidates, kinds))
}

/// SUGGEST の応答を作る。候補は「種類\x1d読み\x1f表記\x1e読み\x1f表記...」のタブ区切り、
/// 候補なしは OK のみ
fn suggest_response(
    kana: &str,
    ctx: Option<&convert::Context>,
    keys: &typo::Keys,
    data: &EngineData,
) -> String {
    let cfg = *data.config.lock().expect("config lock");
    let user = data.user.lock().expect("user lock");
    let learning = data.learning.lock().expect("learning lock");
    let list = suggest::suggest(
        kana,
        ctx,
        keys,
        &data.dictionary,
        &user,
        &data.matrix,
        &data.functional,
        &learning,
        &cfg,
        &typo::TUNING,
    );
    if list.is_empty() {
        return "OK\n".to_string();
    }
    let body = list
        .iter()
        .map(|s| {
            let segments = s
                .segments
                .iter()
                .map(|(reading, surface)| format!("{reading}{FIELD_SEPARATOR}{surface}"))
                .collect::<Vec<_>>()
                .join(&SEGMENT_SEPARATOR.to_string());
            format!("{}{KIND_SEPARATOR}{segments}", s.kind.name())
        })
        .collect::<Vec<_>>()
        .join("\t");
    format!("OK\t{body}\n")
}

/// 入力全体の候補を CONVNBEST の応答の形 (候補はタブ区切り、文節は \x1e 区切り) にする。
/// kinds なら補正の候補の末尾に \x1d と種類 typo を付ける
fn format_candidates(candidates: &[convert::SentenceCandidate], kinds: bool) -> String {
    candidates
        .iter()
        .map(|c| {
            let mut field = c
                .segments
                .iter()
                .map(|(reading, surface)| format!("{reading}{FIELD_SEPARATOR}{surface}"))
                .collect::<Vec<_>>()
                .join(&SEGMENT_SEPARATOR.to_string());
            if kinds && c.typo {
                field.push(KIND_SEPARATOR);
                field.push_str("typo");
            }
            field
        })
        .collect::<Vec<_>>()
        .join("\t")
}

/// PREDICT・NEXTWORD の応答を作る (候補は「読み\x1f表記」のタブ区切り、候補なしは OK のみ)
fn predictions_response(candidates: &[(String, String)]) -> String {
    if candidates.is_empty() {
        return "OK\n".to_string();
    }
    let body = candidates
        .iter()
        .map(|(reading, surface)| format!("{reading}{FIELD_SEPARATOR}{surface}"))
        .collect::<Vec<_>>()
        .join("\t");
    format!("OK\t{body}\n")
}

/// LEARN・LEARN2 の組 (読み, 表記, 直前の文節の表記) を学習として数える
/// (docs/spec/learning.md「確定の数え方 (LEARN・LEARN2)」)。3 組以上で、末尾の組の読みと表記がそれより前の組の連結なら、
/// TSF 層が 2 文節以上の確定に足した読み全体なので、句の学習だけを数える
fn learn_entries(entries: &[(&str, &str, &str)], data: &EngineData, now: chrono::NaiveDateTime) {
    let whole = entries.len() >= 3 && {
        let (rest, last) = entries.split_at(entries.len() - 1);
        let reading: String = rest.iter().map(|e| e.0).collect();
        let surface: String = rest.iter().map(|e| e.1).collect();
        last[0].0 == reading && last[0].1 == surface
    };
    let user = data.user.lock().expect("user lock");
    let mut learning = data.learning.lock().expect("learning lock");
    for (i, &(reading, surface, context)) in entries.iter().enumerate() {
        // 日付・時刻の動的候補は時間が経つと古くなるため学習しない
        // (学習すると翌日以降も昨日の日付が先頭に来てしまう)
        if reading.is_empty()
            || surface.is_empty()
            || datetime::candidates_at(reading, now).iter().any(|c| c == surface)
        {
            continue;
        }
        let words =
            convert::split_words(reading, surface, &data.dictionary, &user, &data.matrix, &data.functional);
        if whole && i == entries.len() - 1 {
            learning.learn_phrase(reading, surface, learn::PhraseKind::Whole, words.is_some());
        } else {
            learning.learn_segment(reading, surface, context, words.as_deref());
        }
    }
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
            // CONVCTX\t<文脈読み>\x1f<文脈表記>\t<かな>[\t<文節長>][\tkinds] :
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
            // CONVNBEST\t<文脈読み>\x1f<文脈表記>\t<かな>[\tkinds[\t<打鍵>]] : 入力全体の候補を返す
            let ctx = fields.next().and_then(parse_context);
            match fields.next() {
                Some(kana) if !kana.is_empty() => match parse_kinds_and_keys(&mut fields, kana) {
                    Ok((kinds, keys)) => {
                        nbest_response(kana, ctx.as_ref(), kinds, keys.as_ref(), data)
                    }
                    Err(e) => format!("ERR\t{e}\n"),
                },
                _ => "ERR\tかなが空です\n".to_string(),
            }
        }
        Some("SUGGEST") => {
            // SUGGEST\t<文脈読み>\x1f<文脈表記>\t<かな>\t<打鍵> : 全体変換と予測を 1 つの並びで返す
            let ctx = fields.next().and_then(parse_context);
            let Some(kana) = fields.next().filter(|k| !k.is_empty()) else {
                return "ERR\tかなが空です\n".to_string();
            };
            let Some(field) = fields.next() else {
                return "ERR\t打鍵がありません\n".to_string();
            };
            match typo::Keys::parse(kana, field) {
                Ok(keys) => suggest_response(kana, ctx.as_ref(), &keys, data),
                Err(e) => format!("ERR\t{e}\n"),
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
        Some("REVERSE") => {
            // REVERSE\t<表記> : 表記を語の (読み, 表記) の列にする (逆変換。応答は PREDICT と同じ形)
            let Some(text) = fields.next().filter(|t| !t.is_empty()) else {
                return "ERR\t表記が空です\n".to_string();
            };
            let Some(index) = data.surface_index.get() else {
                return "ERR\t逆引きの索引を作っています\n".to_string();
            };
            let user = data.user.lock().expect("user lock");
            let learning = data.learning.lock().expect("learning lock");
            match reverse::reverse(
                text,
                &data.dictionary,
                index,
                &user,
                &data.matrix,
                &data.functional,
                &learning,
            ) {
                Some(words) => predictions_response(&words),
                None => "ERR\t読めない文字があります\n".to_string(),
            }
        }
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
        Some("CONVNAME") => match fields.next() {
            // 人名変換 (F3 用): 読みに完全一致する人名 (ユーザ辞書の名詞系 → インポート辞書の名詞系
            // → システム辞書のコスト順) のみを返す。変換の候補で抑えた人名も含める。該当なしは候補ゼロの OK
            Some(kana) if !kana.is_empty() => {
                let user = data.user.lock().expect("user lock");
                let is_name = |id: u16| data.functional.is_person_name(id);
                let mut system: Vec<&dict::Entry> =
                    data.dictionary.lookup(kana).iter().filter(|e| is_name(e.left_id)).collect();
                system.sort_by_key(|e| e.cost);
                let words = user.lookup_words(kana).into_iter().map(|w| (w.left_id, w.surface.as_str()));
                let imported =
                    user.imported_words(kana).into_iter().map(|e| (e.left_id, e.surface.as_str()));
                let surfaces = words
                    .chain(imported)
                    .filter(|&(id, _)| is_name(id))
                    .map(|(_, surface)| surface)
                    .chain(system.into_iter().map(|e| e.surface.as_str()));
                let mut candidates: Vec<&str> = Vec::new();
                for surface in surfaces {
                    if !candidates.contains(&surface) {
                        candidates.push(surface);
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
            // PREDICT\t<かな>[\t<文脈読み>\x1f<文脈表記>] : 前文脈があれば、その後に確定した語を上位に寄せる。
            // 最小文字数未満・該当なし・サジェスト無効時は候補ゼロの OK (エラーにしない)
            Some(kana) if !kana.is_empty() => {
                let cfg = *data.config.lock().expect("config lock");
                if !cfg.suggest {
                    return "OK\n".to_string();
                }
                let ctx = fields.next().and_then(parse_context);
                let user = data.user.lock().expect("user lock");
                let learning = data.learning.lock().expect("learning lock");
                let candidates = predict::predict(
                    kana,
                    ctx.as_ref().map(|c| c.surface.as_str()),
                    &data.dictionary,
                    &user,
                    &learning,
                    &cfg,
                );
                predictions_response(&candidates)
            }
            _ => "ERR\tかなが空です\n".to_string(),
        },
        Some("NEXTWORD") => {
            // NEXTWORD\t<文脈読み>\x1f<文脈表記> : 読みなしの後続語予測。前文脈の後に確定した
            // ことのある語を返す。該当なし・サジェスト無効時は候補ゼロの OK
            let Some(ctx) = fields.next().and_then(parse_context) else {
                return "ERR\t前文脈がありません\n".to_string();
            };
            let cfg = *data.config.lock().expect("config lock");
            if !cfg.suggest {
                return "OK\n".to_string();
            }
            let table = data.next_word.get();
            // バックオフの表は前文脈の最後の自立語で引く。表が無ければ分ける必要が無い
            let head = table.and_then(|_| {
                let user = data.user.lock().expect("user lock");
                let words = convert::split_words(
                    &ctx.reading,
                    &ctx.surface,
                    &data.dictionary,
                    &user,
                    &data.matrix,
                    &data.functional,
                )?;
                words.into_iter().last().map(|(_, surface)| surface)
            });
            let learning = data.learning.lock().expect("learning lock");
            predictions_response(&predict::next_words(&ctx.surface, head.as_deref(), &learning, table, &cfg))
        }
        Some("LEARN") => {
            // LEARN\t読み\x1f表記\t読み\x1f表記... : 文節ごとの確定結果を記録する。
            // 学習が無効なら記録せず OK を返す (既存の学習データは使い続ける)
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let entries: Vec<(&str, &str, &str)> = fields
                .filter_map(|pair| pair.split_once(FIELD_SEPARATOR))
                .map(|(reading, surface)| (reading, surface, ""))
                .collect();
            if entries.is_empty() {
                return "ERR\t記録する内容がありません\n".to_string();
            }
            learn_entries(&entries, data, now);
            "OK\n".to_string()
        }
        Some("LEARN2") => {
            // LEARN2\t読み\x1f表記\x1f文脈\t... : 前文脈付きの確定学習。
            // 文脈 (直前文節の表記) が空の文節は従来の学習のみ記録する
            if !data.config.lock().expect("config lock").learning {
                return "OK\n".to_string();
            }
            let mut entries: Vec<(&str, &str, &str)> = Vec::new();
            for entry in fields {
                let mut parts = entry.splitn(3, FIELD_SEPARATOR);
                let (Some(reading), Some(surface)) = (parts.next(), parts.next()) else {
                    continue;
                };
                if reading.is_empty() || surface.is_empty() {
                    continue;
                }
                entries.push((reading, surface, parts.next().unwrap_or("")));
            }
            if entries.is_empty() {
                return "ERR\t記録する内容がありません\n".to_string();
            }
            learn_entries(&entries, data, now);
            "OK\n".to_string()
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
                    // 複合語は高いコストでラティスに載るので、勝つのは辞書語で経路が作れない読みだけ。
                    // 経路が作れる読みは句の学習で候補の上位に来る
                    user.learn_word(reading, surface, &data.functional);
                    let words = convert::split_words(
                        reading, surface, &data.dictionary, &user, &data.matrix, &data.functional,
                    );
                    learning.learn_phrase(reading, surface, learn::PhraseKind::Segment, words.is_some());
                }
                count += 1;
            }
            if count > 0 {
                "OK\n".to_string()
            } else {
                "ERR\t記録する内容がありません\n".to_string()
            }
        }
        Some("FORGET") => {
            // FORGET\t読み\x1f表記\t... : (読み, 表記) の学習を消す。応答は消した件数。
            // 記録ではないので、学習が無効でも消す
            let mut pairs: Vec<(&str, &str)> = Vec::new();
            for pair in fields {
                match pair.split_once(FIELD_SEPARATOR) {
                    Some((reading, surface)) if !reading.is_empty() && !surface.is_empty() => {
                        pairs.push((reading, surface));
                    }
                    _ => return "ERR\t読みまたは表記がありません\n".to_string(),
                }
            }
            if pairs.is_empty() {
                return "ERR\t消す内容がありません\n".to_string();
            }
            let mut user = data.user.lock().expect("user lock");
            let mut learning = data.learning.lock().expect("learning lock");
            let mut count = 0;
            for (reading, surface) in pairs {
                // 学習した複合語を消す前に分ける (複合語も語として分けた学習を消すため)
                let words = convert::split_words(
                    reading, surface, &data.dictionary, &user, &data.matrix, &data.functional,
                );
                count += learning.forget(reading, surface, words.as_deref());
                count += user.forget_learned(reading, surface);
            }
            format!("OK\t{count}\n")
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
        Some("ASCIISTART") => {
            // ASCIISTART\t<prefix|exact>\t<打鍵列1>\x1f<打鍵列2>... : 英字区間を始める要素の
            // 位置を返す (かなのかたまりごとの打鍵列。どこでも一致しなければ NONE)
            let Some(mode) = fields.next().and_then(english::MatchMode::parse) else {
                return "ERR\t照合方法が不正です\n".to_string();
            };
            let Some(field) = fields.next().filter(|f| !f.is_empty()) else {
                return "ERR\t打鍵列が空です\n".to_string();
            };
            let elements: Vec<&str> = field.split(FIELD_SEPARATOR).collect();
            let user = data.user.lock().expect("user lock");
            let start = english::ascii_start(&elements, mode, |text, m| {
                data.english.matches(text, m) || user.english_matches(text, m)
            });
            match start {
                Some(start) => format!("OK\t{start}\n"),
                None => "OK\tNONE\n".to_string(),
            }
        }
        Some("ASCIIWORD") => {
            // ASCIIWORD\t<文字列> : 英字区間を閉じるかの判定用に、文字列全体が英単語の前方一致か
            // (1/0) と、先頭部分のうち英単語と完全一致するものの長さ (カンマ区切り) を返す
            let Some(text) = fields.next().filter(|f| !f.is_empty()) else {
                return "ERR\t文字列が空です\n".to_string();
            };
            if !english::is_ascii_word(text) {
                return "ERR\t英字以外を含みます\n".to_string();
            }
            // 辞書が無いと何も一致せず、英字区間を誤って閉じてしまうため「前方一致あり」にする
            if data.english.is_empty() {
                return "OK\t1\t\n".to_string();
            }
            let user = data.user.lock().expect("user lock");
            let (prefix, lengths) = english::ascii_word(text, |t, m| {
                data.english.matches(t, m) || user.english_matches(t, m)
            });
            let lengths: Vec<String> = lengths.iter().map(usize::to_string).collect();
            format!("OK\t{}\t{}\n", u8::from(prefix), lengths.join(","))
        }
        Some("ASCIIFORM") => {
            // ASCIIFORM\t<文字列> : 大文字小文字を補正した正しい表記を返す
            // (ユーザ辞書を英単語辞書より優先する。表記が無い・文字列と同じなら空)
            let Some(text) = fields.next().filter(|f| !f.is_empty()) else {
                return "ERR\t文字列が空です\n".to_string();
            };
            if !english::is_ascii_word(text) {
                return "ERR\t英字以外を含みます\n".to_string();
            }
            let user = data.user.lock().expect("user lock");
            let form = user.ascii_form(text).filter(|f| *f != text).unwrap_or("");
            format!("OK\t{form}\n")
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
            surface_index: OnceLock::new(),
            next_word: OnceLock::new(),
            matrix: ConnectionMatrix::empty(),
            functional: FunctionalIds::empty(),
            user: Mutex::new(UserDict::empty()),
            learning: Mutex::new(LearningStore::in_memory()),
            config: Mutex::new(Config::default()),
            llm: test_llm(),
            english: EnglishDict::empty(),
            rerank_min_chars: llm::RERANK_MIN_CHARS,
        }
    }

    #[test]
    fn 逆引きの索引は別スレッドで作り終えてから引ける() {
        let data = Arc::new(sample_data());
        assert!(data.surface_index.get().is_none());
        spawn_surface_index_build(&data).join().unwrap();
        let index = data.surface_index.get().expect("索引ができている");
        let chars: Vec<char> = "今日は晴れ".chars().collect();
        let got: Vec<(usize, String)> = index
            .common_prefix_search(&data.dictionary, &chars, chars.len())
            .into_iter()
            .map(|m| (m.len, m.reading))
            .collect();
        assert_eq!(got, vec![(2, "きょう".to_string())]);
    }

    #[test]
    fn reverseは索引ができてから表記を語の読みと表記の列にする() {
        let data = Arc::new(sample_data());
        assert!(handle_request("REVERSE\t今日は晴れ", &data).starts_with("ERR\t"));
        spawn_surface_index_build(&data).join().unwrap();
        assert_eq!(
            handle_request("REVERSE\t今日は晴れ", &data),
            "OK\tきょう\x1f今日\tは\x1fは\tはれ\x1f晴れ\n"
        );
        assert!(handle_request("REVERSE\t今日はabc", &data).starts_with("ERR\t"));
        assert!(handle_request("REVERSE\t", &data).starts_with("ERR\t"));
        assert!(handle_request("REVERSE", &data).starts_with("ERR\t"));
    }

    #[test]
    fn engine_mutex_name_uses_pipe_name() {
        assert_eq!(
            engine_mutex_name("quicklime-engine"),
            "Local\\quicklime-engine-quicklime-engine"
        );
        assert_eq!(engine_mutex_name("a\\b"), "Local\\quicklime-engine-a_b");
    }

    #[test]
    fn engine_mutex_rejects_second_acquire_in_same_name() {
        let pipe = format!("quicklime-test-mutex-{}", std::process::id());
        assert!(acquire_engine_mutex(&pipe));
        assert!(!acquire_engine_mutex(&pipe));
        assert!(acquire_engine_mutex(&format!("{pipe}-other")));
    }

    #[test]
    fn pipe_sddl_contains_user_sid_app_packages_and_low_label() {
        let sddl = pipe_sddl("S-1-5-21-1-2-3-1001");
        assert!(sddl.contains("(A;;GA;;;S-1-5-21-1-2-3-1001)"));
        assert!(sddl.contains("(A;;GA;;;AC)"));
        assert!(sddl.ends_with("S:(ML;;NW;;;LW)"));
    }

    #[test]
    fn 自分のプロセスと開けないプロセスはappcontainerではない() {
        assert!(!is_app_container_process(std::process::id()));
        // PID 0 (System Idle Process) は開けない
        assert!(!is_app_container_process(0));
    }

    #[test]
    fn appcontainerのクライアントには単語登録と再読込を許さない() {
        let data = sample_data();
        data.config.lock().unwrap().learning = false;
        let mut session = Session { app_container: true, ..Session::new() };
        let denied = "ERR\tAppContainer のアプリからは使えません\n";
        assert_eq!(handle_line("ADDWORD\tかんべ\t神戸\t姓", &data, &mut session), denied);
        assert_eq!(handle_line("RELOADUSER", &data, &mut session), denied);
        assert_eq!(handle_line("RELOADCONFIG", &data, &mut session), denied);
        // ユーザ辞書も設定も変わらない
        let response = handle_line("CONVSEG\tかんべは", &data, &mut session);
        assert!(!response.contains("神戸"), "登録されている: {response}");
        assert!(!data.config.lock().unwrap().learning);
        // TSF 層が使う要求はそのまま処理する
        assert_eq!(handle_line("LEARN\tきょうは\x1f今日は", &data, &mut session), "OK\n");
        let suggest = handle_line("SUGGEST\t\tはれ\tha\x1fre", &data, &mut session);
        assert!(suggest.starts_with("OK"), "{suggest}");
        assert_eq!(suggest, handle_request("SUGGEST\t\tはれ\tha\x1fre", &data));
    }

    #[test]
    fn 通常のクライアントには単語登録と再読込を許す() {
        let data = sample_data();
        let mut session = Session::new();
        assert!(!session.app_container);
        assert_eq!(handle_line("ADDWORD\tかんべ\t神戸\t姓", &data, &mut session), "OK\n");
        let response = handle_line("CONVSEG\tかんべは", &data, &mut session);
        assert!(response.contains("神戸は"), "神戸は が候補に無い: {response}");
        assert_eq!(handle_line("RELOADUSER", &data, &mut session), "OK\n");
        assert_eq!(handle_line("RELOADCONFIG", &data, &mut session), "OK\n");
    }

    #[test]
    fn current_user_sid_is_string_sid() {
        assert!(current_user_sid().unwrap().starts_with("S-1-"));
    }

    #[test]
    fn listener_with_pipe_security_descriptor_accepts_connection() {
        let pipe = format!("quicklime-test-sd-{}", std::process::id());
        let _listener = ListenerOptions::new()
            .name(pipe.clone().to_ns_name::<GenericNamespaced>().unwrap())
            .security_descriptor(pipe_security_descriptor().unwrap())
            .create_sync()
            .unwrap();
        assert!(Stream::connect(pipe.to_ns_name::<GenericNamespaced>().unwrap()).is_ok());
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
            surface_index: OnceLock::new(),
            next_word: OnceLock::new(),
            matrix: ConnectionMatrix::empty(),
            functional,
            user: Mutex::new(UserDict::empty()),
            learning: Mutex::new(LearningStore::in_memory()),
            config: Mutex::new(Config::default()),
            llm: test_llm(),
            english: EnglishDict::empty(),
            rerank_min_chars: llm::RERANK_MIN_CHARS,
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

    /// 学習のテスト用 (sample_data の辞書に、同音の 京 と、きょうか・きょうしつ・いい・てんき を足す)
    fn learning_data() -> EngineData {
        let mut data = sample_data();
        let mut dictionary = Dictionary::empty();
        dictionary
            .load_from(
                "きょう\t1\t1\t2000\t今日\nきょう\t1\t1\t3200\t京\nは\t2\t2\t500\tは\n\
                 はれ\t1\t1\t3000\t晴れ\nきょうか\t1\t1\t3000\t強化\nきょうしつ\t1\t1\t3000\t教室\n\
                 いい\t1\t1\t2000\tいい\nてんき\t1\t1\t3000\t天気\n"
                    .as_bytes(),
            )
            .unwrap();
        dictionary.finalize();
        data.dictionary = dictionary;
        data
    }

    #[test]
    fn learn2の文脈はconvctxの変換に効かずnextwordに効く() {
        let data = learning_data();
        // 文脈「晴れ」付きで「京は」を 1 回、文脈なしで「今日は」を 2 回確定する
        assert_eq!(handle_request("LEARN2\tきょうは\x1f京は\x1f晴れ", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうは\x1f今日は", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうは\x1f今日は", &data), "OK\n");

        // 前文脈は変換に効かない (語の学習の多い「今日は」が先頭)
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1f今日は\x1f京は\x1f"), "{response}");
        assert_eq!(handle_request("CONVCTX\tはれ\x1f晴れ\tきょうは", &data), response);
        // 後続語の予測には前文脈に続いた文節が出る
        assert_eq!(handle_request("NEXTWORD\tはれ\x1f晴れ", &data), "OK\tきょうは\x1f京は\n");
    }

    #[test]
    fn learn2の末尾の読み全体は句の学習だけを数える() {
        let data = learning_data();
        let request = "LEARN2\tきょうは\x1f京は\x1f\tはれ\x1f晴れ\x1f京は\tきょうははれ\x1f京は晴れ\x1f";
        assert_eq!(handle_request(request, &data), "OK\n");
        let learning = data.learning.lock().unwrap();
        // 確定の通し番号は文節ごとに 1 増える (読み全体では増やさない)。読み全体を語に分けて数え直さない
        assert_eq!(learning.serial(), 2);
        assert!(learning.word_degree("きょう", "京").is_some_and(|d| d < 1.5));
        assert!(learning.phrase_stat("きょうははれ", "京は晴れ").is_some_and(|p| p.1 == learn::PhraseKind::Whole));
        assert!(learning.ctx_phrase_stat("京は", "はれ", "晴れ").is_some());
        assert!(learning.ctx_phrase_stat("", "きょうは", "京は").is_none());
    }

    #[test]
    fn 学習無効なら数えないが既存の学習の割引は効く() {
        let data = learning_data();
        assert_eq!(handle_request("LEARN\tきょうは\x1f京は", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうは\x1f京は", &data), "OK\n");
        data.config.lock().unwrap().learning = false;
        for _ in 0..3 {
            assert_eq!(handle_request("LEARN\tきょうは\x1f今日は", &data), "OK\n");
        }
        assert_eq!(data.learning.lock().unwrap().serial(), 2);
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1f京は\x1f"), "{response}");
    }

    #[test]
    fn learn2は文脈なしならlearnと同じ学習になる() {
        let data = sample_data();
        // 文脈フィールドが空 (読み\x1f表記\x1f) は LEARN と同じ扱い
        assert_eq!(handle_request("LEARN2\tきょうは\x1fキョウハ\x1f", &data), "OK\n");
        let plain = sample_data();
        assert_eq!(handle_request("LEARN\tきょうは\x1fキョウハ", &plain), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert_eq!(response, handle_request("CONVSEG\tきょうは", &plain));
        // 辞書の語に分けられない表記は経路を変えず、経路の表記の次に出る
        assert!(response.starts_with("OK\tきょうは\x1f今日は\x1fキョウハ\x1f"), "{response}");
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

    /// 補正の候補のテスト用 (よろしく|お願い|します を変換できる辞書)
    fn typo_data() -> EngineData {
        let mut data = sample_data();
        let mut dictionary = Dictionary::empty();
        dictionary
            .load_from(
                "よろしく\t1\t1\t3000\tよろしく\nおねがい\t1\t1\t3000\tお願い\n\
                 ねぎ\t1\t1\t4000\tネギ\nします\t2\t2\t1000\tします\n"
                    .as_bytes(),
            )
            .unwrap();
        dictionary.finalize();
        data.dictionary = dictionary;
        data
    }

    /// 応答の候補のうち、表記 (文節の表記の連結) が surface のものの欄
    fn candidate_field<'a>(response: &'a str, surface: &str) -> Option<&'a str> {
        response.trim_end().strip_prefix("OK\t")?.split('\t').find(|field| {
            let body = field.split(KIND_SEPARATOR).next().unwrap_or("");
            let joined: String = body
                .split(SEGMENT_SEPARATOR)
                .map(|segment| segment.split_once(FIELD_SEPARATOR).map_or("", |s| s.1))
                .collect();
            joined == surface
        })
    }

    #[test]
    fn convnbestのkindsで補正の候補に種類が付く() {
        let data = typo_data();
        let response = handle_request("CONVNBEST\t\tよろしくおねぎあします\tkinds", &data);
        let field = candidate_field(&response, "よろしくお願いします")
            .unwrap_or_else(|| panic!("補正の候補が無い: {response}"));
        assert!(field.ends_with("\x1dtypo"), "{field}");
        // 種類が付くのは補正の候補だけで、文節の読みは打ったかな
        assert_eq!(response.matches(KIND_SEPARATOR).count(), 1, "{response}");
        assert!(field.starts_with("よろしく\x1fよろしく\x1e"), "{field}");
        // kinds が無ければ同じ候補を印なしで返す
        let plain = handle_request("CONVNBEST\t\tよろしくおねぎあします", &data);
        assert!(!plain.contains(KIND_SEPARATOR), "{plain}");
        assert_eq!(plain, response.replace("\x1dtypo", ""));
    }

    #[test]
    fn 補正なしのconvsegは今と同じ応答になる() {
        let data = typo_data();
        assert_eq!(
            handle_request("CONVSEG\tよろしくおねぎあします", &data),
            "OK\tよろしく\x1fよろしく\x1fヨロシク\tお\x1fお\x1fオ\tねぎ\x1fネギ\x1fねぎ\t\
             あします\x1fあします\x1fアシマス\n"
        );
        assert_eq!(
            handle_request("CONVSEG\tよろしくおねがいします", &data),
            "OK\tよろしく\x1fよろしく\x1fヨロシク\t\
             おねがいします\x1fお願いします\x1fオネガイシマス\x1fおねがいします\n"
        );
        assert_eq!(
            handle_request("CONVCTX\tはれ\x1f晴れ\tよろしくおねぎあします", &data),
            handle_request("CONVSEG\tよろしくおねぎあします", &data)
        );
        assert_eq!(
            handle_request("CONVSEG\tきょうははれ", &sample_data()),
            "OK\tきょうは\x1f今日は\x1fキョウハ\x1fきょうは\tはれ\x1f晴れ\x1fハレ\x1fはれ\n"
        );
    }

    #[test]
    fn convsegのkindsで補正の文節の先頭に種類付きの補正の表記が出る() {
        let data = typo_data();
        let response = handle_request("CONVSEG\tよろしくおねぎあします\tkinds", &data);
        assert_eq!(
            response,
            "OK\tよろしく\x1fよろしく\x1fヨロシク\t\
             おねぎあします\x1fお願いします\x1dtypo\x1fおネギあします\x1fオネギアシマス\x1f\
             おねぎあします\n"
        );
        // 前文脈付きの CONVCTX でも同じ
        let response = handle_request("CONVCTX\tはれ\x1f晴れ\tよろしくおねぎあします\tkinds", &data);
        assert_eq!(response.matches(KIND_SEPARATOR).count(), 1, "{response}");
        assert!(response.contains("\x1fお願いします\x1dtypo\x1f"), "{response}");
    }

    #[test]
    fn 打ち間違いの無い読みのconvsegはkindsの有無によらず同じ応答になる() {
        let data = typo_data();
        assert_eq!(
            handle_request("CONVSEG\tよろしくおねがいします\tkinds", &data),
            handle_request("CONVSEG\tよろしくおねがいします", &data)
        );
        let data = sample_data();
        assert_eq!(
            handle_request("CONVSEG\tきょうははれ\tkinds", &data),
            handle_request("CONVSEG\tきょうははれ", &data)
        );
    }

    #[test]
    fn 補正が無効か三文字未満の読みのconvsegには補正が入らない() {
        let data = typo_data();
        assert_eq!(
            handle_request("CONVSEG\tはr\tkinds", &data),
            handle_request("CONVSEG\tはr", &data)
        );
        data.config.lock().unwrap().typo_correction = false;
        let response = handle_request("CONVSEG\tよろしくおねぎあします\tkinds", &data);
        assert_eq!(response, handle_request("CONVSEG\tよろしくおねぎあします", &data));
        assert!(!response.contains(KIND_SEPARATOR), "{response}");
    }

    #[test]
    fn 文節長を指定したconvsegにはkindsがあっても補正が入らない() {
        let data = typo_data();
        let response = handle_request("CONVSEG\tよろしくおねぎあします\t4,7\tkinds", &data);
        assert_eq!(response, handle_request("CONVSEG\tよろしくおねぎあします\t4,7", &data));
        assert!(!response.contains(KIND_SEPARATOR), "{response}");
        assert!(!response.contains("お願いします"), "{response}");
        let response = handle_request("CONVCTX\tはれ\x1f晴れ\tよろしくおねぎあします\t4,7\tkinds", &data);
        assert!(!response.contains(KIND_SEPARATOR), "{response}");
    }

    #[test]
    fn 文節長の欄がkindsのconvsegは文節長の指定なしとして読む() {
        let data = typo_data();
        let response = handle_request("CONVSEG\tよろしくおねぎあします\tkinds", &data);
        assert!(response.starts_with("OK\t"), "{response}");
        // 種類の印を除けば、補正の経路で区切った文節長の指定なしの変換
        assert!(response.starts_with("OK\tよろしく\x1f"), "{response}");
        assert!(response.contains("\tおねぎあします\x1f"), "{response}");
        let response = handle_request("CONVSEG\tきょうははれ\tkinds", &sample_data());
        assert_eq!(response, "OK\tきょうは\x1f今日は\x1fキョウハ\x1fきょうは\tはれ\x1f晴れ\x1fハレ\x1fはれ\n");
    }

    #[test]
    fn タイプミス補正が無効ならconvnbestに補正の候補が出ない() {
        let data = typo_data();
        data.config.lock().unwrap().typo_correction = false;
        let response = handle_request("CONVNBEST\t\tよろしくおねぎあします\tkinds", &data);
        assert!(candidate_field(&response, "よろしくお願いします").is_none(), "{response}");
        assert!(!response.contains(KIND_SEPARATOR), "{response}");
    }

    /// よろしくおねぎあします の打鍵 (onegiasimasu: gi|a の入れ替え)
    const TYPO_KEYS: &str = "yo\x1fro\x1fsi\x1fku\x1fo\x1fne\x1fgi\x1fa\x1fsi\x1fma\x1fsu";

    #[test]
    fn convnbestに打鍵を付けると打鍵から補正の候補を作る() {
        let data = typo_data();
        let response =
            handle_request(&format!("CONVNBEST\t\tよろしくおねぎあします\tkinds\t{TYPO_KEYS}"), &data);
        let field = candidate_field(&response, "よろしくお願いします")
            .unwrap_or_else(|| panic!("補正の候補が無い: {response}"));
        assert!(field.ends_with("\x1dtypo"), "{field}");
        // 打鍵なしの要素 (文書から読んだかな) は標準の綴りで補うので ERR にならない
        let response = handle_request(
            "CONVNBEST\t\tよろしくおねぎあします\tkinds\tよ\x1fろ\x1fし\x1fく\x1fお\x1fね\x1fぎ\x1fあ\x1fし\x1fま\x1fす",
            &data,
        );
        assert!(candidate_field(&response, "よろしくお願いします").is_some(), "{response}");
        // 要素の数がかなの文字数と合わなければ ERR
        assert!(
            handle_request("CONVNBEST\t\tよろしくおねぎあします\tkinds\tyo\x1fro", &data)
                .starts_with("ERR\t")
        );
        let mut session = Session::new();
        assert!(
            handle_line("RERANK\t\t\tよろしくおねぎあします\tkinds\tyo\x1fro", &data, &mut session)
                .starts_with("ERR\t")
        );
    }

    #[test]
    fn suggestは種類付きの全体変換と予測を返す() {
        let data = typo_data();
        let response = handle_request(&format!("SUGGEST\t\tよろしくおねぎあします\t{TYPO_KEYS}"), &data);
        let fields: Vec<&str> = response.trim_end().strip_prefix("OK\t").unwrap().split('\t').collect();
        // 候補は「種類\x1d読み\x1f表記\x1e...」
        let typo = fields.iter().find(|f| f.starts_with("typo\x1d")).unwrap_or_else(|| panic!("{response}"));
        let body = typo.strip_prefix("typo\x1d").unwrap();
        let surface: String =
            body.split(SEGMENT_SEPARATOR).map(|s| s.split_once(FIELD_SEPARATOR).unwrap().1).collect();
        assert_eq!(surface, "よろしくお願いします");
        // CONVNBEST に打鍵を付けたときと同じ補正の候補
        let nbest =
            handle_request(&format!("CONVNBEST\t\tよろしくおねぎあします\tkinds\t{TYPO_KEYS}"), &data);
        let field = candidate_field(&nbest, "よろしくお願いします").unwrap();
        assert_eq!(field.strip_suffix("\x1dtypo").unwrap(), body);
        assert!(fields.iter().all(|f| ["whole\x1d", "typo\x1d", "predict\x1d", "predict_typo\x1d"]
            .iter()
            .any(|k| f.starts_with(k))));
    }

    #[test]
    fn suggestの予測は辞書の正しい読みで返る() {
        let data = typo_data();
        // yorosik (未完成のローマ字 k)
        let response = handle_request("SUGGEST\t\tよろし\tyo\x1fro\x1fsi\x1ek", &data);
        assert!(response.contains("predict\x1dよろしく\x1fよろしく"), "{response}");
    }

    #[test]
    fn suggestの打鍵が無いか合わなければエラー() {
        let data = typo_data();
        assert!(handle_request("SUGGEST\t\tよろし", &data).starts_with("ERR\t"));
        assert!(handle_request("SUGGEST\t\tよろし\tyo\x1fro", &data).starts_with("ERR\t"));
        assert!(handle_request("SUGGEST\t\t\t", &data).starts_with("ERR\t"));
        assert!(handle_request("SUGGEST", &data).starts_with("ERR\t"));
        // 候補が無ければ OK のみ
        assert_eq!(handle_request("SUGGEST\t\tx\tx", &empty_data()), "OK\n");
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

        // 通常の表記は学習される (分けられない表記なので経路の表記の次に出る)
        assert_eq!(handle_request("LEARN\tきょう\x1fキョウ", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょう", &data);
        assert!(response.starts_with("OK\tきょう\x1f今日\x1fキョウ\x1f"), "{response}");
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
    fn learnで記録した表記が次のconvsegで経路の表記の次に出て多く使うと先頭に来る() {
        let data = learning_data();
        assert_eq!(handle_request("LEARN\tきょうは\x1f京は", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1f今日は\x1f京は\x1f"), "{response}");
        assert_eq!(handle_request("LEARN\tきょうは\x1f京は", &data), "OK\n");
        let response = handle_request("CONVSEG\tきょうは", &data);
        assert!(response.starts_with("OK\tきょうは\x1f京は\x1f今日は\x1f"), "{response}");
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

    /// 「せいこう」に人名 5 件 (名 id 11・姓 id 12) と一般名詞 2 件 (id 1) を置いた辞書。
    /// 品詞表にはユーザ辞書の品詞「人名」の解決先 (id 13) も置く
    fn name_data() -> EngineData {
        let mut data = sample_data();
        let mut dictionary = Dictionary::empty();
        dictionary
            .load_from(
                "せいこう\t11\t11\t2000\t静香\nせいこう\t11\t11\t2100\t正孝\n\
                 せいこう\t12\t12\t2200\t正幸\nせいこう\t11\t11\t2300\t正康\n\
                 せいこう\t11\t11\t2400\t成晃\nせいこう\t1\t1\t5000\t正孔\n\
                 せいこう\t1\t1\t5100\t精工\n"
                    .as_bytes(),
            )
            .unwrap();
        dictionary.finalize();
        data.dictionary = dictionary;
        data.functional = FunctionalIds::load_from(
            "1 名詞,一般\n2 助詞,係助詞\n11 名詞,固有名詞,人名,名\n12 名詞,固有名詞,人名,姓\n\
             13 名詞,固有名詞,人名,一般\n"
                .as_bytes(),
        )
        .unwrap();
        data
    }

    #[test]
    fn convname要求に人名だけを返す() {
        let data = name_data();
        load_user(&data, "せいこう\t匡\t名\nせいこう\t精工社\t組織\nせいこう\t正孝\t姓\n");
        data.user.lock().unwrap().load_imported_from(
            ["せいこう\t杏\t人名\nせいこう\t成功\t名詞\n".as_bytes()],
            &data.functional,
        );
        // ユーザ辞書の人名 → インポート辞書の人名 → システム辞書の人名 (コスト順、変換で抑えた人名も含む)。
        // 人名でない語は出さず、先に入った表記 (正孝) は入れない
        assert_eq!(
            handle_request("CONVNAME\tせいこう", &data),
            "OK\t匡\t正孝\t杏\t静香\t正幸\t正康\t成晃\n"
        );
    }

    #[test]
    fn convnameに人名の無い読みは候補ゼロのokで空の読みはエラー() {
        let data = name_data();
        assert_eq!(handle_request("CONVNAME\tきょう", &data), "OK\n");
        assert!(handle_request("CONVNAME\t", &data).starts_with("ERR\t"));
        assert!(handle_request("CONVNAME", &data).starts_with("ERR\t"));
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
    fn 前文脈つきのpredictは文脈の句の学習を確定履歴より前に出す() {
        let data = learning_data();
        assert_eq!(handle_request("LEARN2\tきょうか\x1f強化\x1f晴れ", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        // 前文脈が無ければ確定履歴 (句の学習) の割引の大きい順
        let without = handle_request("PREDICT\tきょ", &data);
        assert_eq!(without, "OK\tきょうしつ\x1f教室\tきょうか\x1f強化\tきょう\x1f今日\tきょう\x1f京\n");
        // 前文脈フィールドが空・欠けた形でも前文脈なしと同じ
        assert_eq!(handle_request("PREDICT\tきょ\t", &data), without);
        assert_eq!(handle_request("PREDICT\tきょ\tはれ\x1f", &data), without);
        // 前文脈に続いて確定した語が確定履歴より前に出る
        assert_eq!(
            handle_request("PREDICT\tきょ\tはれ\x1f晴れ", &data),
            "OK\tきょうか\x1f強化\tきょうしつ\x1f教室\tきょう\x1f今日\tきょう\x1f京\n"
        );
    }

    #[test]
    fn nextwordは前文脈の後に確定した文節を割引の大きい順に返す() {
        let data = learning_data();
        assert_eq!(handle_request("LEARN2\tいいてんき\x1fいい天気\x1f今日は", &data), "OK\n");
        assert_eq!(handle_request("LEARN2\tはれ\x1f晴れ\x1f今日は", &data), "OK\n");
        // 同じ回数なら新しいほうが先
        assert_eq!(
            handle_request("NEXTWORD\tきょうは\x1f今日は", &data),
            "OK\tはれ\x1f晴れ\tいいてんき\x1fいい天気\n"
        );
        assert_eq!(handle_request("LEARN2\tいいてんき\x1fいい天気\x1f今日は", &data), "OK\n");
        assert_eq!(
            handle_request("NEXTWORD\tきょうは\x1f今日は", &data),
            "OK\tいいてんき\x1fいい天気\tはれ\x1f晴れ\n"
        );
        // 該当なしは候補ゼロの OK
        assert_eq!(handle_request("NEXTWORD\tあした\x1f明日", &data), "OK\n");
    }

    #[test]
    fn nextwordの前文脈が欠けていればエラー() {
        let data = sample_data();
        assert!(handle_request("NEXTWORD", &data).starts_with("ERR\t"));
        assert!(handle_request("NEXTWORD\t", &data).starts_with("ERR\t"));
        assert!(handle_request("NEXTWORD\tきょうは", &data).starts_with("ERR\t"));
        assert!(handle_request("NEXTWORD\tきょうは\x1f", &data).starts_with("ERR\t"));
    }

    #[test]
    fn サジェスト無効ならnextwordと前文脈つきpredictは候補ゼロのok() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN2\tはれ\x1f晴れ\x1f今日は", &data), "OK\n");
        data.config.lock().unwrap().suggest = false;
        assert_eq!(handle_request("NEXTWORD\tきょうは\x1f今日は", &data), "OK\n");
        assert_eq!(handle_request("PREDICT\tはれ\tきょうは\x1f今日は", &data), "OK\n");
    }

    #[test]
    fn 学習無効でもnextwordは既存の学習を使う() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN2\tはれ\x1f晴れ\x1f今日は", &data), "OK\n");
        data.config.lock().unwrap().learning = false;
        assert_eq!(handle_request("LEARN2\tあめ\x1f雨\x1f今日は", &data), "OK\n");
        assert_eq!(handle_request("NEXTWORD\tきょうは\x1f今日は", &data), "OK\tはれ\x1f晴れ\n");
    }

    const NEXT_WORD_TSV: &str = "c\t今日は\tいいてんき\tいい天気\t30\n\
                                 c\t今日は\tはれ\t晴れ\t20\n\
                                 b\t今日\tくもり\t曇り\t50\n\
                                 b\t今日\tはれ\t晴れ\t40\n";

    #[test]
    fn nextwordは統計の表を読み終えてから学習の後に文節の表とバックオフの表を並べる() {
        let data = Arc::new(sample_data());
        assert_eq!(handle_request("LEARN2\tあめ\x1f雨\x1f今日は", &data), "OK\n");
        let path = std::env::temp_dir().join(format!("quicklime-test-next-word-{}.tsv", std::process::id()));
        std::fs::write(&path, NEXT_WORD_TSV).unwrap();
        // 読み込み前は学習だけ
        assert_eq!(handle_request("NEXTWORD\tきょうは\x1f今日は", &data), "OK\tあめ\x1f雨\n");
        spawn_next_word_load(&data, path.clone()).join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(data.next_word.get().map(|t| t.len()), Some(4));
        // 前文脈 今日は を分けた最後の自立語 今日 でバックオフの表を引く (晴れ は重複なので除く)
        assert_eq!(
            handle_request("NEXTWORD\tきょうは\x1f今日は", &data),
            "OK\tあめ\x1f雨\tいいてんき\x1fいい天気\tはれ\x1f晴れ\tくもり\x1f曇り\n"
        );
        // 辞書の語に分けられない前文脈 (も が辞書に無い) はバックオフの表を引かない
        assert_eq!(handle_request("NEXTWORD\tきょうも\x1f今日も", &data), "OK\n");
    }

    #[test]
    fn nextwordの統計の表は学習無効でも使いサジェスト無効なら使わない() {
        let data = sample_data();
        let _ = data.next_word.set(predict::NextWordTable::from_text(NEXT_WORD_TSV.to_string()));
        data.config.lock().unwrap().learning = false;
        assert_eq!(
            handle_request("NEXTWORD\tきょうは\x1f今日は", &data),
            "OK\tいいてんき\x1fいい天気\tはれ\x1f晴れ\tくもり\x1f曇り\n"
        );
        data.config.lock().unwrap().suggest = false;
        assert_eq!(handle_request("NEXTWORD\tきょうは\x1f今日は", &data), "OK\n");
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
    fn forgetで学習と複合語を消して件数を返す() {
        let data = sample_data();
        // 辞書の語に分けられない表記 (句の学習だけ) と、分けられる表記 (語・文脈・句の学習)
        assert_eq!(handle_request("LEARN2\tきょうしつ\x1f教室\x1f晴れ", &data), "OK\n");
        assert_eq!(handle_request("LEARN2\tはれ\x1f晴れ\x1f今日は", &data), "OK\n");
        let request = format!("LEARNWORD\tけいしょうか{FIELD_SEPARATOR}形象化");
        assert_eq!(handle_request(&request, &data), "OK\n");
        assert_eq!(
            handle_request("PREDICT\tきょ", &data),
            "OK\tきょうしつ\x1f教室\tきょう\x1f今日\n"
        );
        // 句の学習と文脈の句の学習
        assert_eq!(handle_request("FORGET\tきょうしつ\x1f教室", &data), "OK\t2\n");
        assert_eq!(handle_request("PREDICT\tきょ", &data), "OK\tきょう\x1f今日\n");
        // 複合語は句の学習と learning_word.tsv の語の両方 (2)、晴れ は句・文脈の句・語の学習 (3) が消える
        assert_eq!(
            handle_request("FORGET\tけいしょうか\x1f形象化\tはれ\x1f晴れ", &data),
            "OK\t5\n"
        );
        assert!(data.user.lock().unwrap().lookup_words("けいしょうか").is_empty());
        // 学習でない候補 (辞書の語) は何も消えない
        assert_eq!(handle_request("FORGET\tきょう\x1f今日", &data), "OK\t0\n");
        assert_eq!(handle_request("PREDICT\tきょ", &data), "OK\tきょう\x1f今日\n");
    }

    #[test]
    fn 学習無効でもforgetは消す() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        data.config.lock().unwrap().learning = false;
        assert_eq!(handle_request("FORGET\tきょうしつ\x1f教室", &data), "OK\t1\n");
        assert_eq!(handle_request("PREDICT\tきょ", &data), "OK\tきょう\x1f今日\n");
    }

    #[test]
    fn forgetの組が無いか欠けていればエラー() {
        let data = sample_data();
        assert_eq!(handle_request("LEARN\tきょうしつ\x1f教室", &data), "OK\n");
        assert!(handle_request("FORGET", &data).starts_with("ERR\t"));
        assert!(handle_request("FORGET\t", &data).starts_with("ERR\t"));
        assert!(handle_request("FORGET\t読みだけ", &data).starts_with("ERR\t"));
        assert!(handle_request("FORGET\t\x1f教室", &data).starts_with("ERR\t"));
        // 欠けた組があれば、ほかの組も消さない
        assert!(handle_request("FORGET\tきょうしつ\x1f教室\tよみ\x1f", &data).starts_with("ERR\t"));
        assert_eq!(handle_request("FORGET\tきょうしつ\x1f教室", &data), "OK\t1\n");
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
        assert_eq!(handle_line("RERANK\t\t\tきょうははれきょうは", &data, &mut session), "OK\t0\n");
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
        assert_eq!(handle_line("RERANK\t熱が出て\t\tきょうははれきょうは", &data, &mut session), "OK\t1\n");
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
        let nbest = handle_request("CONVNBEST\t\tきょうははれきょうは", &data);
        let body = response.strip_prefix("OK\tDONE\t").expect("DONE の応答");
        let mut got: Vec<&str> = body.trim_end().split('\t').collect();
        let mut expected: Vec<&str> =
            nbest.strip_prefix("OK\t").unwrap().trim_end().split('\t').collect();
        assert_ne!(got, expected);
        got.sort();
        expected.sort();
        assert_eq!(got, expected);
        // 古い ID・別の接続の ID は NONE
        assert_eq!(handle_line("RERANK\t\t\tはれきょうははれは", &data, &mut session), "OK\t2\n");
        assert_eq!(handle_line("RERANKGET\t1", &data, &mut session), "OK\tNONE\n");
        let mut other = Session::new();
        assert_eq!(handle_line("RERANKGET\t2", &data, &mut other), "OK\tNONE\n");
    }

    #[test]
    fn 読みが短いrerankはllmが有効でもid0を返す() {
        let data = sample_data();
        data.config.lock().unwrap().llm = true;
        data.llm.configure(true, config::LlmBackend::Cpu);
        llm::tests::wait_ready(&data.llm);
        let mut session = Session::new();
        // 8 文字は並べ替えず、ID も消費しない
        assert_eq!(handle_line("RERANK\t\t\tはれきょうははれ", &data, &mut session), "OK\t0\n");
        assert_eq!(handle_line("RERANK\t\t\tはれきょうははれは", &data, &mut session), "OK\t1\n");
        // 形式の誤りは長さによらず ERR
        assert!(handle_line("RERANK\t\t\tきょう\tkinds\tkyo", &data, &mut session).starts_with("ERR\t"));
    }

    #[test]
    fn rerankの最小文字数を0にすると短い読みも並べ替える() {
        let mut data = sample_data();
        data.rerank_min_chars = 0;
        data.config.lock().unwrap().llm = true;
        data.llm.configure(true, config::LlmBackend::Cpu);
        llm::tests::wait_ready(&data.llm);
        let mut session = Session::new();
        assert_eq!(handle_line("RERANK\t\t\tきょうは", &data, &mut session), "OK\t1\n");
    }

    /// RERANK を送り、DONE になるまで RERANKGET を送り続けて DONE の候補の欄を返す
    fn rerank_done(line: &str, data: &EngineData, session: &mut Session) -> String {
        let response = handle_line(line, data, session);
        let id = response.trim_end().strip_prefix("OK\t").expect("ID の応答").to_string();
        assert_ne!(id, "0", "RERANK が受け付けられない");
        let started = Instant::now();
        loop {
            let response = handle_line(&format!("RERANKGET\t{id}"), data, session);
            if response != "OK\tPENDING\n" {
                return response.strip_prefix("OK\tDONE\t").expect("DONE の応答").to_string();
            }
            assert!(started.elapsed().as_secs() < 5, "DONE にならない");
            thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn rerankの補正の候補は種類が付き位置が変わらない() {
        let data = typo_data();
        data.config.lock().unwrap().llm = true;
        data.llm.configure(true, config::LlmBackend::Cpu);
        llm::tests::wait_ready(&data.llm);
        let mut session = Session::new();
        let nbest = handle_request("CONVNBEST\t\tよろしくおねぎあします\tkinds", &data);
        let fields: Vec<&str> = nbest.strip_prefix("OK\t").unwrap().trim_end().split('\t').collect();
        let position = fields.iter().position(|f| f.ends_with("\x1dtypo")).expect("補正の候補");

        let body = rerank_done("RERANK\t\t\tよろしくおねぎあします\tkinds", &data, &mut session);
        let reranked: Vec<&str> = body.trim_end().split('\t').collect();
        assert_ne!(reranked, fields, "並べ替えで並びが変わっていない");
        assert_eq!(reranked[position], fields[position]);
        assert_eq!(body.matches(KIND_SEPARATOR).count(), 1, "{body}");
        // kinds の無い RERANK の結果には種類を付けない
        let body = rerank_done("RERANK\t\t\tよろしくおねぎあします", &data, &mut session);
        assert!(!body.contains(KIND_SEPARATOR), "{body}");
    }

    fn generated_result(reading: &str, surface: &str) -> llm::Reranked {
        llm::Reranked {
            candidates: vec![convert::SentenceCandidate {
                segments: vec![(reading.to_string(), surface.to_string())],
                protected: false,
                user_chosen: false,
                typo: false,
            }],
            generated: Some(0),
            reading: reading.to_string(),
            segment_context: None,
        }
    }

    #[test]
    fn 生成した候補の文節は辞書でたどれれば経路の文節になる() {
        let data = sample_data();
        let mut reranked = generated_result("きょうははれきょうは", "今日は晴れ今日は");
        decide_generated_segments(&mut reranked, &data);
        let segments = &reranked.candidates[0].segments;
        assert!(segments.len() > 1, "{segments:?}");
        assert_eq!(segments.iter().map(|(r, _)| r.as_str()).collect::<String>(), "きょうははれきょうは");
        assert_eq!(reranked.candidates[0].surface(), "今日は晴れ今日は");
    }

    #[test]
    fn 生成した候補の文節は辞書でたどれなければ読み全体の1文節() {
        let data = sample_data();
        let mut reranked = generated_result("きょうははれきょうは", "京は晴れ今日は");
        decide_generated_segments(&mut reranked, &data);
        assert_eq!(
            reranked.candidates[0].segments,
            [("きょうははれきょうは".to_string(), "京は晴れ今日は".to_string())]
        );
    }

    #[test]
    fn rerankの結果に生成した候補が入る() {
        let mut data = sample_data();
        let fake = llm::tests::fake_with(
            llm::tests::FakeBehavior::Normal,
            std::time::Duration::ZERO,
            llm::Timing::default(),
        );
        *fake.generate.lock().unwrap() = llm::tests::FakeGenerate::Reply("OK\t京は晴れ今日は".to_string());
        let generate = std::sync::Arc::clone(&fake.generate);
        data.llm = fake.manager;
        data.config.lock().unwrap().llm = true;
        data.llm.configure(true, config::LlmBackend::Vulkan);
        llm::tests::wait_ready(&data.llm);
        let mut session = Session::new();
        let body = rerank_done("RERANK\t\t\tきょうははれきょうは", &data, &mut session);
        let nbest = handle_request("CONVNBEST\t\tきょうははれきょうは", &data);
        let reranked: Vec<&str> = body.trim_end().split('\t').collect();
        let fields: Vec<&str> = nbest.strip_prefix("OK\t").unwrap().trim_end().split('\t').collect();
        assert_eq!(reranked[0], "きょうははれきょうは\x1f京は晴れ今日は");
        assert_eq!(reranked.len(), fields.len() + 1);

        // 辞書でたどれる表記は経路の文節になる (N-best に無ければ新しく入る)
        let traceable = "今日は晴れ今日は";
        *generate.lock().unwrap() = llm::tests::FakeGenerate::Reply(format!("OK\t{traceable}"));
        let body = rerank_done("RERANK\t\t\tきょうははれきょうは", &data, &mut session);
        let reranked: Vec<&str> = body.trim_end().split('\t').collect();
        let surface = |field: &str| -> String {
            field.split(SEGMENT_SEPARATOR).map(|s| s.split(FIELD_SEPARATOR).nth(1).unwrap()).collect()
        };
        assert_eq!(surface(reranked[0]), traceable);
        assert!(reranked[0].contains(SEGMENT_SEPARATOR), "{body}");
        assert_eq!(reranked.iter().filter(|f| surface(f) == traceable).count(), 1);
    }

    /// ASCIISTART のテスト用の英単語辞書 (1〜2文字の語も入れて、短い英字区間を候補にしないことを確かめる)
    fn english_data() -> EngineData {
        let mut data = sample_data();
        data.english = EnglishDict::load_from(
            ["an\nant\napple\ngithub\nst\nt\nwant\nworld\n".as_bytes()],
        );
        data
    }

    #[test]
    fn asciistartのprefixは英単語の前方一致になる最初の要素の位置を返す() {
        let data = english_data();
        assert_eq!(
            handle_request("ASCIISTART\tprefix\tkyo\x1fu\x1fha\x1fgi\x1fth", &data),
            "OK\t3\n"
        );
        assert_eq!(handle_request("ASCIISTART\tprefix\two\x1frl", &data), "OK\t0\n");
        // どこでも一致しなければ NONE
        assert_eq!(
            handle_request("ASCIISTART\tprefix\tko\x1fre\x1fha\x1fxyzw", &data),
            "OK\tNONE\n"
        );
        // 完全一致でない前方一致は4文字から (app は apple の頭だが3文字)。開始位置 0 も同じ
        assert_eq!(handle_request("ASCIISTART\tprefix\ta\x1fp\x1fp", &data), "OK\tNONE\n");
        assert_eq!(handle_request("ASCIISTART\tprefix\ta\x1fp\x1fpl", &data), "OK\t0\n");
    }

    #[test]
    fn asciistartのexactは3文字以上の英単語に完全一致する最初の要素の位置を返す() {
        let data = english_data();
        assert_eq!(
            handle_request("ASCIISTART\texact\tkyo\x1fu\x1fha\x1fwa\x1fn\x1ft", &data),
            "OK\t3\n"
        );
        // nt・t、st は短すぎるので、辞書にあっても一致にしない
        assert_eq!(
            handle_request("ASCIISTART\texact\twa\x1fta\x1fsi\x1fha\x1fn\x1ft", &data),
            "OK\tNONE\n"
        );
        assert_eq!(
            handle_request("ASCIISTART\texact\tka\x1fi\x1fsya\x1fs\x1ft", &data),
            "OK\tNONE\n"
        );
        // 開始位置 0 も3文字未満は一致にしない (an は辞書にある)
        assert_eq!(handle_request("ASCIISTART\texact\ta\x1fn", &data), "OK\tNONE\n");
        assert_eq!(handle_request("ASCIISTART\texact\ta\x1fn\x1ft", &data), "OK\t0\n");
        // 前方一致なら一致する打ち途中の語 (wan → want) は完全一致では一致しない
        assert_eq!(
            handle_request("ASCIISTART\texact\tkyo\x1fu\x1fha\x1fwa\x1fn", &data),
            "OK\tNONE\n"
        );
    }

    #[test]
    fn asciistartのexact_anyは長さの制限なしで完全一致する() {
        let data = english_data();
        // an は2文字なので exact では一致しないが、exact-any では一致する
        assert_eq!(handle_request("ASCIISTART\texact\ta\x1fn", &data), "OK\tNONE\n");
        assert_eq!(handle_request("ASCIISTART\texact-any\ta\x1fn", &data), "OK\t0\n");
        assert_eq!(handle_request("ASCIISTART\texact-any\tt", &data), "OK\t0\n");
        // 完全一致だけ (wan は want の頭)
        assert_eq!(handle_request("ASCIISTART\texact-any\twa\x1fn", &data), "OK\tNONE\n");
    }

    #[test]
    fn asciistartの要求が不正ならエラー() {
        let data = english_data();
        assert!(handle_request("ASCIISTART\tfuzzy\tkyo\x1fu", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIISTART\tkyo\x1fu", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIISTART\tprefix\t", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIISTART\tprefix", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIISTART", &data).starts_with("ERR\t"));
    }

    #[test]
    fn asciistartはユーザ辞書の英字の語も使う() {
        let data = sample_data();
        assert_eq!(
            handle_request("ASCIISTART\tprefix\tko\x1fre\x1fha\x1fquic", &data),
            "OK\tNONE\n"
        );
        // 表記が英字だけの語は大文字を含んでも小文字にして英単語として扱う
        assert_eq!(handle_request("ADDWORD\tくいっくらいむ\tQuicklIME\t固有名詞", &data), "OK\n");
        assert_eq!(handle_request("ASCIISTART\tprefix\tko\x1fre\x1fha\x1fquic", &data), "OK\t3\n");
        assert_eq!(
            handle_request("ASCIISTART\texact\tko\x1fre\x1fha\x1fquick\x1flime", &data),
            "OK\t3\n"
        );
        // 英字以外を含む表記は英単語にしない
        assert_eq!(handle_request("ADDWORD\tめーる\tmail@example.com\t短縮よみ", &data), "OK\n");
        assert_eq!(
            handle_request("ASCIISTART\tprefix\tko\x1fre\x1fha\x1fmail", &data),
            "OK\tNONE\n"
        );
    }

    #[test]
    fn asciiwordは前方一致と先頭部分の完全一致の長さを返す() {
        let mut data = english_data();
        data.english = EnglishDict::load_from(["git\ngithub\nmake\nmaker\npython\n".as_bytes()]);
        assert_eq!(handle_request("ASCIIWORD\tgithubd", &data), "OK\t0\t3,6\n");
        assert_eq!(handle_request("ASCIIWORD\tmaker", &data), "OK\t1\t4,5\n");
        assert_eq!(handle_request("ASCIIWORD\tpythonw", &data), "OK\t0\t6\n");
        assert_eq!(handle_request("ASCIIWORD\txyzwa", &data), "OK\t0\t\n");
        assert_eq!(handle_request("ASCIIWORD\tMaker", &data), "OK\t1\t4,5\n");
        // ユーザ辞書の英字の語も使う
        assert_eq!(handle_request("ADDWORD\tくいっくらいむ\tQuicklIME\t固有名詞", &data), "OK\n");
        assert_eq!(handle_request("ASCIIWORD\tquicklimed", &data), "OK\t0\t9\n");
    }

    #[test]
    fn asciiwordは辞書が無ければ前方一致ありを返す() {
        assert_eq!(handle_request("ASCIIWORD\tgithubd", &sample_data()), "OK\t1\t\n");
    }

    #[test]
    fn asciiwordの要求が不正ならエラー() {
        let data = english_data();
        assert!(handle_request("ASCIIWORD", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIWORD\t", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIWORD\tgit-hub", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIWORD\tきょう", &data).starts_with("ERR\t"));
    }

    /// ASCIIFORM のテスト用: 英単語辞書の表記を起動時と同じくユーザ辞書に写したデータ
    fn form_data() -> EngineData {
        let mut data = sample_data();
        data.english = EnglishDict::load_with_names(
            ["apple\nmay\n".as_bytes()],
            ["GitHub\nlocalhost\nmacOS\n".as_bytes()],
        );
        data.user.lock().unwrap().set_name_forms(data.english.forms().clone());
        data
    }

    #[test]
    fn asciiformは英単語辞書の正しい表記を返す() {
        let data = form_data();
        assert_eq!(handle_request("ASCIIFORM\tgithub", &data), "OK\tGitHub\n");
        assert_eq!(handle_request("ASCIIFORM\tGithub", &data), "OK\tGitHub\n");
        assert_eq!(handle_request("ASCIIFORM\tmacos", &data), "OK\tmacOS\n");
        // 表記と同じ、小文字だけの語、一般語、辞書に無い語は空
        assert_eq!(handle_request("ASCIIFORM\tGitHub", &data), "OK\t\n");
        assert_eq!(handle_request("ASCIIFORM\tlocalhost", &data), "OK\t\n");
        assert_eq!(handle_request("ASCIIFORM\tapple", &data), "OK\t\n");
        assert_eq!(handle_request("ASCIIFORM\tMay", &data), "OK\t\n");
        assert_eq!(handle_request("ASCIIFORM\txyzw", &data), "OK\t\n");
    }

    #[test]
    fn asciiformはユーザ辞書の表記を優先する() {
        let data = form_data();
        assert_eq!(handle_request("ADDWORD\tぎっとはぶ\tGithub\t固有名詞", &data), "OK\n");
        assert_eq!(handle_request("ASCIIFORM\tgithub", &data), "OK\tGithub\n");
        assert_eq!(handle_request("ASCIIFORM\tGitHub", &data), "OK\tGithub\n");
        assert_eq!(handle_request("ADDWORD\tくいっくらいむ\tQuicklIME\t固有名詞", &data), "OK\n");
        assert_eq!(handle_request("ASCIIFORM\tquicklime", &data), "OK\tQuicklIME\n");
    }

    #[test]
    fn asciiformの要求が不正ならエラー() {
        let data = form_data();
        assert!(handle_request("ASCIIFORM", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIFORM\t", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIFORM\tgit-hub", &data).starts_with("ERR\t"));
        assert!(handle_request("ASCIIFORM\tきょう", &data).starts_with("ERR\t"));
    }

    #[test]
    fn reloadconfigはokを返す() {
        assert_eq!(handle_request("RELOADCONFIG", &sample_data()), "OK\n");
    }
}
