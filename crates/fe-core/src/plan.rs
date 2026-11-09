//! Argument planning: (settings snapshot, probed media, capabilities) →
//! exact FFmpeg argument arrays plus the expectations the result is verified
//! against. Nothing here runs a process; everything is unit-testable.
//!
//! Rules enforced here (never silently):
//! - the requested codec is what gets produced (no "smaller" substitute);
//! - resolution / frame rate are kept unless the user asked otherwise;
//! - HDR is never turned into SDR, 10-bit never into 8-bit, 4:4:4 never into
//!   4:2:0 without an explicit opt-in; unsupported cases are refused;
//! - lossless only with an encoder/pixel-format pair that passed a pixel-exact
//!   self-test, and every lossless job is verified frame by frame;
//! - audio that cannot be copied is never changed without the user's choice.

use crate::caps::Capabilities;
use crate::encoders::{self, EncArgs, EncoderDef, Lossless, RateArgs, Vendor};
use crate::job::{Issue, JobState, Notice, PlanSummary};
use crate::media::{Chroma, HdrKind, MediaInfo, VideoStream, ff_path, pix_traits};
use crate::settings::*;
use std::path::{Path, PathBuf};

pub struct PlanEnv<'a> {
    pub caps: &'a Capabilities,
    /// Thread budget for this job.
    pub threads: u32,
    pub temp_output: &'a Path,
    /// Per-job work dir; used as the backend's cwd (two-pass stats, subtitle copies).
    pub work_dir: &'a Path,
    /// Keyframe times of the main video stream, needed for stream-copy trims.
    pub keyframes: Option<&'a [f64]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    Blocked(Vec<Issue>),
    /// Run the lossless self-test for this pair, then plan again.
    NeedsSelfTest { encoder: String, pix_fmt: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    pub args: Vec<String>,
    pub label: Option<String>,
    /// Share of the job's total work (all passes + verification sum to 1).
    pub weight: f64,
    pub stage: JobState,
}

