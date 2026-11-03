//! Commands invoked by the window. Errors are returned as short strings for toasts.

use crate::{lock, save_json, AppConfig, AppState, InitStatus, Paths};
use fe_core::caps::{self, Capabilities};
use fe_core::engine::{Concurrency, Engine, QueueStatus};
use fe_core::job::{Issue, Job, Notice, PlanSummary};
use fe_core::manifest::ManifestStatus;
use fe_core::media::{self, MediaInfo};
use fe_core::plan::{self, PlanEnv, PlanError};
use fe_core::presets::{self, Preset};
use fe_core::scan::{self, ScanOptions, ScanResult};
use fe_core::settings::{JobSettings, JobSpec, OutputDir, DEFAULT_SUBFOLDER};
use fe_core::store::{self, LoadOutcome};
use fe_core::trial::{self, TrialError, TrialRequest, TrialResult};
use fe_core::encoders;
use fe_core::fsx;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;

type Res<T> = Result<T, String>;

fn engine<'a>(state: &'a State<'_, AppState>) -> Res<&'a Engine> {
    state.engine.get().ok_or_else(|| "后端尚未就绪".to_string())
}

fn paths(state: &State<'_, AppState>) -> Res<Paths> {
    state.paths.get().cloned().ok_or_else(|| "应用目录不可用".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: String,
    data_dir: PathBuf,
    platform: String,
    arch: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    init: InitStatus,
    caps: Option<Capabilities>,
    config: AppConfig,
    jobs: Vec<Job>,
    status: Option<QueueStatus>,
    manifest: Option<ManifestStatus>,
    presets: Vec<Preset>,
    app: AppInfo,
}

#[tauri::command]
pub async fn bootstrap(app: AppHandle, state: State<'_, AppState>) -> Res<Bootstrap> {
    let e = state.engine.get();
    Ok(Bootstrap {
        init: lock(&state.init).clone(),
        caps: e.map(|e| (*e.caps()).clone()),
        config: lock(&state.config).clone(),
        jobs: e.map(|e| e.jobs()).unwrap_or_default(),
        status: e.map(|e| e.status()),
        manifest: e.map(|e| e.manifest_status()),
        presets: all_presets(&state),
        app: AppInfo {
            version: app.package_info().version.to_string(),
            data_dir: state.paths.get().map(|p| p.data.clone()).unwrap_or_default(),
            platform: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        },
    })
}

/// Choose an FFmpeg folder. Before the engine started this initialises it;
/// afterwards the choice applies on the next launch.
#[tauri::command]
pub async fn set_ffmpeg_dir(app: AppHandle, state: State<'_, AppState>, dir: Option<String>) -> Res<String> {
    let dir = dir.map(PathBuf::from);
    if let Some(d) = &dir
        && fe_core::tools::locate(Some(d), None, None).map(|p| p.source) != Some(fe_core::tools::ToolSource::Custom)
    {
        return Err("该文件夹中没有同时找到 ffmpeg 与 ffprobe".into());
    }
    {
        let mut c = lock(&state.config);
        c.ffmpeg_dir = dir;
        save_json(&paths(&state)?.config, &*c);
    }
    if state.engine.get().is_some() {
        return Ok("restartRequired".into());
    }
    crate::init_backend(app).await;
    Ok("applied".into())
}

#[tauri::command]
pub async fn redetect_caps(state: State<'_, AppState>) -> Res<Capabilities> {
    let backend = lock(&state.backend).clone().ok_or("后端尚未就绪")?;
    let p = paths(&state)?;
    let c = caps::detect(&backend, &caps::selftest_dir(&p.cache)).await.map_err(|e| e.to_string())?;
    save_json(&p.caps, &c);
    engine(&state)?.set_caps(c.clone());
    Ok(c)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityInfo {
    kind: encoders::QualityKind,
    value: f64,
    min: f64,
    max: f64,
    lower_is_better: bool,
    at_best: f64,
    at_smallest: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderOption {
    name: String,
    label: String,
    hardware: bool,
    usable: bool,
    reason: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderChoice {
    encoder: Option<String>,
    label: Option<String>,
    hardware: bool,
    quality: Option<QualityInfo>,
    two_pass: bool,
    bitrate: bool,
    lossless_capable: bool,
    issue: Option<Issue>,
    /// Every encoder for this codec in this build, with availability.
    options: Vec<EncoderOption>,
    /// Codecs that have at least one verified hardware encoder.
    gpu_codecs: Vec<String>,
}

/// Which encoder these settings use and what its native quality value is — no media needed.
#[tauri::command]
pub async fn describe_settings(state: State<'_, AppState>, settings: JobSettings) -> Res<EncoderChoice> {
    let caps = engine(&state)?.caps();
    let v = &settings.video;
    let options = encoders::for_codec(v.codec)
        .filter(|d| caps.encoder(d.name).is_some())
        .map(|d| {
            let ec = caps.encoder(d.name);
            EncoderOption {
                name: d.name.into(),
                label: d.label.into(),
                hardware: d.is_hardware(),
                usable: ec.map(|e| e.usable).unwrap_or(false),
                reason: ec.and_then(|e| e.error.clone()),
            }
        })
        .collect();
    let mut gpu_codecs: Vec<String> = encoders::ENCODERS
        .iter()
        .filter(|d| d.is_hardware() && caps.usable(d.name))
        .map(|d| d.codec.probe_name().to_string())
        .collect();
    gpu_codecs.dedup();
    let base = EncoderChoice { encoder: None, label: None, hardware: false, quality: None, two_pass: false, bitrate: false, lossless_capable: false, issue: None, options, gpu_codecs };
    Ok(match plan::choose_encoder(v, &caps) {
        Err(issue) => EncoderChoice { issue: Some(issue), ..base },
        Ok(def) => {
            let quality = def.quality.map(|q| {
                let value = match v.rate {
                    fe_core::settings::RateControl::Native { value } => value,
                    _ => encoders::level_to_native(&q, v.level),
                };
                QualityInfo { kind: q.kind, value, min: q.min, max: q.max, lower_is_better: q.lower_is_better, at_best: q.at_best, at_smallest: q.at_smallest }
            });
            EncoderChoice {
                encoder: Some(def.name.into()),
                label: Some(def.label.into()),
                hardware: def.is_hardware(),
                quality,
                two_pass: def.two_pass,
                bitrate: def.bitrate,
                lossless_capable: def.lossless != encoders::Lossless::No,
                ..base
            }
        }
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeEntry {
    path: PathBuf,
    info: Option<MediaInfo>,
    error: Option<String>,
}

fn file_stamp(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((m.len(), t))
}

/// Probe with a small cache keyed by size + mtime (re-probed when the file changes).
async fn probe_cached(state: &State<'_, AppState>, path: &Path) -> Result<Arc<MediaInfo>, String> {
    let stamp = file_stamp(path);
    if let Some(s) = stamp
        && let Some((len, t, m)) = lock(&state.probe_cache).get(path)
        && (*len, *t) == s
    {
        return Ok(m.clone());
    }
    let backend = lock(&state.backend).clone().ok_or("后端尚未就绪")?;
    let info = Arc::new(media::probe(&backend.ffprobe, path).await.map_err(|e| e.code().to_string())?);
    if let Some((len, t)) = stamp {
        let mut cache = lock(&state.probe_cache);
        if cache.len() > 2000 {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (len, t, info.clone()));
    }
    Ok(info)
}

#[tauri::command]
pub async fn probe_files(state: State<'_, AppState>, paths: Vec<String>) -> Res<Vec<ProbeEntry>> {
    let mut out = Vec::with_capacity(paths.len());
    // Small batches keep many simultaneous ffprobe processes from flooding the machine.
    for chunk in paths.chunks(4) {
        let futs = chunk.iter().map(|p| {
            let path = PathBuf::from(p);
            let state = &state;
            async move {
                match probe_cached(state, &path).await {
                    Ok(m) => ProbeEntry { path, info: Some((*m).clone()), error: None },
                    Err(e) => ProbeEntry { path, info: None, error: Some(e) },
                }
            }
        });
        let mut results = Vec::new();
        for f in futs {
            results.push(f.await);
        }
        out.extend(results);
    }
    Ok(out)
}

#[tauri::command]
pub async fn scan_inputs(state: State<'_, AppState>, paths: Vec<String>, recursive: bool) -> Res<ScanResult> {
    let cfg = lock(&state.config).clone();
    let mut exclude_dirs = Vec::new();
    if let (OutputDir::Custom, Some(d)) = (cfg.defaults.output.dir, cfg.defaults.output.custom_dir.clone()) {
        exclude_dirs.push(d);
    }
    let mut names = vec![DEFAULT_SUBFOLDER.to_string(), "FrameEase Output".to_string()];
    if !cfg.defaults.output.subfolder.trim().is_empty() {
        names.push(cfg.defaults.output.subfolder.trim().to_string());
    }
    let opts = ScanOptions { recursive, exclude_dirs, exclude_dir_names: names };
    let list: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || scan::scan_inputs(&list, &opts)).await.map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    path: String,
    settings: JobSettings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    path: String,
    ok: bool,
    issues: Vec<Issue>,
    warnings: Vec<Notice>,
    summary: Option<PlanSummary>,
    planned_output: Option<PathBuf>,
    error: Option<String>,
}

/// Plan without running: shows per-file problems and what will happen before queueing.
#[tauri::command]
pub async fn plan_preview(state: State<'_, AppState>, items: Vec<PreviewItem>) -> Res<Vec<PreviewResult>> {
    let e = engine(&state)?;
    let mut out = Vec::new();
    for item in items {
        let path = PathBuf::from(&item.path);
        let media = match probe_cached(&state, &path).await {
            Ok(m) => m,
            Err(err) => {
                out.push(PreviewResult { path: item.path, ok: false, issues: vec![], warnings: vec![], summary: None, planned_output: None, error: Some(err) });
                continue;
            }
        };
        let caps = e.caps();
        let (codec, mode) = plan::naming_tokens(&item.settings, &caps);
        let date = chrono::Local::now().format("%Y%m%d").to_string();
        let planned = fsx::plan_output_path(&path, &item.settings.output, &codec, &mode, &date, &Default::default());
        let temp = fsx::temp_path_for(&planned, "00000000");
        let work = paths(&state)?.cache.join("work").join("preview");
        let threads = e.status().threads_per_job;
        let mut tested = false;
        let result = loop {
            let caps = e.caps();
            let env = PlanEnv { caps: &caps, threads, temp_output: &temp, work_dir: &work, keyframes: None };
            match plan::plan_convert(&item.settings, &media, &env) {
                Ok(p) => break PreviewResult { path: item.path.clone(), ok: true, issues: vec![], warnings: p.warnings, summary: Some(p.summary), planned_output: Some(planned.clone()), error: None },
                Err(PlanError::Blocked(issues)) => break PreviewResult { path: item.path.clone(), ok: false, issues, warnings: vec![], summary: None, planned_output: Some(planned.clone()), error: None },
                Err(PlanError::NeedsSelfTest { encoder, pix_fmt }) if !tested => {
                    tested = true;
                    e.ensure_lossless(&encoder, &pix_fmt).await;
                }
                Err(PlanError::NeedsSelfTest { .. }) => break PreviewResult { path: item.path.clone(), ok: false, issues: vec![], warnings: vec![], summary: None, planned_output: None, error: Some("losslessNotVerified".into()) },
            }
        };
        out.push(result);
    }
    Ok(out)
}

/// Apply an issue fix to the not-yet-queued settings (same logic as for queued jobs).
#[tauri::command]
pub async fn settings_apply_fix(mut settings: JobSettings, fix: String) -> Res<JobSettings> {
    if fix == "useBitrate" {
        settings.video.rate = fe_core::settings::RateControl::Bitrate { kbps: 4000, two_pass: false };
        return Ok(settings);
    }
    if plan::apply_fix(&mut settings, &fix, None) {
        Ok(settings)
    } else {
        Err(format!("unknown fix {fix}"))
    }
}

#[tauri::command]
pub async fn enqueue(state: State<'_, AppState>, specs: Vec<JobSpec>) -> Res<Vec<String>> {
    if specs.is_empty() {
        return Ok(vec![]);
    }
    let ids = engine(&state)?.enqueue(specs);
    Ok(ids)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    jobs: Vec<Job>,
    status: QueueStatus,
}

#[tauri::command]
pub async fn queue_snapshot(state: State<'_, AppState>) -> Res<Snapshot> {
    let e = engine(&state)?;
    Ok(Snapshot { jobs: e.jobs(), status: e.status() })
}

#[tauri::command]
pub async fn queue_set_paused(state: State<'_, AppState>, paused: bool) -> Res<()> {
    engine(&state)?.set_paused(paused);
    Ok(())
}

#[tauri::command]
pub async fn queue_cancel(state: State<'_, AppState>, id: String) -> Res<bool> {
    Ok(engine(&state)?.cancel(&id))
}

#[tauri::command]
pub async fn queue_retry(state: State<'_, AppState>, id: String) -> Res<bool> {
    Ok(engine(&state)?.retry(&id))
}

#[tauri::command]
pub async fn queue_remove(state: State<'_, AppState>, ids: Vec<String>) -> Res<usize> {
    Ok(engine(&state)?.remove(&ids))
}

#[tauri::command]
pub async fn queue_move(state: State<'_, AppState>, id: String, to: usize) -> Res<bool> {
    Ok(engine(&state)?.move_job(&id, to))
}

#[tauri::command]
pub async fn queue_clear_finished(state: State<'_, AppState>) -> Res<usize> {
    Ok(engine(&state)?.clear_finished())
}

#[tauri::command]
pub async fn queue_resolve(state: State<'_, AppState>, id: String, fix: String) -> Res<bool> {
    Ok(engine(&state)?.resolve(&id, &fix))
}

#[tauri::command]
pub async fn queue_set_concurrency(state: State<'_, AppState>, concurrency: Concurrency) -> Res<QueueStatus> {
    let e = engine(&state)?;
    e.set_concurrency(concurrency);
    {
        let mut c = lock(&state.config);
        c.concurrency = concurrency;
        save_json(&paths(&state)?.config, &*c);
    }
    Ok(e.status())
}

#[tauri::command]
pub async fn manifest_flush(state: State<'_, AppState>) -> Res<ManifestStatus> {
    Ok(engine(&state)?.flush_manifest())
}

/// Paths the UI may reveal/open: job inputs/outputs/logs, the data folder and the results CSV.
fn allowed_path(state: &State<'_, AppState>, path: &Path) -> bool {
    let data = state.paths.get().map(|p| p.data.clone());
    if data.as_ref().map(|d| path.starts_with(d)).unwrap_or(false) {
        return true;
    }
    let Some(e) = state.engine.get() else { return false };
    let manifest = e.manifest_status().path;
    path == manifest
        || e.jobs().iter().any(|j| {
            j.output.as_deref() == Some(path)
                || j.log_path.as_deref() == Some(path)
                || j.spec.primary_input() == path
                || j.planned_output.as_deref().and_then(|p| p.parent()) == Some(path)
                || j.output.as_deref().and_then(|p| p.parent()) == Some(path)
        })
}

#[tauri::command]
pub async fn reveal_path(app: AppHandle, state: State<'_, AppState>, path: String) -> Res<()> {
    let p = PathBuf::from(&path);
    if !allowed_path(&state, &p) {
        return Err("不允许打开该位置".into());
    }
    if !p.exists() {
        return Err("文件已不存在".into());
    }
    app.opener().reveal_item_in_dir(&p).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_path(app: AppHandle, state: State<'_, AppState>, path: String) -> Res<()> {
    let p = PathBuf::from(&path);
    if !allowed_path(&state, &p) {
        return Err("不允许打开该文件".into());
    }
    if !p.exists() {
        return Err("文件已不存在".into());
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Tail of a job's backend log (bounded, for the log viewer).
#[tauri::command]
pub async fn read_log(state: State<'_, AppState>, id: String) -> Res<String> {
    let job = engine(&state)?.job(&id).ok_or("任务不存在")?;
    let Some(p) = job.log_path else { return Ok(String::new()) };
    let bytes = std::fs::read(&p).unwrap_or_default();
    let start = bytes.len().saturating_sub(200_000);
    Ok(String::from_utf8_lossy(&bytes[start..]).into_owned())
}

#[tauri::command]
pub async fn config_get(state: State<'_, AppState>) -> Res<AppConfig> {
    Ok(lock(&state.config).clone())
}

#[tauri::command]
pub async fn config_set(state: State<'_, AppState>, config: AppConfig) -> Res<()> {
    let p = paths(&state)?;
    let mut c = lock(&state.config);
    // The backend folder is changed through `set_ffmpeg_dir` only.
    let ffmpeg_dir = c.ffmpeg_dir.clone();
    *c = config;
    c.ffmpeg_dir = ffmpeg_dir;
    save_json(&p.config, &*c);
    Ok(())
}

fn user_presets(state: &State<'_, AppState>) -> Vec<Preset> {
    let Some(p) = state.paths.get() else { return vec![] };
    match store::load_json::<Vec<Preset>>(&p.presets) {
        LoadOutcome::Loaded(v) | LoadOutcome::FromBackup(v) => v,
        _ => vec![],
    }
}

fn all_presets(state: &State<'_, AppState>) -> Vec<Preset> {
    let mut v = presets::builtin_presets();
    v.extend(user_presets(state));
    v
}

#[tauri::command]
pub async fn presets_list(state: State<'_, AppState>) -> Res<Vec<Preset>> {
    Ok(all_presets(&state))
}

#[tauri::command]
pub async fn preset_save(state: State<'_, AppState>, name: String, settings: JobSettings) -> Res<Preset> {
    let name = name.trim().chars().take(60).collect::<String>();
    if name.is_empty() {
        return Err("请输入预设名称".into());
    }
    let mut list = user_presets(&state);
    let p = match list.iter_mut().find(|p| p.name == name) {
        Some(existing) => {
            existing.settings = settings;
            existing.clone()
        }
        None => {
            let p = Preset { id: uuid_like(), name, settings, builtin: false };
            list.push(p.clone());
            p
        }
    };
    save_json(&paths(&state)?.presets, &list);
    Ok(p)
}

fn uuid_like() -> String {
    format!("user-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default())
}

#[tauri::command]
pub async fn preset_delete(state: State<'_, AppState>, id: String) -> Res<()> {
    let mut list = user_presets(&state);
    list.retain(|p| p.id != id);
    save_json(&paths(&state)?.presets, &list);
    Ok(())
}

#[tauri::command]
pub async fn preset_export(state: State<'_, AppState>, id: String, path: String) -> Res<()> {
    let p = all_presets(&state).into_iter().find(|p| p.id == id).ok_or("预设不存在")?;
    std::fs::write(&path, presets::export_json(&p)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn preset_import(state: State<'_, AppState>, path: String) -> Res<Preset> {
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let p = presets::import_json(&text)?;
    let mut list = user_presets(&state);
    list.push(p.clone());
    save_json(&paths(&state)?.presets, &list);
    Ok(p)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialArgs {
    path: String,
    settings: JobSettings,
    start: f64,
    duration: f64,
    metrics: bool,
}

#[tauri::command]
pub async fn trial_run(state: State<'_, AppState>, args: TrialArgs) -> Res<TrialResult> {
    let e = engine(&state)?;
    let backend = lock(&state.backend).clone().ok_or("后端尚未就绪")?;
    let work = paths(&state)?.previews.join("trial");
    let token = CancellationToken::new();
    if let Some(old) = lock(&state.trial_cancel).replace(token.clone()) {
        old.cancel();
    }
    let req = TrialRequest { input: PathBuf::from(&args.path), settings: args.settings, start: args.start, duration: args.duration, metrics: args.metrics };
    let mut tested = false;
    loop {
        let caps = e.caps();
        match trial::run_trial(&backend, &caps, &req, &work, &token).await {
            Ok(r) => return Ok(r),
            Err(TrialError::NeedsSelfTest { encoder, pix_fmt }) if !tested => {
                tested = true;
                e.ensure_lossless(&encoder, &pix_fmt).await;
            }
            Err(TrialError::NeedsSelfTest { .. }) => return Err("losslessNotVerified".into()),
            Err(TrialError::Rejected(n)) => return Err(n.code),
            Err(TrialError::Cancelled) => return Err("cancelled".into()),
            Err(TrialError::Failed(d)) => return Err(d.lines().last().unwrap_or("trial failed").to_string()),
        }
    }
}

#[tauri::command]
pub async fn trial_cancel(state: State<'_, AppState>) -> Res<()> {
    if let Some(t) = lock(&state.trial_cancel).take() {
        t.cancel();
    }
    Ok(())
}

/// Extract one frame into the previews folder; the UI loads it via the asset protocol.
#[tauri::command]
pub async fn extract_frame(state: State<'_, AppState>, path: String, time: f64, max_width: u32, tag: String) -> Res<String> {
    let backend = lock(&state.backend).clone().ok_or("后端尚未就绪")?;
    let safe_tag: String = tag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    let out = paths(&state)?.previews.join(format!("{safe_tag}-{}.png", chrono::Utc::now().timestamp_millis()));
    trial::extract_frame(&backend.ffmpeg, Path::new(&path), time, max_width.clamp(160, 3840), &out).await?;
    Ok(out.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn keyframes(state: State<'_, AppState>, path: String) -> Res<Vec<f64>> {
    let p = PathBuf::from(&path);
    let m = probe_cached(&state, &p).await?;
    let backend = lock(&state.backend).clone().ok_or("后端尚未就绪")?;
    let idx = m.main_video().map(|v| v.index).ok_or("没有视频流")?;
    media::keyframes(&backend.ffprobe, &p, idx).await.map_err(|e| e.to_string())
}

/// Quit after the user confirmed: running jobs stop (temp files removed) and
/// return to the queue as "interrupted"; they restart from the beginning next time.
#[tauri::command]
pub async fn quit_app(app: AppHandle, state: State<'_, AppState>) -> Res<()> {
    state.quitting.store(true, Ordering::SeqCst);
    if let Some(e) = state.engine.get() {
        e.shutdown().await;
    }
    app.exit(0);
    Ok(())
}
