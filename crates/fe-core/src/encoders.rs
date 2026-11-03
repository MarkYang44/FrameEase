//! Static knowledge about video encoders: how the user-facing compression
//! level, speed, bitrate, lossless mode and thread budget map onto each
//! encoder's *own* options. CRF/CQ values are encoder specific and are never
//! compared or copied across encoders.

use crate::settings::{Container, Speed, VideoCodec};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Cpu,
    Apple,
    Nvidia,
    Intel,
    Amd,
}

/// Name of the native quality knob, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum QualityKind {
    #[serde(rename = "CRF")]
    Crf,
    #[serde(rename = "CQ")]
    Cq,
    #[serde(rename = "q:v")]
    Qv,
    #[serde(rename = "ICQ")]
    Icq,
    #[serde(rename = "QP")]
    Qp,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityScale {
    pub kind: QualityKind,
    pub min: f64,
    pub max: f64,
    /// CRF-style: smaller = better. VideoToolbox q:v: larger = better.
    pub lower_is_better: bool,
    /// Native value at compression level 0 (best) and 100 (smallest).
    pub at_best: f64,
    pub at_smallest: f64,
    pub integer: bool,
}

/// Where encoder-private `key=value` parameters go (x265-params etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivParams {
    None,
    X265,
    Svt,
    Aom,
}

