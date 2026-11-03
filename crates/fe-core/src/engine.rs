//! The persistent batch queue.
//!
//! - Each job owns an immutable settings snapshot; global settings never leak in.
//! - Jobs run through the state machine in `job.rs`; every transition is persisted.
//! - One failing job (bad input, crashed backend, failed verification, even a
//!   panic) never stops the queue. A locked results CSV only defers rows.
//! - Concurrency is resource-aware: CPU encodes, and hardware/stream-copy jobs,
//!   use separate slots; each CPU job gets a thread budget; memory and disk are
//!   checked before a job starts and disk is watched while it runs.
//! - Output goes to an owned temp file, is verified, then atomically committed.
//! - After a crash, orphaned backends are killed, owned temp files deleted and
//!   interrupted jobs restart from the beginning (never "resumed" mid-stream).

use crate::caps::{self, Capabilities, LosslessCheck};
use crate::fsx;
use crate::job::*;
use crate::manifest::{Manifest, ManifestRow, ManifestStatus};
use crate::media::{self, MediaInfo};
use crate::plan::{self, EncodePlan, PlanEnv, PlanError, Prep};
use crate::process::{self, Outcome, RunOptions};
use crate::settings::*;
use crate::store::{self, LoadOutcome, QUEUE_VERSION, QueueFile};
use crate::tools::FfmpegPaths;
use crate::verify::{self, VerifyAbort, VerifyCtx};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

pub struct EngineConfig {
    pub ffmpeg: FfmpegPaths,
    /// queue.json, logs/, results CSV
    pub data_dir: PathBuf,
    /// Per-job scratch dirs (two-pass stats, frame hashes); wiped on start.
    pub work_root: PathBuf,
    pub manifest_path: PathBuf,
    /// Override for how long a backend may make no progress before it counts as hung
    /// (default: per encoder, see `stall_limit_for`).
    pub stall_limit: Option<Duration>,
}

/// Longest legitimate time without new output: lookahead before the first packet plus
/// the slowest per-frame time we use. libaom at low cpu-used is far slower than the rest.
pub fn stall_limit_for(encoder: Option<&str>) -> Duration {
    match encoder {
        Some("libaom-av1") => Duration::from_secs(15 * 60),
        _ => process::STALL_LIMIT,
    }
}

/// Receives state changes (the GUI forwards them to the window).
pub trait EventSink: Send + Sync + 'static {
    fn job(&self, _job: &Job) {}
    fn removed(&self, _ids: &[JobId]) {}
    fn queue(&self, _status: &QueueStatus) {}
    fn manifest(&self, _status: &ManifestStatus) {}
    fn caps(&self, _caps: &Capabilities) {}
}

pub struct NoopSink;
impl EventSink for NoopSink {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Concurrency {
    /// Derived from the hardware: CPU encodes get the whole machine (1 slot, 2 on
    /// 16+ threads with 16+ GB), plus one slot for hardware-encoder / copy jobs.
    Auto,
    Fixed { n: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    pub paused: bool,
    pub running: usize,
    pub waiting: usize,
    pub needs_input: usize,
    pub done: usize,
    pub failed: usize,
    pub concurrency: Concurrency,
    pub cpu_slots: u32,
    pub light_slots: u32,
    pub threads_per_job: u32,
    /// "memory" when a job waits for RAM.
    pub blocked: Option<String>,
    pub order: Vec<JobId>,
    /// Unfinished jobs restored from the previous session.
    pub recovered: usize,
}

pub struct Engine {
    inner: Arc<Inner>,
}

const RESERVE_DISK: u64 = 512 * 1024 * 1024;
const MIN_FREE_WHILE_RUNNING: u64 = 256 * 1024 * 1024;
const RESERVE_MEM_MB: u64 = 1024;
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
const LOG_RETENTION: Duration = Duration::from_secs(30 * 24 * 3600);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Engine {
    pub async fn start(cfg: EngineConfig, caps: Capabilities, sink: Arc<dyn EventSink>) -> Engine {
        let _ = std::fs::create_dir_all(cfg.data_dir.join("logs"));
        // The work root is our own scratch area; anything left there is from a previous run.
        if cfg.work_root.components().count() >= 3 {
            let _ = std::fs::remove_dir_all(&cfg.work_root);
        }
        let _ = std::fs::create_dir_all(&cfg.work_root);
        prune_logs(&cfg.data_dir.join("logs"));

        let (mut jobs, was_paused) = match store::load_json::<QueueFile>(&cfg.data_dir.join("queue.json")) {
            LoadOutcome::Loaded(q) | LoadOutcome::FromBackup(q) => (q.jobs, q.paused),
            LoadOutcome::Missing | LoadOutcome::Corrupt { .. } => (Vec::new(), false),
        };
        for j in jobs.iter().filter(|j| j.state.is_active()) {
            if let Some(pid) = j.pid {
                kill_orphan(pid);
            }
        }
        store::recover_interrupted(&mut jobs);
        for j in jobs.iter_mut() {
            // Only files matching our temp pattern are ever deleted.
            j.temp_files.retain(|t| fsx::remove_owned_temp(t).is_err() && t.exists());
            j.pid = None;
        }
        let recovered = jobs.iter().filter(|j| matches!(j.state, JobState::Waiting | JobState::NeedsInput)).count();
        let waiting: Vec<JobId> = jobs.iter().filter(|j| j.state == JobState::Waiting).map(|j| j.id.clone()).collect();
        let manifest = Manifest::new(cfg.manifest_path.clone());
        let inner = Arc::new(Inner {
            cfg,
            caps: RwLock::new(Arc::new(caps)),
            state: Mutex::new(State {
                jobs,
                paused: was_paused || recovered > 0,
                concurrency: Concurrency::Auto,
                running: HashMap::new(),
                last_emit: HashMap::new(),
                blocked: None,
                recovered,
                remove_after: HashSet::new(),
            }),
            sink,
            wake: Notify::new(),
            persist: Notify::new(),
            manifest,
            shutdown: CancellationToken::new(),
            preplan: Semaphore::new(2),
            selftest: tokio::sync::Mutex::new(()),
            save_lock: Mutex::new(()),
        });
        inner.save_now();
        tokio::spawn(scheduler_loop(inner.clone()));
        tokio::spawn(persist_loop(inner.clone()));
        tokio::spawn(manifest_loop(inner.clone()));
        for id in waiting {
            inner.preplan(id);
        }
        inner.emit_status();
        Engine { inner }
    }

    /// Queue jobs; each keeps its own settings snapshot.
    pub fn enqueue(&self, specs: Vec<JobSpec>) -> Vec<JobId> {
        let mut ids = Vec::new();
        let mut created = Vec::new();
        {
            let mut st = self.inner.lock();
            for spec in specs {
                let id = uuid::Uuid::new_v4().to_string();
                let job = Job::new(id.clone(), spec, now_ms());
                created.push(job.clone());
                st.jobs.push(job);
                ids.push(id);
            }
        }
        for j in &created {
            self.inner.sink.job(j);
        }
        for id in &ids {
            self.inner.preplan(id.clone());
        }
        self.inner.changed();
        ids
    }

    pub fn jobs(&self) -> Vec<Job> {
        self.inner.lock().jobs.clone()
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.inner.lock().jobs.iter().find(|j| j.id == id).cloned()
    }

    pub fn status(&self) -> QueueStatus {
        self.inner.status()
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.lock().paused = paused;
        self.inner.changed();
    }

    /// Cancel a waiting or running job. Running backends are killed and their temp files removed.
    pub fn cancel(&self, id: &str) -> bool {
        let snapshot = {
            let mut st = self.inner.lock();
            if let Some(r) = st.running.get(id) {
                *lock(&r.reason) = Some(Stop::User);
                r.cancel.cancel();
                return true;
            }
            let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) else { return false };
            if j.transition(JobState::Cancelled).is_err() {
                return false;
            }
            j.finished_at = Some(now_ms());
            j.clone()
        };
        self.inner.sink.job(&snapshot);
        self.inner.changed();
        true
    }

