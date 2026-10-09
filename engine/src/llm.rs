// LLM による候補の並べ替え (docs/design/llm-rerank.md)
//
// llama.cpp は子プロセス (quicklime-llm.exe / quicklime-llm-vulkan.exe) で動かし、標準入出力の
// 行プロトコルでやり取りする。子プロセスとのやり取りはエンジン内の専用スレッド (LLM ワーカー)
// 1本だけが行う。接続ごとの依頼は RERANK で受け付けて ID を返し、結果は RERANKGET で問い合わせる。
// 子プロセスの起動は Launcher で差し替えられる (テストでは偽の子プロセスを使う)。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::LlmBackend;
use crate::convert::{SentenceCandidate, to_katakana};

/// 並べ替えの対象にする CONVNBEST の上位件数
pub const RERANK_TOP: usize = 10;
/// LLM に渡す左文脈の最大文字数 (zenz の前処理と同じ)
pub const LLM_CONTEXT_CHARS: usize = 40;
/// 1要求の応答を待つ上限。超えたらその依頼は NONE にするが、子プロセスは止めずに応答を待ち続ける
const LLM_TIMEOUT_MS: u64 = 2000;
/// 子プロセスを異常とみなす応答待ちの上限。Vulkan 版はシェーダキャッシュの無い形の初回に十数秒
/// かかることがあり、その途中で終了させるとキャッシュが残らず同じ失敗を繰り返すため、長くとる
const LLM_HANG_MS: u64 = 60_000;
/// 子プロセスの異常の後、起動し直すまでの間隔
const LLM_RESTART_COOLDOWN_MS: u64 = 10_000;
/// 続けてこの回数失敗したら、設定を読み直すまで起動しない
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// 読み込み (READY 待ち) と遅れた応答の待ちの間に、設定の読み直し・終了の指示と新しい依頼を
/// 確かめる間隔
const READY_POLL_MS: u64 = 100;

/// 応答待ちと再起動の時間 (テストでは短くする)
#[derive(Clone, Copy)]
pub struct Timing {
    pub timeout: Duration,
    pub hang: Duration,
    pub cooldown: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            timeout: Duration::from_millis(LLM_TIMEOUT_MS),
            hang: Duration::from_millis(LLM_HANG_MS),
            cooldown: Duration::from_millis(LLM_RESTART_COOLDOWN_MS),
        }
    }
}

const CPU_EXE: &str = "quicklime-llm.exe";
const VULKAN_EXE: &str = "quicklime-llm-vulkan.exe";
const CPU_MODEL: &str = "zenz-v3.2-xsmall-Q5_K_M.gguf";
const VULKAN_MODEL: &str = "zenz-v3.2-small-Q5_K_M.gguf";

/// LLM に渡す左文脈の前処理: 先頭の空白を除き、末尾 LLM_CONTEXT_CHARS 文字にし、
/// 半角スペースを U+3000 にする (zenz のトークナイザは半角スペースを扱えない)
pub fn preprocess_context(context: &str) -> String {
    let trimmed = context.trim_start();
    let n = trimmed.chars().count();
    let tail: String = trimmed.chars().skip(n.saturating_sub(LLM_CONTEXT_CHARS)).collect();
    tail.replace(' ', "\u{3000}")
}

/// 上位 RERANK_TOP 件のうち、並べ替えの対象 (保護しない候補) の位置
pub fn rerank_targets(candidates: &[SentenceCandidate]) -> Vec<usize> {
    candidates.iter().take(RERANK_TOP).enumerate().filter(|(_, c)| !c.protected).map(|(i, _)| i).collect()
}

/// targets の位置の候補を scores (targets と同じ並び) の高い順に、targets の位置へ詰め直す。
/// それ以外の位置の候補は動かさない。同点は元の順を保つ
pub fn reorder(mut candidates: Vec<SentenceCandidate>, targets: &[usize], scores: &[f64]) -> Vec<SentenceCandidate> {
    let mut ranked: Vec<usize> = (0..targets.len()).collect();
    ranked.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let mut slots: Vec<Option<SentenceCandidate>> = candidates.drain(..).map(Some).collect();
    let picked: Vec<SentenceCandidate> =
        ranked.iter().map(|&k| slots[targets[k]].take().expect("targets に重複は無い")).collect();
    for (&position, candidate) in targets.iter().zip(picked) {
        slots[position] = Some(candidate);
    }
    slots.into_iter().map(|c| c.expect("全位置が埋まっている")).collect()
}

// ---------------------------------------------------------------------------
// 子プロセス

/// 子プロセスからの受信の失敗
#[derive(Debug, PartialEq, Eq)]
pub enum RecvError {
    Timeout,
    /// 子プロセスが終了した (標準出力が閉じた)
    Closed,
}

