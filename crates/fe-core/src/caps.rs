//! Backend capability detection. Nothing is assumed from a binary's name or
//! the platform: hardware encoders and decoders count only after a real test
//! run, and lossless modes count only after a pixel-exact round trip.

use crate::encoders::{self, EncArgs, EncoderDef, Lossless, RateArgs};
use crate::media::ff_path;
use crate::process;
use crate::settings::Speed;
use crate::tools::FfmpegPaths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Capability cache format; bump to invalidate cached results.
pub const CAPS_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCaps {
    pub name: String,
    pub hardware: bool,
    /// Usable on this machine (software: listed; hardware: test encode passed).
    pub usable: bool,
    /// Constant-quality mode works (some hardware only does bitrate mode).
    pub quality_mode: bool,
    pub pix_fmts: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LosslessCheck {
    pub encoder: String,
    pub pix_fmt: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CpuInfo {
    pub logical: u32,
    pub physical: Option<u32>,
    /// Performance cores on hybrid CPUs (Apple Silicon), if known.
    pub performance: Option<u32>,
    pub brand: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub caps_version: u32,
    pub cache_key: String,
    pub ffmpeg: FfmpegPaths,
    pub version: String,
    pub version_num: Option<(u32, u32)>,
    pub license: String,
    pub configuration: String,
    pub encoders: Vec<EncoderCaps>,
    pub audio_encoders: Vec<String>,
    pub decoders: Vec<String>,
    pub filters: Vec<String>,
    pub muxers: Vec<String>,
    pub hwaccels: Vec<String>,
    /// Verified hardware decode paths, e.g. "videotoolbox:h264".
    pub hw_decode: Vec<String>,
    pub lossless: Vec<LosslessCheck>,
    pub cpu: CpuInfo,
    pub memory_mb: u64,
    pub detected_at: i64,
}

impl Capabilities {
    pub fn encoder(&self, name: &str) -> Option<&EncoderCaps> {
        self.encoders.iter().find(|e| e.name == name)
    }
    pub fn usable(&self, name: &str) -> bool {
        self.encoder(name).map(|e| e.usable).unwrap_or(false)
    }
    pub fn supports_pix_fmt(&self, encoder: &str, pix_fmt: &str) -> bool {
        self.encoder(encoder).map(|e| e.pix_fmts.iter().any(|p| p == pix_fmt)).unwrap_or(false)
    }
    /// `Some(true)` only after a passed pixel-exact self-test for this pair.
    pub fn lossless_ok(&self, encoder: &str, pix_fmt: &str) -> Option<bool> {
        self.lossless.iter().find(|c| c.encoder == encoder && c.pix_fmt == pix_fmt).map(|c| c.ok)
    }
    pub fn has_filter(&self, name: &str) -> bool {
        self.filters.iter().any(|f| f == name)
    }
    pub fn has_audio_encoder(&self, name: &str) -> bool {
        self.audio_encoders.iter().any(|f| f == name)
    }
    /// `-fps_mode` exists since FFmpeg 5.1; older builds use `-vsync`.
    pub fn fps_mode_option(&self) -> &'static str {
        match self.version_num {
            Some((maj, min)) if maj < 5 || (maj == 5 && min < 1) => "-vsync",
            _ => "-fps_mode",
        }
    }
    pub fn hw_decode_for(&self, codec: &str) -> Option<String> {
        self.hw_decode.iter().find_map(|h| {
            let (accel, c) = h.split_once(':')?;
            (c == codec).then(|| accel.to_string())
        })
    }
}

/// "ffmpeg version 8.1.2 Copyright…" → ("8.1.2", Some((8, 1)))
pub fn parse_version(first_line: &str) -> (String, Option<(u32, u32)>) {
    let v = first_line.split_whitespace().skip_while(|w| *w != "version").nth(1).unwrap_or("").to_string();
    let digits = v.strip_prefix('n').unwrap_or(&v);
    let mut parts = digits.split(|c: char| !c.is_ascii_digit());
    let num = match (parts.next().and_then(|p| p.parse().ok()), parts.next().and_then(|p| p.parse().ok())) {
        (Some(a), Some(b)) if !digits.starts_with('N') => Some((a, b)),
        _ => None,
    };
    (v, num)
}

/// Rows of `-encoders` / `-decoders`: (flags, name).
pub fn parse_codec_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("---"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let flags = it.next()?;
            let name = it.next()?;
            (flags.len() == 6).then(|| (flags.to_string(), name.to_string()))
        })
        .collect()
}

