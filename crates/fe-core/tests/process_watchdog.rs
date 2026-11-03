//! The process runner never hangs on a stuck backend: frozen progress is
//! detected and the process is killed, while normal exits are always seen.
#![cfg(unix)]

use fe_core::process::{run_ffmpeg, Outcome, RunOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn alive(pid: u32) -> bool {
    // kill(pid, 0) succeeds while the process exists (and is not yet reaped).
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[tokio::test(flavor = "multi_thread")]
async fn frozen_progress_is_detected_and_the_backend_is_killed() {
    let d = tempfile::tempdir().unwrap();
    // Keeps printing the same progress block forever, like an encoder thread that deadlocked.
    let hung = script(d.path(), "hung-ffmpeg", "while true; do printf 'frame=12\\nout_time_us=400000\\ntotal_size=1000\\nprogress=continue\\n'; sleep 0.2; done");
    let mut pid = 0;
    let started = Instant::now();
    let out = run_ffmpeg(
        RunOptions { program: &hung, args: &[], cwd: None, log_path: None, stall_limit: Some(Duration::from_secs(2)) },
        &CancellationToken::new(),
        |p| pid = p,
        |_| {},
    )
    .await
    .unwrap();
    assert!(matches!(out, Outcome::Stalled { .. }), "{out:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(pid > 0 && !alive(pid), "stalled backend must be killed and reaped");
}

#[tokio::test(flavor = "multi_thread")]
async fn advancing_progress_is_not_a_stall() {
    let d = tempfile::tempdir().unwrap();
    let ok = script(
        d.path(),
        "busy-ffmpeg",
        "i=0; while [ $i -lt 15 ]; do i=$((i+1)); printf 'frame=%d\\nout_time_us=%d\\nprogress=continue\\n' $i $((i*100000)); sleep 0.2; done; printf 'progress=end\\n'; exit 0",
    );
    let mut blocks = 0;
    let out = run_ffmpeg(
        RunOptions { program: &ok, args: &[], cwd: None, log_path: None, stall_limit: Some(Duration::from_secs(1)) },
        &CancellationToken::new(),
        |_| {},
        |_| blocks += 1,
    )
    .await
    .unwrap();
    assert_eq!(out, Outcome::Success);
    assert_eq!(blocks, 16);
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_codes_and_quick_exits_are_always_observed() {
    let d = tempfile::tempdir().unwrap();
    let fail = script(d.path(), "failing-ffmpeg", "echo 'Invalid data found when processing input' >&2; exit 3");
    for _ in 0..20 {
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            run_ffmpeg(RunOptions { program: &fail, args: &[], cwd: None, log_path: None, stall_limit: None }, &CancellationToken::new(), |_| {}, |_| {}),
        )
        .await
        .expect("exit must never be missed")
        .unwrap();
        match out {
            Outcome::Failed { code, tail, .. } => {
                assert_eq!(code, Some(3));
                assert!(tail.iter().any(|l| l.contains("Invalid data")));
            }
            other => panic!("{other:?}"),
        }
    }
}
