// Typed bridge to the Rust backend. Outside Tauri (plain browser preview) a
// labelled mock is used for layout checks only — it never stands in for tests.
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { has, t } from "./i18n";
import type {
  AppConfig,
  Bootstrap,
  Capabilities,
  Concurrency,
  EncoderChoice,
  JobSettings,
  JobSpec,
  ManifestStatus,
  Preset,
  PreviewResult,
  ProbeEntry,
  QueueStatus,
  ScanResult,
  TrialResult,
  Job,
} from "./types";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
/** Dev only: `?bridge` drives the real backend of a debug build over 127.0.0.1 (see devbridge.rs). */
export const viaBridge = !inTauri && typeof location !== "undefined" && new URLSearchParams(location.search).has("bridge");
const BRIDGE = "http://127.0.0.1:1430";

const bridgeListeners = new Map<string, Set<(p: unknown) => void>>();
let bridgePolling = false;
function pollBridge() {
  if (bridgePolling) return;
  bridgePolling = true;
  let seq = 0;
  const tick = async () => {
    try {
      const r = await fetch(`${BRIDGE}/events?since=${seq}`);
      const j = (await r.json()) as { seq: number; events: { event: string; payload: unknown }[] };
      if (seq === 0) seq = j.seq; // start from "now"; bootstrap provides the current state
      else {
        seq = j.seq;
        j.events.forEach((e) => bridgeListeners.get(e.event)?.forEach((cb) => cb(e.payload)));
      }
    } catch {
      /* backend not up yet */
    }
    setTimeout(tick, 200);
  };
  tick();
}

async function callRaw<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return invoke<T>(cmd, args);
  if (viaBridge) {
    const r = await fetch(`${BRIDGE}/invoke/${cmd}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args ?? {}) });
    const j = (await r.json()) as { ok: boolean; data?: T; error?: string };
    if (!j.ok) throw j.error;
    return j.data as T;
  }
  const mock = await import("./mock");
  return mock.invoke<T>(cmd, args);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await callRaw<T>(cmd, args);
  } catch (error) {
    const code = String(error);
    throw code.startsWith("backend.") && has(code) ? t(code) : error;
  }
}

export function on<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (inTauri) return listen<T>(event, (e) => cb(e.payload));
  if (viaBridge) {
    if (!bridgeListeners.has(event)) bridgeListeners.set(event, new Set());
    bridgeListeners.get(event)!.add(cb as (p: unknown) => void);
    pollBridge();
    return Promise.resolve(() => bridgeListeners.get(event)?.delete(cb as (p: unknown) => void));
  }
  return import("./mock").then((m) => m.listen(event, cb as (p: unknown) => void));
}

export const fileSrc = (path: string) => (inTauri ? convertFileSrc(path) : viaBridge ? `${BRIDGE}/file?path=${encodeURIComponent(path)}` : path);

export const api = {
  bootstrap: () => call<Bootstrap>("bootstrap"),
  setFfmpegDir: (dir: string | null) => call<string>("set_ffmpeg_dir", { dir }),
  redetectCaps: () => call<Capabilities>("redetect_caps"),
  describeSettings: (settings: JobSettings) => call<EncoderChoice>("describe_settings", { settings }),
  probeFiles: (paths: string[]) => call<ProbeEntry[]>("probe_files", { paths }),
  scanInputs: (paths: string[], recursive: boolean) => call<ScanResult>("scan_inputs", { paths, recursive }),
  planPreview: (items: { path: string; settings: JobSettings }[]) => call<PreviewResult[]>("plan_preview", { items }),
  applyFix: (settings: JobSettings, fix: string) => call<JobSettings>("settings_apply_fix", { settings, fix }),
  enqueue: (specs: JobSpec[]) => call<string[]>("enqueue", { specs }),
  queueSnapshot: () => call<{ jobs: Job[]; status: QueueStatus }>("queue_snapshot"),
  setPaused: (paused: boolean) => call<void>("queue_set_paused", { paused }),
  cancel: (id: string) => call<boolean>("queue_cancel", { id }),
  retry: (id: string) => call<boolean>("queue_retry", { id }),
  remove: (ids: string[]) => call<number>("queue_remove", { ids }),
  move: (id: string, to: number) => call<boolean>("queue_move", { id, to }),
  clearFinished: () => call<number>("queue_clear_finished"),
  resolve: (id: string, fix: string) => call<boolean>("queue_resolve", { id, fix }),
  setConcurrency: (concurrency: Concurrency) => call<QueueStatus>("queue_set_concurrency", { concurrency }),
  manifestFlush: () => call<ManifestStatus>("manifest_flush"),
  reveal: (path: string) => call<void>("reveal_path", { path }),
  openPath: (path: string) => call<void>("open_path", { path }),
  readLog: (id: string) => call<string>("read_log", { id }),
  configGet: () => call<AppConfig>("config_get"),
  configSet: (config: AppConfig) => call<void>("config_set", { config }),
  presetsList: () => call<Preset[]>("presets_list"),
  presetSave: (name: string, settings: JobSettings) => call<Preset>("preset_save", { name, settings }),
  presetDelete: (id: string) => call<void>("preset_delete", { id }),
  presetExport: (id: string, path: string) => call<void>("preset_export", { id, path }),
  presetImport: (path: string) => call<Preset>("preset_import", { path, defaultName: t("preset.importDefault") }),
  trialRun: (args: { path: string; settings: JobSettings; start: number; duration: number; metrics: boolean }) =>
    call<TrialResult>("trial_run", { args }),
  trialCancel: () => call<void>("trial_cancel"),
  extractFrame: (path: string, time: number, maxWidth: number, tag: string) =>
    call<string>("extract_frame", { path, time, maxWidth, tag }),
  keyframes: (path: string) => call<number[]>("keyframes", { path }),
  quitApp: () => call<void>("quit_app"),
};

/** Native open dialog (files or a folder). Returns absolute paths. */
export async function pickPaths(opts: { directory?: boolean; multiple?: boolean; filters?: { name: string; extensions: string[] }[] }): Promise<string[]> {
  if (!inTauri) return [];
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: opts.directory ?? false, multiple: opts.multiple ?? true, filters: opts.filters });
  if (!r) return [];
  return Array.isArray(r) ? r : [r];
}

export async function pickSavePath(defaultPath: string, filters?: { name: string; extensions: string[] }[]): Promise<string | null> {
  if (!inTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({ defaultPath, filters });
}

/** OS file drops onto the window (Tauri delivers real paths). */
export async function onFileDrop(handlers: { over: () => void; leave: () => void; drop: (paths: string[]) => void }) {
  if (!inTauri) return () => {};
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  return getCurrentWebview().onDragDropEvent((e) => {
    const p = e.payload;
    if (p.type === "enter" || p.type === "over") handlers.over();
    else if (p.type === "leave") handlers.leave();
    else if (p.type === "drop") handlers.drop(p.paths);
  });
}

export const VIDEO_EXTENSIONS = [
  "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "3gp", "ogv", "mxf", "y4m", "ivf", "obu", "264", "h264", "265", "hevc", "nut",
];