pub fn parse_filters(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            (t.len() >= 3 && t[2].contains("->")).then(|| t[1].to_string())
        })
        .collect()
}

/// Names of formats that can be written (`-muxers`).
pub fn parse_muxers(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| l.trim() != "---")
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let flags = it.next()?;
            let name = it.next()?;
            flags.contains('E').then(|| name.to_string())
        })
        .collect()
}

pub fn parse_hwaccels(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.starts_with("Hardware acceleration methods"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// "Supported pixel formats: a b c" from `-h encoder=NAME`.
pub fn parse_pix_fmts(help: &str) -> Vec<String> {
    help.lines()
        .find_map(|l| l.trim().strip_prefix("Supported pixel formats:"))
        .map(|rest| rest.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Licence of the build from its configure flags.
pub fn parse_license(configuration: &str) -> String {
    let has = |f: &str| configuration.split_whitespace().any(|w| w == f);
    match (has("--enable-nonfree"), has("--enable-gpl"), has("--enable-version3")) {
        (true, _, _) => "non-free (not redistributable)",
        (false, true, true) => "GPL v3",
        (false, true, false) => "GPL v2+",
        (false, false, true) => "LGPL v3",
        (false, false, false) => "LGPL v2.1+",
    }
    .to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("ffmpeg cannot run: {0}")]
    FfmpegNotRunnable(String),
    #[error("ffprobe cannot run: {0}")]
    FfprobeNotRunnable(String),
}

const SHORT: Duration = Duration::from_secs(30);
const TEST_TIMEOUT: Duration = Duration::from_secs(45);

const AUDIO_ENCODERS: &[&str] =
    &["aac", "aac_at", "libfdk_aac", "libopus", "libmp3lame", "flac", "alac", "ac3", "eac3", "pcm_s16le", "pcm_s24le", "libvorbis"];
const DECODERS: &[&str] = &[
    "h264", "hevc", "av1", "libdav1d", "libaom-av1", "vp9", "libvpx-vp9", "vp8", "prores", "mpeg2video", "mpeg4", "ffv1",
    "vvc", "hdmv_pgs_subtitle", "dvd_subtitle", "ass", "subrip", "mov_text", "webvtt",
];
const FILTERS: &[&str] = &[
    "subtitles", "ass", "overlay", "libvmaf", "psnr", "ssim", "scale", "zscale", "transpose", "hflip", "vflip", "crop",
    "fps", "setparams", "tonemap", "yadif", "bwdif", "cropdetect", "format", "setpts", "aformat", "pad",
];
const MUXERS: &[&str] = &[
    "mp4", "mov", "matroska", "ipod", "adts", "ac3", "eac3", "flac", "ogg", "opus", "mp3", "wav", "srt", "ass",
    "webvtt", "sup", "null", "framemd5", "streamhash", "hash", "dts", "truehd", "image2", "w64",
];
const HW_ACCELS: &[&str] = &["videotoolbox", "cuda", "d3d11va", "dxva2", "qsv", "vaapi", "d3d12va"];

async fn stdout_text(ff: &Path, args: &[&str]) -> String {
    match process::run_capture(ff, args, SHORT).await {
        Ok(o) if o.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Changes whenever the backend binaries change, so cached capabilities stay honest.
pub fn cache_key(paths: &FfmpegPaths) -> String {
    let stamp = |p: &Path| {
        std::fs::metadata(p)
            .map(|m| {
                let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
                format!("{}:{}", m.len(), t.unwrap_or(0))
            })
            .unwrap_or_default()
    };
    format!(
        "v{CAPS_VERSION}|{}|{}|{}|{}",
        paths.ffmpeg.display(),
        stamp(&paths.ffmpeg),
        paths.ffprobe.display(),
        stamp(&paths.ffprobe)
    )
}

pub fn cpu_info() -> CpuInfo {
    let logical = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
    CpuInfo {
        logical,
        physical: sysinfo::System::physical_core_count().map(|n| n as u32),
        performance: perf_cores(),
        brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

#[cfg(target_vendor = "apple")]
fn perf_cores() -> Option<u32> {
    let name = std::ffi::CString::new("hw.perflevel0.physicalcpu").ok()?;
    let mut value: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    // SAFETY: sysctlbyname writes at most `size` bytes into `value`.
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), &mut value as *mut i32 as *mut _, &mut size, std::ptr::null_mut(), 0) };
    (r == 0 && value > 0).then_some(value as u32)
}

#[cfg(not(target_vendor = "apple"))]
fn perf_cores() -> Option<u32> {
    None
}

pub fn memory_mb() -> (u64, u64) {
    let mut s = sysinfo::System::new();
    s.refresh_memory();
    (s.total_memory() / 1_048_576, s.available_memory() / 1_048_576)
}

fn test_input_args(size: &str, frames: u32) -> Vec<String> {
    let src = format!("testsrc2=size={size}:rate=30");
    ["-hide_banner", "-v", "error", "-f", "lavfi", "-i", &src, "-frames:v", &frames.to_string()]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

async fn try_encode(ff: &Path, mut args: Vec<String>, extra: &[String]) -> Result<(), String> {
    args.extend(extra.iter().cloned());
    args.extend(["-f".into(), "null".into(), "-".into()]);
    match process::run_capture(ff, &args, TEST_TIMEOUT).await {
        Ok(o) if o.success() => Ok(()),
        Ok(o) => Err(o.stderr_tail(3)),
        Err(e) => Err(e.to_string()),
    }
}

/// Hardware encoders are only "available" after a real test encode with the
/// exact options a job would use. Quality mode is tried first, then bitrate.
pub async fn smoke_test_hw(ff: &Path, def: &'static EncoderDef, mut pix_fmts: Vec<String>) -> EncoderCaps {
    let enc = |rate| encoders::video_args(def, &EncArgs { rate, speed: Speed::Gpu, threads: None, logical_cpus: 1, pass: None });
    let q = def.quality.map(|q| encoders::level_to_native(&q, 50)).unwrap_or(0.0);
    let base = test_input_args("320x240", 10);
    let quality = try_encode(ff, base.clone(), &enc(RateArgs::Quality(q))).await;
    let (usable, quality_mode, error) = match quality {
        Ok(()) => (true, true, None),
        Err(qe) => match try_encode(ff, base.clone(), &enc(RateArgs::Bitrate(2000))).await {
            Ok(()) => (true, false, Some(qe)),
            Err(be) => (false, false, Some(be)),
        },
    };
    if usable {
        // Keep 10-bit formats only if a 10-bit encode really works on this device.
        let high: Vec<String> = pix_fmts.iter().filter(|p| p.contains("10") || p.contains("12") || p.contains("16")).cloned().collect();
        if let Some(tenbit) = high.first() {
            let rate = if quality_mode { RateArgs::Quality(q) } else { RateArgs::Bitrate(2000) };
            let mut extra = enc(rate);
            extra.extend(["-pix_fmt".into(), tenbit.clone()]);
            if try_encode(ff, base, &extra).await.is_err() {
                pix_fmts.retain(|p| !high.contains(p));
            }
        }
    }
    EncoderCaps { name: def.name.into(), hardware: true, usable, quality_mode, pix_fmts, error }
}

/// Framemd5 hashes (hash column only) of the main video stream.
pub async fn frame_hashes(ff: &Path, input: &Path, input_opts: &[String]) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-v".into(), "error".into()];
    args.extend(input_opts.iter().cloned());
    args.extend(["-i".into(), ff_path(input), "-map".into(), "0:v:0".into(), "-f".into(), "framemd5".into(), "-".into()]);
    let out = process::run_capture(ff, &args, Duration::from_secs(7200)).await.map_err(|e| e.to_string())?;
    if !out.success() {
        return Err(out.stderr_tail(3));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string()))
        .filter(|h| !h.is_empty())
        .collect())
}

/// Encode a noisy synthetic clip losslessly and require identical decoded frames.
pub async fn lossless_self_test(ff: &Path, def: &'static EncoderDef, pix_fmt: &str, work_dir: &Path) -> LosslessCheck {
    let check = |ok: bool, detail: String| LosslessCheck { encoder: def.name.into(), pix_fmt: pix_fmt.into(), ok, detail };
    if def.lossless == Lossless::No {
        return check(false, "encoder has no lossless mode".into());
    }
    let dir = work_dir.join("lossless-selftest");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return check(false, e.to_string());
    }
    let src = dir.join(format!("src_{pix_fmt}.nut"));
    let out = dir.join(format!("{}_{pix_fmt}.mkv", def.name));
    let source_graph = format!("testsrc2=size=128x72:rate=10,noise=alls=25:allf=t,format={pix_fmt}");
    let mk = ["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i", &source_graph, "-frames:v", "8", "-c:v", "rawvideo", "-f", "nut"];
    let mut mk: Vec<String> = mk.iter().map(|s| s.to_string()).collect();
    mk.push(ff_path(&src));
    match process::run_capture(ff, &mk, TEST_TIMEOUT).await {
        Ok(o) if o.success() => {}
        Ok(o) => return check(false, format!("cannot create {pix_fmt} source: {}", o.stderr_tail(2))),
        Err(e) => return check(false, e.to_string()),
    }
    let mut enc: Vec<String> = ["-hide_banner", "-v", "error", "-y", "-i"].iter().map(|s| s.to_string()).collect();
    enc.push(ff_path(&src));
    enc.extend(encoders::video_args(
        def,
        // Small explicit thread budget: several self-tests run at once at startup.
        &EncArgs { rate: RateArgs::Lossless, speed: Speed::Balanced, threads: Some(2), logical_cpus: 2, pass: None },
    ));
    enc.extend(["-f".into(), "matroska".into(), ff_path(&out)]);
    let result = match process::run_capture(ff, &enc, TEST_TIMEOUT).await {
        Ok(o) if o.success() => match (frame_hashes(ff, &src, &[]).await, frame_hashes(ff, &out, &[]).await) {
            (Ok(a), Ok(b)) if !a.is_empty() && a == b => check(true, format!("{} frames identical", a.len())),
            (Ok(a), Ok(b)) => check(false, format!("decoded frames differ ({} vs {})", a.len(), b.len())),
            (Err(e), _) | (_, Err(e)) => check(false, e),
        },
        Ok(o) => check(false, o.stderr_tail(2)),
        Err(e) => check(false, e.to_string()),
    };
    let _ = std::fs::remove_file(&out);
    result
}

/// Decode a small sample through each hardware accelerator; keep the ones that work.
async fn verify_hw_decode(ff: &Path, hwaccels: &[String], listed: &[String], work_dir: &Path) -> Vec<String> {
    let mut ok = Vec::new();
    let dir = work_dir.join("hwdec-selftest");
    if std::fs::create_dir_all(&dir).is_err() {
        return ok;
    }
    for (codec, enc) in [("h264", "libx264"), ("hevc", "libx265")] {
        if !listed.iter().any(|e| e == enc) {
            continue;
        }
        let sample = dir.join(format!("{codec}.mp4"));
        let mut mk = test_input_args("320x240", 10);
        mk.extend(["-y".into(), "-pix_fmt".into(), "yuv420p".into(), "-c:v".into(), enc.into(), "-preset".into(), "ultrafast".into()]);
        if enc == "libx265" {
            mk.extend(["-x265-params".into(), "log-level=error".into()]);
        }
        mk.push(ff_path(&sample));
        if !matches!(process::run_capture(ff, &mk, TEST_TIMEOUT).await, Ok(o) if o.success()) {
            continue;
        }
        for accel in hwaccels.iter().filter(|a| HW_ACCELS.contains(&a.as_str())) {
            let args = ["-hide_banner", "-v", "verbose", "-hwaccel", accel, "-i", &ff_path(&sample), "-f", "null", "-"];
            if let Ok(o) = process::run_capture(ff, &args, TEST_TIMEOUT).await {
                let failed = o.stderr.contains("Failed setup") || o.stderr.contains("returned error") || o.stderr.contains("Device creation failed");
                if o.success() && !failed {
                    ok.push(format!("{accel}:{codec}"));
                }
            }
        }
    }
    ok
}

/// Full detection. Takes a few seconds the first time; callers cache by `cache_key`.
pub async fn detect(paths: &FfmpegPaths, work_dir: &Path) -> Result<Capabilities, CapsError> {
    let ff = &paths.ffmpeg;
    let ver = process::run_capture(ff, &["-hide_banner", "-version"], SHORT)
        .await
        .map_err(|e| CapsError::FfmpegNotRunnable(e.to_string()))?;
    if !ver.success() {
        return Err(CapsError::FfmpegNotRunnable(ver.stderr_tail(3)));
    }
    let vtext = String::from_utf8_lossy(&ver.stdout).into_owned();
    let (version, version_num) = parse_version(vtext.lines().next().unwrap_or(""));
    let configuration = vtext.lines().find_map(|l| l.strip_prefix("configuration:")).unwrap_or("").trim().to_string();
    match process::run_capture(&paths.ffprobe, &["-hide_banner", "-version"], SHORT).await {
        Ok(o) if o.success() => {}
        Ok(o) => return Err(CapsError::FfprobeNotRunnable(o.stderr_tail(3))),
        Err(e) => return Err(CapsError::FfprobeNotRunnable(e.to_string())),
    }

    let (enc, dec, fil, mux, hw) = tokio::join!(
        stdout_text(ff, &["-hide_banner", "-encoders"]),
        stdout_text(ff, &["-hide_banner", "-decoders"]),
        stdout_text(ff, &["-hide_banner", "-filters"]),
        stdout_text(ff, &["-hide_banner", "-muxers"]),
        stdout_text(ff, &["-hide_banner", "-hwaccels"]),
    );
    let enc_rows = parse_codec_list(&enc);
    let listed: Vec<String> = enc_rows.iter().map(|(_, n)| n.clone()).collect();
    let keep = |names: Vec<String>, wanted: &[&str]| names.into_iter().filter(|n| wanted.contains(&n.as_str())).collect::<Vec<_>>();
    let audio_encoders = keep(enc_rows.iter().filter(|(f, _)| f.starts_with('A')).map(|(_, n)| n.clone()).collect(), AUDIO_ENCODERS);
    let decoders = keep(parse_codec_list(&dec).into_iter().map(|(_, n)| n).collect(), DECODERS);
    let filters = keep(parse_filters(&fil), FILTERS);
    let muxers = keep(parse_muxers(&mux), MUXERS);
    let hwaccels = parse_hwaccels(&hw);

    let mut set = tokio::task::JoinSet::new();
    for def in encoders::ENCODERS.iter().filter(|d| listed.iter().any(|n| n == d.name)) {
        let ff = ff.clone();
        set.spawn(async move {
            let help = stdout_text(&ff, &["-hide_banner", "-h", &format!("encoder={}", def.name)]).await;
            let pix = parse_pix_fmts(&help);
            if def.is_hardware() {
                smoke_test_hw(&ff, def, pix).await
            } else {
                EncoderCaps { name: def.name.into(), hardware: false, usable: true, quality_mode: true, pix_fmts: pix, error: None }
            }
        });
    }
    let mut found = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(ec) = r {
            found.push(ec);
        }
    }
    found.sort_by_key(|e| encoders::ENCODERS.iter().position(|d| d.name == e.name));

    let hw_decode = verify_hw_decode(ff, &hwaccels, &listed, work_dir).await;

    let mut lossless = Vec::new();
    let mut tests = tokio::task::JoinSet::new();
    for ec in found.iter().filter(|e| e.usable) {
        let Some(def) = encoders::get(&ec.name) else { continue };
        if def.lossless == Lossless::No {
            continue;
        }
        for pix in ["yuv420p", "yuv420p10le"].into_iter().filter(|p| ec.pix_fmts.iter().any(|x| x == p)) {
            let (ff, wd) = (ff.clone(), work_dir.to_path_buf());
            tests.spawn(async move { lossless_self_test(&ff, def, pix, &wd).await });
        }
    }
    while let Some(r) = tests.join_next().await {
        if let Ok(c) = r {
            lossless.push(c);
        }
    }
    lossless.sort_by(|a, b| (&a.encoder, &a.pix_fmt).cmp(&(&b.encoder, &b.pix_fmt)));

    Ok(Capabilities {
        caps_version: CAPS_VERSION,
        cache_key: cache_key(paths),
        ffmpeg: paths.clone(),
        version,
        version_num,
        license: parse_license(&configuration),
        configuration,
        encoders: found,
        audio_encoders,
        decoders,
        filters,
        muxers,
        hwaccels,
        hw_decode,
        lossless,
        cpu: cpu_info(),
        memory_mb: memory_mb().0,
        detected_at: chrono::Utc::now().timestamp_millis(),
    })
}

