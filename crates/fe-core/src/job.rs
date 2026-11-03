//! Job model and the job state machine.
//!
//! States: Waiting → Probing → Encoding → Muxing → Verifying → Done, with
//! Cancelled / Failed exits and NeedsInput when the plan needs a user decision.

use crate::settings::JobSpec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type JobId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Waiting,
    /// Blocked until the user resolves an issue (e.g. incompatible audio copy).
    NeedsInput,
    Probing,
    Encoding,
    Muxing,
    Verifying,
    Done,
    Cancelled,
    Failed,
}

impl JobState {
    /// A worker owns the job and an external process may be running.
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Probing | JobState::Encoding | JobState::Muxing | JobState::Verifying)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Done | JobState::Cancelled | JobState::Failed)
    }

    pub fn can_transition_to(self, next: JobState) -> bool {
        use JobState::*;
        match self {
            Waiting => matches!(next, Probing | NeedsInput | Cancelled),
            NeedsInput => matches!(next, Waiting | Cancelled),
            // Active states may always fail, be cancelled, or be interrupted
            // back to Waiting when the app exits mid-job.
            Probing => matches!(next, Encoding | Muxing | NeedsInput | Failed | Cancelled | Waiting),
            Encoding => matches!(next, Muxing | Verifying | Failed | Cancelled | Waiting),
            Muxing => matches!(next, Verifying | Failed | Cancelled | Waiting),
            Verifying => matches!(next, Done | Failed | Cancelled | Waiting),
            // A verified result is never re-run; "process again" creates a new job.
            Done => false,
            Failed | Cancelled => matches!(next, Waiting),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid job state transition {from:?} -> {to:?}")]
pub struct InvalidTransition {
    pub from: JobState,
    pub to: JobState,
}

/// A coded, parameterised message. The UI localises `code`; `params` fill placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub code: String,
    pub params: BTreeMap<String, String>,
}

impl Notice {
    pub fn new(code: &str) -> Self {
        Self { code: code.to_string(), params: BTreeMap::new() }
    }
    pub fn with(mut self, key: &str, value: impl ToString) -> Self {
        self.params.insert(key.to_string(), value.to_string());
        self
    }
}

/// A user decision that unblocks a job, e.g. `{ id: "audioTranscode" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub id: String,
}

/// A blocking problem found while planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    #[serde(flatten)]
    pub notice: Notice,
    pub fixes: Vec<Fix>,
}

impl Issue {
    pub fn new(notice: Notice) -> Self {
        Self { notice, fixes: Vec::new() }
    }
    pub fn fix(mut self, id: &str) -> Self {
        self.fixes.push(Fix { id: id.to_string() });
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    #[serde(flatten)]
    pub notice: Notice,
    /// Stage where it happened (state name).
    pub stage: JobState,
    /// Raw backend detail (last stderr lines) for the log viewer.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LiveStats {
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    /// Seconds of output written so far.
    pub out_time: Option<f64>,
    pub size_bytes: Option<u64>,
    pub eta_secs: Option<f64>,
    /// "1/2" style pass label for two-pass encodes.
    pub pass: Option<String>,
}

/// One check performed on the finished output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub ok: bool,
    pub checks: Vec<Check>,
}

/// Short, display-oriented description of what the job will do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    pub kind: String,
    pub video: String,
    pub encoder: Option<String>,
    pub quality: Option<String>,
    pub lossless: bool,
    pub reencode_video: bool,
    pub audio: String,
    pub subtitles: String,
    pub container: String,
    pub est_size: Option<u64>,
    /// "exact" | "estimate" | "unknown"
    pub est_reliability: String,
    pub threads: Option<u32>,
    pub hw: bool,
    /// Scheduler inputs: estimated peak memory and a conservative disk need.
    pub est_memory_mb: Option<u64>,
    pub est_disk_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceSummary {
    pub container: String,
    pub video: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub duration: Option<f64>,
    pub size: u64,
    pub audio_tracks: u32,
    pub subtitle_tracks: u32,
    pub hdr: bool,
    pub bit_depth: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: JobId,
    pub spec: JobSpec,
    pub name: String,
    pub state: JobState,
    /// Overall progress 0..=1 across encode passes and verification.
    pub progress: f64,
    pub stats: LiveStats,
    pub source: Option<SourceSummary>,
    pub plan: Option<PlanSummary>,
    pub planned_output: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub output_size: Option<u64>,
    /// Temp files this job created and owns. Only these are ever deleted.
    pub temp_files: Vec<PathBuf>,
    pub issues: Vec<Issue>,
    pub warnings: Vec<Notice>,
    pub error: Option<JobError>,
    pub verify: Option<VerifyReport>,
    pub attempts: u32,
    /// Set when the app stopped while this job was running; it restarts from the beginning.
    pub interrupted: bool,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub elapsed_ms: u64,
    pub log_path: Option<PathBuf>,
    /// PID of the running backend process (for orphan cleanup after a crash).
    pub pid: Option<u32>,
}

