//! Trial encodes ("试压") and frame extraction against the real backend.

mod common;

use common::*;
use fe_core::settings::*;
use fe_core::trial::{self, TrialError, TrialRequest};
use tokio_util::sync::CancellationToken;

fn req(codec: VideoCodec, mode: VideoMode) -> TrialRequest {
    let mut s = JobSettings::default();
    s.video.codec = codec;
    s.video.mode = mode;
    TrialRequest { input: media("long_720p.mp4"), settings: s, start: 5.0, duration: 3.0, metrics: true }
}

#[tokio::test(flavor = "multi_thread")]
async fn trial_encodes_a_segment_and_reports_size_time_and_metrics() {
    let caps = capabilities().await;
    let work = tempfile::tempdir().unwrap();
    let r = trial::run_trial(&ffmpeg(), &caps, &req(VideoCodec::Av1, VideoMode::Lossy), work.path(), &CancellationToken::new()).await.unwrap();
    assert!(r.bytes > 0 && r.output.exists());
    assert!((r.segment_secs - 3.0).abs() < 0.2, "{}", r.segment_secs);
    assert!(r.encode_secs > 0.0);
    let est = r.est_full_bytes.unwrap();
    assert!(est > r.bytes * 5 && est < r.bytes * 8, "20 s / 3 s extrapolation: {est} vs {}", r.bytes);
    assert!(r.psnr.unwrap() > 25.0, "psnr {:?}", r.psnr);
    assert!(r.ssim.unwrap() > 0.8 && r.ssim.unwrap() <= 1.0, "ssim {:?}", r.ssim);
    if caps.has_filter("libvmaf") {
        let v = r.vmaf.unwrap();
        assert!((0.0..=100.0).contains(&v), "vmaf {v}");
    }
    assert_eq!(r.summary.encoder.as_deref(), Some("libsvtav1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn lossless_trial_reports_size_but_no_similarity_scores() {
    let caps = capabilities().await;
    let work = tempfile::tempdir().unwrap();
    let r = trial::run_trial(&ffmpeg(), &caps, &req(VideoCodec::H264, VideoMode::Lossless), work.path(), &CancellationToken::new()).await.unwrap();
    assert!(r.bytes > 0);
    assert_eq!((r.psnr, r.ssim, r.vmaf), (None, None, None), "lossless is verified by hashes, not scores");
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_mode_has_nothing_to_trial() {
    let caps = capabilities().await;
    let work = tempfile::tempdir().unwrap();
    let e = trial::run_trial(&ffmpeg(), &caps, &req(VideoCodec::H264, VideoMode::Copy), work.path(), &CancellationToken::new()).await.unwrap_err();
    assert!(matches!(e, TrialError::Rejected(ref n) if n.code == "trialCopyMode"), "{e:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_can_be_extracted_for_side_by_side_compare() {
    let work = tempfile::tempdir().unwrap();
    let out = work.path().join("预览 frame.png");
    trial::extract_frame(&ffmpeg().ffmpeg, &media("rotated.mp4"), 1.0, 480, &out).await.unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
}