    pub fn retry(&self, id: &str) -> bool {
        let ok = self
            .inner
            .update(id, true, |j| {
                if matches!(j.state, JobState::Failed | JobState::Cancelled) && j.transition(JobState::Waiting).is_ok() {
                    j.error = None;
                    j.issues.clear();
                    j.verify = None;
                    j.progress = 0.0;
                    j.stats = LiveStats::default();
                    j.output = None;
                    j.output_size = None;
                    j.interrupted = false;
                    j.finished_at = None;
                }
            })
            .map(|j| j.state == JobState::Waiting)
            .unwrap_or(false);
        if ok {
            self.inner.preplan(id.to_string());
            self.inner.changed();
        }
        ok
    }

    /// Remove jobs from the queue. Running ones are cancelled first.
    pub fn remove(&self, ids: &[JobId]) -> usize {
        let mut removed = Vec::new();
        {
            let mut st = self.inner.lock();
            for id in ids {
                if let Some(r) = st.running.get(id).cloned() {
                    *lock(&r.reason) = Some(Stop::User);
                    r.cancel.cancel();
                    st.remove_after.insert(id.clone());
                } else if let Some(pos) = st.jobs.iter().position(|j| &j.id == id) {
                    st.jobs.remove(pos);
                    removed.push(id.clone());
                }
            }
        }
        if !removed.is_empty() {
            self.inner.sink.removed(&removed);
        }
        self.inner.changed();
        removed.len()
    }

    pub fn move_job(&self, id: &str, to: usize) -> bool {
        {
            let mut st = self.inner.lock();
            let Some(from) = st.jobs.iter().position(|j| j.id == id) else { return false };
            let job = st.jobs.remove(from);
            let to = to.min(st.jobs.len());
            st.jobs.insert(to, job);
        }
        self.inner.changed();
        true
    }

    /// Remove all verified, finished jobs from the list (their outputs stay on disk).
    pub fn clear_finished(&self) -> usize {
        let ids: Vec<JobId> = self.inner.lock().jobs.iter().filter(|j| j.state == JobState::Done).map(|j| j.id.clone()).collect();
        self.remove(&ids)
    }

    /// Apply a fix from an issue to this job's own snapshot and queue it again.
    pub fn resolve(&self, id: &str, fix: &str) -> bool {
        let mut ok = false;
        self.inner.update(id, true, |j| {
            if !matches!(j.state, JobState::NeedsInput | JobState::Waiting | JobState::Failed) {
                return;
            }
            let source_kbps = j.source.as_ref().and_then(|s| s.duration.filter(|d| *d > 0.0).map(|d| (s.size as f64 * 8.0 / d / 1000.0) as u32));
            ok = match &mut j.spec {
                JobSpec::Convert { settings, .. } if fix == "useBitrate" => {
                    settings.video.rate = RateControl::Bitrate { kbps: source_kbps.unwrap_or(4000).max(100), two_pass: false };
                    true
                }
                JobSpec::Convert { settings, .. } => plan::apply_fix(settings, fix, None),
                JobSpec::Mux(m) if fix == "useMkv" => {
                    m.output.container = Container::Mkv;
                    true
                }
                _ => false,
            };
            if ok {
                j.issues.clear();
                j.error = None;
                if j.state != JobState::Waiting {
                    ok = j.transition(JobState::Waiting).is_ok();
                }
            }
        });
        if ok {
            self.inner.preplan(id.to_string());
            self.inner.changed();
        }
        ok
    }

    pub fn set_concurrency(&self, c: Concurrency) {
        self.inner.lock().concurrency = c;
        self.inner.changed();
    }

    pub fn set_caps(&self, caps: Capabilities) {
        *self.inner.caps.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(caps);
        self.inner.sink.caps(&self.inner.caps());
        self.inner.changed();
    }

    pub fn caps(&self) -> Arc<Capabilities> {
        self.inner.caps()
    }

    pub fn manifest_status(&self) -> ManifestStatus {
        self.inner.manifest.status()
    }

