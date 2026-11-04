//! Capability detection against the real backend on this machine.

mod common;

use common::*;
use fe_core::caps;
use fe_core::encoders;

#[tokio::test(flavor = "multi_thread")]
async fn detects_versions_encoders_and_verified_lossless_pairs() {
    let c = capabilities().await;
    assert!(c.version_num.is_some(), "version parsed: {}", c.version);
    assert!(!c.license.is_empty());
    for name in ["libx264", "libx265", "libsvtav1", "ffv1"] {
        assert!(c.usable(name), "{name} usable");
    }
    for pair in [("libx264", "yuv420p"), ("libx265", "yuv420p"), ("libsvtav1", "yuv420p"), ("ffv1", "yuv420p"), ("libsvtav1", "yuv420p10le")] {
        assert_eq!(c.lossless_ok(pair.0, pair.1), Some(true), "{pair:?} passes the pixel-exact self-test");
    }
    assert!(c.cpu.logical >= 1 && c.memory_mb > 0);
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn videotoolbox_is_verified_by_a_real_encode_and_has_no_av1_encoder() {
    let c = capabilities().await;
    if c.encoder("h264_videotoolbox").is_some() {
        assert!(c.usable("h264_videotoolbox"));
        assert!(c.hw_decode.iter().any(|h| h == "videotoolbox:h264"), "{:?}", c.hw_decode);
    }
    assert!(encoders::for_codec(fe_core::settings::VideoCodec::Av1).filter(|d| d.is_hardware()).all(|d| !c.usable(d.name)));
}

#[tokio::test(flavor = "multi_thread")]
async fn self_test_catches_implicit_chroma_conversion() {
    let ff = ffmpeg();
    let work = tempfile::tempdir().unwrap();
    // SVT-AV1 cannot take 4:4:4; FFmpeg would silently convert to 4:2:0. The test must fail.
    let check = caps::lossless_self_test(&ff.ffmpeg, encoders::get("libsvtav1").unwrap(), "yuv444p", work.path()).await;
    assert!(!check.ok, "{check:?}");
    let ok = caps::lossless_self_test(&ff.ffmpeg, encoders::get("libx264").unwrap(), "yuv444p", work.path()).await;
    assert!(ok.ok, "{ok:?}");
}

/// Detection runs every self-test at once in one work folder, so each run needs its own
/// files: a source clip shared per pixel format, rewritten by a neighbouring test while
/// being read, made healthy encoders fail at random (x264 "not lossless" on Windows CI).
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_self_tests_in_one_folder_all_pass_and_clean_up() {
    let ff = ffmpeg();
    let work = tempfile::tempdir().unwrap();
    for round in 0..4 {
        let mut set = tokio::task::JoinSet::new();
        for name in ["libx264", "libx265", "libsvtav1", "ffv1"] {
            for pix in ["yuv420p", "yuv420p10le"] {
                let (ff, dir, def) = (ff.ffmpeg.clone(), work.path().to_path_buf(), encoders::get(name).unwrap());
                set.spawn(async move { caps::lossless_self_test(&ff, def, pix, &dir).await });
            }
        }
        while let Some(check) = set.join_next().await {
            let check = check.unwrap();
            assert!(check.ok, "round {round}: {check:?}");
        }
    }
    assert!(walk(work.path()).is_empty(), "self-tests leave no files behind: {:?}", walk(work.path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_backend_is_an_error_not_a_panic() {
    let mut ff = ffmpeg();
    ff.ffmpeg = "/nonexistent/ffmpeg".into();
    let work = tempfile::tempdir().unwrap();
    assert!(caps::detect(&ff, work.path()).await.is_err());
}
