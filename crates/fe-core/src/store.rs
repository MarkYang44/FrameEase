//! Crash-safe persistence of the queue (and other small JSON files).
//!
//! Writes go to a temp file that is fsynced and renamed over the target; the
//! previous good version is kept as `.bak`. Loading falls back to the backup
//! and never crashes on corrupt data.

use crate::job::{Job, JobId, JobState};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const QUEUE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueueFile {
    pub version: u32,
    pub paused: bool,
    pub jobs: Vec<Job>,
}

#[derive(Debug)]
pub enum LoadOutcome<T> {
    Loaded(T),
    Missing,
    /// Main file was unreadable; the backup was used.
    FromBackup(T),
    /// Neither file was usable; the corrupt file was moved aside for diagnosis.
    Corrupt { moved_to: Option<PathBuf> },
}

/// Atomically replace `path` with `bytes`, keeping the previous version as `.bak`.
pub fn save_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "state".into());
    // Unique per call: concurrent saves must never share (and truncate) one temp file.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.tmp-{}-{seq}", std::process::id()));
    let write = || -> io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if path.exists() {
        // The backup is replaced atomically too, so it is never half-written.
        let bak_tmp = dir.join(format!(".{name}.bak-{}-{seq}", std::process::id()));
        if std::fs::copy(path, &bak_tmp).is_ok() && std::fs::rename(&bak_tmp, backup_path(path)).is_err() {
            let _ = std::fs::remove_file(&bak_tmp);
        }
    }
    // Windows may briefly refuse the replace while a scanner holds the file.
    let mut last = None;
    for attempt in 0..5 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(30 * (attempt + 1)));
            }
        }
    }
    if let Some(e) = last {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn read_parse<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, ()> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|_| ()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn move_aside(path: &Path) -> Option<PathBuf> {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".corrupt-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let dest = PathBuf::from(s);
    std::fs::rename(path, &dest).ok().map(|_| dest)
}

pub fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> LoadOutcome<T> {
    let backup = backup_path(path);
    match read_parse::<T>(path) {
        Ok(Some(v)) => LoadOutcome::Loaded(v),
        Ok(None) => match read_parse::<T>(&backup) {
            Ok(Some(v)) => LoadOutcome::FromBackup(v),
            _ => LoadOutcome::Missing,
        },
        Err(()) => {
            // Move the bad file aside so the next save cannot copy it over the good backup.
            let moved_to = move_aside(path);
            match read_parse::<T>(&backup) {
                Ok(Some(v)) => LoadOutcome::FromBackup(v),
                _ => LoadOutcome::Corrupt { moved_to },
            }
        }
    }
}

/// After a crash or forced quit: active jobs restart from the beginning.
/// Returns the ids that were interrupted (their temp files must be cleaned).
pub fn recover_interrupted(jobs: &mut [Job]) -> Vec<JobId> {
    let mut ids = Vec::new();
    for j in jobs.iter_mut().filter(|j| j.state.is_active()) {
        j.state = JobState::Waiting;
        j.interrupted = true;
        j.pid = None;
        j.progress = 0.0;
        j.stats = Default::default();
        ids.push(j.id.clone());
    }
    ids
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{JobSettings, JobSpec};
    use std::fs;

    fn job(id: &str, state: JobState) -> Job {
        let spec = JobSpec::Convert { input: PathBuf::from(format!("/视频/{id} 片段.mp4")), settings: JobSettings::default() };
        let mut j = Job::new(id.into(), spec, 1);
        j.state = state;
        j
    }

    fn queue(jobs: Vec<Job>) -> QueueFile {
        QueueFile { version: QUEUE_VERSION, paused: false, jobs }
    }

    #[test]
    fn roundtrip_keeps_jobs_and_unicode_paths() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue.json");
        let q = queue(vec![job("a", JobState::Waiting), job("b", JobState::Done)]);
        save_atomic(&p, &serde_json::to_vec(&q).unwrap()).unwrap();
        match load_json::<QueueFile>(&p) {
            LoadOutcome::Loaded(back) => assert_eq!(back, q),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn save_replaces_previous_and_keeps_backup_without_leftovers() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue.json");
        save_atomic(&p, b"{\"v\":1}").unwrap();
        save_atomic(&p, b"{\"v\":2}").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"{\"v\":2}");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"{\"v\":1}");
        let entries: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(entries.len(), 2, "no temp files left behind: {entries:?}");
    }

    /// Several tasks may persist at once; each save must be atomic on its own.
    #[test]
    fn concurrent_saves_never_corrupt_the_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..40 {
                        let payload = serde_json::json!({ "writer": t, "n": i, "pad": "x".repeat(200_000) });
                        save_atomic(&p, &serde_json::to_vec(&payload).unwrap()).expect("every save succeeds");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).expect("final file is complete JSON");
        assert_eq!(v["pad"].as_str().unwrap().len(), 200_000);
        let leftovers: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).filter(|n| n.to_string_lossy().contains(".tmp")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn missing_file_is_reported_as_missing() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(load_json::<QueueFile>(&d.path().join("none.json")), LoadOutcome::Missing));
    }

    #[test]
    fn corrupt_main_file_falls_back_to_backup() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue.json");
        let q = queue(vec![job("a", JobState::Waiting)]);
        save_atomic(&p, &serde_json::to_vec(&q).unwrap()).unwrap();
        save_atomic(&p, b"{ truncated").unwrap();
        match load_json::<QueueFile>(&p) {
            LoadOutcome::FromBackup(back) => assert_eq!(back, q),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn corrupt_without_backup_is_moved_aside_not_deleted() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue.json");
        fs::write(&p, b"\x00garbage").unwrap();
        match load_json::<QueueFile>(&p) {
            LoadOutcome::Corrupt { moved_to: Some(m) } => {
                assert!(m.exists());
                assert!(!p.exists());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn interrupted_jobs_restart_from_waiting_and_finished_jobs_stay() {
        let mut jobs = vec![
            job("enc", JobState::Encoding),
            job("ver", JobState::Verifying),
            job("done", JobState::Done),
            job("wait", JobState::Waiting),
            job("ask", JobState::NeedsInput),
        ];
        jobs[0].pid = Some(4242);
        let ids = recover_interrupted(&mut jobs);
        assert_eq!(ids, vec!["enc".to_string(), "ver".to_string()]);
        assert_eq!(jobs[0].state, JobState::Waiting);
        assert!(jobs[0].interrupted && jobs[0].pid.is_none() && jobs[0].progress == 0.0);
        assert_eq!(jobs[2].state, JobState::Done);
        assert!(!jobs[2].interrupted);
        assert_eq!(jobs[4].state, JobState::NeedsInput);
    }
}
