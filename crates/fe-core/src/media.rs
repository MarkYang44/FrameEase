//! Media probing via ffprobe (JSON), normalised into `MediaInfo`.

use crate::job::SourceSummary;
use crate::process;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Chroma {
    #[serde(rename = "420")]
    C420,
    #[serde(rename = "422")]
    C422,
    #[serde(rename = "444")]
    C444,
    #[serde(rename = "440")]
    C440,
    #[serde(rename = "411")]
    C411,
    #[serde(rename = "410")]
    C410,
    #[serde(rename = "gray")]
    Gray,
}

impl Chroma {
    /// Relative chroma resolution, used to detect subsampling (lower = more subsampled).
    pub fn rank(self) -> u8 {
        match self {
            Chroma::Gray => 0,
            Chroma::C410 => 1,
            Chroma::C411 | Chroma::C420 => 2,
            Chroma::C440 | Chroma::C422 => 3,
            Chroma::C444 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixTraits {
    pub bit_depth: u32,
    pub chroma: Chroma,
    pub alpha: bool,
    pub rgb: bool,
    /// Legacy "yuvj" JPEG-range formats.
    pub full_range: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HdrKind {
    Sdr,
    Pq,
    Hlg,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasteringDisplay {
    pub red: (f64, f64),
    pub green: (f64, f64),
    pub blue: (f64, f64),
    pub white: (f64, f64),
    pub max_luminance: f64,
    pub min_luminance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentLight {
    pub max_cll: u32,
    pub max_fall: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DolbyVision {
    pub profile: u32,
    pub bl_compat_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub fn value(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoStream {
    pub index: u32,
    pub codec: String,
    pub codec_long: String,
    pub profile: Option<String>,
    pub level: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub sar: Option<String>,
    pub dar: Option<String>,
    pub pix_fmt: Option<String>,
    pub bit_depth: Option<u32>,
    pub chroma: Option<Chroma>,
    pub alpha: bool,
    pub rgb: bool,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub hdr: HdrKind,
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
    pub dolby_vision: Option<DolbyVision>,
    /// Clockwise rotation needed for display: 0, 90, 180 or 270.
    pub rotation: i32,
    pub r_frame_rate: Option<Rational>,
    pub avg_frame_rate: Option<Rational>,
    pub fps: Option<f64>,
    /// Frame timing varies (from packet timestamps); informational.
    pub vfr: bool,
    pub field_order: Option<String>,
    pub interlaced: bool,
    pub nb_frames: Option<u64>,
    pub duration: Option<f64>,
    pub bit_rate: Option<u64>,
    pub attached_pic: bool,
    pub default: bool,
    pub language: Option<String>,
    pub title: Option<String>,
}

impl VideoStream {
    /// Width/height as displayed (after applying rotation).
    pub fn display_dims(&self) -> (u32, u32) {
        if self.rotation % 180 == 90 { (self.height, self.width) } else { (self.width, self.height) }
    }
    pub fn full_range(&self) -> bool {
        self.color_range.as_deref() == Some("pc")
            || self.pix_fmt.as_deref().map(|p| p.starts_with("yuvj")).unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub codec_long: String,
    pub profile: Option<String>,
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<u32>,
    pub sample_fmt: Option<String>,
    pub bit_rate: Option<u64>,
    pub duration: Option<f64>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStream {
    pub index: u32,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    pub bitmap: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub path: PathBuf,
    pub size: u64,
    pub format_name: String,
    pub format_long_name: String,
    pub duration: Option<f64>,
    pub start_time: Option<f64>,
    pub bit_rate: Option<u64>,
    pub video: Vec<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub subtitles: Vec<SubtitleStream>,
    pub attachments: u32,
    pub data_streams: u32,
    pub chapters: u32,
    pub tags: BTreeMap<String, String>,
}

impl MediaInfo {
    /// The main video stream: first non-cover-art video stream.
    pub fn main_video(&self) -> Option<&VideoStream> {
        self.video.iter().find(|v| !v.attached_pic)
    }

    pub fn is_mp4_family(&self) -> bool {
        self.format_name.contains("mp4") || self.format_name.contains("mov")
    }

    pub fn summary(&self) -> SourceSummary {
        let v = self.main_video();
        let dims = v.map(|v| v.display_dims());
        SourceSummary {
            container: self.format_name.clone(),
            video: v.map(|v| v.codec.clone()),
            width: dims.map(|d| d.0),
            height: dims.map(|d| d.1),
            fps: v.and_then(|v| v.fps),
            duration: self.duration.or_else(|| v.and_then(|v| v.duration)),
            size: self.size,
            audio_tracks: self.audio.len() as u32,
            subtitle_tracks: self.subtitles.len() as u32,
            hdr: v.map(|v| v.hdr != HdrKind::Sdr).unwrap_or(false),
            bit_depth: v.and_then(|v| v.bit_depth),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProbeError {
    #[error("file not found")]
    NotFound,
    #[error("cannot read file: {0}")]
    Unreadable(String),
    #[error("ffprobe failed: {0}")]
    Backend(String),
    #[error("ffprobe output could not be parsed: {0}")]
    Invalid(String),
}

impl ProbeError {
    pub fn code(&self) -> &'static str {
        match self {
            ProbeError::NotFound => "inputMissing",
            ProbeError::Unreadable(_) => "inputUnreadable",
            ProbeError::Backend(_) => "inputCorrupt",
            ProbeError::Invalid(_) => "inputCorrupt",
        }
    }
}

/// Bit depth / chroma layout / alpha / RGB of an FFmpeg pixel format name.
pub fn pix_traits(fmt: &str) -> Option<PixTraits> {
    use Chroma::*;
    let f = fmt.strip_suffix("le").or_else(|| fmt.strip_suffix("be")).unwrap_or(fmt);
    let t = |bit_depth, chroma, alpha, rgb| Some(PixTraits { bit_depth, chroma, alpha, rgb, full_range: false });
    let chroma_of = |s: &str| match s {
        "420" => Some(C420),
        "422" => Some(C422),
        "444" => Some(C444),
        "440" => Some(C440),
        "411" => Some(C411),
        "410" => Some(C410),
        _ => None,
    };
    // "p" / "p10" / "p16" → depth
    let planar_depth = |tail: &str| -> Option<u32> {
        let d = tail.strip_prefix('p')?;
        if d.is_empty() { Some(8) } else { d.parse().ok().filter(|n| (8..=16).contains(n)) }
    };
    let suffix_depth = |d: &str| -> Option<u32> {
        if d.is_empty() { Some(8) } else { d.parse().ok().filter(|n| (8..=16).contains(n)) }
    };
    if let Some(rest) = f.strip_prefix("yuvj") {
        let c = chroma_of(rest.get(..3)?)?;
        return Some(PixTraits { bit_depth: 8, chroma: c, alpha: false, rgb: false, full_range: true });
    }
    if let Some(rest) = f.strip_prefix("yuva") {
        return t(planar_depth(rest.get(3..)?)?, chroma_of(rest.get(..3)?)?, true, false);
    }
    if let Some(rest) = f.strip_prefix("yuv") {
        return t(planar_depth(rest.get(3..)?)?, chroma_of(rest.get(..3)?)?, false, false);
    }
    if let Some(rest) = f.strip_prefix("gbrap") {
        return t(suffix_depth(rest)?, C444, true, true);
    }
    if let Some(rest) = f.strip_prefix("gbrp") {
        return t(suffix_depth(rest)?, C444, false, true);
    }
    if let Some(rest) = f.strip_prefix("gray") {
        return t(suffix_depth(rest)?, Gray, false, false);
    }
    match f {
        "nv12" | "nv21" => t(8, C420, false, false),
        "nv16" => t(8, C422, false, false),
        "nv20" => t(10, C422, false, false),
        "nv24" | "nv42" => t(8, C444, false, false),
        "p010" => t(10, C420, false, false),
        "p012" => t(12, C420, false, false),
        "p016" => t(16, C420, false, false),
        "p210" | "y210" => t(10, C422, false, false),
        "p212" | "y212" => t(12, C422, false, false),
        "p216" => t(16, C422, false, false),
        "p410" | "xv30" => t(10, C444, false, false),
        "p412" | "xv36" => t(12, C444, false, false),
        "p416" => t(16, C444, false, false),
        "uyvy422" | "yuyv422" | "yvyu422" => t(8, C422, false, false),
        "vuya" | "ayuv" => t(8, C444, true, false),
        "ayuv64" => t(16, C444, true, false),
        "ya8" => t(8, Gray, true, false),
        "ya16" => t(16, Gray, true, false),
        "rgb24" | "bgr24" | "0rgb" | "rgb0" | "0bgr" | "bgr0" => t(8, C444, false, true),
        "argb" | "rgba" | "abgr" | "bgra" => t(8, C444, true, true),
        "rgb48" | "bgr48" => t(16, C444, false, true),
        "rgba64" | "bgra64" => t(16, C444, true, true),
        "x2rgb10" | "x2bgr10" => t(10, C444, false, true),
        _ => None,
    }
}

pub fn parse_rational(s: &str) -> Option<Rational> {
    let (num, den) = match s.split_once('/') {
        Some((n, d)) => (n.trim().parse::<i64>().ok()?, d.trim().parse::<i64>().ok()?),
        None => (s.trim().parse::<i64>().ok()?, 1),
    };
    (num != 0 && den != 0).then_some(Rational { num, den })
}

/// Convert ffprobe's display-matrix `rotation` (counter-clockwise degrees) to
/// the clockwise rotation needed for display, snapped to 0/90/180/270.
pub fn display_rotation_cw(rotation: f64) -> i32 {
    let quarter = (-rotation / 90.0).round() as i64;
    (quarter * 90).rem_euclid(360) as i32
}

/// Detect variable frame timing from presentation timestamps (any order):
/// more than 5% of frame intervals differ from the median by over 25% (and 2 ms).
pub fn vfr_from_pts(pts: &[f64]) -> bool {
    let mut p: Vec<f64> = pts.iter().copied().filter(|v| v.is_finite()).collect();
    p.sort_by(|a, b| a.total_cmp(b));
    let deltas: Vec<f64> = p.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 1e-6).collect();
    if deltas.len() < 4 {
        return false;
    }
    let mut sorted = deltas.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let off = deltas.iter().filter(|d| (*d - median).abs() > (median * 0.25).max(0.002)).count();
    off * 20 > deltas.len()
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty() && !matches!(s.as_str(), "unknown" | "unspecified" | "reserved" | "N/A"))
}

fn num_of(v: &Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
}

fn int_of(v: &Value, key: &str) -> Option<u64> {
    let x = v.get(key)?;
    x.as_u64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
}

fn tags_of(v: &Value) -> BTreeMap<String, String> {
    v.get("tags")
        .and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

fn tag(tags: &BTreeMap<String, String>, key: &str) -> Option<String> {
    tags.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).filter(|s| !s.is_empty())
}

/// "00:00:04.000000000" → 4.0
fn parse_hms(s: &str) -> Option<f64> {
    let mut total = 0.0;
    for part in s.trim().split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(total)
}

fn ratio(v: &Value, key: &str) -> Option<f64> {
    let s = v.get(key)?.as_str()?;
    let r = s.split_once('/').map(|(n, d)| (n.parse::<f64>(), d.parse::<f64>()));
    match r {
        Some((Ok(n), Ok(d))) if d != 0.0 => Some(n / d),
        _ => s.parse().ok(),
    }
}

fn parse_mastering(sd: &Value) -> Option<MasteringDisplay> {
    Some(MasteringDisplay {
        red: (ratio(sd, "red_x")?, ratio(sd, "red_y")?),
        green: (ratio(sd, "green_x")?, ratio(sd, "green_y")?),
        blue: (ratio(sd, "blue_x")?, ratio(sd, "blue_y")?),
        white: (ratio(sd, "white_point_x")?, ratio(sd, "white_point_y")?),
        max_luminance: ratio(sd, "max_luminance")?,
        min_luminance: ratio(sd, "min_luminance")?,
    })
}

fn parse_cll(sd: &Value) -> Option<ContentLight> {
    Some(ContentLight { max_cll: int_of(sd, "max_content")? as u32, max_fall: int_of(sd, "max_average")? as u32 })
}

fn disposition(v: &Value, key: &str) -> bool {
    v.get("disposition").and_then(|d| d.get(key)).and_then(Value::as_i64).unwrap_or(0) == 1
}

const BITMAP_SUBS: &[&str] = &["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub", "dvb_teletext"];

fn parse_video(st: &Value, index: u32) -> VideoStream {
    let tags = tags_of(st);
    let pix_fmt = str_of(st, "pix_fmt");
    let traits = pix_fmt.as_deref().and_then(pix_traits);
    let color_transfer = str_of(st, "color_transfer");
    let hdr = match color_transfer.as_deref() {
        Some("smpte2084") => HdrKind::Pq,
        Some("arib-std-b67") => HdrKind::Hlg,
        _ => HdrKind::Sdr,
    };
    let mut rotation = None;
    let (mut md, mut cl, mut dv) = (None, None, None);
    for sd in st.get("side_data_list").and_then(Value::as_array).into_iter().flatten() {
        match sd.get("side_data_type").and_then(Value::as_str).unwrap_or("") {
            "Display Matrix" => rotation = num_of(sd, "rotation").map(display_rotation_cw),
            "Mastering display metadata" => md = parse_mastering(sd),
            "Content light level metadata" => cl = parse_cll(sd),
            "DOVI configuration record" => {
                dv = Some(DolbyVision {
                    profile: int_of(sd, "dv_profile").unwrap_or(0) as u32,
                    bl_compat_id: int_of(sd, "dv_bl_signal_compatibility_id").unwrap_or(0) as u32,
                })
            }
            _ => {}
        }
    }
    // Legacy MP4 "rotate" tag is already clockwise.
    let rotation = rotation
        .or_else(|| tag(&tags, "rotate").and_then(|r| r.parse::<f64>().ok()).map(|r| display_rotation_cw(-r)))
        .unwrap_or(0);
    let r_frame_rate = st.get("r_frame_rate").and_then(Value::as_str).and_then(parse_rational);
    let avg_frame_rate = st.get("avg_frame_rate").and_then(Value::as_str).and_then(parse_rational);
    let field_order = str_of(st, "field_order");
    VideoStream {
        index,
        codec: str_of(st, "codec_name").unwrap_or_else(|| "unknown".into()),
        codec_long: str_of(st, "codec_long_name").unwrap_or_default(),
        profile: str_of(st, "profile"),
        level: st.get("level").and_then(Value::as_i64).filter(|l| *l > 0),
        width: int_of(st, "width").unwrap_or(0) as u32,
        height: int_of(st, "height").unwrap_or(0) as u32,
        sar: str_of(st, "sample_aspect_ratio"),
        dar: str_of(st, "display_aspect_ratio"),
        bit_depth: traits.map(|t| t.bit_depth),
        chroma: traits.map(|t| t.chroma),
        alpha: traits.map(|t| t.alpha).unwrap_or(false),
        rgb: traits.map(|t| t.rgb).unwrap_or(false),
        pix_fmt,
        color_range: str_of(st, "color_range"),
        color_space: str_of(st, "color_space"),
        color_transfer,
        color_primaries: str_of(st, "color_primaries"),
        hdr,
        mastering_display: md,
        content_light: cl,
        dolby_vision: dv,
        rotation,
        fps: avg_frame_rate.or(r_frame_rate).map(Rational::value),
        r_frame_rate,
        avg_frame_rate,
        vfr: false,
        interlaced: matches!(field_order.as_deref(), Some("tt" | "bb" | "tb" | "bt")),
        field_order,
        nb_frames: int_of(st, "nb_frames"),
        duration: num_of(st, "duration").or_else(|| tag(&tags, "DURATION").and_then(|d| parse_hms(&d))),
        bit_rate: int_of(st, "bit_rate"),
        attached_pic: disposition(st, "attached_pic"),
        default: disposition(st, "default"),
        language: tag(&tags, "language").filter(|l| l != "und"),
        title: tag(&tags, "title"),
    }
}

/// Parse `ffprobe -show_format -show_streams -show_chapters -print_format json`.
pub fn parse_probe_json(path: &Path, json: &str) -> Result<MediaInfo, ProbeError> {
    let v: Value = serde_json::from_str(json).map_err(|e| ProbeError::Invalid(e.to_string()))?;
    let fmt = v.get("format").cloned().unwrap_or(Value::Null);
    let streams = v.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    if fmt.is_null() && streams.is_empty() {
        return Err(ProbeError::Invalid("no format or streams".into()));
    }
    let mut info = MediaInfo {
        path: path.to_path_buf(),
        size: int_of(&fmt, "size").unwrap_or(0),
        format_name: str_of(&fmt, "format_name").unwrap_or_default(),
        format_long_name: str_of(&fmt, "format_long_name").unwrap_or_default(),
        duration: num_of(&fmt, "duration"),
        start_time: num_of(&fmt, "start_time"),
        bit_rate: int_of(&fmt, "bit_rate"),
        video: Vec::new(),
        audio: Vec::new(),
        subtitles: Vec::new(),
        attachments: 0,
        data_streams: 0,
        chapters: v.get("chapters").and_then(Value::as_array).map(|c| c.len() as u32).unwrap_or(0),
        tags: tags_of(&fmt),
    };
    for st in &streams {
        let index = int_of(st, "index").unwrap_or(0) as u32;
        let tags = tags_of(st);
        match st.get("codec_type").and_then(Value::as_str).unwrap_or("") {
            "video" => info.video.push(parse_video(st, index)),
            "audio" => info.audio.push(AudioStream {
                index,
                codec: str_of(st, "codec_name").unwrap_or_else(|| "unknown".into()),
                codec_long: str_of(st, "codec_long_name").unwrap_or_default(),
                profile: str_of(st, "profile"),
                channels: int_of(st, "channels").unwrap_or(0) as u32,
                channel_layout: str_of(st, "channel_layout"),
                sample_rate: int_of(st, "sample_rate").map(|n| n as u32),
                sample_fmt: str_of(st, "sample_fmt"),
                bit_rate: int_of(st, "bit_rate"),
                duration: num_of(st, "duration").or_else(|| tag(&tags, "DURATION").and_then(|d| parse_hms(&d))),
                language: tag(&tags, "language").filter(|l| l != "und"),
                title: tag(&tags, "title"),
                default: disposition(st, "default"),
            }),
            "subtitle" => {
                let codec = str_of(st, "codec_name").unwrap_or_else(|| "unknown".into());
                info.subtitles.push(SubtitleStream {
                    index,
                    bitmap: BITMAP_SUBS.contains(&codec.as_str()),
                    codec,
                    language: tag(&tags, "language").filter(|l| l != "und"),
                    title: tag(&tags, "title"),
                    default: disposition(st, "default"),
                    forced: disposition(st, "forced"),
                })
            }
            "attachment" => info.attachments += 1,
            "data" => info.data_streams += 1,
            _ => {}
        }
    }
    Ok(info)
}

/// Mastering display / content light from `-show_frames -show_entries frame=side_data_list` JSON.
pub fn parse_frame_side_data(json: &str) -> (Option<MasteringDisplay>, Option<ContentLight>) {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return (None, None) };
    let (mut md, mut cl) = (None, None);
    let frames = v.get("frames").and_then(Value::as_array).cloned().unwrap_or_default();
    for sd in frames.iter().flat_map(|f| f.get("side_data_list").and_then(Value::as_array).cloned().unwrap_or_default()) {
        match sd.get("side_data_type").and_then(Value::as_str).unwrap_or("") {
            "Mastering display metadata" => md = md.or_else(|| parse_mastering(&sd)),
            "Content light level metadata" => cl = cl.or_else(|| parse_cll(&sd)),
            _ => {}
        }
    }
    (md, cl)
}

/// `file:` prefix stops FFmpeg from interpreting names like `concat:x` or `http:x` as protocols.
pub fn ff_path(p: &Path) -> String {
    format!("file:{}", p.to_string_lossy())
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Probe a file: container + streams, plus HDR side data and a VFR sample when relevant.
pub async fn probe(ffprobe: &Path, path: &Path) -> Result<MediaInfo, ProbeError> {
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ProbeError::NotFound,
        _ => ProbeError::Unreadable(e.to_string()),
    })?;
    if meta.is_dir() {
        return Err(ProbeError::Unreadable("is a directory".into()));
    }
    let src = ff_path(path);
    let args = ["-v", "error", "-hide_banner", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters", "-i", &src];
    let out = process::run_capture(ffprobe, &args, PROBE_TIMEOUT).await.map_err(|e| ProbeError::Backend(e.to_string()))?;
    if !out.success() {
        return Err(ProbeError::Backend(out.stderr_tail(6)));
    }
    let mut info = parse_probe_json(path, &String::from_utf8_lossy(&out.stdout))?;
    let Some(vi) = info.video.iter().position(|v| !v.attached_pic) else { return Ok(info) };
    let sel = info.video[vi].index.to_string();

    if info.video[vi].hdr != HdrKind::Sdr && info.video[vi].mastering_display.is_none() {
        let args = ["-v", "error", "-select_streams", &sel, "-read_intervals", "%+#1", "-show_frames", "-show_entries", "frame=side_data_list", "-print_format", "json", "-i", &src];
        if let Ok(o) = process::run_capture(ffprobe, &args, PROBE_TIMEOUT).await {
            let (md, cl) = parse_frame_side_data(&String::from_utf8_lossy(&o.stdout));
            let v = &mut info.video[vi];
            v.mastering_display = v.mastering_display.or(md);
            v.content_light = v.content_light.or(cl);
        }
    }

    let args = ["-v", "error", "-select_streams", &sel, "-read_intervals", "%+#600", "-show_entries", "packet=pts_time", "-of", "csv=p=0", "-i", &src];
    if let Ok(o) = process::run_capture(ffprobe, &args, PROBE_TIMEOUT).await {
        let pts: Vec<f64> = String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().trim_end_matches(',').parse().ok()).collect();
        info.video[vi].vfr = vfr_from_pts(&pts);
    }
    Ok(info)
}

/// Number of packets in one stream (reads the file, no decoding).
pub async fn count_packets(ffprobe: &Path, path: &Path, stream_index: u32) -> Result<u64, ProbeError> {
    let src = ff_path(path);
    let sel = stream_index.to_string();
    let args = ["-v", "error", "-select_streams", &sel, "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0", "-i", &src];
    let out = process::run_capture(ffprobe, &args, Duration::from_secs(1800)).await.map_err(|e| ProbeError::Backend(e.to_string()))?;
    if !out.success() {
        return Err(ProbeError::Backend(out.stderr_tail(4)));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_end_matches(',')
        .parse()
        .map_err(|_| ProbeError::Invalid("packet count".into()))
}

/// Keyframe timestamps (seconds) of the main video stream, from packet flags.
pub async fn keyframes(ffprobe: &Path, path: &Path, stream_index: u32) -> Result<Vec<f64>, ProbeError> {
    let src = ff_path(path);
    let sel = stream_index.to_string();
    let args = ["-v", "error", "-select_streams", &sel, "-show_entries", "packet=pts_time,flags", "-of", "csv=p=0", "-i", &src];
    let out = process::run_capture(ffprobe, &args, Duration::from_secs(1800)).await.map_err(|e| ProbeError::Backend(e.to_string()))?;
    if !out.success() {
        return Err(ProbeError::Backend(out.stderr_tail(4)));
    }
    let mut v: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split(',');
            let t: f64 = it.next()?.trim().parse().ok()?;
            it.next().filter(|f| f.contains('K')).map(|_| t)
        })
        .collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v.dedup();
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> MediaInfo {
        let json = match name {
            "h264_aac" => include_str!("../tests/fixtures/h264_aac.json"),
            "rotated" => include_str!("../tests/fixtures/rotated.json"),
            "hdr10" => include_str!("../tests/fixtures/hdr10.json"),
            "av1_opus" => include_str!("../tests/fixtures/av1_opus.json"),
            "multi_audio" => include_str!("../tests/fixtures/multi_audio.json"),
            "pcm_audio" => include_str!("../tests/fixtures/pcm_audio.json"),
            "no_audio" => include_str!("../tests/fixtures/no_audio.json"),
            "fullrange" => include_str!("../tests/fixtures/fullrange.json"),
            "yuv444" => include_str!("../tests/fixtures/yuv444.json"),
            _ => unreachable!(),
        };
        parse_probe_json(Path::new("/v/x"), json).unwrap()
    }

    #[test]
    fn parses_h264_aac_mp4() {
        let m = fixture("h264_aac");
        assert!(m.is_mp4_family());
        assert_eq!(m.duration, Some(4.0));
        assert_eq!(m.size, 446683);
        let v = m.main_video().unwrap();
        assert_eq!((v.codec.as_str(), v.width, v.height), ("h264", 640, 360));
        assert_eq!((v.bit_depth, v.chroma, v.hdr), (Some(8), Some(Chroma::C420), HdrKind::Sdr));
        assert_eq!(v.fps, Some(30.0));
        assert_eq!(v.nb_frames, Some(120));
        assert_eq!(v.rotation, 0);
        assert_eq!(m.audio.len(), 1);
        assert_eq!((m.audio[0].codec.as_str(), m.audio[0].channels, m.audio[0].sample_rate), ("aac", 2, Some(48000)));
    }

    #[test]
    fn rotation_comes_from_display_matrix() {
        let v = fixture("rotated").main_video().unwrap().clone();
        assert_eq!(v.rotation, 90);
        assert_eq!(v.display_dims(), (360, 640));
    }

    #[test]
    fn hdr10_is_detected_as_pq_bt2020_10bit() {
        let m = fixture("hdr10");
        let v = m.main_video().unwrap();
        assert_eq!(v.hdr, HdrKind::Pq);
        assert_eq!(v.bit_depth, Some(10));
        assert_eq!(v.color_primaries.as_deref(), Some("bt2020"));
    }

    #[test]
    fn frame_side_data_gives_mastering_display_and_cll() {
        let (md, cl) = parse_frame_side_data(include_str!("../tests/fixtures/hdr10_frame_side.json"));
        let md = md.unwrap();
        assert!((md.max_luminance - 1000.0).abs() < 1e-9);
        assert!((md.min_luminance - 0.0001).abs() < 1e-12);
        assert!((md.red.0 - 0.68).abs() < 1e-9);
        assert_eq!(cl, Some(ContentLight { max_cll: 1000, max_fall: 400 }));
    }

    #[test]
    fn mkv_stream_duration_comes_from_tags() {
        let m = fixture("av1_opus");
        let v = m.main_video().unwrap();
        assert_eq!(v.codec, "av1");
        assert_eq!(v.duration, Some(4.0));
        assert_eq!(m.duration, Some(4.008));
        assert_eq!(v.nb_frames, None);
    }

    #[test]
    fn multiple_audio_tracks_keep_language_title_and_layout() {
        let m = fixture("multi_audio");
        assert_eq!(m.audio.len(), 2);
        assert_eq!(m.audio[0].language.as_deref(), Some("zho"));
        assert_eq!(m.audio[0].title.as_deref(), Some("中文 立体声"));
        assert_eq!((m.audio[1].codec.as_str(), m.audio[1].channels), ("ac3", 6));
        assert_eq!(m.subtitles.len(), 1);
        assert_eq!(m.subtitles[0].codec, "subrip");
        assert!(!m.subtitles[0].bitmap);
        assert_eq!(m.audio[1].index, 2, "absolute stream index");
    }

    #[test]
    fn pcm_and_no_audio_sources() {
        assert_eq!(fixture("pcm_audio").audio[0].codec, "pcm_s16le");
        assert!(fixture("no_audio").audio.is_empty());
    }

    #[test]
    fn full_range_and_444_sources() {
        let f = fixture("fullrange");
        let v = f.main_video().unwrap();
        assert!(v.full_range());
        assert_eq!(v.pix_fmt.as_deref(), Some("yuvj420p"));
        let y = fixture("yuv444");
        assert_eq!(y.main_video().unwrap().chroma, Some(Chroma::C444));
    }

    #[test]
    fn pix_traits_cover_common_formats() {
        let t = |f| pix_traits(f).unwrap();
        assert_eq!((t("yuv420p").bit_depth, t("yuv420p").chroma), (8, Chroma::C420));
        assert_eq!((t("yuv420p10le").bit_depth, t("yuv420p10le").chroma), (10, Chroma::C420));
        assert!(t("yuvj420p").full_range);
        assert_eq!(t("nv12").chroma, Chroma::C420);
        assert_eq!((t("p010le").bit_depth, t("p010le").chroma), (10, Chroma::C420));
        assert_eq!((t("yuv422p10le").bit_depth, t("yuv422p10le").chroma), (10, Chroma::C422));
        assert_eq!(t("yuv444p12le").bit_depth, 12);
        assert!(t("gbrp").rgb && t("gbrp").chroma == Chroma::C444);
        assert_eq!((t("gray10le").bit_depth, t("gray10le").chroma), (10, Chroma::Gray));
        assert!(t("yuva420p").alpha);
        assert!(t("bgra").alpha && t("bgra").rgb);
        assert_eq!(t("rgb48le").bit_depth, 16);
        assert_eq!(t("yuv420p16le").bit_depth, 16);
        assert!(pix_traits("weird_format_x").is_none());
    }

    #[test]
    fn rational_parsing() {
        assert!((parse_rational("30000/1001").unwrap().value() - 29.97).abs() < 0.001);
        assert_eq!(parse_rational("0/0"), None);
        assert_eq!(parse_rational("25"), Some(Rational { num: 25, den: 1 }));
    }

    #[test]
    fn display_rotation_normalisation() {
        assert_eq!(display_rotation_cw(-90.0), 90);
        assert_eq!(display_rotation_cw(90.0), 270);
        assert_eq!(display_rotation_cw(180.0), 180);
        assert_eq!(display_rotation_cw(-180.0), 180);
        assert_eq!(display_rotation_cw(0.0), 0);
        assert_eq!(display_rotation_cw(-89.6), 90);
    }

    #[test]
    fn vfr_detection_from_timestamps() {
        let cfr: Vec<f64> = (0..60).map(|i| i as f64 / 30.0).collect();
        assert!(!vfr_from_pts(&cfr));
        // millisecond-rounded CFR (0.033/0.034 steps) is still constant
        let rounded: Vec<f64> = (0..60).map(|i| ((i as f64 / 30.0) * 1000.0).round() / 1000.0).collect();
        assert!(!vfr_from_pts(&rounded));
        let mut vfr: Vec<f64> = (0..45).map(|i| i as f64 / 30.0).collect();
        vfr.extend((1..30).map(|i| 1.5 + i as f64 / 15.0));
        vfr.reverse(); // decode order does not matter
        assert!(vfr_from_pts(&vfr));
        assert!(!vfr_from_pts(&[0.0, 0.04]));
    }

    #[test]
    fn summary_describes_the_source() {
        let s = fixture("multi_audio").summary();
        assert_eq!(s.video.as_deref(), Some("h264"));
        assert_eq!((s.width, s.height, s.audio_tracks, s.subtitle_tracks), (Some(640), Some(360), 2, 1));
        assert_eq!(s.container, "matroska,webm");
        let r = fixture("rotated").summary();
        assert_eq!((r.width, r.height), (Some(360), Some(640)), "summary uses display orientation");
    }
}
