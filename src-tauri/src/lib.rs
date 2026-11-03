//! FrameEase desktop shell: wires fe-core to the window (commands + events),
//! locates the bundled backend, caches capability detection and guards app exit.

mod commands;
#[cfg(all(feature = "dev-bridge", debug_assertions))]
mod devbridge;

use fe_core::caps::{self, Capabilities, CAPS_VERSION};
use fe_core::engine::{Concurrency, Engine, EngineConfig, EventSink, QueueStatus};
use fe_core::job::{Job, JobId};
use fe_core::manifest::ManifestStatus;
use fe_core::media::MediaInfo;
use fe_core::settings::JobSettings;
use fe_core::store::{self, LoadOutcome};
use fe_core::tools::{self, FfmpegPaths};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// User-chosen FFmpeg folder (overrides the bundled one).
    pub ffmpeg_dir: Option<PathBuf>,
    pub concurrency: Concurrency,
    /// Settings shown for new files (each queued job snapshots them).
    pub defaults: JobSettings,
    pub recursive_scan: bool,
    pub theme: String,
    pub locale: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ffmpeg_dir: None,
            concurrency: Concurrency::Auto,
            defaults: JobSettings::default(),
            recursive_scan: false,
            theme: "system".into(),
            locale: "zh-CN".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InitStatus {
    Pending,
    /// No ffmpeg/ffprobe pair was found anywhere.
    NoBackend,
    BackendError { message: String },
    Ready,
}

#[derive(Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
    pub previews: PathBuf,
    pub config: PathBuf,
    pub presets: PathBuf,
    pub caps: PathBuf,
}

/// Probe results keyed by path, valid while (size, mtime) are unchanged.
pub type ProbeCache = HashMap<PathBuf, (u64, u64, Arc<MediaInfo>)>;

pub struct AppState {
    pub engine: tokio::sync::OnceCell<Engine>,
    pub init: Mutex<InitStatus>,
    pub config: Mutex<AppConfig>,
    pub paths: OnceLock<Paths>,
    pub backend: Mutex<Option<FfmpegPaths>>,
    pub probe_cache: Mutex<ProbeCache>,
    pub trial_cancel: Mutex<Option<CancellationToken>>,
    pub quitting: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            engine: tokio::sync::OnceCell::new(),
            init: Mutex::new(InitStatus::Pending),
            config: Mutex::new(AppConfig::default()),
            paths: OnceLock::new(),
            backend: Mutex::new(None),
            probe_cache: Mutex::new(HashMap::new()),
            trial_cancel: Mutex::new(None),
            quitting: AtomicBool::new(false),
        }
    }
}

pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Emit to the window (and, in debug builds with `dev-bridge`, to the HTTP bridge).
/// Failures are ignored: the window may be closing.
pub fn emit_all<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    #[cfg(all(feature = "dev-bridge", debug_assertions))]
    devbridge::record(event, &payload);
    let _ = app.emit(event, payload);
}

/// Forwards engine events to the window.
struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn job(&self, job: &Job) {
        emit_all(&self.0, "fe://job", job);
    }
    fn removed(&self, ids: &[JobId]) {
        emit_all(&self.0, "fe://removed", ids);
    }
    fn queue(&self, status: &QueueStatus) {
        emit_all(&self.0, "fe://queue", status);
    }
    fn manifest(&self, status: &ManifestStatus) {
        emit_all(&self.0, "fe://manifest", status);
    }
    fn caps(&self, caps: &Capabilities) {
        if let Some(p) = self.0.state::<AppState>().paths.get() {
            save_caps(&p.caps, caps);
        }
        emit_all(&self.0, "fe://caps", caps);
    }
}

pub fn save_json<T: Serialize>(path: &std::path::Path, value: &T) {
    if let Ok(bytes) = serde_json::to_vec_pretty(value) {
        let _ = store::save_atomic(path, &bytes);
    }
}

fn save_caps(path: &std::path::Path, caps: &Capabilities) {
    save_json(path, caps);
}

fn load_cached_caps(path: &std::path::Path, backend: &FfmpegPaths) -> Option<Capabilities> {
    match store::load_json::<Capabilities>(path) {
        LoadOutcome::Loaded(c) | LoadOutcome::FromBackup(c)
            if c.caps_version == CAPS_VERSION && c.cache_key == caps::cache_key(backend) =>
        {
            // Hardware facts that may change between launches are refreshed cheaply.
            let mut c = c;
            c.cpu = caps::cpu_info();
            c.memory_mb = caps::memory_mb().0;
            Some(c)
        }
        _ => None,
    }
}