/// 起動した子プロセス。drop で終了させる
pub trait ChildProcess: Send {
    fn send(&mut self, line: &str) -> std::io::Result<()>;
    fn recv(&mut self, timeout: Duration) -> Result<String, RecvError>;
}

/// 子プロセスの起動 (テストでは偽の子プロセスに差し替える)
pub trait Launcher: Send + Sync {
    fn launch(&self, backend: LlmBackend) -> std::io::Result<Box<dyn ChildProcess>>;
}

/// quicklime-llm の exe を子プロセスとして起動する
pub struct ProcessLauncher;

impl Launcher for ProcessLauncher {
    fn launch(&self, backend: LlmBackend) -> std::io::Result<Box<dyn ChildProcess>> {
        let (exe_name, model_name) = match backend {
            LlmBackend::Cpu => (CPU_EXE, CPU_MODEL),
            LlmBackend::Vulkan => (VULKAN_EXE, VULKAN_MODEL),
        };
        let not_found = |what: &str| std::io::Error::new(std::io::ErrorKind::NotFound, what.to_string());
        let exe = llm_exe_path(exe_name).ok_or_else(|| not_found("LLM の exe が見つかりません"))?;
        let model = model_dir().ok_or_else(|| not_found("モデルのディレクトリを特定できません"))?.join(model_name);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(4));
        ProcessChild::spawn(&exe, &model, threads)
    }
}

/// エンジンの exe のディレクトリと、開発レイアウトのリポジトリ直下
/// (exe が engine/target/{debug,release}/ にある前提)
fn exe_dir_and_repo_root() -> Option<(PathBuf, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    let root = dir.parent()?.parent()?.parent()?.to_path_buf();
    Some((dir, root))
}

/// LLM の exe の場所。exe と同じディレクトリ → 開発レイアウトの llm\target\release\
fn llm_exe_path(name: &str) -> Option<PathBuf> {
    let (dir, root) = exe_dir_and_repo_root()?;
    [dir.join(name), root.join("llm").join("target").join("release").join(name)]
        .into_iter()
        .find(|p| p.is_file())
}

/// モデルのディレクトリ。環境変数 QUICKLIME_LLM_MODEL_DIR → exe と同じディレクトリの models\
/// (存在するときのみ) → 開発レイアウトのリポジトリ直下の models\
fn model_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("QUICKLIME_LLM_MODEL_DIR") {
        return Some(PathBuf::from(dir));
    }
    let (dir, root) = exe_dir_and_repo_root()?;
    let bundled = dir.join("models");
    if bundled.is_dir() {
        return Some(bundled);
    }
    Some(root.join("models"))
}

struct ProcessChild {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
}

impl ProcessChild {
    fn spawn(exe: &std::path::Path, model: &std::path::Path, threads: usize) -> std::io::Result<Box<dyn ChildProcess>> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = std::process::Command::new(exe)
            .arg("--model")
            .arg(model)
            .arg("--threads")
            .arg(threads.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
        assign_to_kill_on_close_job(&child);
        let stdin = child.stdin.take().expect("stdin は piped");
        let stdout = child.stdout.take().expect("stdout は piped");
        // 応答待ちに時間制限を付けるため、読み取りは別スレッドで行ってチャネルで受け取る
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Box::new(ProcessChild { child, stdin, lines }))
    }
}

impl ChildProcess for ProcessChild {
    fn send(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }

    fn recv(&mut self, timeout: Duration) -> Result<String, RecvError> {
        use std::sync::mpsc::RecvTimeoutError;
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(line.trim_end_matches('\r').to_string()),
            Err(RecvTimeoutError::Timeout) => Err(RecvError::Timeout),
            Err(RecvTimeoutError::Disconnected) => Err(RecvError::Closed),
        }
    }
}

impl Drop for ProcessChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 子プロセスをジョブオブジェクトに入れ、エンジンが異常終了しても巻き込んで終了させる
/// (正常時は標準入力が閉じたら子プロセスは自分で終了する)
fn assign_to_kill_on_close_job(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // ジョブのハンドルはエンジンの終了まで開いたままにする (閉じたときに子プロセスが終了する)
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job as usize
    });
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
        }
    }
}

// ---------------------------------------------------------------------------
// LLM ワーカー

/// 並べ替えの依頼1件
pub struct Job {
    pub id: u64,
    /// 前処理済みの左文脈
    pub context: String,
    /// 読み (ひらがな。送るときにカタカナにする)
    pub reading: String,
    /// CONVNBEST と同じ並びの候補 (保護の印つき)
    pub candidates: Vec<SentenceCandidate>,
}

