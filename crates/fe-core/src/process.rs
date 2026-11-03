//! Process management for FFmpeg/ffprobe: argument arrays only (no shell),
//! no console windows on Windows, streamed progress, cancellation and logs.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

/// Base command: stdin closed, killed when dropped, quiet SVT-AV1 banner.
pub fn command(program: &Path) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.stdin(Stdio::null()).kill_on_drop(true);
    // SVT-AV1 logs through its own logger and ignores -loglevel; 2 = warnings and errors.
    c.env("SVT_LOG", "2").env("AV_LOG_FORCE_NOCOLOR", "1");
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: never flash a console window for the backend.
        c.creation_flags(0x0800_0000);
    }
    c
}

pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Captured {
    pub fn success(&self) -> bool {
        self.status.success()
    }
    pub fn stderr_tail(&self, n: usize) -> String {
        let lines: Vec<&str> = self.stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(n)..].join("\n")
    }
}

/// Run a short command to completion and capture its output.
pub async fn run_capture<S: AsRef<OsStr>>(program: &Path, args: &[S], timeout: Duration) -> std::io::Result<Captured> {
    use tokio::io::AsyncReadExt;
    let mut cmd = command(program);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("no stderr"))?;
    let run = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (a, b) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        a?;
        b?;
        let status = wait_exit(&mut child).await?;
        Ok::<_, std::io::Error>(Captured { status, stdout: out, stderr: String::from_utf8_lossy(&err).into_owned() })
    };
    match tokio::time::timeout(timeout, run).await {
        Ok(r) => r,
        Err(_) => {
            let _ = child.start_kill();
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "backend command timed out"))
        }
    }
}

/// Wait for exit; the notification is the fast path and polling covers a missed one.
async fn wait_exit(child: &mut tokio::process::Child) -> std::io::Result<ExitStatus> {
    loop {
        tokio::select! {
            st = child.wait() => return st,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if let Some(st) = child.try_wait()? {
                    return Ok(st);
                }
            }
        }
    }
}

/// One `-progress` block from FFmpeg.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub out_time_us: Option<i64>,
    pub total_size: Option<u64>,
    pub speed: Option<f64>,
    pub end: bool,
}

/// Incremental parser for `-progress pipe:1` key=value lines.
#[derive(Debug, Default)]
pub struct ProgressParser {
    cur: Progress,
}

impl ProgressParser {
    /// Feed one line; returns a complete block when a `progress=` line arrives.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        let (k, v) = line.trim().split_once('=')?;
        let v = v.trim();
        match k.trim() {
            "frame" => self.cur.frame = v.parse().ok(),
            "fps" => self.cur.fps = v.parse().ok(),
            "out_time_us" => self.cur.out_time_us = v.parse().ok(),
            "total_size" => self.cur.total_size = v.parse().ok(),
            "speed" => self.cur.speed = v.trim_end_matches('x').trim().parse().ok(),
            "progress" => {
                self.cur.end = v == "end";
                return Some(self.cur.clone());
            }
            _ => {}
        }
        None
    }
}

/// Detects a backend that stopped making progress: FFmpeg keeps printing
/// progress blocks even when an encoder thread is stuck, so frozen counters
/// (frame / time / size) are the signal, not silence.
#[derive(Debug)]
pub struct StallTracker {
    limit: Duration,
    last_change: std::time::Instant,
    last: (Option<u64>, Option<i64>, Option<u64>),
    finished: bool,
}

impl StallTracker {
    pub fn new(limit: Duration, now: std::time::Instant) -> Self {
        Self { limit, last_change: now, last: (None, None, None), finished: false }
    }
    pub fn observe(&mut self, p: &Progress, now: std::time::Instant) {
        if p.end {
            self.finished = true;
        }
        let cur = (p.frame, p.out_time_us, p.total_size);
        if cur != self.last {
            self.last = cur;
            self.last_change = now;
        }
    }
    pub fn stalled(&self, now: std::time::Instant) -> bool {
        !self.finished && now.duration_since(self.last_change) > self.limit
    }
}

/// Keeps the last `cap` lines of backend output for error messages.
#[derive(Debug)]
pub struct Tail {
    cap: usize,
    lines: VecDeque<String>,
}

