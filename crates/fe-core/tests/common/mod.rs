//! Shared helpers for integration tests that run the real FFmpeg backend.
#![allow(dead_code)]

use fe_core::caps::{self, Capabilities};
use fe_core::engine::{Engine, EngineConfig, EventSink};
use fe_core::job::{Job, JobState};
use fe_core::manifest::ManifestStatus;
use fe_core::tools::{self, FfmpegPaths};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

pub fn ffmpeg() -> FfmpegPaths {
    let custom = std::env::var_os("FRAMEEASE_FFMPEG_DIR").map(PathBuf::from);
    let bundled = repo_root().join("src-tauri/resources/ffmpeg");
    tools::locate(custom.as_deref(), Some(&bundled), None).expect("FFmpeg not found: set FRAMEEASE_FFMPEG_DIR")
}

/// Generated once per test run (scripts/gen-test-media.sh, synthetic content only).
pub fn media_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = repo_root().join("test-media");
        // Regenerate when any clip added since the folder was made is missing.
        let complete = ["long_720p.mp4", "hdr10.mkv", "edit_list.mov", "incomplete_download.mp4"].iter().all(|f| dir.join(f).exists());
        if !complete {
            let ff = ffmpeg();
            let status = std::process::Command::new("bash")
                .arg(repo_root().join("scripts/gen-test-media.sh"))
                .arg(&dir)
                .arg(ff.ffmpeg.parent().unwrap())
                .status()
                .expect("run gen-test-media.sh");
            assert!(status.success());
        }
        dir
    })
    .clone()
}

pub fn media(name: &str) -> PathBuf {
    media_dir().join(name)
}

/// Capability detection is slow-ish; share one result across tests in a binary.
pub async fn capabilities() -> Capabilities {
    static CAPS: OnceCell<Capabilities> = OnceCell::const_new();
    CAPS.get_or_init(|| async {
        let work = std::env::temp_dir().join(format!("fe-caps-{}", std::process::id()));
        caps::detect(&ffmpeg(), &work).await.expect("detect")
    })
    .await
    .clone()
}

#[derive(Default)]
pub struct Collect {
    pub jobs: Mutex<Vec<Job>>,
    pub manifest: Mutex<Vec<ManifestStatus>>,
}

impl EventSink for Collect {
    fn job(&self, job: &Job) {
        self.jobs.lock().unwrap().push(job.clone());
    }
    fn manifest(&self, s: &ManifestStatus) {
        self.manifest.lock().unwrap().push(s.clone());
    }
}

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub engine: Engine,
    pub sink: Arc<Collect>,
}

impl Rig {
    pub fn data_dir(&self) -> PathBuf {
        self.dir.path().join("data")
    }
    pub fn out_dir(&self) -> PathBuf {
        self.dir.path().join("输出 目录")
    }
    pub fn manifest(&self) -> PathBuf {
        self.data_dir().join("results.csv")
    }
}

pub fn config(dir: &Path, ff: FfmpegPaths) -> EngineConfig {
    EngineConfig {
        ffmpeg: ff,
        data_dir: dir.join("data"),
        work_root: dir.join("work"),
        manifest_path: dir.join("data").join("results.csv"),
        // Short enough that a deadlocked encoder (seen with SVT-AV1 under heavy test
        // parallelism) is detected and retried well within the test timeouts.
        stall_limit: Some(Duration::from_secs(20)),
    }
}

pub async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    rig_in(dir).await
}

pub async fn rig_in(dir: tempfile::TempDir) -> Rig {
    let sink = Arc::new(Collect::default());
    let engine = Engine::start(config(dir.path(), ffmpeg()), capabilities().await, sink.clone()).await;
    engine.set_paused(false);
    Rig { dir, engine, sink }
}

/// Wait until every listed job reached a terminal state.
pub async fn wait_terminal(engine: &Engine, ids: &[String], timeout: Duration) -> Vec<Job> {
    let start = Instant::now();
    loop {
        let jobs: Vec<Job> = ids.iter().map(|id| engine.job(id).expect("job exists")).collect();
        if jobs.iter().all(|j| j.state.is_terminal() || j.state == JobState::NeedsInput) {
            return jobs;
        }
        assert!(start.elapsed() < timeout, "timeout; states: {:?}", jobs.iter().map(|j| (j.name.clone(), j.state)).collect::<Vec<_>>());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn wait_until(engine: &Engine, id: &str, timeout: Duration, pred: impl Fn(&Job) -> bool) -> Job {
    let start = Instant::now();
    loop {
        let j = engine.job(id).expect("job");
        if pred(&j) {
            return j;
        }
        assert!(start.elapsed() < timeout, "timeout waiting; state {:?} progress {}", j.state, j.progress);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn assert_done(j: &Job) {
    assert_eq!(
        j.state,
        JobState::Done,
        "{}: error {:?} issues {:?} warnings {:?} verify {:?}",
        j.name,
        j.error,
        j.issues,
        j.warnings.iter().map(|w| (&w.code, &w.params)).collect::<Vec<_>>(),
        j.verify.as_ref().map(|v| v.checks.iter().filter(|c| !c.ok).collect::<Vec<_>>())
    );
    let out = j.output.as_ref().expect("output path");
    assert!(out.exists(), "output must exist: {}", out.display());
    assert!(j.verify.as_ref().map(|v| v.ok).unwrap_or(false));
}

pub fn check_ok(j: &Job, name: &str) -> bool {
    j.verify.as_ref().map(|v| v.checks.iter().any(|c| c.name == name && c.ok)).unwrap_or(false)
}

pub fn part_files(dir: &Path) -> Vec<PathBuf> {
    walk(dir).into_iter().filter(|p| p.to_string_lossy().ends_with(".part")).collect()
}

pub fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