/// RERANKGET の結果
pub enum Poll {
    Pending,
    Done(Vec<SentenceCandidate>),
    None,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Settings {
    enabled: bool,
    backend: LlmBackend,
}

struct State {
    settings: Settings,
    /// configure で、動いている子プロセスを止めて起動し直す
    restart: bool,
    /// 子プロセスが READY を返し、依頼を受け付けられる
    ready: bool,
    /// Vulkan 版が起動できず CPU 版で動いている (設定を読み直すまで続ける)
    fell_back: bool,
    failures: u32,
    cooldown_until: Option<Instant>,
    /// 接続ごとの、まだ始まっていない依頼 (到着順。同じ接続は1件だけ)
    pending: Vec<(u64, Job)>,
    /// 推論中の依頼 (接続, ID)
    running: Option<(u64, u64)>,
    /// 接続ごとの最新の依頼 ID
    latest: HashMap<u64, u64>,
    /// 接続ごとの最新の結果 (ID, 並べ替えた候補。失敗なら None)
    results: HashMap<u64, (u64, Option<Vec<SentenceCandidate>>)>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    timing: Timing,
}

/// LLM ワーカーと、接続ごとの依頼・結果の管理
pub struct LlmManager {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl LlmManager {
    /// ワーカーを起動する (設定は無効の状態で始まり、configure で有効にする)
    pub fn new(launcher: Box<dyn Launcher>) -> Self {
        Self::with_timing(launcher, Timing::default())
    }

    pub fn with_timing(launcher: Box<dyn Launcher>, timing: Timing) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                settings: Settings { enabled: false, backend: LlmBackend::Cpu },
                restart: false,
                ready: false,
                fell_back: false,
                failures: 0,
                cooldown_until: None,
                pending: Vec::new(),
                running: None,
                latest: HashMap::new(),
                results: HashMap::new(),
                shutdown: false,
            }),
            wake: Condvar::new(),
            timing,
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || worker_loop(&worker_shared, launcher.as_ref()));
        LlmManager { shared, worker: Some(worker) }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().expect("llm state lock")
    }

    /// 設定を反映する (起動時と RELOADCONFIG)。設定を読み直したことになるので、失敗の回数と
    /// Vulkan 版から CPU 版への切り替えを戻す。設定が変わったか CPU 版へ切り替えていたら起動し直す
    pub fn configure(&self, enabled: bool, backend: LlmBackend) {
        let mut state = self.lock();
        let settings = Settings { enabled, backend };
        if settings != state.settings || state.fell_back {
            state.restart = true;
        }
        state.settings = settings;
        state.fell_back = false;
        state.failures = 0;
        state.cooldown_until = None;
        self.shared.wake.notify_all();
    }

    /// 依頼を受け付ける。子プロセスが READY でなければ受け付けずに false。
    /// 同じ接続のまだ始まっていない依頼は新しいものに置き換える
    pub fn request(&self, conn: u64, job: Job) -> bool {
        let mut state = self.lock();
        if !state.settings.enabled || !state.ready {
            return false;
        }
        state.latest.insert(conn, job.id);
        if let Some(slot) = state.pending.iter_mut().find(|(c, _)| *c == conn) {
            slot.1 = job;
        } else {
            state.pending.push((conn, job));
        }
        self.shared.wake.notify_all();
        true
    }

    pub fn poll(&self, conn: u64, id: u64) -> Poll {
        let state = self.lock();
        if id == 0 || state.latest.get(&conn) != Some(&id) {
            return Poll::None;
        }
        if let Some((result_id, result)) = state.results.get(&conn)
            && *result_id == id
        {
            return match result {
                Some(candidates) => Poll::Done(candidates.iter().map(clone_candidate).collect()),
                None => Poll::None,
            };
        }
        let pending = state.pending.iter().any(|(c, job)| *c == conn && job.id == id);
        if pending || state.running == Some((conn, id)) { Poll::Pending } else { Poll::None }
    }

    /// 接続が切れたので、その接続の依頼と結果を捨てる
    pub fn disconnect(&self, conn: u64) {
        let mut state = self.lock();
        state.pending.retain(|(c, _)| *c != conn);
        state.latest.remove(&conn);
        state.results.remove(&conn);
    }
}

