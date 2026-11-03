//! User-facing job settings. A queued job owns an immutable snapshot of these,
//! so editing the global/default settings never changes already-queued work.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Ffv1,
}

impl VideoCodec {
    /// Codec name as reported by ffprobe `codec_name`.
    pub fn probe_name(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::Av1 => "av1",
            VideoCodec::Ffv1 => "ffv1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }
    /// FFmpeg muxer name passed with `-f` (temp files do not carry the real extension).
    pub fn muxer(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "matroska",
        }
    }
}

/// How the video stream is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VideoMode {
    /// Re-encode with a lossy rate control.
    Lossy,
    /// Re-encode with a mathematically lossless encoder setup, verified frame by frame.
    Lossless,
    /// No re-encode: stream copy / remux.
    Copy,
}

/// Speed/efficiency trade-off. `Gpu` selects a verified hardware encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Speed {
    Gpu,
    Fast,
    Balanced,
    Slow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RateControl {
    /// Encoder-native quality value derived from the 0–100 compression level.
    Level,
    /// Explicit encoder-native CRF/CQ/q value. Meaning differs per encoder.
    Native { value: f64 },
    /// Average bitrate in kbit/s.
    Bitrate { kbps: u32, two_pass: bool },
    /// Whole-file size target in MB (10^6 bytes), converted to a video bitrate.
    TargetSize { megabytes: f64, two_pass: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scale {
    /// Downscale only when taller than `height`, keeping aspect ratio.
    MaxHeight { height: u32 },
    Exact { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotate {
    Cw90,
    Ccw90,
    R180,
    FlipH,
    FlipV,
}

/// Geometry/time changes. All are opt-in; the default keeps resolution and frame rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Transforms {
    pub crop: Option<Crop>,
    pub scale: Option<Scale>,
    pub rotate: Option<Rotate>,
    pub fps: Option<f64>,
}

impl Transforms {
    pub fn is_identity(&self) -> bool {
        self.crop.is_none() && self.scale.is_none() && self.rotate.is_none() && self.fps.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    /// Explicit encoder (e.g. "libx264", "h264_videotoolbox"). None = chosen from `speed`.
    pub encoder: Option<String>,
    pub mode: VideoMode,
    /// 0 = highest quality … 100 = smallest file. Presets: 25 / 50 / 75.
    pub level: u8,
    pub rate: RateControl,
    pub speed: Speed,
    pub transforms: Transforms,
    /// Explicit opt-in to turn SDR 10/12-bit into 8-bit. Never applied silently; HDR never.
    pub allow_bit_depth_reduction: bool,
    /// Explicit opt-in to turn 4:4:4/4:2:2 into 4:2:0 for lossy encodes.
    pub allow_chroma_subsampling: bool,
    /// Use verified hardware decoding for lossy jobs when available.
    pub hw_decode: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            encoder: None,
            mode: VideoMode::Lossy,
            level: 50,
            rate: RateControl::Level,
            speed: Speed::Balanced,
            transforms: Transforms::default(),
            allow_bit_depth_reduction: false,
            allow_chroma_subsampling: false,
            hw_decode: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioMode {
    Copy,
    Transcode,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    Aac,
    Opus,
    Mp3,
    Flac,
}

impl AudioCodec {
    pub fn encoder(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "libopus",
            AudioCodec::Mp3 => "libmp3lame",
            AudioCodec::Flac => "flac",
        }
    }
    pub fn probe_name(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
            AudioCodec::Mp3 => "mp3",
            AudioCodec::Flac => "flac",
        }
    }
    pub fn is_lossless(self) -> bool {
        matches!(self, AudioCodec::Flac)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackSelection {
    All,
    First,
    /// Ordinals among audio streams listed in `AudioSettings::track_list`.
    List,
}

/// What to do when an audio track cannot be stream-copied into the chosen container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IncompatiblePolicy {
    /// Stop and let the user decide (default: never change a track silently).
    Ask,
    Transcode,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub mode: AudioMode,
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub tracks: TrackSelection,
    pub track_list: Vec<u32>,
    pub on_incompatible: IncompatiblePolicy,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            mode: AudioMode::Copy,
            codec: AudioCodec::Aac,
            bitrate_kbps: 192,
            tracks: TrackSelection::All,
            track_list: Vec::new(),
            on_incompatible: IncompatiblePolicy::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleMode {
    None,
    /// Keep embedded subtitles (copy, or convert text subs to mov_text for MP4) and add external ones.
    Keep,
    /// Render one subtitle source into the picture (requires re-encoding).
    Burn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BurnSource {
    Embedded { ordinal: u32 },
    External { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSubtitle {
    pub path: PathBuf,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleSettings {
    pub mode: SubtitleMode,
    pub burn: Option<BurnSource>,
    pub external: Vec<ExternalSubtitle>,
}

impl Default for SubtitleSettings {
    fn default() -> Self {
        Self { mode: SubtitleMode::Keep, burn: None, external: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputDir {
    /// `<source dir>/<subfolder>/`
    BesideSource,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    /// Pick a free name like `name (2).mp4`. Never overwrites.
    Rename,
    /// Leave the existing file alone and skip the job.
    Skip,
}

pub const DEFAULT_SUBFOLDER: &str = "FrameEase 输出";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputSettings {
    pub container: Container,
    pub dir: OutputDir,
    pub custom_dir: Option<PathBuf>,
    pub subfolder: String,
    /// Tokens: {name} {codec} {mode} {date}
    pub name_template: String,
    pub conflict: ConflictPolicy,
    pub faststart: bool,
    pub keep_metadata: bool,
    pub keep_chapters: bool,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self {
            container: Container::Mp4,
            dir: OutputDir::BesideSource,
            custom_dir: None,
            subfolder: DEFAULT_SUBFOLDER.to_string(),
            name_template: "{name}_{codec}".to_string(),
            conflict: ConflictPolicy::Rename,
            faststart: true,
            keep_metadata: true,
            keep_chapters: true,
        }
    }
}

/// Time range. With `VideoMode::Copy` the cut snaps to the keyframe at/before `start`;
/// with re-encoding it is frame accurate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trim {
    pub start: f64,
    pub end: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct JobSettings {
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub subtitles: SubtitleSettings,
    pub output: OutputSettings,
    pub trim: Option<Trim>,
    /// Issue codes the user explicitly accepted for this job (e.g. "alphaDropped").
    #[serde(default)]
    pub accept: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackKind {
    Audio,
    Subtitle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxTrack {
    pub path: PathBuf,
    pub kind: TrackKind,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MuxSpec {
    pub main: PathBuf,
    pub keep_main_audio: bool,
    pub keep_main_subtitles: bool,
    pub tracks: Vec<MuxTrack>,
    pub output: OutputSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractSpec {
    pub input: PathBuf,
    /// Absolute stream index in the input.
    pub stream_index: u32,
    pub output: OutputSettings,
}

/// What a job does. Stored verbatim in the persisted queue.
/// One value per job, so the size difference between variants is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobSpec {
    Convert { input: PathBuf, settings: JobSettings },
    Mux(MuxSpec),
    Extract(ExtractSpec),
}

impl JobSpec {
    pub fn primary_input(&self) -> &PathBuf {
        match self {
            JobSpec::Convert { input, .. } => input,
            JobSpec::Mux(m) => &m.main,
            JobSpec::Extract(e) => &e.input,
        }
    }
    pub fn output_settings(&self) -> &OutputSettings {
        match self {
            JobSpec::Convert { settings, .. } => &settings.output,
            JobSpec::Mux(m) => &m.output,
            JobSpec::Extract(e) => &e.output,
        }
    }
}