/// Work done before the main passes.
#[derive(Debug, Clone, PartialEq)]
pub enum Prep {
    /// Extract an embedded text subtitle to an ASCII-named file for burning.
    ExtractSubtitle { stream_index: u32, to: PathBuf },
    CopyFile { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameCheck {
    /// Same packet count as the source's main video stream.
    SameAsSource,
    /// About this many frames (frame-rate change / trim), ±2%.
    Approx(u64),
    Skip,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColorTags {
    pub primaries: Option<String>,
    pub transfer: Option<String>,
    pub space: Option<String>,
    pub range: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedVideo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: Option<String>,
    pub frames: FrameCheck,
    pub color: ColorTags,
    /// Source carried mastering-display metadata that must survive.
    pub hdr_metadata: bool,
    /// Display rotation that must remain (stream copy keeps the matrix).
    pub rotation: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedAudio {
    pub source_index: u32,
    pub codec: String,
    pub copied: bool,
}

/// A stream copied bit-exactly; packet hashes of both sides must match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamCopy {
    pub input: u32,
    pub source_index: u32,
    pub output_index: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expected {
    /// None for single-stream extractions (.m4a, .srt …).
    pub container: Option<Container>,
    pub video: Option<ExpectedVideo>,
    pub audio: Vec<ExpectedAudio>,
    pub subtitles: usize,
    pub duration: Option<f64>,
    pub hash_streams: Vec<StreamCopy>,
    /// Lossless: compare decoded frames with the source read using these input options.
    pub lossless_source_opts: Option<Vec<String>>,
    pub source_video_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodePlan {
    pub passes: Vec<Pass>,
    pub prep: Vec<Prep>,
    pub expected: Expected,
    pub summary: PlanSummary,
    pub warnings: Vec<Notice>,
    /// Expected output duration (progress denominator).
    pub duration: Option<f64>,
    pub encoder: Option<String>,
    /// Conservative upper bound for the disk-space check.
    pub est_upper_bytes: u64,
    pub est_memory_mb: u64,
    pub hw_encoder: bool,
    /// Share of the work done by verification (for progress weighting).
    pub verify_weight: f64,
}

// ---------------------------------------------------------------- tables

/// Audio codecs that can be stream-copied into MP4 *and* play in mainstream players.
/// (FFmpeg 8 can also write PCM into MP4, but most players cannot read it.)
const MP4_AUDIO_COPY: &[&str] = &["aac", "mp3", "ac3", "eac3", "alac", "opus", "flac"];
const MP4_VIDEO_COPY: &[&str] = &["h264", "hevc", "av1", "vp9", "mpeg4", "mpeg2video", "mjpeg"];
const TEXT_SUBS: &[&str] = &["subrip", "ass", "ssa", "webvtt", "mov_text", "text", "srt"];
/// Encoders whose HDR10 signalling + static metadata pass-through was verified.
const HDR_ENCODERS: &[&str] = &["libx265", "libsvtav1"];
const STATS_FILE: &str = "fe2pass";

fn audio_copy_ok(codec: &str, container: Container) -> bool {
    match container {
        Container::Mp4 => MP4_AUDIO_COPY.contains(&codec),
        Container::Mkv => true,
    }
}

fn video_copy_ok(codec: &str, container: Container) -> bool {
    match container {
        Container::Mp4 => MP4_VIDEO_COPY.contains(&codec),
        Container::Mkv => true,
    }
}

fn fix_for_encoder(name: &str) -> &'static str {
    match name {
        "libx264" => "useX264",
        "libx265" => "useX265",
        "libsvtav1" => "useSvtAv1",
        "libaom-av1" => "useAom",
        "ffv1" => "useFfv1",
        _ => "speedBalanced",
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn secs(t: f64) -> String {
    format!("{t:.3}")
}

/// "11:12" / "1:02:03", for messages.
fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn head() -> Vec<String> {
    strs(&["-hide_banner", "-nostdin", "-n", "-loglevel", "level+warning", "-progress", "pipe:1", "-nostats", "-stats_period", "0.5"])
}

fn container_tail(container: Container, settings: &OutputSettings, temp: &Path) -> Vec<String> {
    let mut a = Vec::new();
    if container == Container::Mp4 && settings.faststart {
        a.extend(strs(&["-movflags", "+faststart"]));
    }
    a.extend(strs(&["-max_muxing_queue_size", "4096", "-f", container.muxer()]));
    a.push(ff_path(temp));
    a
}

fn metadata_args(out: &OutputSettings) -> Vec<String> {
    let mut a = strs(&["-map_metadata", if out.keep_metadata { "0" } else { "-1" }]);
    a.extend(strs(&["-map_chapters", if out.keep_chapters { "0" } else { "-1" }]));
    a
}

// ---------------------------------------------------------------- encoder choice

/// Encoder used for these settings, or the issue explaining why there is none.
pub fn choose_encoder(v: &VideoSettings, caps: &Capabilities) -> Result<&'static EncoderDef, Issue> {
    let unavailable = |name: &str| {
        let reason = caps.encoder(name).and_then(|e| e.error.clone()).unwrap_or_else(|| "notInBuild".into());
        Issue::new(Notice::new("encoderUnavailable").with("encoder", name).with("reason", reason))
    };
    if let Some(name) = v.encoder.as_deref().filter(|n| !n.is_empty()) {
        let def = encoders::get(name).ok_or_else(|| unavailable(name))?;
        if def.codec != v.codec {
            return Err(Issue::new(Notice::new("encoderCodecMismatch").with("encoder", name)).fix("speedBalanced"));
        }
        if !caps.usable(name) {
            return Err(unavailable(name).fix("speedBalanced"));
        }
        return Ok(def);
    }
    if v.speed == Speed::Gpu && v.mode == VideoMode::Lossy {
        for vendor in [Vendor::Apple, Vendor::Nvidia, Vendor::Intel, Vendor::Amd] {
            if let Some(def) = encoders::for_codec(v.codec).find(|d| d.vendor == vendor && caps.usable(d.name)) {
                return Ok(def);
            }
        }
        return Err(Issue::new(Notice::new("noHardwareEncoder").with("codec", v.codec.probe_name())).fix("speedBalanced"));
    }
    let order: &[&str] = match v.codec {
        VideoCodec::H264 => &["libx264"],
        VideoCodec::Hevc => &["libx265"],
        VideoCodec::Av1 => &["libsvtav1", "libaom-av1"],
        VideoCodec::Ffv1 => &["ffv1"],
    };
    order
        .iter()
        .find(|n| caps.usable(n))
        .and_then(|n| encoders::get(n))
        .ok_or_else(|| unavailable(order[0]))
}

fn kind_token(k: encoders::QualityKind) -> &'static str {
    match k {
        encoders::QualityKind::Crf => "crf",
        encoders::QualityKind::Cq => "cq",
        encoders::QualityKind::Qv => "q",
        encoders::QualityKind::Icq => "icq",
        encoders::QualityKind::Qp => "qp",
    }
}

/// `{codec}` and `{mode}` tokens for output names, e.g. ("av1", "crf34").
pub fn naming_tokens(settings: &JobSettings, caps: &Capabilities) -> (String, String) {
    let v = &settings.video;
    match v.mode {
        VideoMode::Copy => ("remux".into(), "copy".into()),
        VideoMode::Lossless => (v.codec.probe_name().into(), "lossless".into()),
        VideoMode::Lossy => {
            let q = choose_encoder(v, caps).ok().and_then(|d| d.quality);
            let mode = match (&v.rate, q) {
                (RateControl::Level, Some(q)) => format!("{}{}", kind_token(q.kind), encoders::fmt_value(&q, encoders::level_to_native(&q, v.level))),
                (RateControl::Native { value }, Some(q)) => format!("{}{}", kind_token(q.kind), encoders::fmt_value(&q, *value)),
                (RateControl::Bitrate { kbps, .. }, _) => format!("{kbps}k"),
                (RateControl::TargetSize { megabytes, .. }, _) => format!("{megabytes}MB"),
                _ => "q".into(),
            };
            (v.codec.probe_name().into(), mode)
        }
    }
}

// ---------------------------------------------------------------- audio

struct AudioPlan {
    args: Vec<String>,
    expected: Vec<ExpectedAudio>,
    hashes: Vec<StreamCopy>,
    kbps: u32,
    summary: String,
}

fn default_transcode_kbps(channels: u32) -> u32 {
    match channels {
        0..=2 => 192,
        3..=6 => 384,
        _ => 512,
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_audio(
    a: &AudioSettings,
    media: &MediaInfo,
    container: Container,
    caps: &Capabilities,
    first_out: u32,
    hash_ok: bool,
    issues: &mut Vec<Issue>,
    warnings: &mut Vec<Notice>,
) -> AudioPlan {
    let mut p = AudioPlan { args: Vec::new(), expected: Vec::new(), hashes: Vec::new(), kbps: 0, summary: "none".into() };
    let selected: Vec<(usize, &crate::media::AudioStream)> = match a.tracks {
        _ if a.mode == AudioMode::None => Vec::new(),
        TrackSelection::All => media.audio.iter().enumerate().collect(),
        TrackSelection::First => media.audio.iter().enumerate().take(1).collect(),
        TrackSelection::List => media.audio.iter().enumerate().filter(|(i, _)| a.track_list.contains(&(*i as u32))).collect(),
    };
    let (mut copied, mut transcoded) = (0, 0);
    for (ordinal, st) in selected {
        let k = p.expected.len() as u32;
        let track = (ordinal + 1).to_string();
        let push = |codec_args: Vec<String>, expected: ExpectedAudio, kbps: u32, p: &mut AudioPlan| {
            p.args.extend(["-map".to_string(), format!("0:{}", st.index)]);
            p.args.extend(codec_args);
            if expected.copied && hash_ok {
                p.hashes.push(StreamCopy { input: 0, source_index: st.index, output_index: first_out + k });
            }
            p.expected.push(expected);
            p.kbps += kbps;
        };
        let src_kbps = st.bit_rate.map(|b| (b / 1000) as u32).unwrap_or(192);
        let transcode = |codec: AudioCodec, kbps: u32| {
            let mut v = vec![format!("-c:a:{k}"), codec.encoder().to_string()];
            if !codec.is_lossless() {
                v.extend([format!("-b:a:{k}"), format!("{kbps}k")]);
            }
            if codec == AudioCodec::Mp3 && st.channels > 2 {
                v.extend([format!("-ac:a:{k}"), "2".into()]);
            }
            v
        };
        match a.mode {
            AudioMode::Copy if audio_copy_ok(&st.codec, container) => {
                copied += 1;
                push(vec![format!("-c:a:{k}"), "copy".into()], ExpectedAudio { source_index: st.index, codec: st.codec.clone(), copied: true }, src_kbps, &mut p);
            }
            AudioMode::Copy => match a.on_incompatible {
                IncompatiblePolicy::Ask => {
                    let mut issue = Issue::new(
                        Notice::new("audioIncompatible").with("track", &track).with("codec", &st.codec).with("container", container.ext()),
                    )
                    .fix("audioTranscode")
                    .fix("audioDrop");
                    if container == Container::Mp4 {
                        issue = issue.fix("useMkv");
                    }
                    issues.push(issue);
                }
                IncompatiblePolicy::Transcode => {
                    transcoded += 1;
                    let kbps = default_transcode_kbps(st.channels);
                    warnings.push(Notice::new("audioTranscoded").with("track", &track).with("from", &st.codec).with("codec", "aac"));
                    push(transcode(AudioCodec::Aac, kbps), ExpectedAudio { source_index: st.index, codec: "aac".into(), copied: false }, kbps, &mut p);
                }
                IncompatiblePolicy::Drop => warnings.push(Notice::new("audioDropped").with("track", &track).with("codec", &st.codec)),
            },
            AudioMode::Transcode => {
                if !caps.has_audio_encoder(a.codec.encoder()) {
                    issues.push(Issue::new(Notice::new("audioEncoderUnavailable").with("codec", a.codec.probe_name())));
                    continue;
                }
                if a.codec == AudioCodec::Mp3 && st.channels > 2 {
                    warnings.push(Notice::new("audioDownmixed").with("track", &track).with("channels", st.channels));
                }
                transcoded += 1;
                let kbps = if a.codec.is_lossless() { 1000 } else { a.bitrate_kbps };
                push(transcode(a.codec, a.bitrate_kbps), ExpectedAudio { source_index: st.index, codec: a.codec.probe_name().into(), copied: false }, kbps, &mut p);
            }
            AudioMode::None => {}
        }
    }
    if p.expected.is_empty() {
        p.args.push("-an".into());
    }
    p.summary = match (copied, transcoded) {
        (0, 0) => "none".into(),
        (_, 0) => "copy".into(),
        (0, _) if a.mode == AudioMode::Transcode => format!("transcode:{}:{}", a.codec.probe_name(), a.bitrate_kbps),
        (0, _) => "transcode:aac".into(),
        _ => "mixed".into(),
    };
    p
}

// ---------------------------------------------------------------- subtitles

struct SubPlan {
    inputs: Vec<String>,
    args: Vec<String>,
    count: usize,
    prep: Vec<Prep>,
    /// Text subtitle burn: `subtitles=<file>` (relative to the work dir).
    burn_text: Option<String>,
    /// Bitmap subtitle burn: overlay source, e.g. "0:3" or "1:0".
    burn_bitmap: Option<String>,
    summary: String,
}

fn sub_ext_codec(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("srt") => "subrip",
        Some("ass") | Some("ssa") => "ass",
        Some("vtt") => "webvtt",
        Some("sup") => "hdmv_pgs_subtitle",
        _ => "unknown",
    }
}

#[allow(clippy::too_many_arguments)]
fn plan_subs(
    st: &SubtitleSettings,
    media: &MediaInfo,
    container: Container,
    caps: &Capabilities,
    reencode: bool,
    work_dir: &Path,
    issues: &mut Vec<Issue>,
    warnings: &mut Vec<Notice>,
) -> SubPlan {
    let mut p = SubPlan { inputs: Vec::new(), args: Vec::new(), count: 0, prep: Vec::new(), burn_text: None, burn_bitmap: None, summary: "none".into() };
    match st.mode {
        SubtitleMode::None => p.args.push("-sn".into()),
        SubtitleMode::Keep => {
            let mut converted = false;
            for (j, sub) in media.subtitles.iter().enumerate() {
                let track = (j + 1).to_string();
                let k = p.count;
                let codec = match container {
                    Container::Mkv => "copy",
                    Container::Mp4 if sub.codec == "mov_text" => "copy",
                    Container::Mp4 if TEXT_SUBS.contains(&sub.codec.as_str()) => {
                        warnings.push(Notice::new("subtitleConverted").with("track", &track).with("from", &sub.codec));
                        if sub.codec == "ass" || sub.codec == "ssa" {
                            warnings.push(Notice::new("assStylingLost").with("track", &track));
                        }
                        converted = true;
                        "mov_text"
                    }
                    Container::Mp4 => {
                        issues.push(
                            Issue::new(Notice::new("bitmapSubtitleInMp4").with("track", &track).with("codec", &sub.codec))
                                .fix("useMkv")
                                .fix("subtitlesNone"),
                        );
                        continue;
                    }
                };
                p.args.extend(["-map".into(), format!("0:{}", sub.index), format!("-c:s:{k}"), codec.into()]);
                p.count += 1;
            }
            for (i, ext) in st.external.iter().enumerate() {
                let input = (1 + i) as u32;
                let codec = sub_ext_codec(&ext.path);
                let k = p.count;
                let out_codec = match (container, codec) {
                    (_, "unknown") => {
                        issues.push(Issue::new(Notice::new("subtitleUnsupported").with("file", ext.path.display())));
                        continue;
                    }
                    (Container::Mkv, _) => "copy",
                    (Container::Mp4, "hdmv_pgs_subtitle") => {
                        issues.push(Issue::new(Notice::new("bitmapSubtitleInMp4").with("file", ext.path.display())).fix("useMkv"));
                        continue;
                    }
                    (Container::Mp4, _) => {
                        converted = true;
                        "mov_text"
                    }
                };
                p.inputs.extend(["-i".into(), ff_path(&ext.path)]);
                p.args.extend(["-map".into(), format!("{input}:0"), format!("-c:s:{k}"), out_codec.into()]);
                if let Some(lang) = ext.language.as_deref().filter(|l| !l.is_empty()) {
                    p.args.extend([format!("-metadata:s:s:{k}"), format!("language={lang}")]);
                }
                p.count += 1;
            }
            if p.count == 0 {
                p.args.push("-sn".into());
            }
            p.summary = match (p.count, converted) {
                (0, _) => "none".into(),
                (_, true) => "convert".into(),
                _ => "keep".into(),
            };
        }
        SubtitleMode::Burn => {
            p.args.push("-sn".into());
            p.summary = "burn".into();
            if !reencode {
                issues.push(Issue::new(Notice::new("burnNeedsReencode")).fix("subtitlesKeep"));
                return p;
            }
            let needs_libass = |issues: &mut Vec<Issue>| {
                let ok = caps.has_filter("subtitles");
                if !ok {
                    issues.push(Issue::new(Notice::new("burnNeedsLibass")).fix("subtitlesKeep"));
                }
                ok
            };
            match &st.burn {
                Some(BurnSource::Embedded { ordinal }) => match media.subtitles.get(*ordinal as usize) {
                    None => issues.push(Issue::new(Notice::new("burnSourceMissing")).fix("subtitlesKeep")),
                    Some(sub) if sub.bitmap => p.burn_bitmap = Some(format!("0:{}", sub.index)),
                    Some(sub) => {
                        if needs_libass(issues) {
                            let to = work_dir.join("burn.ass");
                            p.prep.push(Prep::ExtractSubtitle { stream_index: sub.index, to });
                            p.burn_text = Some("subtitles=burn.ass".into());
                        }
                    }
                },
                Some(BurnSource::External { path }) => match sub_ext_codec(path) {
                    "hdmv_pgs_subtitle" => {
                        p.inputs.extend(["-i".into(), ff_path(path)]);
                        p.burn_bitmap = Some("1:0".into());
                    }
                    "unknown" => issues.push(Issue::new(Notice::new("subtitleUnsupported").with("file", path.display()))),
                    _ => {
                        if needs_libass(issues) {
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("srt").to_ascii_lowercase();
                            let name = format!("burn.{ext}");
                            p.prep.push(Prep::CopyFile { from: path.clone(), to: work_dir.join(&name) });
                            p.burn_text = Some(format!("subtitles={name}"));
                        }
                    }
                },
                None => issues.push(Issue::new(Notice::new("burnSourceMissing")).fix("subtitlesKeep")),
            }
        }
    }
    p
}

// ---------------------------------------------------------------- video geometry

struct Geometry {
    filters: Vec<String>,
    width: u32,
    height: u32,
    fps: Option<f64>,
}

fn even(n: u32) -> u32 {
    n & !1
}

fn plan_geometry(t: &Transforms, width: u32, height: u32, issues: &mut Vec<Issue>) -> Geometry {
    let mut g = Geometry { filters: Vec::new(), width, height, fps: None };
    if let Some(c) = t.crop {
        let w = width.saturating_sub(c.left + c.right);
        let h = height.saturating_sub(c.top + c.bottom);
        if w < 16 || h < 16 {
            issues.push(Issue::new(Notice::new("cropInvalid")).fix("clearTransforms"));
        } else {
            let (w, h) = (even(w), even(h));
            g.filters.push(format!("crop={w}:{h}:{}:{}", even(c.left), even(c.top)));
            g.width = w;
            g.height = h;
        }
    }
    if let Some(r) = t.rotate {
        let f = match r {
            Rotate::Cw90 => "transpose=1",
            Rotate::Ccw90 => "transpose=2",
            Rotate::R180 => "hflip,vflip",
            Rotate::FlipH => "hflip",
            Rotate::FlipV => "vflip",
        };
        g.filters.push(f.into());
        if matches!(r, Rotate::Cw90 | Rotate::Ccw90) {
            std::mem::swap(&mut g.width, &mut g.height);
        }
    }
    match t.scale {
        Some(Scale::MaxHeight { height: max_h }) if g.height > max_h && max_h >= 16 => {
            let h = even(max_h);
            // FFmpeg's -2: round(h * iw / ih / 2) * 2
            let w = (((h as f64) * (g.width as f64) / (g.height as f64) / 2.0).round() as u32) * 2;
            g.filters.push(format!("scale=-2:{h}:flags=lanczos"));
            g.width = w;
            g.height = h;
        }
        Some(Scale::Exact { width: w, height: h }) if w >= 16 && h >= 16 => {
            let (w, h) = (even(w), even(h));
            g.filters.push(format!("scale={w}:{h}:flags=lanczos"));
            g.width = w;
            g.height = h;
        }
        Some(Scale::Exact { .. }) => issues.push(Issue::new(Notice::new("scaleInvalid")).fix("clearTransforms")),
        _ => {}
    }
    if let Some(fps) = t.fps {
        if fps > 0.0 && fps <= 240.0 {
            g.filters.push(format!("fps={}", trim_float(fps)));
            g.fps = Some(fps);
        } else {
            issues.push(Issue::new(Notice::new("fpsInvalid")).fix("clearTransforms"));
        }
    }
    g
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------------------------------------------------------------- pixel format

/// Lossy target pixel format: keep chroma layout and bit depth unless the user opted in.
fn choose_pix_fmt(
    v: &VideoStream,
    def: &EncoderDef,
    caps: &Capabilities,
    vs: &VideoSettings,
    issues: &mut Vec<Issue>,
    warnings: &mut Vec<Notice>,
) -> Option<String> {
    let supported: Vec<(String, crate::media::PixTraits)> = caps
        .encoder(def.name)
        .map(|e| e.pix_fmts.iter().filter_map(|p| pix_traits(p).map(|t| (p.clone(), t))).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, t)| !t.alpha && !t.rgb)
        .collect();
    let src = v.pix_fmt.clone().unwrap_or_else(|| "yuv420p".into());
    let st = pix_traits(&src).unwrap_or(crate::media::PixTraits { bit_depth: 8, chroma: Chroma::C420, alpha: false, rgb: false, full_range: false });
    if supported.iter().any(|(p, _)| *p == src) && !st.alpha {
        return Some(src);
    }
    let src_chroma = if st.rgb { Chroma::C444 } else { st.chroma };
    // Prefer planar "yuv…" names, then exact depth, then the smallest depth above it.
    let rank = |(p, t): &(String, crate::media::PixTraits)| (t.bit_depth, if p.starts_with("yuv") { 0 } else { 1 });
    let mut same_chroma: Vec<&(String, crate::media::PixTraits)> = supported.iter().filter(|(_, t)| t.chroma == src_chroma).collect();
    same_chroma.sort_by_key(|c| rank(c));
    if let Some((p, _)) = same_chroma.iter().find(|(_, t)| t.bit_depth >= st.bit_depth) {
        if st.rgb {
            warnings.push(Notice::new("rgbToYuv").with("to", p));
        }
        return Some(p.clone());
    }
    let alt_fixes = |issue: Issue, wanted: &dyn Fn(&str) -> bool| {
        let mut issue = issue;
        for alt in ["libx264", "libx265", "libsvtav1", "ffv1"] {
            if alt != def.name && caps.usable(alt) && caps.encoder(alt).map(|e| e.pix_fmts.iter().any(|p| wanted(p))).unwrap_or(false) {
                issue = issue.fix(fix_for_encoder(alt));
            }
        }
        issue
    };
    if same_chroma.is_empty() {
        // The encoder cannot keep this chroma layout.
        if !vs.allow_chroma_subsampling {
            let issue = Issue::new(Notice::new("chromaUnsupported").with("encoder", def.name).with("chroma", format!("{src_chroma:?}").trim_start_matches('C')))
                .fix("allowChromaSubsampling");
            let wanted = |p: &str| pix_traits(p).map(|t| t.chroma == src_chroma && !t.rgb).unwrap_or(false);
            issues.push(alt_fixes(issue, &wanted));
            return None;
        }
        let mut lower: Vec<&(String, crate::media::PixTraits)> =
            supported.iter().filter(|(_, t)| t.chroma.rank() < src_chroma.rank() && t.chroma != Chroma::Gray).collect();
        lower.sort_by_key(|(p, t)| (std::cmp::Reverse(t.chroma.rank()), t.bit_depth < st.bit_depth, t.bit_depth, if p.starts_with("yuv") { 0 } else { 1 }));
        let Some((p, t)) = lower.first() else {
            issues.push(Issue::new(Notice::new("chromaUnsupported").with("encoder", def.name)));
            return None;
        };
        warnings.push(Notice::new("chromaSubsampled").with("from", &src).with("to", p));
        if t.bit_depth < st.bit_depth {
            if !vs.allow_bit_depth_reduction {
                issues.push(Issue::new(Notice::new("bitDepthUnsupported").with("encoder", def.name).with("depth", st.bit_depth)).fix("allowBitDepthReduction"));
                return None;
            }
            warnings.push(Notice::new("bitDepthReduced").with("from", st.bit_depth).with("to", t.bit_depth));
        }
        return Some(p.clone());
    }
    // Chroma fits, depth does not.
    if !vs.allow_bit_depth_reduction {
        let issue = Issue::new(Notice::new("bitDepthUnsupported").with("encoder", def.name).with("depth", st.bit_depth)).fix("allowBitDepthReduction");
        let wanted = |p: &str| pix_traits(p).map(|t| t.bit_depth >= st.bit_depth && t.chroma == src_chroma).unwrap_or(false);
        issues.push(alt_fixes(issue, &wanted));
        return None;
    }
    let best = same_chroma.iter().filter(|(_, t)| t.bit_depth < st.bit_depth).max_by_key(|(p, t)| (t.bit_depth, if p.starts_with("yuv") { 1 } else { 0 }))?;
    warnings.push(Notice::new("bitDepthReduced").with("from", st.bit_depth).with("to", best.1.bit_depth));
    Some(best.0.clone())
}

// ---------------------------------------------------------------- estimates

fn memory_estimate_mb(def: Option<&EncoderDef>, width: u32, height: u32, speed: Speed) -> u64 {
    let mp = (width as f64 * height as f64) / 1e6;
    let (base, per_mp) = match def.map(|d| d.name) {
        None => (80.0, 0.0),
        Some("libx264") => (150.0, 120.0),
        Some("libx265") => (250.0, 250.0),
        Some("libsvtav1") => (400.0, if speed == Speed::Slow { 900.0 } else { 600.0 }),
        Some("libaom-av1") => (300.0, 500.0),
        Some("ffv1") => (100.0, 60.0),
        Some(_) => (150.0, 60.0),
    };
    (base + per_mp * mp) as u64
}

fn raw_lossless_estimate(v: &VideoStream, duration: f64) -> u64 {
    let bytes_per_sample = if v.bit_depth.unwrap_or(8) > 8 { 2.0 } else { 1.0 };
    let samples = match v.chroma {
        Some(Chroma::C444) => 3.0,
        Some(Chroma::C422) | Some(Chroma::C440) => 2.0,
        Some(Chroma::Gray) => 1.0,
        _ => 1.5,
    };
    let frame = v.width as f64 * v.height as f64 * samples * bytes_per_sample;
    (frame * v.fps.unwrap_or(30.0) * duration / 2.5) as u64
}

fn color_of(v: &VideoStream) -> ColorTags {
    ColorTags {
        primaries: v.color_primaries.clone(),
        transfer: v.color_transfer.clone(),
        space: v.color_space.clone(),
        range: if v.full_range() { Some("pc".into()) } else { v.color_range.clone() },
    }
}

// ---------------------------------------------------------------- main entry

pub fn plan_convert(settings: &JobSettings, media: &MediaInfo, env: &PlanEnv) -> Result<EncodePlan, PlanError> {
    let mut issues: Vec<Issue> = Vec::new();
    let mut warnings: Vec<Notice> = Vec::new();
    let caps = env.caps;
    let vs = &settings.video;
    let out = &settings.output;
    let container = out.container;
    let accepted = |code: &str| settings.accept.iter().any(|c| c == code);

    if media.path.to_str().is_none() {
        issues.push(Issue::new(Notice::new("pathNotUtf8")));
    }
    if crate::fsx::is_owned_temp(&media.path) {
        issues.push(Issue::new(Notice::new("inputIsTemp")));
    }
    let Some(v) = media.main_video() else {
        return Err(PlanError::Blocked(vec![Issue::new(Notice::new("noVideoStream"))]));
    };
    if media.video.iter().any(|x| x.attached_pic) {
        warnings.push(Notice::new("coverArtDropped"));
    }
    if v.vfr {
        warnings.push(Notice::new("vfrSource"));
    }
    if v.interlaced && vs.mode != VideoMode::Copy {
        warnings.push(Notice::new("interlacedSource").with("fieldOrder", v.field_order.clone().unwrap_or_default()));
    }
    let src_duration = media.duration.or(v.duration);

    // ---- time range
    let mut trim_range: Option<(f64, Option<f64>)> = None;
    if let Some(t) = settings.trim {
        let end_ok = t.end.map(|e| e > t.start + 0.01).unwrap_or(true);
        let within = src_duration.map(|d| t.start < d).unwrap_or(true);
        if t.start < 0.0 || !end_ok || !within {
            issues.push(Issue::new(Notice::new("trimOutOfRange")));
        } else {
            let end = t.end.filter(|e| src_duration.map(|d| *e < d).unwrap_or(true));
            trim_range = Some((t.start, end));
        }
    }
    // ---- incomplete source (an unfinished download): refuse, or, once the user accepts,
    // process only the part that can be read
    if let Some(readable) = media.readable_until {
        let start = trim_range.map(|(s, _)| s).unwrap_or(0.0);
        let past_readable = trim_range.and_then(|(_, e)| e).map(|e| e > readable).unwrap_or(true);
        if past_readable {
            if !accepted("sourceIncomplete") {
                let notice = Notice::new("sourceIncomplete").with("readable", clock(readable)).with("declared", src_duration.map(clock).unwrap_or_default());
                issues.push(Issue::new(notice).fix("acceptIncomplete"));
            } else if start + 0.01 < readable {
                trim_range = Some((start, Some(readable)));
                warnings.push(Notice::new("sourceIncompleteTrimmed").with("readable", clock(readable)));
            } else {
                issues.push(Issue::new(Notice::new("trimOutOfRange")));
            }
        }
    }
    let mut out_duration = match trim_range {
        Some((start, Some(end))) => Some(end - start),
        Some((start, None)) => src_duration.map(|d| d - start),
        None => src_duration,
    };
    let reencode = vs.mode != VideoMode::Copy;
    let hash_ok = trim_range.is_none();

    let audio = plan_audio(&settings.audio, media, container, caps, 1, hash_ok, &mut issues, &mut warnings);
    let subs = plan_subs(&settings.subtitles, media, container, caps, reencode, env.work_dir, &mut issues, &mut warnings);
    if subs.count > 0 && container == Container::Mkv && media.attachments > 0 {
        // fonts for ASS subtitles are carried along (handled below)
    } else if media.attachments > 0 {
        warnings.push(Notice::new("attachmentsDropped").with("count", media.attachments));
    }

    let mut summary = PlanSummary {
        kind: if reencode { "convert".into() } else { "remux".into() },
        container: container.ext().into(),
        audio: audio.summary.clone(),
        subtitles: subs.summary.clone(),
        threads: Some(env.threads),
        ..Default::default()
    };

    let mut input_opts: Vec<String> = Vec::new();
    let mut out_opts: Vec<String> = Vec::new();
    let mut passes: Vec<Pass> = Vec::new();
    let expected_video: ExpectedVideo;
    let mut hashes: Vec<StreamCopy> = Vec::new();
    let est_upper: u64;
    let mut est_size: Option<u64> = None;
    let mut reliability = "unknown";
    let mut encoder_name: Option<String> = None;
    let mut def_used: Option<&'static EncoderDef> = None;
    let mut hw_encoder = false;
    let mut lossless_opts: Option<Vec<String>> = None;
    let verify_weight;

    let frames_for = |fps_change: Option<f64>| -> FrameCheck {
        match (fps_change, trim_range) {
            (Some(fps), _) => out_duration.map(|d| FrameCheck::Approx((d * fps).round() as u64)).unwrap_or(FrameCheck::Skip),
            (None, Some(_)) => match (out_duration, v.fps) {
                (Some(d), Some(f)) if !v.vfr => FrameCheck::Approx((d * f).round() as u64),
                _ => FrameCheck::Skip,
            },
            (None, None) => FrameCheck::SameAsSource,
        }
    };

    match vs.mode {
        VideoMode::Copy => {
            if !video_copy_ok(&v.codec, container) {
                let mut i = Issue::new(Notice::new("videoCopyIncompatible").with("codec", &v.codec).with("container", container.ext()));
                if container == Container::Mp4 {
                    i = i.fix("useMkv");
                }
                issues.push(i.fix("modeLossy"));
            }
            if !vs.transforms.is_identity() {
                issues.push(Issue::new(Notice::new("copyWithTransforms")).fix("clearTransforms"));
            }
            if let Some((start, end)) = trim_range {
                let kf = env
                    .keyframes
                    .and_then(|k| k.iter().copied().filter(|t| *t <= start + 1e-6).fold(None, |acc: Option<f64>, t| Some(acc.map_or(t, |a| a.max(t)))));
                let actual = kf.unwrap_or(start);
                if (actual - start).abs() > 0.001 {
                    warnings.push(Notice::new("keyframeTrim").with("requested", secs(start)).with("actual", secs(actual)));
                }
                if env.keyframes.is_none() {
                    warnings.push(Notice::new("keyframeUnknown"));
                }
                input_opts.extend(["-ss".into(), secs(actual)]);
                if let Some(e) = end {
                    out_opts.extend(["-t".into(), secs(e - actual)]);
                }
                // The copy starts at the keyframe, so the output is that much longer.
                out_duration = match end {
                    Some(e) => Some(e - actual),
                    None => src_duration.map(|d| d - actual),
                };
                out_opts.extend(strs(&["-avoid_negative_ts", "make_zero"]));
            }
            if hash_ok {
                hashes.push(StreamCopy { input: 0, source_index: v.index, output_index: 0 });
            }
            hashes.extend(audio.hashes.iter().copied());
            let mut a = head();
            a.extend(input_opts.clone());
            a.extend(["-i".into(), ff_path(&media.path)]);
            a.extend(subs.inputs.clone());
            a.extend(out_opts.clone());
            a.extend(["-map".into(), format!("0:{}", v.index), "-c:v".into(), "copy".into()]);
            if container == Container::Mp4 && v.codec == "hevc" {
                a.extend(strs(&["-tag:v", "hvc1"]));
            }
            a.extend(audio.args.clone());
            a.extend(subs.args.clone());
            if container == Container::Mkv && subs.count > 0 && media.attachments > 0 {
                a.extend(strs(&["-map", "0:t?", "-c:t", "copy"]));
            }
            a.extend(metadata_args(out));
            a.extend(container_tail(container, out, env.temp_output));
            verify_weight = if hash_ok { 0.3 } else { 0.25 };
            passes.push(Pass { args: a, label: None, weight: 1.0 - verify_weight, stage: JobState::Muxing });
            let fraction = match (out_duration, src_duration) {
                (Some(o), Some(sd)) if sd > 0.0 => (o / sd).clamp(0.0, 1.0),
                _ => 1.0,
            };
            est_size = Some((media.size as f64 * fraction) as u64);
            est_upper = (media.size as f64 * fraction * 1.05) as u64 + 16_000_000;
            reliability = "estimate";
            summary.video = "copy".into();
            summary.quality = Some("copy".into());
            expected_video = ExpectedVideo {
                codec: v.codec.clone(),
                width: v.width,
                height: v.height,
                pix_fmt: v.pix_fmt.clone(),
                frames: if hash_ok { FrameCheck::SameAsSource } else { FrameCheck::Skip },
                color: color_of(v),
                hdr_metadata: false,
                rotation: v.rotation,
            };
        }
        VideoMode::Lossless | VideoMode::Lossy => {
            let lossless = vs.mode == VideoMode::Lossless;
            let def = match choose_encoder(vs, caps) {
                Ok(d) => d,
                Err(i) => return Err(PlanError::Blocked(vec![i])),
            };
            def_used = Some(def);
            encoder_name = Some(def.name.into());
            hw_encoder = def.is_hardware();
            if !def.containers.contains(&container) {
                issues.push(Issue::new(Notice::new("containerUnsupported").with("encoder", def.name).with("container", container.ext())).fix("useMkv"));
            }
            if def.codec == VideoCodec::Ffv1 && !lossless {
                issues.push(Issue::new(Notice::new("ffv1LossyUnsupported")).fix("modeLossless"));
            }
            // HDR / Dolby Vision
            if v.hdr != HdrKind::Sdr && !lossless && !HDR_ENCODERS.contains(&def.name) {
                let mut i = Issue::new(Notice::new("hdrUnsupportedEncoder").with("encoder", def.name).with("hdr", format!("{:?}", v.hdr)));
                for alt in HDR_ENCODERS {
                    if caps.usable(alt) {
                        i = i.fix(fix_for_encoder(alt));
                    }
                }
                issues.push(i);
            }
            if let Some(dv) = v.dolby_vision {
                if dv.bl_compat_id == 0 || dv.profile == 5 {
                    issues.push(Issue::new(Notice::new("dolbyVisionProfile5").with("profile", dv.profile)));
                } else if !accepted("dolbyVisionLoss") {
                    issues.push(Issue::new(Notice::new("dolbyVisionLoss").with("profile", dv.profile)).fix("acceptDolbyVisionLoss"));
                } else {
                    warnings.push(Notice::new("dolbyVisionDropped"));
                }
            }
            if lossless && (!subs.prep.is_empty() || subs.burn_bitmap.is_some() || subs.burn_text.is_some()) {
                issues.push(Issue::new(Notice::new("losslessWithBurn")).fix("subtitlesKeep"));
            }
            if v.rotation != 0 {
                warnings.push(Notice::new("rotationApplied").with("degrees", v.rotation));
            }
            let (dw, dh) = v.display_dims();

            // ---- pixel format + geometry
            let mut pix_target: Option<String> = None;
            let mut geometry = Geometry { filters: Vec::new(), width: dw, height: dh, fps: None };
            let mut enc_pix: Option<String> = v.pix_fmt.clone();
            if lossless {
                if def.lossless == Lossless::No {
                    let mut i = Issue::new(Notice::new("losslessUnsupported").with("encoder", def.name));
                    for alt in ["libx264", "libx265", "ffv1"] {
                        if caps.usable(alt) {
                            i = i.fix(fix_for_encoder(alt));
                        }
                    }
                    issues.push(i);
                }
                if !vs.transforms.is_identity() {
                    issues.push(Issue::new(Notice::new("losslessWithTransforms")).fix("clearTransforms"));
                }
                match v.pix_fmt.as_deref() {
                    None => issues.push(Issue::new(Notice::new("pixFmtUnknown"))),
                    Some(src) => {
                        let norm = crate::media::plain_pix_fmt(src);
                        let key = if caps.supports_pix_fmt(def.name, src) {
                            Some(src.to_string())
                        } else if caps.supports_pix_fmt(def.name, &norm) {
                            Some(norm.clone())
                        } else {
                            None
                        };
                        match key {
                            None => {
                                let mut i = Issue::new(Notice::new("losslessPixFmtUnsupported").with("encoder", def.name).with("pixFmt", src));
                                for alt in ["libx264", "libx265", "libsvtav1", "libaom-av1", "ffv1"] {
                                    let fits = caps.supports_pix_fmt(alt, src) || caps.supports_pix_fmt(alt, &norm);
                                    let not_failed = caps.lossless_ok(alt, &norm) != Some(false) && caps.lossless_ok(alt, src) != Some(false);
                                    if alt != def.name && caps.usable(alt) && fits && not_failed {
                                        i = i.fix(fix_for_encoder(alt));
                                    }
                                }
                                issues.push(i);
                            }
                            Some(key) => {
                                if issues.is_empty() {
                                    match caps.lossless_ok(def.name, &key) {
                                        Some(true) => {}
                                        Some(false) => issues.push(Issue::new(
                                            Notice::new("losslessNotVerified").with("encoder", def.name).with("pixFmt", &key),
                                        )),
                                        None => return Err(PlanError::NeedsSelfTest { encoder: def.name.into(), pix_fmt: key }),
                                    }
                                }
                                enc_pix = Some(if key == norm && src != norm && !caps.supports_pix_fmt(def.name, src) { norm } else { src.to_string() });
                            }
                        }
                    }
                }
                if v.alpha && !enc_pix.as_deref().map(|p| pix_traits(p).map(|t| t.alpha).unwrap_or(false)).unwrap_or(false) {
                    issues.push(Issue::new(Notice::new("alphaDropped")).fix("useFfv1"));
                }
            } else {
                if v.alpha {
                    if accepted("alphaDropped") {
                        warnings.push(Notice::new("alphaDropped"));
                    } else {
                        issues.push(Issue::new(Notice::new("alphaDropped")).fix("acceptAlphaDrop"));
                    }
                }
                pix_target = choose_pix_fmt(v, def, caps, vs, &mut issues, &mut warnings);
                enc_pix = pix_target.clone();
                geometry = plan_geometry(&vs.transforms, dw, dh, &mut issues);
            }

            // ---- rate control
            let rate = if lossless {
                RateArgs::Lossless
            } else {
                match &vs.rate {
                    RateControl::Level | RateControl::Native { .. } => match def.quality {
                        None => {
                            issues.push(Issue::new(Notice::new("qualityModeUnsupported").with("encoder", def.name)).fix("useBitrate"));
                            RateArgs::Quality(0.0)
                        }
                        Some(q) => {
                            if def.is_hardware() && !caps.encoder(def.name).map(|e| e.quality_mode).unwrap_or(false) {
                                issues.push(Issue::new(Notice::new("qualityModeUnsupported").with("encoder", def.name)).fix("useBitrate"));
                            }
                            let value = match vs.rate {
                                RateControl::Native { value } => {
                                    if value < q.min || value > q.max {
                                        issues.push(Issue::new(Notice::new("qualityOutOfRange").with("min", q.min).with("max", q.max)));
                                    }
                                    value
                                }
                                _ => encoders::level_to_native(&q, vs.level),
                            };
                            summary.quality = Some(format!("{}:{}", kind_token(q.kind), encoders::fmt_value(&q, value)));
                            RateArgs::Quality(value)
                        }
                    },
                    RateControl::Bitrate { kbps, two_pass } => {
                        if *two_pass && !def.two_pass {
                            issues.push(Issue::new(Notice::new("twoPassUnsupported").with("encoder", def.name)).fix("singlePass"));
                        }
                        summary.quality = Some(format!("bitrate:{kbps}"));
                        reliability = "estimate";
                        RateArgs::Bitrate(*kbps)
                    }
                    RateControl::TargetSize { megabytes, two_pass } => {
                        if *two_pass && !def.two_pass {
                            issues.push(Issue::new(Notice::new("twoPassUnsupported").with("encoder", def.name)).fix("singlePass"));
                        }
                        summary.quality = Some(format!("size:{megabytes}"));
                        reliability = "estimate";
                        match out_duration.filter(|d| *d > 0.0) {
                            None => {
                                issues.push(Issue::new(Notice::new("durationUnknown")).fix("useBitrate"));
                                RateArgs::Bitrate(1000)
                            }
                            Some(d) => {
                                let total_kbps = megabytes * 8_000.0 / d;
                                let video_kbps = total_kbps * 0.98 - audio.kbps as f64;
                                if video_kbps < 50.0 {
                                    let min_mb = ((audio.kbps as f64 + 50.0) / 0.98) * d / 8_000.0;
                                    issues.push(Issue::new(Notice::new("targetSizeTooSmall").with("minMb", format!("{:.1}", min_mb.ceil()))));
                                    RateArgs::Bitrate(50)
                                } else {
                                    RateArgs::Bitrate(video_kbps.floor() as u32)
                                }
                            }
                        }
                    }
                }
            };
            if let RateArgs::Bitrate(k) = rate {
                if !def.bitrate {
                    issues.push(Issue::new(Notice::new("bitrateUnsupported").with("encoder", def.name)));
                }
                if let Some(d) = out_duration {
                    let bytes = ((k + audio.kbps) as f64 * 1000.0 / 8.0 * d * 1.02) as u64;
                    est_size = Some(bytes);
                }
            }
            if lossless {
                summary.quality = Some("lossless".into());
            }
            let two_pass = matches!(vs.rate, RateControl::Bitrate { two_pass: true, .. } | RateControl::TargetSize { two_pass: true, .. })
                && !lossless
                && def.two_pass;

            if !issues.is_empty() {
                return Err(PlanError::Blocked(issues));
            }

            // ---- inputs
            if !lossless
                && vs.hw_decode
                && let Some(accel) = caps.hw_decode_for(&v.codec)
            {
                input_opts.extend(["-hwaccel".into(), accel]);
            }
            let logical = caps.cpu.logical.max(1);
            if env.threads > 0 && env.threads < logical {
                input_opts.extend(["-threads".into(), env.threads.to_string()]);
            }
            if let Some((start, end)) = trim_range {
                input_opts.extend(["-ss".into(), secs(start)]);
                if let Some(e) = end {
                    out_opts.extend(["-t".into(), secs(e - start)]);
                }
            }
            if lossless {
                let mut src_opts = Vec::new();
                if let Some((start, end)) = trim_range {
                    src_opts.extend(["-ss".into(), secs(start)]);
                    if let Some(e) = end {
                        src_opts.extend(["-t".into(), secs(e - start)]);
                    }
                }
                lossless_opts = Some(src_opts);
            }

            // ---- video graph
            let mut chain: Vec<String> = Vec::new();
            if let Some(f) = &subs.burn_text {
                chain.push(f.clone());
            }
            chain.extend(geometry.filters.iter().cloned());
            let mut graph: Vec<String> = Vec::new();
            let video_map: String;
            if let Some(src) = &subs.burn_bitmap {
                let rest = if chain.is_empty() { "null".to_string() } else { chain.join(",") };
                graph.extend(["-filter_complex".into(), format!("[0:{}][{src}]overlay=eof_action=pass[fe_b];[fe_b]{rest}[fe_v]", v.index)]);
                video_map = "[fe_v]".into();
            } else {
                if !chain.is_empty() {
                    graph.extend(["-vf".into(), chain.join(",")]);
                }
                video_map = format!("0:{}", v.index);
            }
            if !chain.is_empty() && env.threads < logical {
                graph.extend(["-filter_threads".into(), env.threads.to_string()]);
            }
            let fps_mode = if geometry.fps.is_some() { "cfr" } else { "passthrough" };

            let video_enc = |pass: Option<(u8, String)>| {
                let mut a = encoders::video_args(
                    def,
                    &EncArgs { rate: rate.clone(), speed: vs.speed, threads: Some(env.threads), logical_cpus: logical, pass },
                );
                if let Some(p) = &pix_target {
                    a.extend(["-pix_fmt".into(), p.clone()]);
                }
                if container == Container::Mp4 && def.codec == VideoCodec::Hevc {
                    a.extend(strs(&["-tag:v", "hvc1"]));
                }
                a
            };
            let base_inputs = |a: &mut Vec<String>, with_subs: bool| {
                a.extend(input_opts.clone());
                a.extend(["-i".into(), ff_path(&media.path)]);
                if with_subs {
                    a.extend(subs.inputs.clone());
                }
            };
            let mut main = head();
            base_inputs(&mut main, true);
            main.extend(out_opts.clone());
            main.extend(graph.clone());
            main.extend(["-map".into(), video_map.clone()]);
            main.extend(video_enc(two_pass.then(|| (2u8, STATS_FILE.to_string()))));
            main.extend([caps.fps_mode_option().into(), fps_mode.into()]);
            main.extend(audio.args.clone());
            main.extend(subs.args.clone());
            if container == Container::Mkv && subs.count > 0 && media.attachments > 0 {
                main.extend(strs(&["-map", "0:t?", "-c:t", "copy"]));
            }
            main.extend(metadata_args(out));
            main.extend(container_tail(container, out, env.temp_output));

            verify_weight = if lossless { 0.35 } else { 0.12 };
            let encode_share = 1.0 - verify_weight;
            if two_pass {
                let mut first = head();
                base_inputs(&mut first, subs.burn_bitmap.is_some());
                first.extend(out_opts.clone());
                first.extend(graph);
                first.extend(["-map".into(), video_map]);
                first.extend(video_enc(Some((1, STATS_FILE.to_string()))));
                first.extend([caps.fps_mode_option().into(), fps_mode.into()]);
                first.extend(strs(&["-an", "-sn", "-dn", "-f", "null", "-"]));
                passes.push(Pass { args: first, label: Some("1/2".into()), weight: encode_share * 0.35, stage: JobState::Encoding });
                passes.push(Pass { args: main, label: Some("2/2".into()), weight: encode_share * 0.65, stage: JobState::Encoding });
            } else {
                passes.push(Pass { args: main, label: None, weight: encode_share, stage: JobState::Encoding });
            }

            // Copied audio is hash-verified even when the video is re-encoded.
            hashes.extend(audio.hashes.iter().copied());
            summary.video = def.codec.probe_name().into();
            summary.encoder = Some(def.name.into());
            summary.lossless = lossless;
            summary.hw = hw_encoder;
            let d = out_duration.unwrap_or(0.0);
            est_upper = if lossless {
                let e = raw_lossless_estimate(v, d);
                est_size = Some(e);
                reliability = "estimate";
                warnings.push(Notice::new("losslessLarge"));
                (e as f64 * 1.6) as u64
            } else {
                match est_size {
                    Some(e) => (e as f64 * 1.25) as u64,
                    None => (media.size as f64 * 1.5) as u64,
                }
            } + 32_000_000;
            expected_video = ExpectedVideo {
                codec: def.codec.probe_name().into(),
                width: if lossless { dw } else { geometry.width },
                height: if lossless { dh } else { geometry.height },
                pix_fmt: if lossless { enc_pix.clone() } else { pix_target.clone() },
                frames: frames_for(geometry.fps),
                color: color_of(v),
                hdr_metadata: v.hdr != HdrKind::Sdr && v.mastering_display.is_some(),
                rotation: 0,
            };
        }
    }
    if !issues.is_empty() {
        return Err(PlanError::Blocked(issues));
    }
    summary.reencode_video = reencode;
    summary.est_size = est_size;
    summary.est_reliability = reliability.into();
    let est_memory_mb = memory_estimate_mb(def_used, v.width, v.height, vs.speed);
    summary.est_memory_mb = Some(est_memory_mb);
    summary.est_disk_bytes = Some(est_upper);
    Ok(EncodePlan {
        passes,
        prep: subs.prep,
        expected: Expected {
            container: Some(container),
            video: Some(expected_video),
            audio: audio.expected,
            subtitles: subs.count,
            duration: out_duration,
            hash_streams: hashes,
            lossless_source_opts: lossless_opts,
            source_video_index: Some(v.index),
        },
        summary,
        warnings,
        duration: out_duration,
        encoder: encoder_name,
        est_upper_bytes: est_upper,
        est_memory_mb,
        hw_encoder,
        verify_weight,
    })
}

// ---------------------------------------------------------------- mux / extract / fixes

/// File extension, muxer and optional conversion for extracting one stream.
pub fn extract_target(kind: &str, codec: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match (kind, codec) {
        ("video", "h264" | "hevc" | "av1" | "mpeg4") => ("mp4", "mp4", None),
        ("video", _) => ("mkv", "matroska", None),
        ("audio", "aac" | "alac") => ("m4a", "ipod", None),
        ("audio", "mp3") => ("mp3", "mp3", None),
        ("audio", "ac3") => ("ac3", "ac3", None),
        ("audio", "eac3") => ("eac3", "eac3", None),
        ("audio", "flac") => ("flac", "flac", None),
        ("audio", "opus") => ("opus", "opus", None),
        ("audio", "vorbis") => ("ogg", "ogg", None),
        ("audio", "dts") => ("dts", "dts", None),
        ("audio", "truehd") => ("thd", "truehd", None),
        ("audio", c) if c.starts_with("pcm_") && (c.ends_with("le") || c == "pcm_u8") => ("wav", "wav", None),
        ("audio", _) => ("mka", "matroska", None),
        ("subtitle", "subrip" | "srt") => ("srt", "srt", None),
        ("subtitle", "ass" | "ssa") => ("ass", "ass", None),
        ("subtitle", "webvtt") => ("vtt", "webvtt", None),
        ("subtitle", "mov_text" | "text") => ("srt", "srt", Some("srt")),
        ("subtitle", "hdmv_pgs_subtitle") => ("sup", "sup", None),
        _ => ("mks", "matroska", None),
    }
}

/// Kind and codec of the stream with this absolute index.
pub fn stream_kind(media: &MediaInfo, index: u32) -> Option<(&'static str, String)> {
    if let Some(v) = media.video.iter().find(|v| v.index == index) {
        return Some(("video", v.codec.clone()));
    }
    if let Some(a) = media.audio.iter().find(|a| a.index == index) {
        return Some(("audio", a.codec.clone()));
    }
    media.subtitles.iter().find(|s| s.index == index).map(|s| ("subtitle", s.codec.clone()))
}

/// Extract one stream without re-encoding (text subs in MP4 become SRT).
pub fn plan_extract(spec: &ExtractSpec, media: &MediaInfo, env: &PlanEnv) -> Result<EncodePlan, PlanError> {
    let Some((kind, codec)) = stream_kind(media, spec.stream_index) else {
        return Err(PlanError::Blocked(vec![Issue::new(Notice::new("streamMissing").with("stream", spec.stream_index))]));
    };
    let (ext, muxer, conv) = extract_target(kind, &codec);
    let idx = spec.stream_index;
    let mut a = head();
    a.extend(["-i".into(), ff_path(&media.path), "-map".into(), format!("0:{idx}")]);
    match conv {
        Some(c) => a.extend(["-c:s".into(), c.into()]),
        None => a.extend(strs(&["-c", "copy"])),
    }
    if kind == "video" && codec == "hevc" && ext == "mp4" {
        a.extend(strs(&["-tag:v", "hvc1"]));
    }
    a.extend(["-f".into(), muxer.into(), ff_path(env.temp_output)]);
    let mut warnings = Vec::new();
    if conv.is_some() {
        warnings.push(Notice::new("subtitleConverted").with("track", idx).with("from", &codec));
    }
    let video = media.video.iter().find(|v| v.index == idx).map(|v| ExpectedVideo {
        codec: v.codec.clone(),
        width: v.width,
        height: v.height,
        pix_fmt: v.pix_fmt.clone(),
        frames: FrameCheck::SameAsSource,
        color: color_of(v),
        hdr_metadata: false,
        rotation: v.rotation,
    });
    let audio: Vec<ExpectedAudio> = media
        .audio
        .iter()
        .filter(|x| x.index == idx)
        .map(|x| ExpectedAudio { source_index: idx, codec: x.codec.clone(), copied: true })
        .collect();
    let stream_duration = media.audio.iter().find(|x| x.index == idx).and_then(|x| x.duration)
        .or_else(|| media.video.iter().find(|x| x.index == idx).and_then(|x| x.duration));
    let duration = if kind == "subtitle" { None } else { stream_duration.or(media.duration) };
    let summary = PlanSummary {
        kind: "extract".into(),
        video: kind.into(),
        container: ext.into(),
        audio: if kind == "audio" { "copy".into() } else { "none".into() },
        subtitles: if kind == "subtitle" { "keep".into() } else { "none".into() },
        quality: Some("copy".into()),
        est_reliability: "unknown".into(),
        threads: Some(1),
        ..Default::default()
    };
    Ok(EncodePlan {
        passes: vec![Pass { args: a, label: None, weight: 0.7, stage: JobState::Muxing }],
        prep: Vec::new(),
        expected: Expected {
            container: None,
            video,
            audio,
            subtitles: usize::from(kind == "subtitle"),
            duration,
            hash_streams: if conv.is_none() { vec![StreamCopy { input: 0, source_index: idx, output_index: 0 }] } else { Vec::new() },
            lossless_source_opts: None,
            source_video_index: (kind == "video").then_some(idx),
        },
        summary,
        warnings,
        duration,
        encoder: None,
        est_upper_bytes: media.size + 16_000_000,
        est_memory_mb: 80,
        hw_encoder: false,
        verify_weight: 0.3,
    })
}

/// Combine a main file with external audio/subtitle files, all stream-copied.
pub fn plan_mux(spec: &MuxSpec, main: &MediaInfo, tracks: &[MediaInfo], env: &PlanEnv) -> Result<EncodePlan, PlanError> {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let container = spec.output.container;
    let Some(v) = main.main_video() else {
        return Err(PlanError::Blocked(vec![Issue::new(Notice::new("noVideoStream"))]));
    };
    if !video_copy_ok(&v.codec, container) {
        issues.push(Issue::new(Notice::new("videoCopyIncompatible").with("codec", &v.codec).with("container", container.ext())).fix("useMkv"));
    }
    let mut a = head();
    a.extend(["-i".into(), ff_path(&main.path)]);
    for t in tracks {
        a.extend(["-i".into(), ff_path(&t.path)]);
    }
    let mut maps: Vec<String> = vec!["-map".into(), format!("0:{}", v.index)];
    let mut per_stream: Vec<String> = Vec::new();
    let mut expected_audio = Vec::new();
    let mut hashes = vec![StreamCopy { input: 0, source_index: v.index, output_index: 0 }];
    let mut out_index = 1u32;
    let mut sub_count = 0usize;
    let mut add_audio = |input: u32, st: &crate::media::AudioStream, issues: &mut Vec<Issue>, maps: &mut Vec<String>| -> Option<u32> {
        if !audio_copy_ok(&st.codec, container) {
            issues.push(Issue::new(Notice::new("audioIncompatible").with("codec", &st.codec).with("container", container.ext())).fix("useMkv"));
            return None;
        }
        maps.extend(["-map".into(), format!("{input}:{}", st.index)]);
        expected_audio.push(ExpectedAudio { source_index: st.index, codec: st.codec.clone(), copied: true });
        hashes.push(StreamCopy { input, source_index: st.index, output_index: out_index });
        out_index += 1;
        Some(out_index - 1)
    };
    if spec.keep_main_audio {
        for st in &main.audio {
            add_audio(0, st, &mut issues, &mut maps);
        }
    }
    let mut track_meta: Vec<(u32, &MuxTrack)> = Vec::new();
    for (i, (t, m)) in spec.tracks.iter().zip(tracks.iter()).enumerate() {
        let input = (i + 1) as u32;
        if t.kind == TrackKind::Audio {
            match m.audio.first() {
                Some(st) => {
                    if let Some(o) = add_audio(input, st, &mut issues, &mut maps) {
                        track_meta.push((o, t));
                    }
                }
                None => issues.push(Issue::new(Notice::new("trackNoStream").with("file", t.path.display()))),
            }
        }
    }
    let mut add_sub = |input: u32, sub: &crate::media::SubtitleStream, issues: &mut Vec<Issue>, out_index: &mut u32| -> Option<u32> {
        let k = sub_count;
        let codec = match container {
            Container::Mkv => "copy",
            Container::Mp4 if sub.codec == "mov_text" => "copy",
            Container::Mp4 if TEXT_SUBS.contains(&sub.codec.as_str()) => "mov_text",
            Container::Mp4 => {
                issues.push(Issue::new(Notice::new("bitmapSubtitleInMp4").with("codec", &sub.codec)).fix("useMkv"));
                return None;
            }
        };
        maps.extend(["-map".into(), format!("{input}:{}", sub.index)]);
        if codec != "copy" {
            per_stream.extend([format!("-c:s:{k}"), codec.into()]);
        }
        sub_count += 1;
        *out_index += 1;
        Some(*out_index - 1)
    };
    if spec.keep_main_subtitles {
        for sub in &main.subtitles {
            add_sub(0, sub, &mut issues, &mut out_index);
        }
    }
    for (i, (t, m)) in spec.tracks.iter().zip(tracks.iter()).enumerate() {
        if t.kind == TrackKind::Subtitle {
            match m.subtitles.first() {
                Some(sub) => {
                    if let Some(o) = add_sub((i + 1) as u32, sub, &mut issues, &mut out_index) {
                        track_meta.push((o, t));
                    }
                }
                None => issues.push(Issue::new(Notice::new("trackNoStream").with("file", t.path.display()))),
            }
        }
    }
    if !issues.is_empty() {
        return Err(PlanError::Blocked(issues));
    }
    a.extend(maps);
    a.extend(strs(&["-c", "copy"]));
    a.extend(per_stream);
    if container == Container::Mp4 && v.codec == "hevc" {
        a.extend(strs(&["-tag:v", "hvc1"]));
    }
    for (o, t) in &track_meta {
        if let Some(lang) = t.language.as_deref().filter(|l| !l.is_empty()) {
            a.extend([format!("-metadata:s:{o}"), format!("language={lang}")]);
        }
        if let Some(title) = t.title.as_deref().filter(|l| !l.is_empty()) {
            a.extend([format!("-metadata:s:{o}"), format!("title={title}")]);
        }
        a.extend([format!("-disposition:{o}"), if t.default { "default".into() } else { "0".into() }]);
    }
    a.extend(metadata_args(&spec.output));
    a.extend(container_tail(container, &spec.output, env.temp_output));
    if spec.tracks.iter().any(|t| t.kind == TrackKind::Subtitle) && container == Container::Mp4 {
        warnings.push(Notice::new("subtitleConverted").with("track", "+"));
    }
    let duration = std::iter::once(main.duration).chain(tracks.iter().map(|t| t.duration)).flatten().fold(None, |acc: Option<f64>, d| Some(acc.map_or(d, |a| a.max(d))));
    let total: u64 = main.size + tracks.iter().map(|t| t.size).sum::<u64>();
    Ok(EncodePlan {
        passes: vec![Pass { args: a, label: None, weight: 0.7, stage: JobState::Muxing }],
        prep: Vec::new(),
        expected: Expected {
            container: Some(container),
            video: Some(ExpectedVideo {
                codec: v.codec.clone(),
                width: v.width,
                height: v.height,
                pix_fmt: v.pix_fmt.clone(),
                frames: FrameCheck::SameAsSource,
                color: color_of(v),
                hdr_metadata: false,
                rotation: v.rotation,
            }),
            audio: expected_audio,
            subtitles: sub_count,
            duration,
            hash_streams: hashes,
            lossless_source_opts: None,
            source_video_index: Some(v.index),
        },
        summary: PlanSummary {
            kind: "mux".into(),
            video: "copy".into(),
            container: container.ext().into(),
            audio: "copy".into(),
            subtitles: if sub_count > 0 { "keep".into() } else { "none".into() },
            quality: Some("copy".into()),
            est_size: Some(total),
            est_reliability: "estimate".into(),
            threads: Some(1),
            ..Default::default()
        },
        warnings,
        duration,
        encoder: None,
        est_upper_bytes: total + 16_000_000,
        est_memory_mb: 80,
        hw_encoder: false,
        verify_weight: 0.3,
    })
}

/// Apply a user-chosen fix (from an `Issue`) to a job's settings snapshot.
pub fn apply_fix(settings: &mut JobSettings, fix: &str, media: Option<&MediaInfo>) -> bool {
    let v = &mut settings.video;
    let use_encoder = |codec: VideoCodec, name: &str, v: &mut VideoSettings| {
        v.codec = codec;
        v.encoder = Some(name.into());
        if v.speed == Speed::Gpu {
            v.speed = Speed::Balanced;
        }
    };
    match fix {
        "useMkv" => settings.output.container = Container::Mkv,
        "audioTranscode" => settings.audio.on_incompatible = IncompatiblePolicy::Transcode,
        "audioDrop" => settings.audio.on_incompatible = IncompatiblePolicy::Drop,
        "subtitlesNone" => settings.subtitles.mode = SubtitleMode::None,
        "subtitlesKeep" => {
            settings.subtitles.mode = SubtitleMode::Keep;
            settings.subtitles.burn = None;
        }
        "allowBitDepthReduction" => v.allow_bit_depth_reduction = true,
        "allowChromaSubsampling" => v.allow_chroma_subsampling = true,
        "acceptAlphaDrop" => settings.accept.push("alphaDropped".into()),
        "acceptDolbyVisionLoss" => settings.accept.push("dolbyVisionLoss".into()),
        "acceptIncomplete" => settings.accept.push("sourceIncomplete".into()),
        "useX264" => use_encoder(VideoCodec::H264, "libx264", v),
        "useX265" => use_encoder(VideoCodec::Hevc, "libx265", v),
        "useSvtAv1" => use_encoder(VideoCodec::Av1, "libsvtav1", v),
        "useAom" => use_encoder(VideoCodec::Av1, "libaom-av1", v),
        "useFfv1" => {
            use_encoder(VideoCodec::Ffv1, "ffv1", v);
            v.mode = VideoMode::Lossless;
            settings.output.container = Container::Mkv;
        }
        "speedBalanced" => {
            v.speed = Speed::Balanced;
            v.encoder = None;
        }
        "modeLossless" => v.mode = VideoMode::Lossless,
        "modeLossy" => v.mode = VideoMode::Lossy,
        "clearTransforms" => v.transforms = Transforms::default(),
        "singlePass" => match &mut v.rate {
            RateControl::Bitrate { two_pass, .. } | RateControl::TargetSize { two_pass, .. } => *two_pass = false,
            _ => return false,
        },
        "useBitrate" => {
            let kbps = media
                .and_then(|m| m.main_video().and_then(|x| x.bit_rate).or(m.bit_rate))
                .map(|b| (b / 1000) as u32)
                .unwrap_or(4000)
                .max(100);
            v.rate = RateControl::Bitrate { kbps, two_pass: false };
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::{EncoderCaps, LosslessCheck};
    use crate::media::parse_probe_json;

    fn media(name: &str) -> MediaInfo {
        let json = match name {
            "h264_aac" => include_str!("../tests/fixtures/h264_aac.json"),
            "av1_opus" => include_str!("../tests/fixtures/av1_opus.json"),
            "rotated" => include_str!("../tests/fixtures/rotated.json"),
            "hdr10" => include_str!("../tests/fixtures/hdr10.json"),
            "multi_audio" => include_str!("../tests/fixtures/multi_audio.json"),
            "pcm_audio" => include_str!("../tests/fixtures/pcm_audio.json"),
            "yuv444" => include_str!("../tests/fixtures/yuv444.json"),
            "fullrange" => include_str!("../tests/fixtures/fullrange.json"),
            "no_audio_like" => include_str!("../tests/fixtures/no_audio.json"),
            _ => unreachable!(),
        };
        parse_probe_json(Path::new(&format!("/视频/{name} 素材.mkv")), json).unwrap()
    }

    fn ten_bit_sdr() -> MediaInfo {
        let mut m = media("hdr10");
        let v = &mut m.video[0];
        v.color_transfer = Some("bt709".into());
        v.color_primaries = Some("bt709".into());
        v.color_space = Some("bt709".into());
        v.hdr = HdrKind::Sdr;
        m
    }

    fn enc(name: &str, pix: &[&str], hardware: bool) -> EncoderCaps {
        EncoderCaps {
            name: name.into(),
            hardware,
            usable: true,
            quality_mode: true,
            pix_fmts: pix.iter().map(|s| s.to_string()).collect(),
            error: None,
        }
    }

    fn caps() -> Capabilities {
        let mut c = crate::caps::tests::sample();
        let x264 = ["yuv420p", "yuvj420p", "yuv422p", "yuv444p", "nv12", "yuv420p10le", "yuv422p10le", "yuv444p10le"];
        c.encoders = vec![
            enc("libx264", &x264, false),
            enc("libx265", &["yuv420p", "yuv422p", "yuv444p", "yuv420p10le", "yuv444p10le"], false),
            enc("libsvtav1", &["yuv420p", "yuv420p10le"], false),
            enc("ffv1", &["yuv420p", "yuv444p", "yuv420p10le", "yuv444p10le"], false),
            enc("h264_videotoolbox", &["nv12", "yuv420p"], true),
            enc("hevc_videotoolbox", &["nv12", "yuv420p", "p010le"], true),
        ];
        c.audio_encoders = vec!["aac".into(), "libopus".into(), "flac".into(), "libmp3lame".into()];
        c.filters = vec!["scale".into(), "transpose".into(), "overlay".into(), "fps".into(), "crop".into()];
        c.hw_decode = vec!["videotoolbox:h264".into()];
        c.lossless = vec![
            LosslessCheck { encoder: "libx264".into(), pix_fmt: "yuv420p".into(), ok: true, detail: String::new() },
            LosslessCheck { encoder: "libsvtav1".into(), pix_fmt: "yuv420p".into(), ok: true, detail: String::new() },
            LosslessCheck { encoder: "ffv1".into(), pix_fmt: "yuv420p".into(), ok: true, detail: String::new() },
        ];
        c
    }

    fn run(settings: &JobSettings, m: &MediaInfo) -> Result<EncodePlan, PlanError> {
        let c = caps();
        let env = PlanEnv {
            caps: &c,
            threads: 10,
            temp_output: Path::new("/out/x.mp4.fe-00000000.part"),
            work_dir: Path::new("/work/job"),
            keyframes: Some(&[0.0, 2.0, 4.0]),
        };
        plan_convert(settings, m, &env)
    }

    fn settings(codec: VideoCodec, mode: VideoMode, container: Container) -> JobSettings {
        let mut s = JobSettings::default();
        s.video.codec = codec;
        s.video.mode = mode;
        s.output.container = container;
        s
    }

    fn args(p: &EncodePlan) -> Vec<String> {
        p.passes.last().unwrap().args.clone()
    }

    fn has(a: &[String], seq: &[&str]) -> bool {
        a.windows(seq.len()).any(|w| w.iter().zip(seq).all(|(x, y)| x == y))
    }

    fn issue_codes(e: PlanError) -> Vec<String> {
        match e {
            PlanError::Blocked(v) => v.into_iter().map(|i| i.notice.code).collect(),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    fn fixes(e: PlanError) -> Vec<String> {
        match e {
            PlanError::Blocked(v) => v.into_iter().flat_map(|i| i.fixes.into_iter().map(|f| f.id)).collect(),
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn extract_targets_pick_native_containers() {
        assert_eq!(extract_target("audio", "aac"), ("m4a", "ipod", None));
        assert_eq!(extract_target("audio", "ac3"), ("ac3", "ac3", None));
        assert_eq!(extract_target("audio", "pcm_s16le"), ("wav", "wav", None));
        assert_eq!(extract_target("audio", "weird"), ("mka", "matroska", None));
        assert_eq!(extract_target("subtitle", "subrip"), ("srt", "srt", None));
        assert_eq!(extract_target("subtitle", "mov_text"), ("srt", "srt", Some("srt")));
        assert_eq!(extract_target("subtitle", "hdmv_pgs_subtitle"), ("sup", "sup", None));
        assert_eq!(extract_target("video", "h264"), ("mp4", "mp4", None));
        assert_eq!(extract_target("video", "prores"), ("mkv", "matroska", None));
    }

    #[test]
    fn extract_second_audio_track_is_a_hashed_copy() {
        let m = media("multi_audio");
        let c = caps();
        let env = PlanEnv { caps: &c, threads: 2, temp_output: Path::new("/o/a.ac3.fe-00000000.part"), work_dir: Path::new("/w"), keyframes: None };
        let spec = ExtractSpec { input: m.path.clone(), stream_index: 2, output: OutputSettings::default() };
        let p = plan_extract(&spec, &m, &env).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-map", "0:2"]) && has(&a, &["-c", "copy"]) && has(&a, &["-f", "ac3"]));
        assert_eq!(p.expected.container, None);
        assert_eq!(p.expected.audio[0].codec, "ac3");
        assert_eq!(p.expected.hash_streams, vec![StreamCopy { input: 0, source_index: 2, output_index: 0 }]);
    }

    #[test]
    fn mux_adds_external_tracks_with_language_and_default_flag() {
        let main = media("no_audio_like");
        let audio = media("h264_aac");
        let c = caps();
        let env = PlanEnv { caps: &c, threads: 2, temp_output: Path::new("/o/m.mkv.fe-00000000.part"), work_dir: Path::new("/w"), keyframes: None };
        let spec = MuxSpec {
            main: main.path.clone(),
            keep_main_audio: false,
            keep_main_subtitles: false,
            tracks: vec![MuxTrack { path: audio.path.clone(), kind: TrackKind::Audio, language: Some("jpn".into()), title: Some("日本語".into()), default: true }],
            output: OutputSettings { container: Container::Mkv, ..OutputSettings::default() },
        };
        let p = plan_mux(&spec, &main, &[audio], &env).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-map", "0:0"]) && has(&a, &["-map", "1:1"]));
        assert!(has(&a, &["-metadata:s:1", "language=jpn"]) && has(&a, &["-disposition:1", "default"]));
        assert_eq!(p.expected.hash_streams.len(), 2);
        assert_eq!(p.expected.hash_streams[1], StreamCopy { input: 1, source_index: 1, output_index: 1 });
    }

    #[test]
    fn fixes_change_only_the_snapshot_they_target() {
        let mut s = settings(VideoCodec::Av1, VideoMode::Lossless, Container::Mp4);
        assert!(apply_fix(&mut s, "useFfv1", None));
        assert_eq!((s.video.codec, s.video.mode, s.output.container), (VideoCodec::Ffv1, VideoMode::Lossless, Container::Mkv));
        assert!(apply_fix(&mut s, "acceptAlphaDrop", None));
        assert_eq!(s.accept, vec!["alphaDropped"]);
        let mut t = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        assert!(apply_fix(&mut t, "audioTranscode", None));
        assert_eq!(t.audio.on_incompatible, IncompatiblePolicy::Transcode);
        t.video.rate = RateControl::Bitrate { kbps: 900, two_pass: true };
        assert!(apply_fix(&mut t, "singlePass", None));
        assert_eq!(t.video.rate, RateControl::Bitrate { kbps: 900, two_pass: false });
        assert!(!apply_fix(&mut t, "noSuchFix", None));
    }

    #[test]
    fn h264_to_av1_uses_svt_crf_and_copies_audio() {
        let p = run(&settings(VideoCodec::Av1, VideoMode::Lossy, Container::Mp4), &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-c:v", "libsvtav1"]) && has(&a, &["-crf", "34"]), "{a:?}");
        assert!(has(&a, &["-c:a:0", "copy"]));
        assert!(has(&a, &["-fps_mode", "passthrough"]));
        assert!(has(&a, &["-pix_fmt", "yuv420p"]));
        assert!(has(&a, &["-f", "mp4"]));
        let v = p.expected.video.as_ref().unwrap();
        assert_eq!((v.codec.as_str(), v.width, v.height), ("av1", 640, 360));
        assert_eq!(v.frames, FrameCheck::SameAsSource);
        assert_eq!(p.expected.audio, vec![ExpectedAudio { source_index: 1, codec: "aac".into(), copied: true }]);
        assert_eq!(p.expected.hash_streams, vec![StreamCopy { input: 0, source_index: 1, output_index: 1 }]);
        assert_eq!(p.summary.est_reliability, "unknown", "CRF size is never promised");
    }

    #[test]
    fn av1_to_h264_keeps_opus_copy_in_mp4() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4), &media("av1_opus")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-c:v", "libx264"]) && has(&a, &["-crf", "23"]));
        assert_eq!(p.expected.video.as_ref().unwrap().codec, "h264");
        assert!(p.expected.audio[0].copied);
    }

    #[test]
    fn requested_codec_is_never_substituted() {
        for codec in [VideoCodec::H264, VideoCodec::Hevc, VideoCodec::Av1] {
            let p = run(&settings(codec, VideoMode::Lossy, Container::Mkv), &media("h264_aac")).unwrap();
            assert_eq!(p.expected.video.unwrap().codec, codec.probe_name());
        }
    }

    #[test]
    fn gpu_speed_without_hardware_encoder_is_an_issue_not_a_fallback() {
        let mut s = settings(VideoCodec::Av1, VideoMode::Lossy, Container::Mp4);
        s.video.speed = Speed::Gpu;
        let e = run(&s, &media("h264_aac")).unwrap_err();
        assert_eq!(issue_codes(e.clone()), vec!["noHardwareEncoder"]);
        assert_eq!(fixes(e), vec!["speedBalanced"]);
    }

    #[test]
    fn gpu_speed_picks_verified_videotoolbox_and_hw_decode() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.video.speed = Speed::Gpu;
        let p = run(&s, &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-c:v", "h264_videotoolbox"]) && has(&a, &["-q:v", "60"]) && has(&a, &["-allow_sw", "0"]));
        assert!(has(&a, &["-hwaccel", "videotoolbox"]), "verified hw decode is used for lossy jobs");
        assert!(p.hw_encoder);
    }

    #[test]
    fn lossless_x264_uses_qp0_and_expects_frame_exact_output() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Lossless, Container::Mkv), &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-qp", "0"]) && !a.iter().any(|x| x == "-crf"));
        assert!(!a.iter().any(|x| x == "-pix_fmt"), "lossless never forces a pixel format");
        assert!(!a.iter().any(|x| x == "-hwaccel"), "lossless decodes in software");
        assert_eq!(p.expected.lossless_source_opts, Some(vec![]));
        let v = p.expected.video.unwrap();
        assert_eq!((v.frames, v.pix_fmt.as_deref()), (FrameCheck::SameAsSource, Some("yuv420p")));
        assert!(p.summary.lossless);
    }

    #[test]
    fn lossless_without_a_passed_self_test_asks_for_one() {
        let e = run(&settings(VideoCodec::Hevc, VideoMode::Lossless, Container::Mkv), &media("h264_aac")).unwrap_err();
        assert_eq!(e, PlanError::NeedsSelfTest { encoder: "libx265".into(), pix_fmt: "yuv420p".into() });
    }

    #[test]
    fn svt_lossless_refuses_444_instead_of_downsampling() {
        let e = run(&settings(VideoCodec::Av1, VideoMode::Lossless, Container::Mkv), &media("yuv444")).unwrap_err();
        let f = fixes(e.clone());
        assert_eq!(issue_codes(e), vec!["losslessPixFmtUnsupported"]);
        assert!(f.contains(&"useX264".to_string()) && f.contains(&"useFfv1".to_string()), "{f:?}");
    }

    #[test]
    fn svt_lossless_accepts_full_range_source() {
        let p = run(&settings(VideoCodec::Av1, VideoMode::Lossless, Container::Mkv), &media("fullrange")).unwrap();
        assert_eq!(p.expected.video.unwrap().color.range.as_deref(), Some("pc"));
    }

    #[test]
    fn incomplete_source_is_refused_until_the_readable_part_is_accepted() {
        let mut m = media("h264_aac");
        let d = m.duration.unwrap();
        m.readable_until = Some(d / 2.0);
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        assert_eq!(fixes(run(&s, &m).unwrap_err()), vec!["acceptIncomplete"]);

        assert!(apply_fix(&mut s, "acceptIncomplete", None));
        let p = run(&s, &m).unwrap();
        assert!((p.expected.duration.unwrap() - d / 2.0).abs() < 0.01, "only the readable part is expected");
        assert!(p.warnings.iter().any(|w| w.code == "sourceIncompleteTrimmed"));

        // A trim that ends inside the readable part needs no decision.
        let mut t = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        t.trim = Some(Trim { start: 0.0, end: Some(d / 4.0) });
        assert!(run(&t, &m).is_ok());
        assert_eq!((clock(671.96), clock(1190.88), clock(3725.0)), ("11:12".into(), "19:51".into(), "1:02:05".into()));
    }

    #[test]
    fn lossless_refuses_transforms() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossless, Container::Mkv);
        s.video.transforms.scale = Some(Scale::MaxHeight { height: 240 });
        assert_eq!(issue_codes(run(&s, &media("h264_aac")).unwrap_err()), vec!["losslessWithTransforms"]);
    }

    #[test]
    fn ffv1_requires_mkv() {
        let e = run(&settings(VideoCodec::Ffv1, VideoMode::Lossless, Container::Mp4), &media("h264_aac")).unwrap_err();
        assert_eq!(issue_codes(e.clone()), vec!["containerUnsupported"]);
        assert_eq!(fixes(e), vec!["useMkv"]);
    }

    #[test]
    fn pcm_copy_into_mp4_asks_the_user() {
        let e = run(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4), &media("pcm_audio")).unwrap_err();
        assert_eq!(issue_codes(e.clone()), vec!["audioIncompatible"]);
        assert_eq!(fixes(e), vec!["audioTranscode", "audioDrop", "useMkv"]);
    }

    #[test]
    fn pcm_with_transcode_policy_becomes_aac_and_says_so() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.audio.on_incompatible = IncompatiblePolicy::Transcode;
        let p = run(&s, &media("pcm_audio")).unwrap();
        assert!(has(&args(&p), &["-c:a:0", "aac"]));
        assert_eq!(p.expected.audio[0].codec, "aac");
        assert!(p.warnings.iter().any(|w| w.code == "audioTranscoded"));
        assert!(p.expected.hash_streams.is_empty());
    }

    #[test]
    fn hdr_to_h264_is_refused() {
        let e = run(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mkv), &media("hdr10")).unwrap_err();
        assert_eq!(issue_codes(e.clone()), vec!["hdrUnsupportedEncoder"]);
        assert_eq!(fixes(e), vec!["useX265", "useSvtAv1"]);
    }

    #[test]
    fn hdr_to_svt_keeps_10bit_and_pq_tags() {
        let p = run(&settings(VideoCodec::Av1, VideoMode::Lossy, Container::Mkv), &media("hdr10")).unwrap();
        assert!(has(&args(&p), &["-pix_fmt", "yuv420p10le"]));
        let v = p.expected.video.unwrap();
        assert_eq!(v.color.transfer.as_deref(), Some("smpte2084"));
        assert_eq!(v.color.primaries.as_deref(), Some("bt2020"));
    }

    #[test]
    fn ten_bit_to_8bit_only_encoder_needs_explicit_consent() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.video.speed = Speed::Gpu;
        let e = run(&s, &ten_bit_sdr()).unwrap_err();
        assert_eq!(issue_codes(e.clone()), vec!["bitDepthUnsupported"]);
        assert!(fixes(e).contains(&"allowBitDepthReduction".to_string()));
        s.video.allow_bit_depth_reduction = true;
        let p = run(&s, &ten_bit_sdr()).unwrap();
        assert!(has(&args(&p), &["-pix_fmt", "yuv420p"]));
        assert!(p.warnings.iter().any(|w| w.code == "bitDepthReduced"));
    }

    #[test]
    fn rotated_source_is_encoded_upright_with_display_dims() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4), &media("rotated")).unwrap();
        let v = p.expected.video.unwrap();
        assert_eq!((v.width, v.height, v.rotation), (360, 640, 0));
        assert!(p.warnings.iter().any(|w| w.code == "rotationApplied"));
    }

    #[test]
    fn stream_copy_keeps_rotation_matrix() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Copy, Container::Mkv), &media("rotated")).unwrap();
        let v = p.expected.video.unwrap();
        assert_eq!((v.width, v.height, v.rotation), (640, 360, 90));
    }

    #[test]
    fn max_height_scale_computes_exact_dims() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.video.transforms.scale = Some(Scale::MaxHeight { height: 240 });
        let p = run(&s, &media("h264_aac")).unwrap();
        assert!(has(&args(&p), &["-vf", "scale=-2:240:flags=lanczos"]));
        let v = p.expected.video.unwrap();
        assert_eq!((v.width, v.height), (426, 240));
    }

    #[test]
    fn default_never_scales_or_changes_frame_rate() {
        let p = run(&settings(VideoCodec::Hevc, VideoMode::Lossy, Container::Mp4), &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(!a.iter().any(|x| x == "-vf" || x == "-r" || x == "-s"));
        assert!(has(&a, &["-tag:v", "hvc1"]), "HEVC in MP4 tagged for Apple players");
    }

    #[test]
    fn two_pass_x264_runs_analysis_pass_into_null() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.video.rate = RateControl::Bitrate { kbps: 1500, two_pass: true };
        let p = run(&s, &media("h264_aac")).unwrap();
        assert_eq!(p.passes.len(), 2);
        let first = &p.passes[0].args;
        assert!(has(first, &["-pass", "1"]) && has(first, &["-f", "null", "-"]) && first.iter().any(|x| x == "-an"));
        assert!(has(&p.passes[1].args, &["-pass", "2"]));
        assert_eq!(p.summary.est_reliability, "estimate");
    }

    #[test]
    fn two_pass_is_refused_where_unsupported() {
        let mut s = settings(VideoCodec::Av1, VideoMode::Lossy, Container::Mp4);
        s.video.rate = RateControl::Bitrate { kbps: 1500, two_pass: true };
        let e = run(&s, &media("h264_aac")).unwrap_err();
        assert_eq!(fixes(e), vec!["singlePass"]);
    }

    #[test]
    fn target_size_becomes_video_bitrate_minus_audio() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.video.rate = RateControl::TargetSize { megabytes: 2.0, two_pass: false };
        let p = run(&s, &media("h264_aac")).unwrap();
        let a = args(&p);
        let i = a.iter().position(|x| x == "-b:v").unwrap();
        let kbps: u32 = a[i + 1].trim_end_matches('k').parse().unwrap();
        // 2 MB over 4 s = 4000 kbps total, minus copied audio and ~2% overhead.
        assert!((3600..=3900).contains(&kbps), "{kbps}");
    }

    #[test]
    fn remux_to_mp4_converts_srt_to_mov_text_and_hashes_copied_streams() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Copy, Container::Mp4), &media("multi_audio")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-c:v", "copy"]) && has(&a, &["-c:s:0", "mov_text"]));
        assert_eq!(p.passes[0].stage, JobState::Muxing);
        assert_eq!(p.expected.hash_streams.len(), 3, "video + 2 audio copied bit-exactly");
        assert!(p.warnings.iter().any(|w| w.code == "subtitleConverted"));
        assert_eq!(p.summary.est_reliability, "estimate");
    }

    #[test]
    fn keyframe_trim_snaps_start_and_reports_it() {
        let mut s = settings(VideoCodec::H264, VideoMode::Copy, Container::Mp4);
        s.trim = Some(Trim { start: 2.5, end: Some(3.5) });
        let p = run(&s, &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(has(&a, &["-ss", "2.000"]) && has(&a, &["-t", "1.500"]), "{a:?}");
        assert_eq!(p.expected.duration, Some(1.5), "expected length starts at the keyframe actually used");
        assert_eq!(p.duration, Some(1.5));
        assert!(p.warnings.iter().any(|w| w.code == "keyframeTrim"));
        assert!(p.expected.hash_streams.is_empty(), "partial streams are not hash-compared");
    }

    #[test]
    fn precise_trim_re_encodes_exact_range() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.trim = Some(Trim { start: 1.0, end: Some(3.0) });
        let p = run(&s, &media("h264_aac")).unwrap();
        let a = args(&p);
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert!(ss < i, "input seeking is frame accurate when re-encoding");
        assert!(has(&a, &["-t", "2.000"]));
        assert_eq!(p.duration, Some(2.0));
    }

    #[test]
    fn output_is_never_overwritten_and_paths_use_file_protocol() {
        let p = run(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4), &media("h264_aac")).unwrap();
        let a = args(&p);
        assert!(a.iter().any(|x| x == "-n") && !a.iter().any(|x| x == "-y"));
        assert!(a.iter().any(|x| x == "file:/视频/h264_aac 素材.mkv"));
        assert_eq!(a.last().unwrap(), "file:/out/x.mp4.fe-00000000.part");
    }

    #[test]
    fn thread_budget_is_applied() {
        let c = caps();
        let env = PlanEnv { caps: &c, threads: 5, temp_output: Path::new("/o/t.part"), work_dir: Path::new("/w"), keyframes: None };
        let p = plan_convert(&settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4), &media("h264_aac"), &env).unwrap();
        assert!(has(&args(&p), &["-threads", "5"]));
    }

    #[test]
    fn burning_text_subtitles_needs_libass() {
        let mut s = settings(VideoCodec::H264, VideoMode::Lossy, Container::Mp4);
        s.subtitles.mode = SubtitleMode::Burn;
        s.subtitles.burn = Some(BurnSource::Embedded { ordinal: 0 });
        assert_eq!(issue_codes(run(&s, &media("multi_audio")).unwrap_err()), vec!["burnNeedsLibass"]);
    }

    #[test]
    fn naming_tokens_reflect_codec_and_mode() {
        let c = caps();
        assert_eq!(naming_tokens(&settings(VideoCodec::Av1, VideoMode::Lossy, Container::Mp4), &c), ("av1".into(), "crf34".into()));
        assert_eq!(naming_tokens(&settings(VideoCodec::H264, VideoMode::Lossless, Container::Mkv), &c), ("h264".into(), "lossless".into()));
        assert_eq!(naming_tokens(&settings(VideoCodec::H264, VideoMode::Copy, Container::Mkv), &c), ("remux".into(), "copy".into()));
    }
}