fn set_init(app: &AppHandle, status: InitStatus) {
    *lock(&app.state::<AppState>().init) = status.clone();
    emit_all(app, "fe://init", status);
}

/// Locate the backend (custom → bundled → portable → system), detect or load
/// cached capabilities, then start the queue engine.
pub async fn init_backend(app: AppHandle) {
    let state = app.state::<AppState>();
    let Some(paths) = state.paths.get().cloned() else { return };
    if state.engine.get().is_some() {
        set_init(&app, InitStatus::Ready);
        return;
    }
    set_init(&app, InitStatus::Pending);
    let custom = lock(&state.config).ffmpeg_dir.clone();
    let bundled = app.path().resource_dir().ok().map(|d| d.join("ffmpeg"));
    let portable = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("ffmpeg")));
    let Some(backend) = tools::locate(custom.as_deref(), bundled.as_deref(), portable.as_deref()) else {
        set_init(&app, InitStatus::NoBackend);
        return;
    };
    let caps = match load_cached_caps(&paths.caps, &backend) {
        Some(c) => c,
        None => match caps::detect(&backend, &caps::selftest_dir(&paths.cache)).await {
            Ok(c) => {
                save_caps(&paths.caps, &c);
                c
            }
            Err(e) => {
                set_init(&app, InitStatus::BackendError { message: e.to_string() });
                return;
            }
        },
    };
    *lock(&state.backend) = Some(backend.clone());
    let cfg = EngineConfig {
        ffmpeg: backend,
        data_dir: paths.data.clone(),
        work_root: paths.cache.join("work"),
        manifest_path: paths.data.join("FrameEase 结果记录.csv"),
        stall_limit: None,
    };
    let engine = Engine::start(cfg, caps.clone(), Arc::new(TauriSink(app.clone()))).await;
    engine.set_concurrency(lock(&state.config).concurrency);
    let _ = state.engine.set(engine);
    emit_all(&app, "fe://caps", &caps);
    set_init(&app, InitStatus::Ready);
}

fn running_jobs(app: &AppHandle) -> usize {
    app.state::<AppState>().engine.get().map(|e| e.status().running).unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let cache = app.path().app_cache_dir()?;
            let previews = cache.join("previews");
            for d in [&data, &cache, &previews] {
                std::fs::create_dir_all(d)?;
            }
            // Previews from a previous run are disposable.
            let _ = std::fs::remove_dir_all(&previews);
            std::fs::create_dir_all(&previews)?;
            let paths = Paths {
                config: data.join("config.json"),
                presets: data.join("presets.json"),
                caps: data.join("capabilities.json"),
                data,
                cache,
                previews,
            };
            let state = app.state::<AppState>();
            if let LoadOutcome::Loaded(c) | LoadOutcome::FromBackup(c) = store::load_json::<AppConfig>(&paths.config) {
                *lock(&state.config) = c;
            }
            let _ = state.paths.set(paths);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(init_backend(handle));
            #[cfg(all(feature = "dev-bridge", debug_assertions))]
            devbridge::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let quitting = app.state::<AppState>().quitting.load(Ordering::SeqCst);
                let running = running_jobs(app);
                if running > 0 && !quitting {
                    // Ask in the UI instead of silently killing an encode.
                    api.prevent_close();
                    emit_all(app, "fe://close-requested", running);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::set_ffmpeg_dir,
            commands::redetect_caps,
            commands::describe_settings,
            commands::probe_files,
            commands::scan_inputs,
            commands::plan_preview,
            commands::settings_apply_fix,
            commands::enqueue,
            commands::queue_snapshot,
            commands::queue_set_paused,
            commands::queue_cancel,
            commands::queue_retry,
            commands::queue_remove,
            commands::queue_move,
            commands::queue_clear_finished,
            commands::queue_resolve,
            commands::queue_set_concurrency,
            commands::manifest_flush,
            commands::reveal_path,
            commands::open_path,
            commands::read_log,
            commands::config_get,
            commands::config_set,
            commands::presets_list,
            commands::preset_save,
            commands::preset_delete,
            commands::preset_export,
            commands::preset_import,
            commands::trial_run,
            commands::trial_cancel,
            commands::extract_frame,
            commands::keyframes,
            commands::quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building FrameEase");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let quitting = app.state::<AppState>().quitting.load(Ordering::SeqCst);
            let running = running_jobs(app);
            if running > 0 && !quitting {
                api.prevent_exit();
                emit_all(app, "fe://close-requested", running);
            }
        }
    });
}