impl Drop for LlmManager {
    fn drop(&mut self) {
        self.lock().shutdown = true;
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn clone_candidate(c: &SentenceCandidate) -> SentenceCandidate {
    SentenceCandidate { segments: c.segments.clone(), protected: c.protected }
}

/// 子プロセスを止めたときに、待っている依頼を失敗にする (RERANKGET は NONE になる)
fn fail_pending(state: &mut State) {
    for (conn, job) in std::mem::take(&mut state.pending) {
        state.results.insert(conn, (job.id, None));
    }
}

/// 子プロセスの異常: 次の起動まで間を空け、続けて失敗した回数を数える
fn note_failure(state: &mut State, timing: &Timing) {
    state.ready = false;
    state.failures += 1;
    state.cooldown_until = Some(Instant::now() + timing.cooldown);
    fail_pending(state);
}

fn worker_loop(shared: &Shared, launcher: &dyn Launcher) {
    let mut child: Option<Box<dyn ChildProcess>> = None;
    let mut state = shared.state.lock().expect("llm state lock");
    loop {
        if state.shutdown {
            return;
        }
        if state.restart {
            state.restart = false;
            state.ready = false;
            child = None;
            fail_pending(&mut state);
        }
        if !state.settings.enabled || state.failures >= MAX_CONSECUTIVE_FAILURES {
            state.ready = false;
            child = None;
            fail_pending(&mut state);
            state = shared.wake.wait(state).expect("llm state lock");
            continue;
        }
        if child.is_none() {
            if let Some(until) = state.cooldown_until {
                let now = Instant::now();
                if now < until {
                    state = shared.wake.wait_timeout(state, until - now).expect("llm state lock").0;
                    continue;
                }
                state.cooldown_until = None;
            }
            let backend = if state.fell_back { LlmBackend::Cpu } else { state.settings.backend };
            drop(state);
            let started = start_child(shared, launcher, backend);
            state = shared.state.lock().expect("llm state lock");
            match started {
                StartResult::Ready(started) => {
                    if !state.restart && !state.shutdown {
                        eprintln!("LLM の子プロセスを起動しました ({backend:?})");
                        child = Some(started);
                        state.ready = true;
                    }
                }
                StartResult::Interrupted => {}
                StartResult::Failed(message) => {
                    eprintln!("LLM の子プロセスを起動できません ({backend:?}): {message}");
                    if backend == LlmBackend::Vulkan {
                        // Vulkan 版が動かない環境では、待たずに CPU 版で動く
                        state.fell_back = true;
                    } else {
                        note_failure(&mut state, &shared.timing);
                    }
                }
            }
            continue;
        }
        if state.pending.is_empty() {
            state = shared.wake.wait(state).expect("llm state lock");
            continue;
        }
        let (conn, job) = state.pending.remove(0);
        state.running = Some((conn, job.id));
        drop(state);
        let running = child.as_mut().expect("子プロセスは起動済み");
        let outcome = score_job(running.as_mut(), job, &shared.timing);
        state = shared.state.lock().expect("llm state lock");
        state.running = None;
        let broken = match outcome {
            JobOutcome::Done(id, candidates) => {
                state.failures = 0;
                state.results.insert(conn, (id, Some(candidates)));
                None
            }
            JobOutcome::Rejected(id) => {
                state.results.insert(conn, (id, None));
                None
            }
            JobOutcome::TimedOut(id, sent_at) => {
                // この依頼は統計の並びのままにし、子プロセスは止めずに応答を待ち続ける
                state.results.insert(conn, (id, None));
                drop(state);
                let late = wait_late_response(shared, running.as_mut(), sent_at);
                state = shared.state.lock().expect("llm state lock");
                late.err()
            }
            JobOutcome::Broken(id, message) => {
                state.results.insert(conn, (id, None));
                Some(message)
            }
        };
        if let Some(message) = broken {
            eprintln!("LLM の子プロセスの異常: {message}。起動し直します");
            child = None;
            note_failure(&mut state, &shared.timing);
        }
    }
}

/// LLM_TIMEOUT_MS を超えた依頼の応答を待ち、届いたら捨てる。その間に来た依頼は子プロセスへ
/// 送らずに NONE にする。LLM_HANG_MS を超えた・子プロセスが終了したときは Err。
/// 設定の読み直し・終了の指示が来たら待つのをやめる (子プロセスは呼び出し側の後始末で止まる)
fn wait_late_response(shared: &Shared, child: &mut dyn ChildProcess, sent_at: Instant) -> Result<(), String> {
    loop {
        let elapsed = sent_at.elapsed();
        if elapsed >= shared.timing.hang {
            return Err("応答がありません".to_string());
        }
        let slice = Duration::from_millis(READY_POLL_MS).min(shared.timing.hang - elapsed);
        match child.recv(slice) {
            Ok(_) => return Ok(()),
            Err(RecvError::Closed) => return Err("子プロセスが終了しました".to_string()),
            Err(RecvError::Timeout) => {
                let mut state = shared.state.lock().expect("llm state lock");
                fail_pending(&mut state);
                if state.restart || state.shutdown {
                    return Ok(());
                }
            }
        }
    }
}

enum StartResult {
    Ready(Box<dyn ChildProcess>),
    /// 読み込み中に設定の読み直し・終了の指示が来た
    Interrupted,
    Failed(String),
}

fn start_child(shared: &Shared, launcher: &dyn Launcher, backend: LlmBackend) -> StartResult {
    let mut child = match launcher.launch(backend) {
        Ok(child) => child,
        Err(e) => return StartResult::Failed(e.to_string()),
    };
    loop {
        match child.recv(Duration::from_millis(READY_POLL_MS)) {
            Ok(line) if line == "READY" => return StartResult::Ready(child),
            Ok(line) => return StartResult::Failed(line),
            Err(RecvError::Closed) => return StartResult::Failed("子プロセスが終了しました".to_string()),
            Err(RecvError::Timeout) => {
                let state = shared.state.lock().expect("llm state lock");
                if state.restart || state.shutdown {
                    return StartResult::Interrupted;
                }
            }
        }
    }
}

enum JobOutcome {
    Done(u64, Vec<SentenceCandidate>),
    /// 子プロセスが ERR を返した (入力が長すぎるなど。子プロセスは正常)
    Rejected(u64),
    /// LLM_TIMEOUT_MS までに応答が無かった (送った時刻)
    TimedOut(u64, Instant),
    /// 終了・不正な応答
    Broken(u64, String),
}

fn score_job(child: &mut dyn ChildProcess, job: Job, timing: &Timing) -> JobOutcome {
    let targets = rerank_targets(&job.candidates);
    if targets.len() < 2 {
        return JobOutcome::Done(job.id, job.candidates);
    }
    let mut line = format!("SCORE\t{}\t{}", job.context, to_katakana(&job.reading));
    for &i in &targets {
        line.push('\t');
        line.push_str(&job.candidates[i].surface());
    }
    let sent_at = Instant::now();
    if let Err(e) = child.send(&line) {
        return JobOutcome::Broken(job.id, format!("送信に失敗: {e}"));
    }
    let response = match child.recv(timing.timeout) {
        Ok(response) => response,
        Err(RecvError::Timeout) => return JobOutcome::TimedOut(job.id, sent_at),
        Err(RecvError::Closed) => return JobOutcome::Broken(job.id, "子プロセスが終了しました".to_string()),
    };
    if response.starts_with("ERR\t") {
        return JobOutcome::Rejected(job.id);
    }
    let scores: Option<Vec<f64>> = response
        .strip_prefix("OK\t")
        .map(|body| body.split('\t').map(|s| s.parse::<f64>().ok()).collect::<Option<Vec<f64>>>())
        .unwrap_or(None);
    match scores {
        Some(scores) if scores.len() == targets.len() => {
            JobOutcome::Done(job.id, reorder(job.candidates, &targets, &scores))
        }
        _ => JobOutcome::Broken(job.id, "応答が不正です".to_string()),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub fn candidate(surface: &str, protected: bool) -> SentenceCandidate {
        SentenceCandidate { segments: vec![("よみ".to_string(), surface.to_string())], protected }
    }

    fn surfaces(candidates: &[SentenceCandidate]) -> Vec<String> {
        candidates.iter().map(SentenceCandidate::surface).collect()
    }

    #[test]
    fn 文脈の前処理() {
        assert_eq!(preprocess_context("  \u{3000}熱が出て"), "熱が出て");
        assert_eq!(preprocess_context("a b"), "a\u{3000}b");
        let long: String = "あ".repeat(10) + &"い".repeat(LLM_CONTEXT_CHARS);
        assert_eq!(preprocess_context(&long), "い".repeat(LLM_CONTEXT_CHARS));
        assert_eq!(preprocess_context(""), "");
    }

    #[test]
    fn 保護しない候補だけが尤度順に詰め直される() {
        let candidates = vec![
            candidate("学習", true),
            candidate("A", false),
            candidate("B", false),
            candidate("英字x", true),
            candidate("C", false),
        ];
        let targets = rerank_targets(&candidates);
        assert_eq!(targets, vec![1, 2, 4]);
        let got = reorder(candidates, &targets, &[-3.0, -1.0, -2.0]);
        assert_eq!(surfaces(&got), ["学習", "B", "C", "英字x", "A"]);
    }

    #[test]
    fn 上位件数より後ろは並べ替えの対象にしない() {
        let candidates: Vec<SentenceCandidate> =
            (0..RERANK_TOP + 3).map(|i| candidate(&format!("候補{i}"), false)).collect();
        let targets = rerank_targets(&candidates);
        assert_eq!(targets, (0..RERANK_TOP).collect::<Vec<_>>());
        let scores: Vec<f64> = (0..RERANK_TOP).map(|i| i as f64).collect();
        let got = reorder(candidates, &targets, &scores);
        let expected: Vec<String> = (0..RERANK_TOP)
            .rev()
            .chain(RERANK_TOP..RERANK_TOP + 3)
            .map(|i| format!("候補{i}"))
            .collect();
        assert_eq!(surfaces(&got), expected);
    }

    // ---- 偽の子プロセス ----

    /// 偽の子プロセスの振る舞い
    #[derive(Clone, Copy)]
    pub enum FakeBehavior {
        /// READY を返し、SCORE には候補の位置の逆順の尤度 (後ろほど高い) を返す
        Normal,
        /// READY の後、SCORE に応答しない
        Silent,
        /// READY の後、SCORE で終了する
        Exit,
        /// READY の後、SCORE に不正な応答を返す
        Garbage,
        /// READY を返さずに ERR で終了する
        LoadError,
    }

    pub struct FakeLauncher {
        pub behavior: Arc<Mutex<FakeBehavior>>,
        pub launches: Arc<AtomicUsize>,
        /// 子プロセスへ送った SCORE の数
        pub sends: Arc<AtomicUsize>,
        /// SCORE を受け取ってから応答するまでの時間
        pub delay: Arc<Mutex<Duration>>,
    }

    struct FakeChild {
        behavior: FakeBehavior,
        delay: Arc<Mutex<Duration>>,
        sends: Arc<AtomicUsize>,
        /// 返す予定の応答と、返せるようになる時刻 (送った順)
        queue: std::collections::VecDeque<(Instant, Result<String, RecvError>)>,
        started: bool,
    }

    impl ChildProcess for FakeChild {
        fn send(&mut self, line: &str) -> std::io::Result<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            let count = line.split('\t').count().saturating_sub(3);
            let response = match self.behavior {
                FakeBehavior::Normal => {
                    let scores: Vec<String> = (0..count).map(|i| i.to_string()).collect();
                    Ok(format!("OK\t{}", scores.join("\t")))
                }
                FakeBehavior::Silent => return Ok(()),
                FakeBehavior::Exit | FakeBehavior::LoadError => Err(RecvError::Closed),
                FakeBehavior::Garbage => Ok("OK\tabc".to_string()),
            };
            let delay = *self.delay.lock().unwrap();
            self.queue.push_back((Instant::now() + delay, response));
            Ok(())
        }

        fn recv(&mut self, timeout: Duration) -> Result<String, RecvError> {
            if !self.started {
                self.started = true;
                return match self.behavior {
                    FakeBehavior::LoadError => Ok("ERR\t読み込みに失敗".to_string()),
                    _ => Ok("READY".to_string()),
                };
            }
            let deadline = Instant::now() + timeout;
            match self.queue.front() {
                Some((ready_at, _)) if *ready_at <= deadline => {
                    std::thread::sleep(ready_at.saturating_duration_since(Instant::now()));
                    self.queue.pop_front().unwrap().1
                }
                _ => {
                    std::thread::sleep(timeout);
                    Err(RecvError::Timeout)
                }
            }
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(&self, _backend: LlmBackend) -> std::io::Result<Box<dyn ChildProcess>> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeChild {
                behavior: *self.behavior.lock().unwrap(),
                delay: Arc::clone(&self.delay),
                sends: Arc::clone(&self.sends),
                queue: Default::default(),
                started: false,
            }))
        }
    }