impl Tail {
    pub fn new(cap: usize) -> Self {
        Self { cap, lines: VecDeque::with_capacity(cap) }
    }
    pub fn push(&mut self, line: String) {
        if self.lines.len() == self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }
}

/// How a backend run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Success,
    Cancelled,
    Failed { code: Option<i32>, signal: Option<i32>, tail: Vec<String> },
    /// No progress for `stall_limit`; the backend was killed.
    Stalled { tail: Vec<String> },
}

/// Default time without any progress before a backend counts as hung.
pub const STALL_LIMIT: Duration = Duration::from_secs(5 * 60);

pub struct RunOptions<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub cwd: Option<&'a Path>,
    /// Backend stderr is appended here (best effort; logging never fails a job).
    pub log_path: Option<&'a Path>,
    /// Kill the backend when its progress counters do not move for this long (default `STALL_LIMIT`).
    pub stall_limit: Option<Duration>,
}

/// Quote an argument list for logs only. Execution never goes through a shell.
pub fn display_command(program: &Path, args: &[String]) -> String {
    let q = |a: &str| {
        if a.is_empty() || a.contains(|c: char| c.is_whitespace() || "\"'$&|;<>()[]{}*?!#`\\".contains(c)) {
            format!("\"{}\"", a.replace('"', "\\\""))
        } else {
            a.to_string()
        }
    };
    std::iter::once(q(&program.to_string_lossy())).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" ")
}

/// Run FFmpeg with `-progress pipe:1`, streaming progress until exit or cancellation.
/// Cancellation kills the process; the caller deletes any partial output.
pub async fn run_ffmpeg(
    opts: RunOptions<'_>,
    cancel: &tokio_util::sync::CancellationToken,
    mut on_spawn: impl FnMut(u32),
    mut on_progress: impl FnMut(&Progress),
) -> std::io::Result<Outcome> {
    use std::io::Write;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut log = opts.log_path.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "\n$ {}", display_command(opts.program, opts.args));
    }
    let mut cmd = command(opts.program);
    cmd.args(opts.args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = opts.cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn()?;
    #[cfg(windows)]
    win_job::adopt(&child);
    if let Some(pid) = child.id() {
        on_spawn(pid);
    }
    let stdout = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| std::io::Error::other("no stderr"))?;

    // Drain stderr on its own task so a chatty encoder can never block on a full pipe.
    let err_task = tokio::spawn(async move {
        let mut tail = Tail::new(40);
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                    if let Some(f) = log.as_mut() {
                        let _ = writeln!(f, "{line}");
                    }
                    if !line.is_empty() {
                        tail.push(line);
                    }
                }
            }
        }
        tail
    });

    enum Ev {
        Cancel,
        Line(std::io::Result<usize>),
        Exit(std::io::Result<ExitStatus>),
        Tick,
    }
    let mut out = BufReader::new(stdout);
    let mut parser = ProgressParser::default();
    let mut buf = Vec::new();
    let mut stdout_open = true;
    let mut stall = StallTracker::new(opts.stall_limit.unwrap_or(STALL_LIMIT), std::time::Instant::now());
    let status = loop {
        let ev = tokio::select! {
            biased;
            _ = cancel.cancelled() => Ev::Cancel,
            r = out.read_until(b'\n', &mut buf), if stdout_open => Ev::Line(r),
            st = child.wait(), if !stdout_open => Ev::Exit(st),
            _ = tokio::time::sleep(Duration::from_millis(500)) => Ev::Tick,
        };
        match ev {
            Ev::Cancel => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = err_task.await;
                return Ok(Outcome::Cancelled);
            }
            Ev::Line(Ok(0)) | Ev::Line(Err(_)) => stdout_open = false,
            Ev::Line(Ok(_)) => {
                if let Some(p) = parser.feed(&String::from_utf8_lossy(&buf)) {
                    stall.observe(&p, std::time::Instant::now());
                    on_progress(&p);
                }
                buf.clear();
            }
            Ev::Exit(st) => break st?,
            // Exit notification is the fast path; polling makes a missed one harmless.
            Ev::Tick if !stdout_open => {
                if let Some(st) = child.try_wait()? {
                    break st;
                }
            }
            Ev::Tick => {}
        }
        if stall.stalled(std::time::Instant::now()) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let mut tail = err_task.await.map(|t| t.lines()).unwrap_or_default();
            tail.push(format!("FrameEase: no progress for {} s, backend stopped", stall.limit.as_secs()));
            return Ok(Outcome::Stalled { tail });
        }
    };
    let tail = err_task.await.map(|t| t.lines()).unwrap_or_default();
    if status.success() {
        return Ok(Outcome::Success);
    }
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    Ok(Outcome::Failed { code: status.code(), signal, tail })
}