impl Job {
    pub fn new(id: JobId, spec: JobSpec, now_ms: i64) -> Self {
        let name = spec
            .primary_input()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            id,
            spec,
            name,
            state: JobState::Waiting,
            progress: 0.0,
            stats: LiveStats::default(),
            source: None,
            plan: None,
            planned_output: None,
            output: None,
            output_size: None,
            temp_files: Vec::new(),
            issues: Vec::new(),
            warnings: Vec::new(),
            error: None,
            verify: None,
            attempts: 0,
            interrupted: false,
            created_at: now_ms,
            started_at: None,
            finished_at: None,
            elapsed_ms: 0,
            log_path: None,
            pid: None,
        }
    }

    /// Move to `next`, enforcing the state machine.
    pub fn transition(&mut self, next: JobState) -> Result<(), InvalidTransition> {
        if !self.state.can_transition_to(next) {
            return Err(InvalidTransition { from: self.state, to: next });
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::JobState::*;
    use super::*;
    use crate::settings::{JobSettings, JobSpec};

    fn job() -> Job {
        let spec = JobSpec::Convert { input: PathBuf::from("/v/旅行 01.mp4"), settings: JobSettings::default() };
        Job::new("j1".into(), spec, 0)
    }

    #[test]
    fn happy_path_runs_through_every_stage() {
        let mut j = job();
        for s in [Probing, Encoding, Muxing, Verifying, Done] {
            j.transition(s).unwrap();
        }
        assert_eq!(j.state, Done);
    }

    #[test]
    fn stream_copy_jobs_go_from_probing_straight_to_muxing() {
        assert!(Probing.can_transition_to(Muxing));
    }

    #[test]
    fn encoding_may_skip_muxing_and_verify_directly() {
        assert!(Encoding.can_transition_to(Verifying));
    }

    #[test]
    fn cannot_skip_from_waiting_to_done() {
        let mut j = job();
        let err = j.transition(Done).unwrap_err();
        assert_eq!(err, InvalidTransition { from: Waiting, to: Done });
        assert_eq!(j.state, Waiting, "state must not change on invalid transition");
    }

    #[test]
    fn done_is_final_and_never_reruns() {
        for s in [Waiting, NeedsInput, Probing, Encoding, Muxing, Verifying, Cancelled, Failed] {
            assert!(!Done.can_transition_to(s), "Done -> {s:?} must be rejected");
        }
    }

    #[test]
    fn failed_and_cancelled_can_be_retried() {
        assert!(Failed.can_transition_to(Waiting));
        assert!(Cancelled.can_transition_to(Waiting));
        assert!(!Failed.can_transition_to(Encoding), "retry must go back through Waiting");
    }

    #[test]
    fn every_active_state_can_fail_be_cancelled_or_be_interrupted() {
        for s in [Probing, Encoding, Muxing, Verifying] {
            assert!(s.is_active());
            assert!(s.can_transition_to(Failed), "{s:?} -> Failed");
            assert!(s.can_transition_to(Cancelled), "{s:?} -> Cancelled");
            assert!(s.can_transition_to(Waiting), "{s:?} -> Waiting (interrupted by app exit)");
        }
    }

    #[test]
    fn needs_input_blocks_until_resolved_or_cancelled() {
        assert!(Waiting.can_transition_to(NeedsInput));
        assert!(Probing.can_transition_to(NeedsInput));
        assert!(NeedsInput.can_transition_to(Waiting));
        assert!(NeedsInput.can_transition_to(Cancelled));
        assert!(!NeedsInput.can_transition_to(Encoding));
    }

    #[test]
    fn waiting_jobs_can_be_cancelled_but_not_failed() {
        assert!(Waiting.can_transition_to(Cancelled));
        assert!(!Waiting.can_transition_to(Failed));
    }

    #[test]
    fn verification_cannot_go_back_to_encoding() {
        assert!(!Verifying.can_transition_to(Encoding));
        assert!(!Muxing.can_transition_to(Encoding));
    }

    #[test]
    fn job_name_keeps_unicode_file_name() {
        assert_eq!(job().name, "旅行 01.mp4");
    }

    #[test]
    fn state_serializes_in_camel_case() {
        assert_eq!(serde_json::to_string(&NeedsInput).unwrap(), "\"needsInput\"");
    }
}
