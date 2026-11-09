//! End-to-end tests against the real FFmpeg backend and generated media.
//! Run with: cargo test -p fe-core --test engine_e2e -- --test-threads=1

mod common;

use common::*;
use fe_core::engine::Concurrency;
use fe_core::job::JobState;
use fe_core::settings::*;
use fe_core::store::{self, QUEUE_VERSION, QueueFile};
use std::path::{Path, PathBuf};
use std::time::Duration;

const T: Duration = Duration::from_secs(180);

fn out(dir: &Path) -> OutputSettings {
    OutputSettings { dir: OutputDir::Custom, custom_dir: Some(dir.to_path_buf()), ..OutputSettings::default() }
}

fn convert(input: PathBuf, codec: VideoCodec, mode: VideoMode, container: Container, out_dir: &Path) -> JobSpec {
    let mut s = JobSettings::default();
    s.video.codec = codec;
    s.video.mode = mode;
    s.video.speed = Speed::Fast;
    s.output = out(out_dir);
    s.output.container = container;
    JobSpec::Convert { input, settings: s }
}

fn with(spec: JobSpec, f: impl FnOnce(&mut JobSettings)) -> JobSpec {
    match spec {
        JobSpec::Convert { input, mut settings } => {
            f(&mut settings);
            JobSpec::Convert { input, settings }
        }
        other => other,
    }
}