    /// 偽の子プロセスを使うワーカーと、起動の回数・送った SCORE の数・応答の遅れ (後から変えられる)
    pub struct Fake {
        pub manager: LlmManager,
        pub launches: Arc<AtomicUsize>,
        pub sends: Arc<AtomicUsize>,
        pub delay: Arc<Mutex<Duration>>,
    }

    pub fn fake_with(behavior: FakeBehavior, delay: Duration, timing: Timing) -> Fake {
        let launches = Arc::new(AtomicUsize::new(0));
        let sends = Arc::new(AtomicUsize::new(0));
        let delay = Arc::new(Mutex::new(delay));
        let launcher = FakeLauncher {
            behavior: Arc::new(Mutex::new(behavior)),
            launches: Arc::clone(&launches),
            sends: Arc::clone(&sends),
            delay: Arc::clone(&delay),
        };
        Fake { manager: LlmManager::with_timing(Box::new(launcher), timing), launches, sends, delay }
    }

    pub fn fake_manager(behavior: FakeBehavior, delay: Duration) -> (LlmManager, Arc<AtomicUsize>) {
        let fake = fake_with(behavior, delay, Timing::default());
        (fake.manager, fake.launches)
    }

    /// 異常の判定と再起動のテスト用の短い時間
    fn short_timing() -> Timing {
        Timing {
            timeout: Duration::from_millis(100),
            hang: Duration::from_millis(600),
            cooldown: Duration::from_millis(200),
        }
    }

