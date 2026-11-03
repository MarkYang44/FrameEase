//! Trial encodes ("试压"): encode a short segment with the job's exact settings
//! to show real size, speed and objective scores before committing to a batch.
//! A segment is a sample — results never predict the whole video exactly, and
//! PSNR/SSIM/VMAF never mean "lossless".

use crate::caps::Capabilities;
use crate::job::{Notice, PlanSummary};
use crate::media::{self, ff_path};
use crate::plan::{self, PlanEnv, PlanError};
use crate::process::{self, Outcome, RunOptions};
use crate::settings::{IncompatiblePolicy, SubtitleMode, Trim, VideoMode};
use crate::tools::FfmpegPaths;
use std::time::Instant;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct TrialRequest {
    pub input: PathBuf,
    pub settings: crate::settings::JobSettings,
    pub start: f64,
    pub duration: f64,
    pub metrics: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialResult {
    pub output: PathBuf,
    pub start: f64,
    pub segment_secs: f64,
    pub encode_secs: f64,
    pub bytes: u64,
    /// Size of the same span of the source (from its average bitrate).
    pub source_segment_bytes: Option<u64>,
    /// Linear extrapolation to the full length; an estimate only.
    pub est_full_bytes: Option<u64>,
    pub speed: Option<f64>,
    pub psnr: Option<f64>,
    pub ssim: Option<f64>,
    pub vmaf: Option<f64>,
    pub summary: PlanSummary,
    pub warnings: Vec<Notice>,
}

#[derive(Debug)]
pub enum TrialError {
    Rejected(Notice),
    NeedsSelfTest { encoder: String, pix_fmt: String },
    Cancelled,
    Failed(String),
}

pub async fn run_trial(
    ff: &FfmpegPaths,
    caps: &Capabilities,
    req: &TrialRequest,
    work_dir: &Path,
    cancel: &CancellationToken,
) -> Result<TrialResult, TrialError> {
    let media = media::probe(&ff.ffprobe, &req.input).await.map_err(|e| TrialError::Rejected(Notice::new(e.code())))?;
    let total = media.duration.unwrap_or(0.0);
    let mut s = req.settings.clone();
    if s.video.mode == VideoMode::Copy {
        return Err(TrialError::Rejected(Notice::new("trialCopyMode")));
    }
    let mut seg = req.duration.clamp(1.0, 60.0);
    if total > 0.0 {
        seg = seg.min(total);
    }
    let start = if total > 0.0 { req.start.clamp(0.0, (total - seg).max(0.0)) } else { req.start.max(0.0) };
    s.trim = Some(Trim { start, end: Some(start + seg) });
    s.output.faststart = false;
    if s.subtitles.mode != SubtitleMode::Burn {
        s.subtitles.mode = SubtitleMode::None;
        s.subtitles.external.clear();
    }
    // Include audio in the size estimate even when its copy would need a decision.
    if s.audio.on_incompatible == IncompatiblePolicy::Ask {
        s.audio.on_incompatible = IncompatiblePolicy::Transcode;
    }
    std::fs::create_dir_all(work_dir).map_err(|e| TrialError::Failed(e.to_string()))?;
    let out = work_dir.join(format!("trial.{}", s.output.container.ext()));
    let _ = std::fs::remove_file(&out);
    let env = PlanEnv { caps, threads: caps.cpu.logical.max(1), temp_output: &out, work_dir, keyframes: None };
    let plan = match plan::plan_convert(&s, &media, &env) {
        Ok(p) => p,
        Err(PlanError::Blocked(issues)) => return Err(TrialError::Rejected(issues[0].notice.clone())),
        Err(PlanError::NeedsSelfTest { encoder, pix_fmt }) => return Err(TrialError::NeedsSelfTest { encoder, pix_fmt }),
    };
    let started = Instant::now();
    for pass in &plan.passes {
        let opts = RunOptions { program: &ff.ffmpeg, args: &pass.args, cwd: Some(work_dir), log_path: None, stall_limit: None };
        match process::run_ffmpeg(opts, cancel, |_| {}, |_| {}).await {
            Ok(Outcome::Success) => {}
            Ok(Outcome::Cancelled) => return Err(TrialError::Cancelled),
            Ok(Outcome::Failed { tail, .. } | Outcome::Stalled { tail }) => return Err(TrialError::Failed(tail.join("\n"))),
            Err(e) => return Err(TrialError::Failed(e.to_string())),
        }
    }
    let encode_secs = started.elapsed().as_secs_f64();
    let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    let (psnr, ssim, vmaf) = if req.metrics && !plan.summary.lossless {
        metrics(ff, caps, &media, &plan, start, seg, &out, cancel).await
    } else {
        (None, None, None)
    };
    Ok(TrialResult {
        output: out,
        start,
        segment_secs: seg,
        encode_secs,
        bytes,
        source_segment_bytes: media
            .bit_rate
            .map(|b| (b as f64 / 8.0 * seg) as u64)
            .or_else(|| (total > 0.0).then(|| (media.size as f64 * seg / total) as u64)),
        est_full_bytes: (total > seg).then(|| (bytes as f64 * total / seg) as u64),
        speed: (encode_secs > 0.0).then(|| seg / encode_secs),
        psnr,
        ssim,
        vmaf,
        summary: plan.summary,
        warnings: plan.warnings,
    })
}

fn number_after(text: &str, marker: &str, key: &str) -> Option<f64> {
    let line = text.lines().rev().find(|l| l.contains(marker) && l.contains(key))?;
    let rest = &line[line.find(key)? + key.len()..];
    rest.split_whitespace().next()?.trim_end_matches(',').parse().ok()
}

/// PSNR/SSIM (+VMAF when libvmaf exists) of the encoded segment against the same
/// source segment, decoded once and compared at the source's display size.
#[allow(clippy::too_many_arguments)]
async fn metrics(
    ff: &FfmpegPaths,
    caps: &Capabilities,
    media: &crate::media::MediaInfo,
    plan: &plan::EncodePlan,
    start: f64,
    seg: f64,
    out: &Path,
    cancel: &CancellationToken,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let (Some(v), Some(ev)) = (media.main_video(), plan.expected.video.as_ref()) else { return (None, None, None) };
    let (w, h) = v.display_dims();
    let fmt = if v.bit_depth.unwrap_or(8) > 8 { "yuv420p10le" } else { "yuv420p" };
    let scale = if (ev.width, ev.height) != (w, h) { format!("scale={w}:{h}:flags=bicubic,") } else { String::new() };
    let vmaf = caps.has_filter("libvmaf");
    let n = if vmaf { 3 } else { 2 };
    let mut graph = format!(
        "[0:v]setpts=PTS-STARTPTS,format={fmt},split={n}[r1][r2]{r3};[1:v]setpts=PTS-STARTPTS,{scale}format={fmt},split={n}[d1][d2]{d3};[d1][r1]psnr[o1];[d2][r2]ssim[o2]",
        r3 = if vmaf { "[r3]" } else { "" },
        d3 = if vmaf { "[d3]" } else { "" },
    );
    if vmaf {
        graph.push_str(&format!(";[d3][r3]libvmaf=n_threads={}[o3]", caps.cpu.logical.max(1)));
    }
    let (ss, t) = (format!("{start:.3}"), format!("{seg:.3}"));
    let (src, dst) = (ff_path(&media.path), ff_path(out));
    let mut args: Vec<&str> = vec!["-hide_banner", "-nostdin", "-v", "info", "-ss", &ss, "-t", &t, "-i", &src, "-i", &dst, "-filter_complex", &graph];
    args.extend(["-map", "[o1]", "-f", "null", "-", "-map", "[o2]", "-f", "null", "-"]);
    if vmaf {
        args.extend(["-map", "[o3]", "-f", "null", "-"]);
    }
    let run = process::run_capture(&ff.ffmpeg, &args, std::time::Duration::from_secs(900));
    let out = tokio::select! {
        r = run => r,
        _ = cancel.cancelled() => return (None, None, None),
    };
    let Ok(o) = out else { return (None, None, None) };
    let text = &o.stderr;
    (
        number_after(text, "PSNR", "average:").filter(|v| v.is_finite()),
        number_after(text, "SSIM", "All:"),
        if vmaf { number_after(text, "VMAF", "score:") } else { None },
    )
}

/// One frame as PNG, scaled down to `max_width`, at `time` seconds (accurate seek).
pub async fn extract_frame(ffmpeg: &Path, input: &Path, time: f64, max_width: u32, out: &Path) -> Result<(), String> {
    let (ss, vf) = (format!("{:.3}", time.max(0.0)), format!("scale='min({max_width},iw)':-2:flags=lanczos"));
    let (src, dst) = (ff_path(input), ff_path(out));
    let args = ["-hide_banner", "-nostdin", "-v", "error", "-y", "-ss", &ss, "-i", &src, "-frames:v", "1", "-vf", &vf, "-update", "1", "-f", "image2", "-c:v", "png", &dst];
    match process::run_capture(ffmpeg, &args, std::time::Duration::from_secs(120)).await {
        Ok(o) if o.success() && out.exists() => Ok(()),
        Ok(o) => Err(o.stderr_tail(3)),
        Err(e) => Err(e.to_string()),
    }
}