async fn probe(p: &Path) -> fe_core::media::MediaInfo {
    fe_core::media::probe(&ffmpeg().ffprobe, p).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn h264_to_av1_to_h264_round_trip_is_verified() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![convert(media("h264_aac.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mkv, &r.out_dir())]);
    let av1 = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_done(&av1);
    assert!(check_ok(&av1, "streamCopy:1"), "copied AAC is hash-identical");
    let av1_out = av1.output.clone().unwrap();
    assert_eq!(probe(&av1_out).await.main_video().unwrap().codec, "av1");

    let ids = r.engine.enqueue(vec![convert(av1_out, VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir())]);
    let h264 = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_done(&h264);
    let m = probe(h264.output.as_ref().unwrap()).await;
    assert_eq!(m.main_video().unwrap().codec, "h264");
    assert_eq!((m.main_video().unwrap().width, m.main_video().unwrap().height), (640, 360));
    assert_eq!(m.audio[0].codec, "aac");
    assert!(check_ok(&h264, "frameCount") && check_ok(&h264, "decode"));
}

#[tokio::test(flavor = "multi_thread")]
async fn lossless_encoders_are_verified_frame_by_frame() {
    let r = rig().await;
    let src = media("h264_aac.mp4");
    let ids = r.engine.enqueue(vec![
        convert(src.clone(), VideoCodec::H264, VideoMode::Lossless, Container::Mkv, &r.out_dir()),
        convert(src.clone(), VideoCodec::Av1, VideoMode::Lossless, Container::Mkv, &r.out_dir()),
        convert(src.clone(), VideoCodec::Ffv1, VideoMode::Lossless, Container::Mkv, &r.out_dir()),
        convert(media("hevc_10bit.mkv"), VideoCodec::Hevc, VideoMode::Lossless, Container::Mkv, &r.out_dir()),
    ]);
    for j in wait_terminal(&r.engine, &ids, T).await {
        assert_done(&j);
        assert!(check_ok(&j, "lossless"), "{} must pass the frame hash check", j.name);
        assert!(j.plan.as_ref().unwrap().lossless);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_and_special_character_paths_work_end_to_end() {
    let r = rig().await;
    let src = media("中文 目录/旅行 vlog #1 (最终版) & 東京タワー 'test'.mp4");
    let jp = media("中文 目录/日本語 フォルダ/猫の動画 [AV1] 100%.mkv");
    let out_dir = r.dir.path().join("出力 フォルダ #2 & 'x'");
    let ids = r.engine.enqueue(vec![
        convert(src, VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &out_dir),
        convert(jp, VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &out_dir),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    for j in &jobs {
        assert_done(j);
    }
    let name = jobs[0].output.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "旅行 vlog #1 (最终版) & 東京タワー 'test'_av1.mp4");
    let name = jobs[1].output.as_ref().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "猫の動画 [AV1] 100%_h264.mp4");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_audio_multi_audio_and_subtitles() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![
        convert(media("no_audio.mp4"), VideoCodec::Hevc, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("multi_audio.mkv"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert!(probe(jobs[0].output.as_ref().unwrap()).await.audio.is_empty());
    assert_done(&jobs[1]);
    let m = probe(jobs[1].output.as_ref().unwrap()).await;
    assert_eq!(m.audio.iter().map(|a| a.codec.as_str()).collect::<Vec<_>>(), ["aac", "ac3"]);
    assert_eq!(m.audio[0].language.as_deref(), Some("zho"), "track language kept");
    assert_eq!(m.subtitles[0].codec, "mov_text");
    assert!(check_ok(&jobs[1], "streamCopy:1") && check_ok(&jobs[1], "streamCopy:2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn vfr_keeps_every_frame_and_rotation_is_applied() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![
        convert(media("vfr.mkv"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("rotated.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert!(check_ok(&jobs[0], "frameCount"), "VFR passthrough keeps every frame");
    assert_done(&jobs[1]);
    let v = probe(jobs[1].output.as_ref().unwrap()).await.main_video().unwrap().clone();
    assert_eq!((v.width, v.height, v.rotation), (360, 640, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn hdr10_and_10bit_are_preserved() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![
        convert(media("hdr10.mkv"), VideoCodec::Av1, VideoMode::Lossy, Container::Mkv, &r.out_dir()),
        convert(media("hevc_10bit.mkv"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("hdr10.mkv"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert!(check_ok(&jobs[0], "hdrMetadata") && check_ok(&jobs[0], "colorTransfer"));
    assert_done(&jobs[1]);
    assert!(check_ok(&jobs[1], "pixFmt"));
    assert_eq!(probe(jobs[1].output.as_ref().unwrap()).await.main_video().unwrap().bit_depth, Some(10));
    assert_eq!(jobs[2].state, JobState::NeedsInput, "HDR → H.264 is refused, not degraded");
    assert_eq!(jobs[2].issues[0].notice.code, "hdrUnsupportedEncoder");
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_outputs_and_the_source_are_never_overwritten() {
    let r = rig().await;
    let out_dir = r.out_dir();
    std::fs::create_dir_all(&out_dir).unwrap();
    let existing = out_dir.join("h264_aac_h264.mp4");
    std::fs::write(&existing, b"keep me").unwrap();
    // Source lives in the output folder and the template would recreate its exact name.
    let src = out_dir.join("clip.mp4");
    std::fs::copy(media("h264_aac.mp4"), &src).unwrap();
    let src_bytes = std::fs::read(&src).unwrap();
    let same_name = with(convert(src.clone(), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &out_dir), |s| s.output.name_template = "{name}".into());
    let skip = with(convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &out_dir), |s| s.output.conflict = ConflictPolicy::Skip);
    let ids = r.engine.enqueue(vec![
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &out_dir),
        same_name,
        skip,
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert_eq!(jobs[0].output.as_ref().unwrap(), &out_dir.join("h264_aac_h264 (2).mp4"));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
    assert_done(&jobs[1]);
    assert_eq!(jobs[1].output.as_ref().unwrap(), &out_dir.join("clip (2).mp4"));
    assert_eq!(std::fs::read(&src).unwrap(), src_bytes, "source untouched");
    assert_eq!(jobs[2].state, JobState::Cancelled);
    assert_eq!(jobs[2].error.as_ref().unwrap().notice.code, "skippedExisting");
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep me");
    assert!(part_files(r.dir.path()).is_empty(), "no temp files left: {:?}", part_files(r.dir.path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_running_job_cleans_up_and_the_queue_continues() {
    let r = rig().await;
    let slow = with(convert(media("long_720p.mp4"), VideoCodec::Hevc, VideoMode::Lossy, Container::Mp4, &r.out_dir()), |s| s.video.speed = Speed::Slow);
    let ids = r.engine.enqueue(vec![slow, convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir())]);
    let running = wait_until(&r.engine, &ids[0], T, |j| j.state == JobState::Encoding && j.progress > 0.01).await;
    assert!(running.stats.speed.is_some(), "real backend progress is reported");
    assert!(!part_files(&r.out_dir()).is_empty(), "encoding into an owned temp file");
    assert!(r.engine.cancel(&ids[0]));
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_eq!(jobs[0].state, JobState::Cancelled);
    assert!(jobs[0].temp_files.is_empty());
    assert_done(&jobs[1]);
    assert!(part_files(r.dir.path()).is_empty());
    assert!(r.engine.retry(&ids[0]), "cancelled jobs can be retried");
    assert_eq!(r.engine.job(&ids[0]).unwrap().state, JobState::Waiting);
    r.engine.cancel(&ids[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupt_inputs_fail_individually() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![
        convert(media("corrupt_truncated.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("corrupt_random.mkv"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(r.dir.path().join("does-not-exist.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_eq!(jobs[0].state, JobState::Failed);
    assert_eq!(jobs[0].error.as_ref().unwrap().notice.code, "inputCorrupt");
    assert_eq!(jobs[1].state, JobState::Failed);
    assert_eq!(jobs[2].error.as_ref().unwrap().notice.code, "inputMissing");
    assert_done(&jobs[3]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn crashed_backend_fails_only_that_job() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![
        with(convert(media("long_720p.mp4"), VideoCodec::Hevc, VideoMode::Lossy, Container::Mp4, &r.out_dir()), |s| s.video.speed = Speed::Slow),
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let j = wait_until(&r.engine, &ids[0], T, |j| j.state == JobState::Encoding && j.pid.is_some() && j.progress > 0.01).await;
    // Simulate the backend dying underneath us.
    unsafe { libc::kill(j.pid.unwrap() as i32, libc::SIGKILL) };
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_eq!(jobs[0].state, JobState::Failed);
    assert_eq!(jobs[0].error.as_ref().unwrap().notice.code, "backendCrashed");
    assert!(part_files(r.dir.path()).is_empty());
    assert_done(&jobs[1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_backend_is_reported_per_job_without_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let mut ff = ffmpeg();
    ff.ffmpeg = dir.path().join("gone/ffmpeg");
    ff.ffprobe = dir.path().join("gone/ffprobe");
    let sink = std::sync::Arc::new(Collect::default());
    let engine = fe_core::engine::Engine::start(config(dir.path(), ff), capabilities().await, sink).await;
    engine.set_paused(false);
    let ids = engine.enqueue(vec![
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &dir.path().join("o")),
        convert(media("h264_aac.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &dir.path().join("o")),
    ]);
    for j in wait_terminal(&engine, &ids, T).await {
        assert_eq!(j.state, JobState::Failed);
        assert_eq!(j.error.as_ref().unwrap().notice.code, "backendMissing");
    }
}

/// Regression: an open/locked results CSV used to abort the whole batch.
#[tokio::test(flavor = "multi_thread")]
async fn locked_results_csv_never_stops_the_queue() {
    let r = rig().await;
    std::fs::create_dir_all(r.data_dir()).unwrap();
    std::fs::write(r.manifest(), "\u{feff}locked by excel\r\n").unwrap();
    let lock = lock_file(&r.manifest());
    let ids = r.engine.enqueue(vec![
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("no_audio.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert_done(&jobs[1]);
    let st = r.engine.manifest_status();
    assert_eq!(st.pending, 2, "rows kept while the CSV is locked");
    assert!(st.last_error.is_some());
    unlock_file(&r.manifest(), lock);
    let st = r.engine.flush_manifest();
    assert_eq!(st.pending, 0);
    let text = std::fs::read_to_string(r.manifest()).unwrap();
    assert_eq!(text.matches("完成").count(), 2, "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn parallel_jobs_are_isolated() {
    let r = rig().await;
    r.engine.set_concurrency(Concurrency::Fixed { n: 2 });
    let src = media("long_720p.mp4");
    let ids = r.engine.enqueue(vec![
        convert(src.clone(), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(src.clone(), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
    ]);
    let start = std::time::Instant::now();
    let mut saw_two = false;
    while start.elapsed() < T && !saw_two {
        saw_two = r.engine.status().running == 2;
        if ids.iter().all(|id| r.engine.job(id).unwrap().state.is_terminal()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(saw_two, "both jobs ran at the same time");
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    assert_done(&jobs[0]);
    assert_done(&jobs[1]);
    assert_ne!(jobs[0].output, jobs[1].output);
    assert_ne!(jobs[0].log_path, jobs[1].log_path);
    assert_eq!(jobs[0].plan.as_ref().unwrap().threads, Some(r.engine.caps().cpu.logical.max(4) / 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_recovers_unfinished_jobs_without_rerunning_finished_ones() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    // A previous session: one verified job, one interrupted mid-encode with a temp file.
    let done_out = out_dir.join("done_already.mp4");
    std::fs::write(&done_out, b"verified earlier").unwrap();
    let mut done = fe_core::job::Job::new("aaaaaaaa-0000-0000-0000-000000000001".into(), convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &out_dir), 1);
    done.state = JobState::Done;
    done.output = Some(done_out.clone());
    done.finished_at = Some(42);
    let mut cut = fe_core::job::Job::new("bbbbbbbb-0000-0000-0000-000000000002".into(), convert(media("h264_aac.mp4"), VideoCodec::Av1, VideoMode::Lossy, Container::Mp4, &out_dir), 2);
    cut.state = JobState::Encoding;
    let temp = out_dir.join("h264_aac_av1.mp4.fe-bbbbbbbb.part");
    std::fs::write(&temp, b"half written").unwrap();
    cut.temp_files = vec![temp.clone()];
    let q = QueueFile { version: QUEUE_VERSION, paused: false, jobs: vec![done, cut] };
    store::save_atomic(&dir.path().join("data/queue.json"), &serde_json::to_vec(&q).unwrap()).unwrap();

    let r = {
        let sink = std::sync::Arc::new(Collect::default());
        let engine = fe_core::engine::Engine::start(config(dir.path(), ffmpeg()), capabilities().await, sink.clone()).await;
        Rig { dir, engine, sink }
    };
    let st = r.engine.status();
    assert!(st.paused, "restored queues wait for the user to resume");
    assert_eq!(st.recovered, 1);
    let cut = r.engine.job("bbbbbbbb-0000-0000-0000-000000000002").unwrap();
    assert_eq!(cut.state, JobState::Waiting);
    assert!(cut.interrupted);
    assert!(!temp.exists(), "owned temp file of the interrupted job removed");
    r.engine.set_paused(false);
    let jobs = wait_terminal(&r.engine, &["bbbbbbbb-0000-0000-0000-000000000002".to_string()], T).await;
    assert_done(&jobs[0]);
    let done = r.engine.job("aaaaaaaa-0000-0000-0000-000000000001").unwrap();
    assert_eq!((done.state, done.finished_at, done.attempts), (JobState::Done, Some(42), 0), "verified job not re-run");
    assert_eq!(std::fs::read(&done_out).unwrap(), b"verified earlier");
}

#[tokio::test(flavor = "multi_thread")]
async fn incompatible_audio_waits_for_a_decision() {
    let r = rig().await;
    let ids = r.engine.enqueue(vec![convert(media("pcm_audio.mov"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir())]);
    let j = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_eq!(j.state, JobState::NeedsInput);
    assert_eq!(j.issues[0].notice.code, "audioIncompatible");
    assert!(r.engine.resolve(&ids[0], "audioTranscode"));
    let j = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_done(&j);
    assert_eq!(probe(j.output.as_ref().unwrap()).await.audio[0].codec, "aac");
    assert!(j.warnings.iter().any(|w| w.code == "audioTranscoded"));
}

#[tokio::test(flavor = "multi_thread")]
async fn full_range_sources_keep_their_range_and_verify() {
    // Phone apps often record full ("JPEG") range. FFmpeg names such 8-bit H.264/HEVC
    // streams yuvj420p, so a faithful output must not fail on the name alone.
    let r = rig().await;
    let mut specs = vec![
        convert(media("fullrange.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir()),
        convert(media("fullrange.mp4"), VideoCodec::Hevc, VideoMode::Lossy, Container::Mkv, &r.out_dir()),
    ];
    if capabilities().await.usable("h264_videotoolbox") {
        let gpu = convert(media("fullrange.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir());
        specs.push(with(gpu, |s| s.video.speed = Speed::Gpu));
    }
    let ids = r.engine.enqueue(specs);
    for j in wait_terminal(&r.engine, &ids, T).await {
        assert_done(&j);
        assert!(check_ok(&j, "pixFmt") && check_ok(&j, "colorRange"), "{:?}", j.verify);
        assert!(probe(j.output.as_ref().unwrap()).await.main_video().unwrap().full_range(), "full range kept");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_hidden_by_an_edit_list_are_not_counted_as_lost() {
    // The edit list hides 15 leading frames; players and FFmpeg skip them, so the output
    // rightly has fewer frames than the source file has video packets.
    let src = media("edit_list.mov");
    let ff = ffmpeg();
    assert_eq!(fe_core::media::count_packets(&ff.ffprobe, &src, 0).await.unwrap(), 105, "only shown frames count");
    let r = rig().await;
    let ids = r.engine.enqueue(vec![convert(src, VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir())]);
    let j = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_done(&j);
    assert!(check_ok(&j, "frameCount"), "{:?}", j.verify);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unfinished_download_is_flagged_and_its_readable_part_can_be_processed() {
    // The index at the front declares 10 s, but the data stops after about half of it.
    let src = media("incomplete_download.mp4");
    let m = probe(&src).await;
    let readable = m.readable_until.expect("missing data is noticed at import");
    assert!(readable > 3.0 && readable < 8.0 && m.duration.unwrap() > 9.0, "readable {readable}s of {:?}", m.duration);
    for complete in ["h264_aac.mp4", "rotated.mp4", "edit_list.mov", "pcm_audio.mov"] {
        assert_eq!(probe(&media(complete)).await.readable_until, None, "{complete} is complete");
    }

    let r = rig().await;
    let ids = r.engine.enqueue(vec![convert(src, VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &r.out_dir())]);
    let j = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_eq!(j.state, JobState::NeedsInput, "refused before encoding: {:?}", j.error);
    assert_eq!(j.issues[0].notice.code, "sourceIncomplete");
    assert!(r.engine.resolve(&ids[0], "acceptIncomplete"));
    let j = wait_terminal(&r.engine, &ids, T).await.remove(0);
    assert_done(&j);
    assert!(j.warnings.iter().any(|w| w.code == "sourceIncompleteTrimmed"));
    let out = probe(j.output.as_ref().unwrap()).await;
    assert!((out.duration.unwrap() - readable).abs() < 1.0, "output {:?}s, readable {readable}s", out.duration);
}

#[tokio::test(flavor = "multi_thread")]
async fn remux_trim_extract_and_mux() {
    let r = rig().await;
    let o = r.out_dir();
    let remux = convert(media("multi_audio.mkv"), VideoCodec::H264, VideoMode::Copy, Container::Mp4, &o);
    let kf_trim = with(convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Copy, Container::Mp4, &o), |s| {
        s.trim = Some(Trim { start: 2.5, end: Some(3.5) });
        s.output.name_template = "{name}_kf".into();
    });
    let precise = with(convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &o), |s| {
        s.trim = Some(Trim { start: 1.0, end: Some(3.0) });
        s.output.name_template = "{name}_cut".into();
    });
    let extract_ac3 = JobSpec::Extract(ExtractSpec { input: media("multi_audio.mkv"), stream_index: 2, output: out(&o) });
    let extract_srt = JobSpec::Extract(ExtractSpec { input: media("multi_audio.mkv"), stream_index: 3, output: out(&o) });
    let mux = JobSpec::Mux(MuxSpec {
        main: media("no_audio.mp4"),
        keep_main_audio: true,
        keep_main_subtitles: true,
        tracks: vec![
            MuxTrack { path: media("h264_aac.mp4"), kind: TrackKind::Audio, language: Some("jpn".into()), title: Some("日本語".into()), default: true },
            MuxTrack { path: media("subs_zh.srt"), kind: TrackKind::Subtitle, language: Some("zho".into()), title: None, default: false },
        ],
        output: OutputSettings { container: Container::Mkv, ..out(&o) },
    });
    let ids = r.engine.enqueue(vec![remux, kf_trim, precise, extract_ac3, extract_srt, mux]);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    for j in &jobs {
        assert_done(j);
    }
    assert!(check_ok(&jobs[0], "streamCopy:0") && check_ok(&jobs[0], "streamCopy:2"));
    assert!(jobs[1].warnings.iter().any(|w| w.code == "keyframeTrim"));
    let kf = probe(jobs[1].output.as_ref().unwrap()).await;
    assert!((kf.duration.unwrap() - 1.5).abs() < 0.2, "{:?}", kf.duration);
    let cut = probe(jobs[2].output.as_ref().unwrap()).await;
    assert!((cut.duration.unwrap() - 2.0).abs() < 0.1, "{:?}", cut.duration);
    assert!(jobs[3].output.as_ref().unwrap().to_string_lossy().ends_with(".ac3"));
    assert!(check_ok(&jobs[3], "streamCopy:0"));
    assert!(jobs[4].output.as_ref().unwrap().to_string_lossy().ends_with(".srt"));
    let m = probe(jobs[5].output.as_ref().unwrap()).await;
    assert_eq!((m.audio.len(), m.subtitles.len()), (1, 1));
    assert_eq!(m.audio[0].language.as_deref(), Some("jpn"));
}

#[tokio::test(flavor = "multi_thread")]
async fn paused_queue_holds_jobs_and_respects_reordering() {
    let r = rig().await;
    r.engine.set_paused(true);
    r.engine.set_concurrency(Concurrency::Fixed { n: 1 });
    let o = r.out_dir();
    let ids = r.engine.enqueue(vec![
        convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &o),
        convert(media("no_audio.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &o),
        convert(media("vfr.mkv"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &o),
    ]);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(ids.iter().all(|id| r.engine.job(id).unwrap().state == JobState::Waiting));
    assert!(r.engine.move_job(&ids[2], 0));
    assert_eq!(&r.engine.status().order[0], &ids[2]);
    r.engine.set_paused(false);
    let jobs = wait_terminal(&r.engine, &ids, T).await;
    let started: Vec<i64> = jobs.iter().map(|j| j.started_at.unwrap()).collect();
    assert!(started[2] <= started[0] && started[0] <= started[1], "{started:?}");
}

/// SVT-AV1 4.1 can deadlock internally (all pipeline threads waiting on semaphores).
/// A hung backend must be detected, killed and the step retried once, without changing settings.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_backend_is_killed_and_the_step_retried_once() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let real = ffmpeg();
    let mark = dir.path().join("hung-once");
    let wrapper = dir.path().join("ffmpeg-hangs-once");
    let body = format!(
        "#!/bin/sh\ncase \" $* \" in *\" -progress \"*) if [ ! -f '{m}' ]; then : > '{m}'; while true; do printf 'frame=0\\nprogress=continue\\n'; sleep 0.2; done; fi;; esac\nexec '{f}' \"$@\"\n",
        m = mark.display(),
        f = real.ffmpeg.display()
    );
    std::fs::write(&wrapper, body).unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut cfg = config(dir.path(), fe_core::tools::FfmpegPaths { ffmpeg: wrapper, ffprobe: real.ffprobe.clone(), source: real.source });
    cfg.stall_limit = Some(Duration::from_secs(3));
    let engine = fe_core::engine::Engine::start(cfg, capabilities().await, std::sync::Arc::new(Collect::default())).await;
    engine.set_paused(false);
    let ids = engine.enqueue(vec![convert(media("h264_aac.mp4"), VideoCodec::H264, VideoMode::Lossy, Container::Mp4, &dir.path().join("o"))]);
    let j = wait_terminal(&engine, &ids, T).await.remove(0);
    assert!(mark.exists(), "the first attempt really hung");
    assert_done(&j);
    assert!(j.warnings.iter().any(|w| w.code == "backendHungRetried"), "{:?}", j.warnings);
    assert!(part_files(dir.path()).is_empty());
}

#[cfg(unix)]
fn lock_file(p: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o444)).unwrap();
    None
}
#[cfg(unix)]
fn unlock_file(p: &Path, _l: Option<std::fs::File>) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644)).unwrap();
}
#[cfg(windows)]
fn lock_file(p: &Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    Some(std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(p).unwrap())
}
#[cfg(windows)]
fn unlock_file(_p: &Path, l: Option<std::fs::File>) {
    drop(l);
}
