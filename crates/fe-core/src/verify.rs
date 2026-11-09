//! Output verification. A job is only "done" when the file it produced has
//! been probed and checked against the plan: actual codec (not the extension),
//! dimensions, pixel format, colour tags, HDR metadata, frame count, duration,
//! audio/subtitle tracks, bit-exact copies (packet hashes), a full decode, and
//! for lossless jobs a frame-by-frame pixel hash comparison with the source.

use crate::job::{Check, VerifyReport};
use crate::media::{self, MediaInfo, ff_path};
use crate::plan::{Expected, FrameCheck};
use crate::process::{self, Outcome, RunOptions};
use crate::settings::Container;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub struct VerifyCtx<'a> {
    pub ffmpeg: &'a Path,
    pub ffprobe: &'a Path,
    pub source: &'a MediaInfo,
    /// Inputs by index for stream-copy hash checks (index 0 = main source).
    pub inputs: &'a [PathBuf],
    pub output: &'a Path,
    pub work_dir: &'a Path,
    pub log_path: Option<&'a Path>,
    pub cancel: &'a CancellationToken,
}

#[derive(Debug, PartialEq)]
pub enum VerifyAbort {
    Cancelled,
}

fn check(name: &str, ok: bool, expected: impl ToString, actual: impl ToString) -> Check {
    Check { name: name.into(), ok, expected: expected.to_string(), actual: actual.to_string() }
}

fn opt(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "—".into())
}

/// Packet-level SHA-256 of one stream (stream copy keeps packets bit-exact).
async fn packet_hash(ff: &Path, input: &Path, stream: u32, decoded: bool) -> Option<String> {
    let map = format!("0:{stream}");
    let src = ff_path(input);
    let mut args = vec!["-hide_banner", "-nostdin", "-v", "error", "-i", &src, "-map", &map];
    if !decoded {
        args.extend(["-c", "copy"]);
    }
    args.extend(["-f", "streamhash", "-hash", "sha256", "-"]);
    let out = process::run_capture(ff, &args, Duration::from_secs(7200)).await.ok()?;
    if !out.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.split("SHA256=").nth(1).map(|h| h.trim().to_string()))
}

/// Run a full-length ffmpeg pass (decode or framemd5) with progress and cancellation.
async fn run_long(
    ctx: &VerifyCtx<'_>,
    args: Vec<String>,
    duration: Option<f64>,
    base: f64,
    span: f64,
    on_progress: &mut impl FnMut(f64),
) -> Result<Outcome, VerifyAbort> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-v", "error", "-progress", "pipe:1", "-nostats", "-stats_period", "0.5"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.extend(args);
    let opts = RunOptions { program: ctx.ffmpeg, args: &a, cwd: Some(ctx.work_dir), log_path: ctx.log_path, stall_limit: None };
    let out = process::run_ffmpeg(opts, ctx.cancel, |_| {}, |p| {
        if let (Some(us), Some(d)) = (p.out_time_us, duration.filter(|d| *d > 0.0)) {
            on_progress(base + span * (us as f64 / 1e6 / d).clamp(0.0, 1.0));
        }
    })
    .await
    .unwrap_or(Outcome::Failed { code: None, signal: None, tail: vec!["could not start ffmpeg".into()] });
    if out == Outcome::Cancelled {
        return Err(VerifyAbort::Cancelled);
    }
    Ok(out)
}

fn read_hash_column(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string()))
        .filter(|h| !h.is_empty())
        .collect()
}