/// Work dir used by self-tests (kept small; files are overwritten each run).
pub fn selftest_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("selftest")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tools::ToolSource;

    const ENCODERS: &str = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264 / AVC (codec h264)\n V....D h264_videotoolbox    VideoToolbox H.264 Encoder (codec h264)\n A....D aac                  AAC (Advanced Audio Coding)\n S..... mov_text             3GPP Timed Text subtitle\n";

    #[test]
    fn version_line() {
        assert_eq!(parse_version("ffmpeg version 8.1.2 Copyright (c) 2000-2026"), ("8.1.2".into(), Some((8, 1))));
        assert_eq!(parse_version("ffmpeg version n7.1-3-gabc Copyright").1, Some((7, 1)));
        assert_eq!(parse_version("ffmpeg version N-118000-g1234 Copyright").1, None);
    }

    #[test]
    fn codec_list_rows_after_separator() {
        let rows = parse_codec_list(ENCODERS);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0], ("V....D".into(), "libx264".into()));
        assert_eq!(rows[2].1, "aac");
    }

    #[test]
    fn filters_list() {
        let text = "Filters:\n  T.. = Timeline support\n  | = Source or sink filter\n T. cropdetect        V->V       Auto-detect crop size.\n .. libvmaf           VV->V      Calculate the VMAF\n TS overlay           VV->V      Overlay a video\n";
        assert_eq!(parse_filters(text), vec!["cropdetect", "libvmaf", "overlay"]);
    }

    #[test]
    fn muxers_list() {
        let text = " File formats:\n D. = Demuxing supported\n .E = Muxing supported\n ---\n  E mp4             MP4 (MPEG-4 Part 14)\n  E matroska        Matroska\n  E null            raw null video\n";
        assert_eq!(parse_muxers(text), vec!["mp4", "matroska", "null"]);
    }

    #[test]
    fn hwaccels_list() {
        assert_eq!(parse_hwaccels("Hardware acceleration methods:\nvideotoolbox\n\n"), vec!["videotoolbox"]);
        assert!(parse_hwaccels("Hardware acceleration methods:\n").is_empty());
    }

    #[test]
    fn pixel_formats_from_encoder_help() {
        let help = "Encoder libsvtav1 [SVT-AV1]:\n    General capabilities: dr1\n    Supported pixel formats: yuv420p yuv420p10le\n";
        assert_eq!(parse_pix_fmts(help), vec!["yuv420p", "yuv420p10le"]);
        assert!(parse_pix_fmts("Encoder x:\n").is_empty());
    }

    #[test]
    fn license_from_configuration() {
        assert_eq!(parse_license("--enable-gpl --enable-version3 --enable-libx264"), "GPL v3");
        assert_eq!(parse_license("--enable-gpl --enable-libx264"), "GPL v2+");
        assert_eq!(parse_license("--enable-version3"), "LGPL v3");
        assert_eq!(parse_license("--disable-static"), "LGPL v2.1+");
        assert_eq!(parse_license("--enable-gpl --enable-nonfree"), "non-free (not redistributable)");
    }

    #[test]
    fn fps_mode_option_by_version() {
        let mut c = sample();
        c.version_num = Some((5, 0));
        assert_eq!(c.fps_mode_option(), "-vsync");
        c.version_num = Some((8, 1));
        assert_eq!(c.fps_mode_option(), "-fps_mode");
        c.version_num = None;
        assert_eq!(c.fps_mode_option(), "-fps_mode", "git builds are new");
    }

    #[test]
    fn hw_decode_lookup() {
        let mut c = sample();
        c.hw_decode = vec!["videotoolbox:h264".into(), "videotoolbox:hevc".into()];
        assert_eq!(c.hw_decode_for("hevc").as_deref(), Some("videotoolbox"));
        assert_eq!(c.hw_decode_for("av1"), None);
    }

    pub(crate) fn sample() -> Capabilities {
        Capabilities {
            caps_version: CAPS_VERSION,
            cache_key: String::new(),
            ffmpeg: FfmpegPaths { ffmpeg: "ffmpeg".into(), ffprobe: "ffprobe".into(), source: ToolSource::System },
            version: "8.1.2".into(),
            version_num: Some((8, 1)),
            license: "GPL v3".into(),
            configuration: String::new(),
            encoders: Vec::new(),
            audio_encoders: Vec::new(),
            decoders: Vec::new(),
            filters: Vec::new(),
            muxers: Vec::new(),
            hwaccels: Vec::new(),
            hw_decode: Vec::new(),
            lossless: Vec::new(),
            cpu: CpuInfo { logical: 10, ..Default::default() },
            memory_mb: 16384,
            detected_at: 0,
        }
    }
}