    pub fn flush_manifest(&self) -> ManifestStatus {
        let st = self.inner.manifest.flush();
        self.inner.sink.manifest(&st);
        st
    }

    /// Run the lossless self-test for a pair now (UI pre-check).
    pub async fn ensure_lossless(&self, encoder: &str, pix_fmt: &str) -> Option<LosslessCheck> {
        self.inner.ensure_lossless(encoder, pix_fmt).await
    }

    /// App exit: stop running jobs (they restart from the beginning next time),
    /// delete their temp files, persist the queue.
    pub async fn shutdown(&self) {
        let runs: Vec<Arc<Running>> = {
            let mut st = self.inner.lock();
            st.paused = true;
            st.running.values().cloned().collect()
        };
        for r in &runs {
            *lock(&r.reason) = Some(Stop::Shutdown);
            r.cancel.cancel();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.inner.lock().running.is_empty() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.inner.shutdown.cancel();
        self.inner.save_now();
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Inner {
    cfg: EngineConfig,
    caps: RwLock<Arc<Capabilities>>,
    state: Mutex<State>,
    sink: Arc<dyn EventSink>,
    wake: Notify,
    persist: Notify,
    manifest: Manifest,
    shutdown: CancellationToken,
    preplan: Semaphore,
    selftest: tokio::sync::Mutex<()>,
    /// Serialises queue saves so the newest snapshot is always the one written last.
    save_lock: Mutex<()>,
}

struct State {
    jobs: Vec<Job>,
    paused: bool,
    concurrency: Concurrency,
    running: HashMap<JobId, Arc<Running>>,
    last_emit: HashMap<JobId, Instant>,
    blocked: Option<String>,
    recovered: usize,
    remove_after: HashSet<JobId>,
}

struct Running {
    cancel: CancellationToken,
    light: bool,
    threads: u32,
    reason: Mutex<Option<Stop>>,
    eta: Mutex<Option<f64>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stop {
    User,
    Shutdown,
}

enum Fail {
    /// Cancelled by the user or stopped by app exit (see `Running::reason`).
    Stopped,
    Skipped(PathBuf),
    Needs(Vec<Issue>),
    Error(Notice, JobState, Option<String>),
    Verify(VerifyReport),
}

fn err(code: &str, stage: JobState) -> Fail {
    Fail::Error(Notice::new(code), stage, None)
}

/// Map backend stderr to a user-facing reason.
fn classify(code: Option<i32>, signal: Option<i32>, tail: &[String]) -> Notice {
    let text = tail.join("\n");
    let has = |s: &str| text.contains(s);
    if has("No space left on device") {
        Notice::new("diskFull")
    } else if has("Permission denied") || has("Operation not permitted") || has("Read-only file system") {
        Notice::new("permissionDenied")
    } else if has("Invalid data found") || has("moov atom not found") || has("Error while decoding") || has("corrupt") || has("Invalid NAL") || has("does not contain any stream") || has("EBML header parsing failed") {
        Notice::new("inputCorrupt")
    } else if has("Unknown encoder") || has("Encoder not found") {
        Notice::new("encoderUnavailable")
    } else if has("Error while opening encoder") || has("Could not open encoder") || has("Error initializing output stream") || has("Generic error in an external library") {
        Notice::new("encoderInitFailed")
    } else if let Some(sig) = signal {
        Notice::new("backendCrashed").with("signal", sig)
    } else {
        Notice::new("encodeFailed").with("code", code.map(|c| c.to_string()).unwrap_or_default())
    }
}

fn kill_orphan(pid: u32) {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    let p = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[p]), true);
    if let Some(proc_) = sys.process(p) {
        // PIDs are reused; only ever kill something that is actually an ffmpeg backend.
        if proc_.name().to_string_lossy().to_lowercase().contains("ffmpeg") {
            proc_.kill();
        }
    }
}

fn prune_logs(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|age| age > LOG_RETENTION).unwrap_or(false);
        if old && p.extension().map(|x| x == "log").unwrap_or(false) {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn is_light(j: &Job) -> bool {
    if let Some(p) = &j.plan {
        return p.hw || !p.reencode_video;
    }
    match &j.spec {
        JobSpec::Convert { settings, .. } => settings.video.mode == VideoMode::Copy || settings.video.speed == Speed::Gpu,
        _ => true,
    }
}

async fn scheduler_loop(inner: Arc<Inner>) {
    loop {
        inner.schedule();
        tokio::select! {
            _ = inner.wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = inner.shutdown.cancelled() => break,
        }
    }
}

async fn persist_loop(inner: Arc<Inner>) {
    loop {
        tokio::select! {
            _ = inner.persist.notified() => {}
            _ = inner.shutdown.cancelled() => break,
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        inner.save_now();
    }
}

async fn manifest_loop(inner: Arc<Inner>) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
            _ = inner.shutdown.cancelled() => break,
        }
        if inner.manifest.status().pending > 0 {
            let st = inner.manifest.flush();
            inner.sink.manifest(&st);
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    fn caps(&self) -> Arc<Capabilities> {
        self.caps.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn job(&self, id: &str) -> Option<Job> {
        self.lock().jobs.iter().find(|j| j.id == id).cloned()
    }

    /// Mutate a job, emit it, optionally persist.
    fn update(&self, id: &str, persist: bool, f: impl FnOnce(&mut Job)) -> Option<Job> {
        let snapshot = {
            let mut st = self.lock();
            let job = st.jobs.iter_mut().find(|j| j.id == id)?;
            f(job);
            let s = job.clone();
            st.last_emit.insert(id.to_string(), Instant::now());
            s
        };
        self.sink.job(&snapshot);
        if persist {
            self.persist.notify_one();
        }
        Some(snapshot)
    }

    /// Progress updates are throttled so the UI never jitters or floods.
    fn progress(&self, id: &str, force: bool, f: impl FnOnce(&mut Job)) {
        let snapshot = {
            let mut st = self.lock();
            let Some(job) = st.jobs.iter_mut().find(|j| j.id == id) else { return };
            let before = job.state;
            f(job);
            let changed = job.state != before;
            let s = job.clone();
            let due = st.last_emit.get(id).map(|t| t.elapsed() >= EMIT_INTERVAL).unwrap_or(true);
            if !(due || force || changed) {
                return;
            }
            st.last_emit.insert(id.to_string(), Instant::now());
            s
        };
        self.sink.job(&snapshot);
    }

    fn changed(&self) {
        self.persist.notify_one();
        self.wake.notify_one();
        self.emit_status();
    }

    fn limits(&self, st: &State) -> (u32, u32, u32) {
        let caps = self.caps();
        let logical = caps.cpu.logical.max(1);
        match st.concurrency {
            Concurrency::Fixed { n } => {
                let n = n.clamp(1, 16);
                (n, 0, (logical / n).max(2))
            }
            Concurrency::Auto => {
                let cpu = if logical >= 16 && caps.memory_mb >= 16_000 { 2 } else { 1 };
                (cpu, 1, (logical / cpu).max(2))
            }
        }
    }

    fn status(&self) -> QueueStatus {
        let st = self.lock();
        let (cpu_slots, light_slots, threads) = self.limits(&st);
        let count = |s: JobState| st.jobs.iter().filter(|j| j.state == s).count();
        QueueStatus {
            paused: st.paused,
            running: st.running.len(),
            waiting: count(JobState::Waiting),
            needs_input: count(JobState::NeedsInput),
            done: count(JobState::Done),
            failed: count(JobState::Failed),
            concurrency: st.concurrency,
            cpu_slots,
            light_slots,
            threads_per_job: threads,
            blocked: st.blocked.clone(),
            order: st.jobs.iter().map(|j| j.id.clone()).collect(),
            recovered: st.recovered,
        }
    }

    fn emit_status(&self) {
        let s = self.status();
        self.sink.queue(&s);
    }

    fn save_now(&self) {
        let _serial = lock(&self.save_lock);
        let q = {
            let st = self.lock();
            QueueFile { version: QUEUE_VERSION, paused: st.paused, jobs: st.jobs.clone() }
        };
        if let Ok(bytes) = serde_json::to_vec(&q) {
            // A failed save is retried on the next change; it never stops the queue.
            let _ = store::save_atomic(&self.cfg.data_dir.join("queue.json"), &bytes);
        }
    }

    fn schedule(self: &Arc<Self>) {
        if self.shutdown.is_cancelled() {
            return;
        }
        let mut to_start: Vec<(JobId, Arc<Running>)> = Vec::new();
        let mut started_snapshots = Vec::new();
        {
            let mut st = self.lock();
            if st.paused {
                return;
            }
            let (cpu_slots, light_slots, threads) = self.limits(&st);
            let logical = self.caps().cpu.logical.max(1);
            let mut cpu_running = st.running.values().filter(|r| !r.light).count() as u32;
            let mut light_running = st.running.values().filter(|r| r.light).count() as u32;
            let available_mb = caps::memory_mb().1;
            let mut blocked = None;
            let candidates: Vec<(JobId, bool, u64)> = st
                .jobs
                .iter()
                .filter(|j| j.state == JobState::Waiting && !st.running.contains_key(&j.id))
                .map(|j| (j.id.clone(), is_light(j), j.plan.as_ref().and_then(|p| p.est_memory_mb).unwrap_or(600)))
                .collect();
            let mut cpu_blocked = false;
            for (id, light, mem) in candidates {
                let use_light = light && light_running < light_slots;
                if !use_light && (cpu_blocked || cpu_running >= cpu_slots) {
                    continue;
                }
                let busy = cpu_running + light_running > 0;
                if busy && available_mb < mem + RESERVE_MEM_MB {
                    blocked = Some("memory".to_string());
                    if !use_light {
                        cpu_blocked = true; // keep queue order for CPU jobs
                    }
                    continue;
                }
                let run = Arc::new(Running {
                    cancel: CancellationToken::new(),
                    light: use_light,
                    threads: if use_light { logical.min(4) } else { threads },
                    reason: Mutex::new(None),
                    eta: Mutex::new(None),
                });
                if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
                    if j.transition(JobState::Probing).is_err() {
                        continue;
                    }
                    j.started_at = Some(now_ms());
                    started_snapshots.push(j.clone());
                }
                st.running.insert(id.clone(), run.clone());
                if use_light {
                    light_running += 1;
                } else {
                    cpu_running += 1;
                }
                to_start.push((id, run));
            }
            st.blocked = blocked;
        }
        for j in &started_snapshots {
            self.sink.job(j);
        }
        if !to_start.is_empty() {
            self.persist.notify_one();
            self.emit_status();
        }
        for (id, run) in to_start {
            self.spawn_worker(id, run);
        }
    }

    fn spawn_worker(self: &Arc<Self>, id: JobId, run: Arc<Running>) {
        let inner = self.clone();
        tokio::spawn(async move {
            let (i2, id2, r2) = (inner.clone(), id.clone(), run.clone());
            let res = tokio::spawn(async move { i2.pipeline(&id2, &r2).await }).await;
            if let Err(e) = res {
                // A panic in one job must not take the queue down.
                let detail = if e.is_panic() { "worker panicked".to_string() } else { e.to_string() };
                inner.finish(&id, Err(Fail::Error(Notice::new("internalError"), JobState::Failed, Some(detail))), &run, Duration::ZERO).await;
            }
            let removed = {
                let mut st = inner.lock();
                st.running.remove(&id);
                if st.remove_after.remove(&id) {
                    st.jobs.retain(|j| j.id != id);
                    true
                } else {
                    false
                }
            };
            if removed {
                inner.sink.removed(std::slice::from_ref(&id));
            }
            inner.changed();
        });
    }

    async fn pipeline(self: &Arc<Self>, id: &str, run: &Arc<Running>) {
        let log = self.cfg.data_dir.join("logs").join(format!("{id}.log"));
        let Some(job) = self.update(id, true, |j| {
            j.attempts += 1;
            j.finished_at = None;
            j.error = None;
            j.verify = None;
            j.issues.clear();
            j.progress = 0.0;
            j.stats = LiveStats::default();
            j.output = None;
            j.output_size = None;
            j.interrupted = false;
            j.log_path = Some(log.clone());
        }) else {
            return;
        };
        let work_dir = self.cfg.work_root.join(id);
        let _ = std::fs::create_dir_all(&work_dir);
        let started = Instant::now();
        let result = match &job.spec {
            JobSpec::Convert { input, settings } => self.run_convert(id, input, settings, run, &work_dir, &log).await,
            JobSpec::Extract(e) => self.run_extract(id, e, run, &work_dir, &log).await,
            JobSpec::Mux(m) => self.run_mux(id, m, run, &work_dir, &log).await,
        };
        self.finish(id, result, run, started.elapsed()).await;
        let _ = std::fs::remove_dir_all(&work_dir);
    }

    fn check_backend(&self) -> Result<(), Fail> {
        let ff = &self.cfg.ffmpeg;
        if ff.ffmpeg.is_file() && ff.ffprobe.is_file() {
            Ok(())
        } else {
            Err(Fail::Error(Notice::new("backendMissing").with("path", ff.ffmpeg.display()), JobState::Probing, None))
        }
    }

    fn check_stop(&self, run: &Running) -> Result<(), Fail> {
        if run.cancel.is_cancelled() { Err(Fail::Stopped) } else { Ok(()) }
    }

    fn transition(&self, id: &str, to: JobState) -> Result<(), Fail> {
        let mut result = Ok(());
        self.update(id, true, |j| {
            if j.state != to
                && let Err(e) = j.transition(to)
            {
                result = Err(Fail::Error(Notice::new("internalError").with("reason", e), to, None));
            }
        });
        self.emit_status();
        result
    }

    async fn probe_input(&self, path: &Path) -> Result<MediaInfo, Fail> {
        media::probe(&self.cfg.ffmpeg.ffprobe, path).await.map_err(|e| {
            let detail = match &e {
                media::ProbeError::Backend(d) | media::ProbeError::Invalid(d) | media::ProbeError::Unreadable(d) => Some(d.clone()),
                media::ProbeError::NotFound => None,
            };
            Fail::Error(Notice::new(e.code()).with("path", path.display()), JobState::Probing, detail)
        })
    }

    fn reserved_outputs(&self, id: &str) -> HashSet<PathBuf> {
        self.lock().jobs.iter().filter(|j| j.id != id && !j.state.is_terminal()).filter_map(|j| j.planned_output.clone()).collect()
    }

    async fn ensure_lossless(&self, encoder: &str, pix_fmt: &str) -> Option<LosslessCheck> {
        let _guard = self.selftest.lock().await;
        if let Some(c) = self.caps().lossless.iter().find(|c| c.encoder == encoder && c.pix_fmt == pix_fmt) {
            return Some(c.clone());
        }
        let def = crate::encoders::get(encoder)?;
        let check = caps::lossless_self_test(&self.cfg.ffmpeg.ffmpeg, def, pix_fmt, &self.cfg.work_root.join("selftest")).await;
        {
            let mut w = self.caps.write().unwrap_or_else(|e| e.into_inner());
            let mut c = (**w).clone();
            c.lossless.push(check.clone());
            *w = Arc::new(c);
        }
        self.sink.caps(&self.caps());
        Some(check)
    }

    async fn run_convert(self: &Arc<Self>, id: &str, input: &Path, settings: &JobSettings, run: &Arc<Running>, work_dir: &Path, log: &Path) -> Result<PathBuf, Fail> {
        self.check_backend()?;
        let media = self.probe_input(input).await?;
        self.update(id, false, |j| j.source = Some(media.summary()));
        self.check_stop(run)?;
        let keyframes = if settings.video.mode == VideoMode::Copy && settings.trim.is_some() {
            let idx = media.main_video().map(|v| v.index).unwrap_or(0);
            media::keyframes(&self.cfg.ffmpeg.ffprobe, input, idx).await.ok()
        } else {
            None
        };
        let (codec_t, mode_t) = plan::naming_tokens(settings, &self.caps());
        let date = chrono::Local::now().format("%Y%m%d").to_string();
        let desired = fsx::plan_output_path(input, &settings.output, &codec_t, &mode_t, &date, &self.reserved_outputs(id));
        let temp = fsx::temp_path_for(&desired, id);
        let mut tested = false;
        let plan = loop {
            let caps = self.caps();
            let env = PlanEnv { caps: &caps, threads: run.threads, temp_output: &temp, work_dir, keyframes: keyframes.as_deref() };
            match plan::plan_convert(settings, &media, &env) {
                Ok(p) => break p,
                Err(PlanError::Blocked(issues)) => return Err(Fail::Needs(issues)),
                Err(PlanError::NeedsSelfTest { encoder, pix_fmt }) => {
                    if tested {
                        return Err(Fail::Error(Notice::new("losslessNotVerified").with("encoder", encoder).with("pixFmt", pix_fmt), JobState::Probing, None));
                    }
                    tested = true;
                    self.ensure_lossless(&encoder, &pix_fmt).await;
                }
            }
        };
        self.execute(id, plan, &media, vec![input.to_path_buf()], &desired, &temp, settings.output.conflict, run, work_dir, log).await
    }

    async fn run_extract(self: &Arc<Self>, id: &str, spec: &ExtractSpec, run: &Arc<Running>, work_dir: &Path, log: &Path) -> Result<PathBuf, Fail> {
        self.check_backend()?;
        let media = self.probe_input(&spec.input).await?;
        self.update(id, false, |j| j.source = Some(media.summary()));
        let Some((kind, codec)) = plan::stream_kind(&media, spec.stream_index) else {
            return Err(Fail::Needs(vec![Issue::new(Notice::new("streamMissing").with("stream", spec.stream_index))]));
        };
        let (ext, _, _) = plan::extract_target(kind, &codec);
        let stem = spec.input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let lang = media
            .audio
            .iter()
            .find(|a| a.index == spec.stream_index)
            .and_then(|a| a.language.clone())
            .or_else(|| media.subtitles.iter().find(|s| s.index == spec.stream_index).and_then(|s| s.language.clone()))
            .map(|l| format!("_{l}"))
            .unwrap_or_default();
        let name = fsx::sanitize_file_name(&format!("{stem}_{kind}{}{lang}", spec.stream_index));
        let dir = fsx::output_dir_for(&spec.input, &spec.output);
        let desired = fsx::plan_named_output(&spec.input, &dir, &name, ext, spec.output.conflict, &self.reserved_outputs(id));
        let temp = fsx::temp_path_for(&desired, id);
        let caps = self.caps();
        let env = PlanEnv { caps: &caps, threads: 1, temp_output: &temp, work_dir, keyframes: None };
        let plan = plan::plan_extract(spec, &media, &env).map_err(|e| match e {
            PlanError::Blocked(i) => Fail::Needs(i),
            PlanError::NeedsSelfTest { .. } => err("internalError", JobState::Probing),
        })?;
        self.execute(id, plan, &media, vec![spec.input.clone()], &desired, &temp, spec.output.conflict, run, work_dir, log).await
    }

    async fn run_mux(self: &Arc<Self>, id: &str, spec: &MuxSpec, run: &Arc<Running>, work_dir: &Path, log: &Path) -> Result<PathBuf, Fail> {
        self.check_backend()?;
        let main = self.probe_input(&spec.main).await?;
        self.update(id, false, |j| j.source = Some(main.summary()));
        let mut tracks = Vec::new();
        for t in &spec.tracks {
            tracks.push(self.probe_input(&t.path).await?);
        }
        let date = chrono::Local::now().format("%Y%m%d").to_string();
        let desired = fsx::plan_output_path(&spec.main, &spec.output, "mux", "copy", &date, &self.reserved_outputs(id));
        let temp = fsx::temp_path_for(&desired, id);
        let caps = self.caps();
        let env = PlanEnv { caps: &caps, threads: 1, temp_output: &temp, work_dir, keyframes: None };
        let plan = plan::plan_mux(spec, &main, &tracks, &env).map_err(|e| match e {
            PlanError::Blocked(i) => Fail::Needs(i),
            PlanError::NeedsSelfTest { .. } => err("internalError", JobState::Probing),
        })?;
        let mut inputs = vec![spec.main.clone()];
        inputs.extend(spec.tracks.iter().map(|t| t.path.clone()));
        self.execute(id, plan, &main, inputs, &desired, &temp, spec.output.conflict, run, work_dir, log).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute(
        self: &Arc<Self>,
        id: &str,
        plan: EncodePlan,
        media: &MediaInfo,
        inputs: Vec<PathBuf>,
        desired: &Path,
        temp: &Path,
        policy: ConflictPolicy,
        run: &Arc<Running>,
        work_dir: &Path,
        log: &Path,
    ) -> Result<PathBuf, Fail> {
        let ff = &self.cfg.ffmpeg;
        if policy == ConflictPolicy::Skip && desired.symlink_metadata().is_ok() {
            return Err(Fail::Skipped(desired.to_path_buf()));
        }
        self.update(id, true, |j| {
            j.plan = Some(plan.summary.clone());
            j.warnings = plan.warnings.clone();
            j.planned_output = Some(desired.to_path_buf());
        });
        let out_dir = desired.parent().unwrap_or(Path::new(".")).to_path_buf();
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            return Err(Fail::Error(Notice::new("outputDirFailed").with("dir", out_dir.display()).with("reason", e), JobState::Probing, None));
        }
        if let Some(free) = fsx::available_space(&out_dir)
            && free < plan.est_upper_bytes + RESERVE_DISK
        {
            let n = Notice::new("diskSpace").with("needed", plan.est_upper_bytes + RESERVE_DISK).with("free", free);
            return Err(Fail::Error(n, JobState::Probing, None));
        }
        // Record the temp file before the backend can create it, so a crash can always clean it up.
        self.update(id, true, |j| j.temp_files = vec![temp.to_path_buf()]);
        self.save_now();

        for step in &plan.prep {
            self.check_stop(run)?;
            let prep_err = |e: String| Fail::Error(Notice::new("prepFailed"), JobState::Probing, Some(e));
            match step {
                Prep::CopyFile { from, to } => {
                    std::fs::copy(from, to).map_err(|e| prep_err(e.to_string()))?;
                }
                Prep::ExtractSubtitle { stream_index, to } => {
                    let args: Vec<String> = vec![
                        "-hide_banner".into(), "-nostdin".into(), "-v".into(), "error".into(), "-y".into(), "-i".into(),
                        media::ff_path(&media.path), "-map".into(), format!("0:{stream_index}"), "-c:s".into(), "ass".into(),
                        media::ff_path(to),
                    ];
                    let opts = RunOptions { program: &ff.ffmpeg, args: &args, cwd: Some(work_dir), log_path: Some(log), stall_limit: None };
                    match process::run_ffmpeg(opts, &run.cancel, |_| {}, |_| {}).await {
                        Ok(Outcome::Success) => {}
                        Ok(Outcome::Cancelled) => return Err(Fail::Stopped),
                        Ok(Outcome::Failed { tail, .. } | Outcome::Stalled { tail }) => return Err(prep_err(tail.join("\n"))),
                        Err(e) => return Err(prep_err(e.to_string())),
                    }
                }
            }
        }

        let started = Instant::now();
        let mut done_w = 0.0;
        let last = plan.passes.len().saturating_sub(1);
        for (i, pass) in plan.passes.iter().enumerate() {
            self.check_stop(run)?;
            self.transition(id, pass.stage)?;
            self.update(id, false, |j| j.stats.pass = pass.label.clone());
            let stall_limit = self.cfg.stall_limit.unwrap_or_else(|| stall_limit_for(plan.encoder.as_deref()));
            // A hung backend (seen: SVT-AV1 internal deadlock) is killed and the same step
            // retried once with identical settings; a second hang fails the job.
            let mut retried = false;
            loop {
                let pass_cancel = run.cancel.child_token();
                let disk_full = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let monitor = {
                    let (dir, token, flag) = (out_dir.clone(), pass_cancel.clone(), disk_full.clone());
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(Duration::from_secs(3)).await;
                            if fsx::available_space(&dir).map(|f| f < MIN_FREE_WHILE_RUNNING).unwrap_or(false) {
                                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                                token.cancel();
                                break;
                            }
                        }
                    })
                };
                let opts = RunOptions { program: &ff.ffmpeg, args: &pass.args, cwd: Some(work_dir), log_path: Some(log), stall_limit: Some(stall_limit) };
                let base = done_w;
                let outcome = process::run_ffmpeg(
                    opts,
                    &pass_cancel,
                    |pid| {
                        self.update(id, true, |j| j.pid = Some(pid));
                        self.save_now();
                    },
                    |p| self.on_progress(id, run, p, base, pass.weight, plan.duration, started, i == last),
                )
                .await;
                monitor.abort();
                self.update(id, true, |j| j.pid = None);
                match outcome {
                    Err(e) => return Err(Fail::Error(Notice::new("backendMissing").with("reason", e), pass.stage, None)),
                    Ok(Outcome::Success) => break,
                    Ok(Outcome::Stalled { .. }) if !retried && !run.cancel.is_cancelled() => {
                        retried = true;
                        // The partial output of the hung attempt would make `-n` refuse to start.
                        let _ = fsx::remove_owned_temp(temp);
                        let minutes = (stall_limit.as_secs() / 60).max(1);
                        self.update(id, true, |j| j.warnings.push(Notice::new("backendHungRetried").with("minutes", minutes)));
                    }
                    Ok(Outcome::Stalled { tail }) => {
                        let n = Notice::new("backendHung").with("minutes", (stall_limit.as_secs() / 60).max(1));
                        return Err(Fail::Error(n, pass.stage, Some(tail.join("\n"))));
                    }
                    Ok(Outcome::Cancelled) if disk_full.load(std::sync::atomic::Ordering::SeqCst) => return Err(err("diskFull", pass.stage)),
                    Ok(Outcome::Cancelled) => return Err(Fail::Stopped),
                    Ok(Outcome::Failed { code, signal, tail }) => {
                        if run.cancel.is_cancelled() {
                            return Err(Fail::Stopped);
                        }
                        return Err(Fail::Error(classify(code, signal, &tail), pass.stage, Some(tail.join("\n"))));
                    }
                }
            }
            done_w += pass.weight;
        }

        self.transition(id, JobState::Verifying)?;
        let vw = plan.verify_weight;
        let ctx = VerifyCtx {
            ffmpeg: &ff.ffmpeg,
            ffprobe: &ff.ffprobe,
            source: media,
            inputs: &inputs,
            output: temp,
            work_dir,
            log_path: Some(log),
            cancel: &run.cancel,
        };
        let report = match verify::verify(&plan.expected, &ctx, |f| {
            self.progress(id, false, |j| {
                j.progress = (done_w + vw * f).min(0.999).max(j.progress);
                j.stats.eta_secs = None;
                j.stats.pass = None;
            })
        })
        .await
        {
            Ok(r) => r,
            Err(VerifyAbort::Cancelled) => return Err(Fail::Stopped),
        };
        if !report.ok {
            return Err(Fail::Verify(report));
        }
        self.update(id, false, |j| j.verify = Some(report));
        match fsx::commit(temp, desired, policy) {
            Ok(p) => Ok(p),
            Err(fsx::CommitError::Skipped(p)) => Err(Fail::Skipped(p)),
            Err(fsx::CommitError::Io(e)) => Err(Fail::Error(Notice::new("commitFailed").with("reason", e), JobState::Verifying, None)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_progress(&self, id: &str, run: &Running, p: &process::Progress, base: f64, weight: f64, duration: Option<f64>, started: Instant, last_pass: bool) {
        let frac = match (p.out_time_us, duration) {
            (Some(us), Some(d)) if d > 0.0 => (us as f64 / 1e6 / d).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let overall = (base + weight * frac).min(0.999);
        let elapsed = started.elapsed().as_secs_f64();
        let eta = if overall > 0.02 && elapsed > 1.5 {
            let raw = elapsed * (1.0 - overall) / overall;
            let mut e = lock(&run.eta);
            let v = e.map(|prev| prev * 0.8 + raw * 0.2).unwrap_or(raw);
            *e = Some(v);
            Some(v)
        } else {
            None
        };
        let p = p.clone();
        self.progress(id, p.end, |j| {
            j.progress = overall.max(j.progress);
            j.stats.fps = p.fps;
            j.stats.speed = p.speed;
            j.stats.out_time = p.out_time_us.map(|u| u as f64 / 1e6);
            j.stats.size_bytes = p.total_size;
            j.stats.eta_secs = eta;
            // FFmpeg has finished encoding; it is now finalising the container (e.g. faststart).
            if p.end && last_pass && j.state == JobState::Encoding {
                let _ = j.transition(JobState::Muxing);
            }
        });
    }

    async fn finish(self: &Arc<Self>, id: &str, result: Result<PathBuf, Fail>, run: &Running, elapsed: Duration) {
        let stop = *lock(&run.reason);
        let Some(job) = self.job(id) else { return };
        let cleanup = |j: &mut Job| {
            for t in &j.temp_files {
                let _ = fsx::remove_owned_temp(t);
            }
            j.temp_files.clear();
            j.pid = None;
            j.stats.eta_secs = None;
        };
        let stage = job.state;
        let mut manifest_status: Option<&str> = None;
        match result {
            Ok(path) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).ok();
                self.update(id, true, |j| {
                    let _ = j.transition(JobState::Done);
                    j.output = Some(path.clone());
                    j.output_size = size;
                    j.progress = 1.0;
                    j.temp_files.clear();
                });
                manifest_status = Some("完成");
            }
            Err(Fail::Stopped) if stop == Some(Stop::Shutdown) => {
                self.update(id, true, |j| {
                    cleanup(j);
                    let _ = j.transition(JobState::Waiting);
                    j.interrupted = true;
                    j.progress = 0.0;
                });
            }
            Err(Fail::Stopped) => {
                self.update(id, true, |j| {
                    cleanup(j);
                    let _ = j.transition(JobState::Cancelled);
                });
                manifest_status = Some("已取消");
            }
            Err(Fail::Skipped(p)) => {
                self.update(id, true, |j| {
                    cleanup(j);
                    let _ = j.transition(JobState::Cancelled);
                    j.error = Some(JobError { notice: Notice::new("skippedExisting").with("path", p.display()), stage, detail: None });
                });
                manifest_status = Some("已跳过");
            }
            Err(Fail::Needs(issues)) => {
                self.update(id, true, |j| {
                    cleanup(j);
                    if j.transition(JobState::NeedsInput).is_err() {
                        let _ = j.transition(JobState::Failed);
                    }
                    j.issues = issues;
                });
            }
            Err(Fail::Error(notice, st, detail)) => {
                self.update(id, true, |j| {
                    cleanup(j);
                    let _ = j.transition(JobState::Failed);
                    j.error = Some(JobError { notice, stage: if st == JobState::Failed { stage } else { st }, detail });
                });
                manifest_status = Some("失败");
            }
            Err(Fail::Verify(report)) => {
                let failed: Vec<String> = report.checks.iter().filter(|c| !c.ok).map(|c| c.name.clone()).collect();
                self.update(id, true, |j| {
                    cleanup(j);
                    let _ = j.transition(JobState::Failed);
                    j.error = Some(JobError { notice: Notice::new("verifyFailed").with("checks", failed.join(", ")), stage: JobState::Verifying, detail: None });
                    j.verify = Some(report);
                });
                manifest_status = Some("校验失败");
            }
        }
        let job = self.update(id, true, |j| {
            if j.state.is_terminal() {
                j.finished_at = Some(now_ms());
            }
            j.elapsed_ms = elapsed.as_millis() as u64;
        });
        if let (Some(status), Some(j)) = (manifest_status, job) {
            let st = self.manifest.record(&manifest_row(&j, status));
            self.sink.manifest(&st);
        }
        self.save_now();
    }

    /// Probe + plan right after queueing so problems show up before the job starts.
    fn preplan(self: &Arc<Self>, id: JobId) {
        let inner = self.clone();
        tokio::spawn(async move {
            let Ok(_permit) = inner.preplan.acquire().await else { return };
            let Some(job) = inner.job(&id) else { return };
            if job.state != JobState::Waiting {
                return;
            }
            let JobSpec::Convert { input, settings } = &job.spec else { return };
            if inner.check_backend().is_err() {
                return;
            }
            let media = match media::probe(&inner.cfg.ffmpeg.ffprobe, input).await {
                Ok(m) => m,
                Err(_) => return, // reported properly when the job runs
            };
            let (codec_t, mode_t) = plan::naming_tokens(settings, &inner.caps());
            let date = chrono::Local::now().format("%Y%m%d").to_string();
            let desired = fsx::plan_output_path(input, &settings.output, &codec_t, &mode_t, &date, &inner.reserved_outputs(&id));
            let temp = fsx::temp_path_for(&desired, &id);
            let work = inner.cfg.work_root.join("preplan");
            let threads = {
                let st = inner.lock();
                inner.limits(&st).2
            };
            let mut tested = false;
            loop {
                let caps = inner.caps();
                let env = PlanEnv { caps: &caps, threads, temp_output: &temp, work_dir: &work, keyframes: None };
                match plan::plan_convert(settings, &media, &env) {
                    Ok(p) => {
                        inner.update(&id, true, |j| {
                            if j.state == JobState::Waiting {
                                j.source = Some(media.summary());
                                j.plan = Some(p.summary.clone());
                                j.warnings = p.warnings.clone();
                                j.planned_output = Some(desired.clone());
                            }
                        });
                        break;
                    }
                    Err(PlanError::Blocked(issues)) => {
                        inner.update(&id, true, |j| {
                            if j.state == JobState::Waiting && j.transition(JobState::NeedsInput).is_ok() {
                                j.source = Some(media.summary());
                                j.issues = issues.clone();
                            }
                        });
                        inner.emit_status();
                        break;
                    }
                    Err(PlanError::NeedsSelfTest { encoder, pix_fmt }) if !tested => {
                        tested = true;
                        inner.ensure_lossless(&encoder, &pix_fmt).await;
                    }
                    Err(PlanError::NeedsSelfTest { .. }) => break,
                }
            }
            inner.wake.notify_one();
        });
    }
}

fn manifest_row(j: &Job, status: &str) -> ManifestRow {
    let plan = j.plan.clone().unwrap_or_default();
    let mode = if plan.lossless {
        "无损"
    } else if !plan.reencode_video {
        "流复制"
    } else {
        "有损"
    };
    ManifestRow {
        time: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        status: status.into(),
        source: j.spec.primary_input().display().to_string(),
        output: j.output.as_ref().or(j.planned_output.as_ref()).map(|p| p.display().to_string()).unwrap_or_default(),
        mode: format!("{mode} {}", plan.kind),
        encoder: plan.encoder.clone().unwrap_or_else(|| "copy".into()),
        params: plan.quality.clone().unwrap_or_default(),
        source_size: j.source.as_ref().map(|s| s.size),
        output_size: j.output_size,
        duration_s: j.source.as_ref().and_then(|s| s.duration),
        elapsed_s: j.elapsed_ms as f64 / 1000.0,
        verify: match &j.verify {
            Some(v) if v.ok => "通过".into(),
            Some(_) => "未通过".into(),
            None => "—".into(),
        },
        error: j
            .error
            .as_ref()
            .map(|e| {
                let first = e.detail.as_deref().and_then(|d| d.lines().last()).unwrap_or("");
                format!("{} {}", e.notice.code, first).trim().to_string()
            })
            .unwrap_or_default(),
    }
}