pub async fn verify(expected: &Expected, ctx: &VerifyCtx<'_>, mut on_progress: impl FnMut(f64)) -> Result<VerifyReport, VerifyAbort> {
    let mut checks: Vec<Check> = Vec::new();
    let done = |checks: Vec<Check>| Ok(VerifyReport { ok: checks.iter().all(|c| c.ok), checks });

    // 1. The file exists and is readable media.
    let size = std::fs::metadata(ctx.output).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        checks.push(check("readable", false, "非空文件", "0 B / 不存在"));
        return done(checks);
    }
    let out = match media::probe(ctx.ffprobe, ctx.output).await {
        Ok(m) => m,
        Err(e) => {
            checks.push(check("readable", false, "可识别的媒体文件", e.to_string()));
            return done(checks);
        }
    };
    checks.push(check("readable", true, "可识别的媒体文件", &out.format_name));
    if let Some(c) = expected.container {
        let fmt_ok = match c {
            Container::Mp4 => out.is_mp4_family(),
            Container::Mkv => out.format_name.contains("matroska"),
        };
        checks.push(check("container", fmt_ok, c.ext(), &out.format_name));
    }
    on_progress(0.05);

    // 2. Video stream properties.
    if let Some(ev) = &expected.video {
        match out.main_video() {
            None => checks.push(check("videoCodec", false, &ev.codec, "无视频流")),
            Some(v) => {
                checks.push(check("videoCodec", v.codec == ev.codec, &ev.codec, &v.codec));
                checks.push(check("dimensions", v.width == ev.width && v.height == ev.height, format!("{}×{}", ev.width, ev.height), format!("{}×{}", v.width, v.height)));
                if let Some(p) = &ev.pix_fmt {
                    // Range has its own check (colorRange): FFmpeg names full-range 8-bit
                    // H.264/HEVC "yuvj…" whatever pixel format the encoder was given.
                    let same = v.pix_fmt.as_deref().map(media::plain_pix_fmt) == Some(media::plain_pix_fmt(p));
                    checks.push(check("pixFmt", same, p, opt(&v.pix_fmt)));
                }
                checks.push(check("rotation", v.rotation == ev.rotation, ev.rotation, v.rotation));
                for (name, want, got) in [
                    ("colorPrimaries", &ev.color.primaries, &v.color_primaries),
                    ("colorTransfer", &ev.color.transfer, &v.color_transfer),
                    ("colorSpace", &ev.color.space, &v.color_space),
                ] {
                    if want.is_some() {
                        checks.push(check(name, want == got, opt(want), opt(got)));
                    }
                }
                match ev.color.range.as_deref() {
                    Some("pc") => checks.push(check("colorRange", v.full_range(), "pc", opt(&v.color_range))),
                    Some(r) => {
                        let ok = !v.full_range();
                        checks.push(check("colorRange", ok, r, opt(&v.color_range)));
                    }
                    None => {}
                }
                if ev.hdr_metadata {
                    checks.push(check("hdrMetadata", v.mastering_display.is_some(), "mastering display", if v.mastering_display.is_some() { "present" } else { "missing" }));
                }
                let src_idx = expected.source_video_index.unwrap_or(0);
                match ev.frames {
                    FrameCheck::SameAsSource => {
                        let (a, b) = tokio::join!(
                            media::count_packets(ctx.ffprobe, &ctx.source.path, src_idx),
                            media::count_packets(ctx.ffprobe, ctx.output, v.index)
                        );
                        match (a, b) {
                            (Ok(a), Ok(b)) => {
                                let tol = (a / 1000).max(1);
                                checks.push(check("frameCount", a.abs_diff(b) <= tol, a, b));
                            }
                            (a, b) => checks.push(check("frameCount", false, format!("{a:?}"), format!("{b:?}"))),
                        }
                    }
                    FrameCheck::Approx(n) => match media::count_packets(ctx.ffprobe, ctx.output, v.index).await {
                        Ok(b) => {
                            let tol = (n / 50).max(2);
                            checks.push(check("frameCount", n.abs_diff(b) <= tol, format!("≈{n}"), b));
                        }
                        Err(e) => checks.push(check("frameCount", false, format!("≈{n}"), e.to_string())),
                    },
                    FrameCheck::Skip => {}
                }
            }
        }
    }
    on_progress(0.12);

    // 3. Duration.
    if let (Some(d), Some(od)) = (expected.duration, out.duration) {
        let tol = (d * 0.01).max(0.6);
        checks.push(check("duration", (od - d).abs() <= tol, format!("{d:.2}s"), format!("{od:.2}s")));
    }

    // 4. Tracks.
    checks.push(check("audioTracks", out.audio.len() == expected.audio.len(), expected.audio.len(), out.audio.len()));
    for (i, (ea, oa)) in expected.audio.iter().zip(out.audio.iter()).enumerate() {
        checks.push(check(&format!("audioCodec:{}", i + 1), ea.codec == oa.codec, &ea.codec, &oa.codec));
    }
    checks.push(check("subtitleTracks", out.subtitles.len() == expected.subtitles, expected.subtitles, out.subtitles.len()));
    on_progress(0.15);

    // 5. Bit-exact stream copies.
    for sc in &expected.hash_streams {
        let Some(input) = ctx.inputs.get(sc.input as usize) else { continue };
        if ctx.cancel.is_cancelled() {
            return Err(VerifyAbort::Cancelled);
        }
        let (a, b) = tokio::join!(packet_hash(ctx.ffmpeg, input, sc.source_index, false), packet_hash(ctx.ffmpeg, ctx.output, sc.output_index, false));
        let name = format!("streamCopy:{}", sc.output_index);
        if a.is_some() && a == b {
            checks.push(check(&name, true, "packet sha256", "identical"));
            continue;
        }
        // Remuxing can legitimately rewrap packets (e.g. ADTS → MP4); then decoded data must match.
        let (da, db) = tokio::join!(packet_hash(ctx.ffmpeg, input, sc.source_index, true), packet_hash(ctx.ffmpeg, ctx.output, sc.output_index, true));
        let ok = da.is_some() && da == db;
        checks.push(check(&name, ok, "packet or decoded sha256", if ok { "decoded identical" } else { "mismatch" }));
    }
    on_progress(0.3);

    // 6. Lossless: every decoded frame must hash identically to the source.
    let duration = expected.duration;
    if let Some(src_opts) = &expected.lossless_source_opts {
        let src_hashes = ctx.work_dir.join("source.framemd5");
        let out_hashes = ctx.work_dir.join("output.framemd5");
        let mut a: Vec<String> = src_opts.clone();
        let src_idx = expected.source_video_index.unwrap_or(0);
        a.extend(["-i".into(), ff_path(&ctx.source.path), "-map".into(), format!("0:{src_idx}"), "-f".into(), "framemd5".into(), "-y".into(), ff_path(&src_hashes)]);
        let o1 = run_long(ctx, a, duration, 0.3, 0.35, &mut on_progress).await?;
        let b: Vec<String> = vec!["-i".into(), ff_path(ctx.output), "-map".into(), "0:v:0".into(), "-f".into(), "framemd5".into(), "-y".into(), ff_path(&out_hashes)];
        let o2 = run_long(ctx, b, duration, 0.65, 0.3, &mut on_progress).await?;
        let (h1, h2) = (read_hash_column(&src_hashes), read_hash_column(&out_hashes));
        let ok = o1 == Outcome::Success && o2 == Outcome::Success && !h1.is_empty() && h1 == h2;
        let actual = if ok {
            format!("{} 帧全部一致", h1.len())
        } else if h1.len() != h2.len() {
            format!("帧数不同：{} vs {}", h1.len(), h2.len())
        } else {
            let first = h1.iter().zip(&h2).position(|(x, y)| x != y).unwrap_or(0);
            format!("第 {} 帧起不一致", first + 1)
        };
        checks.push(check("lossless", ok, "逐帧像素哈希一致", actual));
        let _ = std::fs::remove_file(&src_hashes);
        let _ = std::fs::remove_file(&out_hashes);
        // Audio still needs a decode check.
        if !expected.audio.is_empty() {
            let args = vec!["-xerror".into(), "-i".into(), ff_path(ctx.output), "-map".into(), "0:a?".into(), "-f".into(), "null".into(), "-".into()];
            let o = run_long(ctx, args, duration, 0.95, 0.05, &mut on_progress).await?;
            checks.push(decode_check(o));
        }
    } else {
        // 7. Full decode of every stream: an unplayable file must never count as done.
        let fully_hashed = expected.video.as_ref().map(|v| v.frames == FrameCheck::SameAsSource).unwrap_or(false)
            && !expected.hash_streams.is_empty()
            && expected.hash_streams.iter().any(|h| h.output_index == 0)
            && expected.hash_streams.len() == 1 + expected.audio.len()
            && checks.iter().filter(|c| c.name.starts_with("streamCopy")).all(|c| c.ok && c.actual == "identical");
        let decodable = expected.video.is_some() || !expected.audio.is_empty();
        if !fully_hashed && decodable {
            let args = vec!["-xerror".into(), "-i".into(), ff_path(ctx.output), "-map".into(), "0:v?".into(), "-map".into(), "0:a?".into(), "-f".into(), "null".into(), "-".into()];
            let o = run_long(ctx, args, duration, 0.3, 0.7, &mut on_progress).await?;
            checks.push(decode_check(o));
        }
    }
    on_progress(1.0);
    done(checks)
}

fn decode_check(o: Outcome) -> Check {
    match o {
        Outcome::Success => check("decode", true, "完整解码无错误", "ok"),
        Outcome::Failed { tail, .. } | Outcome::Stalled { tail } => {
            check("decode", false, "完整解码无错误", tail.last().cloned().unwrap_or_else(|| "decode failed".into()))
        }
        Outcome::Cancelled => check("decode", false, "完整解码无错误", "cancelled"),
    }
}