    /// 子プロセスが READY になるまで待つ (異常の後は再起動までの間隔を待つ)
    pub fn wait_ready(manager: &LlmManager) {
        let started = Instant::now();
        let limit = manager.shared.timing.cooldown + Duration::from_secs(5);
        while !manager.lock().ready {
            assert!(started.elapsed() < limit, "READY にならない");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn job(id: u64) -> Job {
        Job {
            id,
            context: String::new(),
            reading: "よみ".to_string(),
            candidates: vec![candidate("A", false), candidate("B", false), candidate("C", false)],
        }
    }

    fn wait_result(manager: &LlmManager, conn: u64, id: u64) -> Poll {
        let started = Instant::now();
        loop {
            match manager.poll(conn, id) {
                Poll::Pending => {}
                other => return other,
            }
            assert!(started.elapsed() < Duration::from_secs(10), "結果が出ない");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn 無効なうちは依頼を受け付けない() {
        let (manager, launches) = fake_manager(FakeBehavior::Normal, Duration::ZERO);
        assert!(!manager.request(1, job(1)));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(launches.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn 依頼を尤度順に並べ替える() {
        let (manager, _) = fake_manager(FakeBehavior::Normal, Duration::ZERO);
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(&manager);
        assert!(manager.request(1, job(1)));
        match wait_result(&manager, 1, 1) {
            Poll::Done(candidates) => assert_eq!(surfaces(&candidates), ["C", "B", "A"]),
            _ => panic!("DONE にならない"),
        }
        // ID 0 と不明な ID は NONE
        assert!(matches!(manager.poll(1, 0), Poll::None));
        assert!(matches!(manager.poll(1, 9), Poll::None));
        assert!(matches!(manager.poll(2, 1), Poll::None));
    }

    #[test]
    fn 待ち中の依頼は新しいものに置き換わる() {
        let (manager, _) = fake_manager(FakeBehavior::Normal, Duration::from_millis(300));
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(&manager);
        assert!(manager.request(1, job(1)));
        // 1 が推論中になるのを待ってから 2・3 を入れる。2 は始まる前に 3 に置き換わる
        let started = Instant::now();
        while manager.lock().running.is_none() {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(manager.request(1, job(2)));
        assert!(manager.request(1, job(3)));
        assert_eq!(manager.lock().pending.len(), 1);
        assert!(matches!(manager.poll(1, 1), Poll::None));
        assert!(matches!(manager.poll(1, 2), Poll::None));
        assert!(matches!(manager.poll(1, 3), Poll::Pending));
        assert!(matches!(wait_result(&manager, 1, 3), Poll::Done(_)));
        assert!(matches!(manager.poll(1, 2), Poll::None));
    }

    #[test]
    fn 応答が遅れた依頼はnoneにして子プロセスは止めない() {
        // 応答は 300ms 後 (応答待ちの上限 100ms と、異常とみなす 600ms の間)
        let fake = fake_with(FakeBehavior::Normal, Duration::from_millis(300), short_timing());
        let manager = &fake.manager;
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(manager);
        let started = Instant::now();
        assert!(manager.request(1, job(1)));
        assert!(matches!(wait_result(manager, 1, 1), Poll::None));
        assert!(started.elapsed() < Duration::from_millis(300), "応答待ちの上限で NONE にならない");

        // 遅れた応答を待っている間に来た依頼は、子プロセスへ送らずに NONE にする
        assert!(manager.request(1, job(2)));
        assert!(matches!(wait_result(manager, 1, 2), Poll::None));
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);

        // 遅れた応答は捨て、次の依頼から通常に戻る。子プロセスは起動し直していない
        *fake.delay.lock().unwrap() = Duration::ZERO;
        std::thread::sleep(Duration::from_millis(400));
        assert!(manager.request(1, job(3)));
        match wait_result(manager, 1, 3) {
            Poll::Done(candidates) => assert_eq!(surfaces(&candidates), ["C", "B", "A"]),
            _ => panic!("遅れた応答の後の依頼が DONE にならない"),
        }
        assert_eq!(fake.sends.load(Ordering::SeqCst), 2);
        assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
        assert_eq!(manager.lock().failures, 0);
    }

    #[test]
    fn 異常とみなす時間を超えたら起動し直す() {
        let fake = fake_with(FakeBehavior::Silent, Duration::ZERO, short_timing());
        let manager = &fake.manager;
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(manager);
        assert!(manager.request(1, job(1)));
        assert!(matches!(wait_result(manager, 1, 1), Poll::None));
        // 応答待ちの上限 (100ms) では止めず、異常とみなす時間 (600ms) を超えてから止める
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
        assert!(manager.lock().ready);
        let started = Instant::now();
        while fake.launches.load(Ordering::SeqCst) < 2 {
            assert!(started.elapsed() < Duration::from_secs(5), "起動し直さない");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(manager.lock().failures, 1);
    }

    /// 異常のたびに起動し直し、続けて3回失敗したら止まることを確かめる
    fn check_restart(behavior: FakeBehavior) {
        let timing = short_timing();
        let fake = fake_with(behavior, Duration::ZERO, timing);
        let (manager, launches) = (&fake.manager, &fake.launches);
        manager.configure(true, LlmBackend::Cpu);
        for attempt in 1..=MAX_CONSECUTIVE_FAILURES as u64 {
            wait_ready(manager);
            assert!(manager.request(1, job(attempt)));
            assert!(matches!(wait_result(manager, 1, attempt), Poll::None));
            // 異常と判定されて子プロセスが止まるまで待つ (応答しない子プロセスは hang まで止まらない)
            let started = Instant::now();
            while manager.lock().ready {
                assert!(started.elapsed() < Duration::from_secs(5), "子プロセスが止まらない");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(launches.load(Ordering::SeqCst), attempt as usize);
        }
        // 3回続けて失敗したので、待っても起動しない。READY でないので依頼は受け付けない (OK\t0)
        std::thread::sleep(timing.cooldown + Duration::from_millis(500));
        assert_eq!(launches.load(Ordering::SeqCst), MAX_CONSECUTIVE_FAILURES as usize);
        assert!(!manager.request(1, job(100)));
        // 設定を読み直すと起動し直す
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(manager);
        assert_eq!(launches.load(Ordering::SeqCst), MAX_CONSECUTIVE_FAILURES as usize + 1);
    }

    #[test]
    fn 応答しない子プロセスは起動し直して3回で止まる() {
        check_restart(FakeBehavior::Silent);
    }

    #[test]
    fn 終了した子プロセスは起動し直して3回で止まる() {
        check_restart(FakeBehavior::Exit);
    }

    #[test]
    fn 不正な応答の子プロセスは起動し直して3回で止まる() {
        check_restart(FakeBehavior::Garbage);
    }

    #[test]
    fn vulkan版が起動できなければcpu版に切り替える() {
        struct VulkanFails(Arc<Mutex<Vec<LlmBackend>>>);
        impl Launcher for VulkanFails {
            fn launch(&self, backend: LlmBackend) -> std::io::Result<Box<dyn ChildProcess>> {
                self.0.lock().unwrap().push(backend);
                let behavior =
                    if backend == LlmBackend::Vulkan { FakeBehavior::LoadError } else { FakeBehavior::Normal };
                Ok(Box::new(FakeChild {
                    behavior,
                    delay: Arc::new(Mutex::new(Duration::ZERO)),
                    sends: Arc::new(AtomicUsize::new(0)),
                    queue: Default::default(),
                    started: false,
                }))
            }
        }
        let launched = Arc::new(Mutex::new(Vec::new()));
        let manager = LlmManager::new(Box::new(VulkanFails(Arc::clone(&launched))));
        manager.configure(true, LlmBackend::Vulkan);
        wait_ready(&manager);
        assert_eq!(*launched.lock().unwrap(), [LlmBackend::Vulkan, LlmBackend::Cpu]);
        // 設定を読み直すと、再び Vulkan 版から試す
        manager.configure(true, LlmBackend::Vulkan);
        let started = Instant::now();
        while launched.lock().unwrap().len() < 4 {
            assert!(started.elapsed() < Duration::from_secs(5), "起動し直さない");
            std::thread::sleep(Duration::from_millis(5));
        }
        wait_ready(&manager);
        assert_eq!(
            *launched.lock().unwrap(),
            [LlmBackend::Vulkan, LlmBackend::Cpu, LlmBackend::Vulkan, LlmBackend::Cpu]
        );
    }

    #[test]
    fn 無効にすると子プロセスを止める() {
        let (manager, _) = fake_manager(FakeBehavior::Normal, Duration::ZERO);
        manager.configure(true, LlmBackend::Cpu);
        wait_ready(&manager);
        manager.configure(false, LlmBackend::Cpu);
        let started = Instant::now();
        while manager.lock().ready {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!manager.request(1, job(1)));
    }
}