/// Windows: put backend processes in a kill-on-close job object so they die
/// with the app even if it crashes.
#[cfg(windows)]
mod win_job {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Job(HANDLE);
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            (ok != 0).then_some(Job(h))
        })
        .as_ref()
    }

    pub fn adopt(child: &tokio::process::Child) {
        if let (Some(job), Some(handle)) = (job(), child.raw_handle()) {
            // SAFETY: both handles are valid for the duration of the call.
            unsafe {
                AssignProcessToJobObject(job.0, handle as HANDLE);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_block_is_emitted_on_progress_line() {
        let mut p = ProgressParser::default();
        let block = [
            "frame=120", "fps=45.5", "stream_0_0_q=28.0", "bitrate=1234.5kbits/s", "total_size=12345",
            "out_time_us=4000000", "out_time_ms=4000000", "out_time=00:00:04.000000", "dup_frames=0",
            "drop_frames=0", "speed=1.5x",
        ];
        for l in block {
            assert_eq!(p.feed(l), None);
        }
        let got = p.feed("progress=continue").unwrap();
        assert_eq!(got.frame, Some(120));
        assert_eq!(got.fps, Some(45.5));
        assert_eq!(got.out_time_us, Some(4_000_000));
        assert_eq!(got.total_size, Some(12345));
        assert_eq!(got.speed, Some(1.5));
        assert!(!got.end);
        let end = p.feed("progress=end").unwrap();
        assert!(end.end);
        assert_eq!(end.frame, Some(120), "values persist across blocks");
    }

    #[test]
    fn not_available_values_are_none() {
        let mut p = ProgressParser::default();
        p.feed("out_time_us=N/A");
        p.feed("speed=N/A");
        p.feed("total_size=N/A");
        let got = p.feed("progress=continue").unwrap();
        assert_eq!((got.out_time_us, got.speed, got.total_size), (None, None, None));
    }

    #[test]
    fn tolerates_windows_line_endings_and_spaces() {
        let mut p = ProgressParser::default();
        p.feed("frame= 7\r");
        p.feed("speed= 0.98x\r");
        let got = p.feed("progress=continue\r").unwrap();
        assert_eq!((got.frame, got.speed), (Some(7), Some(0.98)));
    }

    #[test]
    fn stall_tracker_flags_frozen_counters_only() {
        let t0 = std::time::Instant::now();
        let s = |sec: u64| t0 + Duration::from_secs(sec);
        let mut st = StallTracker::new(Duration::from_secs(60), t0);
        let p = |frame: u64| Progress { frame: Some(frame), out_time_us: Some(frame as i64 * 33_333), total_size: Some(frame * 1000), ..Default::default() };
        st.observe(&p(10), s(5));
        st.observe(&p(10), s(50)); // same numbers: no change
        assert!(!st.stalled(s(64)), "60 s since the last change at 5 s is not exceeded yet");
        assert!(st.stalled(s(66)));
        st.observe(&p(11), s(70));
        assert!(!st.stalled(s(120)), "a new frame resets the clock");
    }

    #[test]
    fn stall_tracker_ignores_the_finalising_phase() {
        let t0 = std::time::Instant::now();
        let mut st = StallTracker::new(Duration::from_secs(10), t0);
        st.observe(&Progress { frame: Some(5), end: true, ..Default::default() }, t0);
        assert!(!st.stalled(t0 + Duration::from_secs(3600)), "moving the moov atom can take long after progress=end");
    }

    #[test]
    fn tail_keeps_only_the_last_lines() {
        let mut t = Tail::new(3);
        for i in 0..5 {
            t.push(format!("l{i}"));
        }
        assert_eq!(t.lines(), vec!["l2", "l3", "l4"]);
    }
}