impl PrivParams {
    pub fn option(self) -> Option<&'static str> {
        match self {
            PrivParams::None => None,
            PrivParams::X265 => Some("-x265-params"),
            PrivParams::Svt => Some("-svtav1-params"),
            PrivParams::Aom => Some("-aom-params"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lossless {
    /// Not supported.
    No,
    /// Regular CLI options, e.g. `-qp 0`.
    Args(&'static [&'static str]),
    /// One private parameter, e.g. x265 `lossless=1`.
    Priv(&'static str, &'static str),
    /// The codec is lossless by construction (FFV1).
    Intrinsic,
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderDef {
    pub name: &'static str,
    pub codec: VideoCodec,
    pub vendor: Vendor,
    pub label: &'static str,
    pub quality: Option<QualityScale>,
    /// Option carrying the speed preset and its fast / balanced / slow values.
    pub speed: Option<(&'static str, [&'static str; 3])>,
    pub two_pass: bool,
    pub bitrate: bool,
    pub lossless: Lossless,
    pub privp: PrivParams,
    /// Always-on options (e.g. never let VideoToolbox fall back to software).
    pub fixed: &'static [&'static str],
    pub containers: &'static [Container],
}

impl EncoderDef {
    pub fn is_hardware(&self) -> bool {
        self.vendor != Vendor::Cpu
    }
}

const BOTH: &[Container] = &[Container::Mp4, Container::Mkv];
const MKV_ONLY: &[Container] = &[Container::Mkv];

const fn crf(min: f64, max: f64, at_best: f64, at_smallest: f64, integer: bool) -> Option<QualityScale> {
    Some(QualityScale { kind: QualityKind::Crf, min, max, lower_is_better: true, at_best, at_smallest, integer })
}
const fn scale(kind: QualityKind, min: f64, max: f64, at_best: f64, at_smallest: f64) -> Option<QualityScale> {
    Some(QualityScale { kind, min, max, lower_is_better: at_best < at_smallest, at_best, at_smallest, integer: true })
}

pub static ENCODERS: &[EncoderDef] = &[
    // ---- CPU ----
    EncoderDef {
        name: "libx264",
        codec: VideoCodec::H264,
        vendor: Vendor::Cpu,
        label: "x264",
        quality: crf(0.0, 51.0, 16.0, 30.0, false),
        speed: Some(("-preset", ["veryfast", "medium", "slow"])),
        two_pass: true,
        bitrate: true,
        lossless: Lossless::Args(&["-qp", "0"]),
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "libx265",
        codec: VideoCodec::Hevc,
        vendor: Vendor::Cpu,
        label: "x265",
        quality: crf(0.0, 51.0, 18.0, 32.0, false),
        speed: Some(("-preset", ["fast", "medium", "slow"])),
        two_pass: true,
        bitrate: true,
        lossless: Lossless::Priv("lossless", "1"),
        privp: PrivParams::X265,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "libsvtav1",
        codec: VideoCodec::Av1,
        vendor: Vendor::Cpu,
        label: "SVT-AV1",
        quality: crf(1.0, 63.0, 22.0, 46.0, true),
        speed: Some(("-preset", ["10", "7", "5"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::Priv("lossless", "1"),
        privp: PrivParams::Svt,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "libaom-av1",
        codec: VideoCodec::Av1,
        vendor: Vendor::Cpu,
        label: "libaom",
        quality: crf(0.0, 63.0, 22.0, 46.0, true),
        speed: Some(("-cpu-used", ["6", "4", "3"])),
        two_pass: true,
        bitrate: true,
        lossless: Lossless::Args(&["-lossless", "1"]),
        privp: PrivParams::Aom,
        fixed: &["-row-mt", "1"],
        containers: BOTH,
    },
    EncoderDef {
        name: "ffv1",
        codec: VideoCodec::Ffv1,
        vendor: Vendor::Cpu,
        label: "FFV1",
        quality: None,
        speed: None,
        two_pass: false,
        bitrate: false,
        lossless: Lossless::Intrinsic,
        privp: PrivParams::None,
        fixed: &["-level", "3", "-g", "1", "-coder", "1", "-context", "1", "-slicecrc", "1"],
        containers: MKV_ONLY,
    },
    // ---- Apple ----
    EncoderDef {
        name: "h264_videotoolbox",
        codec: VideoCodec::H264,
        vendor: Vendor::Apple,
        label: "Apple VideoToolbox",
        quality: scale(QualityKind::Qv, 1.0, 100.0, 75.0, 45.0),
        speed: None,
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &["-allow_sw", "0"],
        containers: BOTH,
    },
    EncoderDef {
        name: "hevc_videotoolbox",
        codec: VideoCodec::Hevc,
        vendor: Vendor::Apple,
        label: "Apple VideoToolbox",
        quality: scale(QualityKind::Qv, 1.0, 100.0, 70.0, 40.0),
        speed: None,
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &["-allow_sw", "0"],
        containers: BOTH,
    },
    // ---- NVIDIA ----
    EncoderDef {
        name: "h264_nvenc",
        codec: VideoCodec::H264,
        vendor: Vendor::Nvidia,
        label: "NVIDIA NVENC",
        quality: scale(QualityKind::Cq, 1.0, 51.0, 19.0, 33.0),
        speed: Some(("-preset", ["p3", "p5", "p7"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::Args(&["-tune", "lossless"]),
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "hevc_nvenc",
        codec: VideoCodec::Hevc,
        vendor: Vendor::Nvidia,
        label: "NVIDIA NVENC",
        quality: scale(QualityKind::Cq, 1.0, 51.0, 21.0, 35.0),
        speed: Some(("-preset", ["p3", "p5", "p7"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::Args(&["-tune", "lossless"]),
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "av1_nvenc",
        codec: VideoCodec::Av1,
        vendor: Vendor::Nvidia,
        label: "NVIDIA NVENC",
        quality: scale(QualityKind::Cq, 1.0, 63.0, 24.0, 45.0),
        speed: Some(("-preset", ["p3", "p5", "p7"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    // ---- Intel ----
    EncoderDef {
        name: "h264_qsv",
        codec: VideoCodec::H264,
        vendor: Vendor::Intel,
        label: "Intel Quick Sync",
        quality: scale(QualityKind::Icq, 1.0, 51.0, 18.0, 32.0),
        speed: Some(("-preset", ["veryfast", "medium", "veryslow"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "hevc_qsv",
        codec: VideoCodec::Hevc,
        vendor: Vendor::Intel,
        label: "Intel Quick Sync",
        quality: scale(QualityKind::Icq, 1.0, 51.0, 20.0, 34.0),
        speed: Some(("-preset", ["veryfast", "medium", "veryslow"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "av1_qsv",
        codec: VideoCodec::Av1,
        vendor: Vendor::Intel,
        label: "Intel Quick Sync",
        quality: scale(QualityKind::Icq, 1.0, 51.0, 24.0, 44.0),
        speed: Some(("-preset", ["veryfast", "medium", "veryslow"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    // ---- AMD ----
    EncoderDef {
        name: "h264_amf",
        codec: VideoCodec::H264,
        vendor: Vendor::Amd,
        label: "AMD AMF",
        quality: scale(QualityKind::Qp, 1.0, 51.0, 18.0, 32.0),
        speed: Some(("-quality", ["speed", "balanced", "quality"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "hevc_amf",
        codec: VideoCodec::Hevc,
        vendor: Vendor::Amd,
        label: "AMD AMF",
        quality: scale(QualityKind::Qp, 1.0, 51.0, 20.0, 34.0),
        speed: Some(("-quality", ["speed", "balanced", "quality"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
    EncoderDef {
        name: "av1_amf",
        codec: VideoCodec::Av1,
        vendor: Vendor::Amd,
        label: "AMD AMF",
        quality: scale(QualityKind::Qp, 1.0, 255.0, 60.0, 180.0),
        speed: Some(("-quality", ["speed", "balanced", "quality"])),
        two_pass: false,
        bitrate: true,
        lossless: Lossless::No,
        privp: PrivParams::None,
        fixed: &[],
        containers: BOTH,
    },
];

pub fn get(name: &str) -> Option<&'static EncoderDef> {
    ENCODERS.iter().find(|e| e.name == name)
}

pub fn for_codec(codec: VideoCodec) -> impl Iterator<Item = &'static EncoderDef> {
    ENCODERS.iter().filter(move |e| e.codec == codec)
}

/// Compression level (0 best … 100 smallest) → native quality value.
pub fn level_to_native(q: &QualityScale, level: u8) -> f64 {
    let t = f64::from(level.min(100)) / 100.0;
    let v = (q.at_best + (q.at_smallest - q.at_best) * t).clamp(q.min, q.max);
    if q.integer { v.round() } else { (v * 10.0).round() / 10.0 }
}

/// Native value → nearest compression level (for showing explicit values on the slider).
pub fn native_to_level(q: &QualityScale, value: f64) -> u8 {
    let t = ((value - q.at_best) / (q.at_smallest - q.at_best)).clamp(0.0, 1.0);
    (t * 100.0).round() as u8
}

/// Options that set the native quality value.
pub fn quality_args(def: &EncoderDef, value: f64) -> Vec<String> {
    let Some(q) = def.quality else { return Vec::new() };
    let v = fmt_value(&q, value);
    let args: Vec<&str> = match def.vendor {
        Vendor::Cpu if def.name == "libaom-av1" => vec!["-crf", &v, "-b:v", "0"],
        Vendor::Cpu => vec!["-crf", &v],
        Vendor::Apple => vec!["-q:v", &v],
        Vendor::Nvidia => vec!["-rc", "vbr", "-cq", &v, "-b:v", "0"],
        Vendor::Intel => vec!["-global_quality", &v],
        Vendor::Amd => vec!["-rc", "cqp", "-qp_i", &v, "-qp_p", &v, "-qp_b", &v],
    };
    args.into_iter().map(String::from).collect()
}

pub fn speed_args(def: &EncoderDef, speed: Speed) -> Vec<String> {
    let Some((opt, values)) = def.speed else { return Vec::new() };
    let v = match speed {
        Speed::Fast => values[0],
        Speed::Balanced | Speed::Gpu => values[1],
        Speed::Slow => values[2],
    };
    vec![opt.to_string(), v.to_string()]
}

pub fn bitrate_args(def: &EncoderDef, kbps: u32) -> Vec<String> {
    let rate = format!("{kbps}k");
    let args: Vec<&str> = match def.vendor {
        Vendor::Nvidia => vec!["-rc", "vbr", "-b:v", &rate],
        Vendor::Amd => vec!["-rc", "vbr_peak", "-b:v", &rate],
        _ => vec!["-b:v", &rate],
    };
    args.into_iter().map(String::from).collect()
}

/// SVT-AV1 `lp` is a parallelism level 1–6, not a thread count. It is always set
/// explicitly: with `lp` left on auto, SVT-AV1 4.1 sizes its pool for the whole
/// machine and several concurrent encoders occasionally deadlock.
pub fn svt_lp_for_threads(threads: u32, logical_cpus: u32) -> u32 {
    match threads.min(logical_cpus.max(1)) {
        0 | 1 => 1,
        2 => 2,
        3..=4 => 3,
        5..=8 => 4,
        9..=16 => 5,
        _ => 6,
    }
}

/// How the video bitrate is controlled for one encode.
#[derive(Debug, Clone, PartialEq)]
pub enum RateArgs {
    Quality(f64),
    Bitrate(u32),
    Lossless,
}

/// Everything needed to build the video encoder options of one ffmpeg run.
#[derive(Debug, Clone)]
pub struct EncArgs {
    pub rate: RateArgs,
    pub speed: Speed,
    /// Thread budget for this job (None = encoder default).
    pub threads: Option<u32>,
    pub logical_cpus: u32,
    /// Two-pass: (pass number, stats file name relative to the job work dir).
    pub pass: Option<(u8, String)>,
}

/// `-c:v <encoder>` plus speed, rate control, fixed options, threads, pass and private params.
/// Used by real jobs *and* by the lossless self-test, so what is verified is what runs.
pub fn video_args(def: &EncoderDef, a: &EncArgs) -> Vec<String> {
    let mut args: Vec<String> = vec!["-c:v".into(), def.name.into()];
    let mut privp: Vec<(&str, String)> = Vec::new();
    if def.privp == PrivParams::X265 {
        privp.push(("log-level", "warning".into()));
    }
    args.extend(speed_args(def, a.speed));
    match &a.rate {
        RateArgs::Quality(v) => args.extend(quality_args(def, *v)),
        RateArgs::Bitrate(kbps) => args.extend(bitrate_args(def, *kbps)),
        RateArgs::Lossless => match def.lossless {
            Lossless::Args(xs) => args.extend(xs.iter().map(|s| s.to_string())),
            Lossless::Priv(k, v) => privp.push((k, v.to_string())),
            Lossless::Intrinsic | Lossless::No => {}
        },
    }
    args.extend(def.fixed.iter().map(|s| s.to_string()));
    if let Some(n) = a.threads.filter(|n| *n > 0) {
        match def.name {
            "libx264" | "libaom-av1" => args.extend(["-threads".to_string(), n.to_string()]),
            "libx265" => privp.push(("pools", n.to_string())),
            "libsvtav1" => privp.push(("lp", svt_lp_for_threads(n, a.logical_cpus).to_string())),
            "ffv1" if n > 1 => {
                let slices = if n >= 8 { "16" } else { "4" };
                args.extend(["-threads".to_string(), n.to_string(), "-slices".to_string(), slices.to_string()]);
            }
            _ => {}
        }
    }
    if let Some((pass, stats)) = &a.pass {
        if def.privp == PrivParams::X265 {
            privp.push(("pass", pass.to_string()));
            privp.push(("stats", format!("{stats}.log")));
        } else {
            args.extend(["-pass".to_string(), pass.to_string(), "-passlogfile".to_string(), stats.clone()]);
        }
    }
    if let (Some(opt), false) = (def.privp.option(), privp.is_empty()) {
        args.push(opt.to_string());
        args.push(privp.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(":"));
    }
    args
}

/// Format a native value the way the encoder expects it.
pub fn fmt_value(q: &QualityScale, value: f64) -> String {
    if q.integer || value.fract() == 0.0 { format!("{}", value.round() as i64) } else { format!("{value:.1}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(name: &str) -> QualityScale {
        get(name).unwrap().quality.unwrap()
    }

    fn enc(rate: RateArgs, speed: Speed, threads: Option<u32>) -> EncArgs {
        EncArgs { rate, speed, threads, logical_cpus: 10, pass: None }
    }

    #[test]
    fn x264_quality_args() {
        let a = video_args(get("libx264").unwrap(), &enc(RateArgs::Quality(23.0), Speed::Balanced, Some(10)));
        assert_eq!(a, ["-c:v", "libx264", "-preset", "medium", "-crf", "23", "-threads", "10"]);
    }

    #[test]
    fn x265_private_params_are_merged_into_one_option() {
        let a = video_args(get("libx265").unwrap(), &enc(RateArgs::Lossless, Speed::Fast, Some(4)));
        assert_eq!(a, ["-c:v", "libx265", "-preset", "fast", "-x265-params", "log-level=warning:lossless=1:pools=4"]);
    }

    #[test]
    fn svt_bitrate_with_limited_parallelism() {
        let a = video_args(get("libsvtav1").unwrap(), &enc(RateArgs::Bitrate(800), Speed::Balanced, Some(2)));
        assert_eq!(a, ["-c:v", "libsvtav1", "-preset", "7", "-b:v", "800k", "-svtav1-params", "lp=2"]);
        let all = video_args(get("libsvtav1").unwrap(), &enc(RateArgs::Quality(34.0), Speed::Slow, Some(10)));
        assert_eq!(all, ["-c:v", "libsvtav1", "-preset", "5", "-crf", "34", "-svtav1-params", "lp=5"], "parallelism is always explicit");
    }

    #[test]
    fn videotoolbox_never_allows_silent_software_fallback() {
        let a = video_args(get("h264_videotoolbox").unwrap(), &enc(RateArgs::Quality(60.0), Speed::Gpu, Some(10)));
        assert_eq!(a, ["-c:v", "h264_videotoolbox", "-q:v", "60", "-allow_sw", "0"]);
    }

    #[test]
    fn ffv1_uses_slices_for_threads() {
        let a = video_args(get("ffv1").unwrap(), &enc(RateArgs::Lossless, Speed::Balanced, Some(8)));
        assert_eq!(
            a,
            ["-c:v", "ffv1", "-level", "3", "-g", "1", "-coder", "1", "-context", "1", "-slicecrc", "1", "-threads", "8", "-slices", "16"]
        );
    }

    #[test]
    fn two_pass_args_per_encoder() {
        let mut e = enc(RateArgs::Bitrate(2000), Speed::Balanced, Some(10));
        e.pass = Some((1, "fe2pass".into()));
        let x264 = video_args(get("libx264").unwrap(), &e);
        assert!(x264.windows(4).any(|w| w == ["-pass", "1", "-passlogfile", "fe2pass"]), "{x264:?}");
        e.pass = Some((2, "fe2pass".into()));
        let x265 = video_args(get("libx265").unwrap(), &e);
        assert_eq!(x265.last().unwrap(), "log-level=warning:pools=10:pass=2:stats=fe2pass.log");
    }

    #[test]
    fn x264_lossless_is_qp_zero_not_crf_zero() {
        let a = video_args(get("libx264").unwrap(), &enc(RateArgs::Lossless, Speed::Balanced, None));
        assert_eq!(a, ["-c:v", "libx264", "-preset", "medium", "-qp", "0"]);
    }

    #[test]
    fn lookup_by_name() {
        assert_eq!(get("libsvtav1").unwrap().codec, VideoCodec::Av1);
        assert!(get("h264_videotoolbox").unwrap().is_hardware());
        assert!(get("nope").is_none());
    }

    #[test]
    fn x264_level_mapping_spans_its_own_crf_range() {
        let x = q("libx264");
        assert_eq!(level_to_native(&x, 0), 16.0);
        assert_eq!(level_to_native(&x, 50), 23.0);
        assert_eq!(level_to_native(&x, 100), 30.0);
    }

    #[test]
    fn same_level_means_different_numbers_per_encoder() {
        assert_eq!(level_to_native(&q("libsvtav1"), 50), 34.0);
        assert_eq!(level_to_native(&q("libx265"), 50), 25.0);
        assert_ne!(level_to_native(&q("libx264"), 50), level_to_native(&q("libsvtav1"), 50));
    }

    #[test]
    fn integer_encoders_round_and_fractional_ones_keep_one_decimal() {
        assert_eq!(level_to_native(&q("libsvtav1"), 33), 30.0);
        assert_eq!(level_to_native(&q("libx264"), 33), 20.6);
    }

    #[test]
    fn videotoolbox_higher_value_means_better_quality() {
        let vt = q("h264_videotoolbox");
        assert!(!vt.lower_is_better);
        assert_eq!(level_to_native(&vt, 0), 75.0);
        assert_eq!(level_to_native(&vt, 100), 45.0);
    }

    #[test]
    fn native_to_level_inverts_the_mapping() {
        for name in ["libx264", "libsvtav1", "h264_videotoolbox", "hevc_nvenc"] {
            let s = q(name);
            // Integer scales quantise: half a native step, expressed in levels.
            let step = if s.integer { 100.0 / (s.at_smallest - s.at_best).abs() } else { 1.0 };
            let tolerance = step / 2.0 + 1.0;
            for level in [0u8, 25, 50, 75, 100] {
                let back = native_to_level(&s, level_to_native(&s, level));
                assert!(f64::from((back as i32 - level as i32).unsigned_abs()) <= tolerance, "{name} {level} -> {back}");
            }
        }
        assert_eq!(native_to_level(&q("libx264"), 5.0), 0, "clamped below");
        assert_eq!(native_to_level(&q("libx264"), 45.0), 100, "clamped above");
    }

    #[test]
    fn quality_args_use_each_encoders_own_option() {
        assert_eq!(quality_args(get("libx264").unwrap(), 23.0), ["-crf", "23"]);
        assert_eq!(quality_args(get("libsvtav1").unwrap(), 34.0), ["-crf", "34"]);
        assert_eq!(quality_args(get("h264_videotoolbox").unwrap(), 60.0), ["-q:v", "60"]);
        assert_eq!(quality_args(get("hevc_nvenc").unwrap(), 28.0), ["-rc", "vbr", "-cq", "28", "-b:v", "0"]);
        assert_eq!(quality_args(get("h264_qsv").unwrap(), 25.0), ["-global_quality", "25"]);
        assert_eq!(quality_args(get("h264_amf").unwrap(), 22.0), ["-rc", "cqp", "-qp_i", "22", "-qp_p", "22", "-qp_b", "22"]);
        assert_eq!(quality_args(get("libaom-av1").unwrap(), 30.0), ["-crf", "30", "-b:v", "0"]);
    }

    #[test]
    fn speed_args_map_to_native_presets() {
        assert_eq!(speed_args(get("libx264").unwrap(), Speed::Slow), ["-preset", "slow"]);
        assert_eq!(speed_args(get("libsvtav1").unwrap(), Speed::Fast), ["-preset", "10"]);
        assert_eq!(speed_args(get("libaom-av1").unwrap(), Speed::Balanced), ["-cpu-used", "4"]);
        assert!(speed_args(get("ffv1").unwrap(), Speed::Balanced).is_empty());
        assert!(speed_args(get("h264_videotoolbox").unwrap(), Speed::Gpu).is_empty());
        assert_eq!(speed_args(get("hevc_nvenc").unwrap(), Speed::Gpu), ["-preset", "p5"], "GPU uses the balanced preset");
    }

    #[test]
    fn bitrate_args_per_vendor() {
        assert_eq!(bitrate_args(get("libx264").unwrap(), 2500), ["-b:v", "2500k"]);
        assert_eq!(bitrate_args(get("hevc_nvenc").unwrap(), 4000), ["-rc", "vbr", "-b:v", "4000k"]);
        assert_eq!(bitrate_args(get("h264_amf").unwrap(), 3000), ["-rc", "vbr_peak", "-b:v", "3000k"]);
    }

    #[test]
    fn svt_parallelism_level_from_thread_budget() {
        // Never left on auto: SVT-AV1 4.1 auto-sized thread pools occasionally deadlock when
        // several encoders oversubscribe the CPU (reproduced: 2/64 hung at 16 parallel, 0/64 with lp set).
        assert_eq!(svt_lp_for_threads(10, 10), 5);
        assert_eq!(svt_lp_for_threads(32, 32), 6);
        assert_eq!(svt_lp_for_threads(1, 10), 1);
        assert_eq!(svt_lp_for_threads(2, 10), 2);
        assert_eq!(svt_lp_for_threads(4, 10), 3);
        assert_eq!(svt_lp_for_threads(5, 16), 4);
        assert_eq!(svt_lp_for_threads(12, 32), 5);
    }

    #[test]
    fn hardware_encoders_never_offer_two_pass_and_ffv1_is_mkv_only() {
        assert!(ENCODERS.iter().filter(|e| e.is_hardware()).all(|e| !e.two_pass));
        assert_eq!(get("ffv1").unwrap().containers, &[Container::Mkv]);
        assert_eq!(get("ffv1").unwrap().lossless, Lossless::Intrinsic);
    }
}
