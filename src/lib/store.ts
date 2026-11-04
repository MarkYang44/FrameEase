import { create } from "zustand";
import { toast } from "sonner";
import { api, on } from "./api";
import { setLocale, t } from "./i18n";
import type {
  AppConfig,
  AppInfo,
  Capabilities,
  EncoderChoice,
  InitStatus,
  Job,
  JobSettings,
  ManifestStatus,
  MediaInfo,
  Preset,
  PreviewResult,
  QueueStatus,
} from "./types";

export type View = "convert" | "trim" | "mux" | "info" | "settings";

export interface Staged {
  path: string;
  info: MediaInfo | null;
  error: string | null;
  preview: PreviewResult | null;
  loading: boolean;
}

interface Store {
  init: InitStatus;
  caps: Capabilities | null;
  config: AppConfig | null;
  app: AppInfo | null;
  jobs: Record<string, Job>;
  order: string[];
  status: QueueStatus | null;
  manifest: ManifestStatus | null;
  presets: Preset[];
  presetId: string | null;
  settings: JobSettings | null;
  choice: EncoderChoice | null;
  staged: Staged[];
  view: View;
  dropActive: boolean;
  dropHandler: ((paths: string[]) => void) | null;
  closeRequest: number | null;

  boot: () => Promise<void>;
  setView: (v: View) => void;
  updateSettings: (fn: (s: JobSettings) => void) => void;
  applyPreset: (p: Preset) => void;
  addPaths: (paths: string[]) => Promise<void>;
  removeStaged: (path: string) => void;
  clearStaged: () => void;
  startStaged: () => Promise<void>;
  setConfig: (fn: (c: AppConfig) => void) => void;
  refreshPresets: () => Promise<void>;
}

/** Settings are plain JSON; avoids structuredClone (Safari 15.4+) on older system WebKit. */
export const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

function debounce(fn: () => void, ms: number) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(fn, ms);
  };
}

export function applyTheme(theme: string) {
  const root = document.documentElement;
  if (theme === "light" || theme === "dark") root.dataset.theme = theme;
  else delete root.dataset.theme;
}

let subscribed = false;
let previewSeq = 0;

export const useStore = create<Store>((set, get) => {
  const syncSettings = debounce(async () => {
    const { settings, staged, init } = get();
    if (!settings || init.kind !== "ready") return;
    const config = get().config;
    if (config) {
      const next = { ...config, defaults: settings };
      set({ config: next });
      api.configSet(next).catch(() => {});
    }
    api.describeSettings(settings).then((choice) => set({ choice })).catch(() => {});
    const ready = staged.filter((s) => s.info);
    if (ready.length === 0) return;
    const seq = ++previewSeq;
    try {
      const results = await api.planPreview(ready.map((s) => ({ path: s.path, settings })));
      if (seq !== previewSeq) return; // a newer change superseded this one
      const byPath = new Map(results.map((r) => [r.path, r]));
      set({ staged: get().staged.map((s) => (byPath.has(s.path) ? { ...s, preview: byPath.get(s.path)! } : s)) });
    } catch {
      /* the next settings change retries */
    }
  }, 160);

  const upsert = (job: Job) =>
    set((st) => {
      const exists = job.id in st.jobs;
      return { jobs: { ...st.jobs, [job.id]: job }, order: exists ? st.order : [...st.order, job.id] };
    });

  async function subscribe() {
    if (subscribed) return;
    subscribed = true;
    await on<Job>("fe://job", upsert);
    await on<string[]>("fe://removed", (ids) =>
      set((st) => {
        const jobs = { ...st.jobs };
        ids.forEach((id) => delete jobs[id]);
        return { jobs, order: st.order.filter((id) => !ids.includes(id)) };
      }),
    );
    await on<QueueStatus>("fe://queue", (status) => set({ status, order: status.order }));
    await on<ManifestStatus>("fe://manifest", (manifest) => set({ manifest }));
    await on<Capabilities>("fe://caps", (caps) => set({ caps }));
    await on<InitStatus>("fe://init", (init) => {
      set({ init });
      if (init.kind === "ready") get().boot();
    });
    await on<number>("fe://close-requested", (n) => set({ closeRequest: n }));
  }

  return {
    init: { kind: "pending" },
    caps: null,
    config: null,
    app: null,
    jobs: {},
    order: [],
    status: null,
    manifest: null,
    presets: [],
    presetId: null,
    settings: null,
    choice: null,
    staged: [],
    view: "convert",
    dropActive: false,
    dropHandler: null,
    closeRequest: null,

    async boot() {
      await subscribe();
      const b = await api.bootstrap();
      setLocale(b.config.locale);
      applyTheme(b.config.theme);
      const jobs: Record<string, Job> = {};
      b.jobs.forEach((j) => (jobs[j.id] = j));
      set({
        init: b.init,
        caps: b.caps,
        config: b.config,
        app: b.app,
        jobs,
        order: b.status?.order ?? b.jobs.map((j) => j.id),
        status: b.status,
        manifest: b.manifest,
        presets: b.presets,
        settings: get().settings ?? clone(b.config.defaults),
      });
      if (b.init.kind === "ready") syncSettings();
    },

    setView: (view) => set({ view }),

    updateSettings(fn) {
      const cur = get().settings;
      if (!cur) return;
      const next = clone(cur);
      fn(next);
      set({ settings: next, presetId: null });
      syncSettings();
    },

    applyPreset(p) {
      const cur = get().settings;
      const next = clone(p.settings);
      // Keep where the user saves files unless the preset says otherwise.
      if (cur && p.builtin) next.output = { ...cur.output, container: next.output.container, nameTemplate: next.output.nameTemplate };
      set({ settings: next, presetId: p.id });
      syncSettings();
    },

    async addPaths(paths) {
      if (paths.length === 0) return;
      const recursive = get().config?.recursiveScan ?? false;
      const scan = await api.scanInputs(paths, recursive);
      const skipped: string[] = [];
      if (scan.skippedTemp) skipped.push(t("add.skippedTemp", { n: scan.skippedTemp }));
      if (scan.skippedOutputDirs) skipped.push(t("add.skippedOutput", { n: scan.skippedOutputDirs }));
      if (scan.skippedNonVideo) skipped.push(t("add.skippedNonVideo", { n: scan.skippedNonVideo }));
      const have = new Set(get().staged.map((s) => s.path));
      const fresh = scan.files.filter((f) => !have.has(f));
      if (fresh.length === 0) {
        toast(t("add.nothing"), { description: skipped.length ? t("add.skipped", { n: skipped.length }) + skipped.join(t("common.listSeparator")) : undefined });
        return;
      }
      set((st) => ({ staged: [...st.staged, ...fresh.map((path) => ({ path, info: null, error: null, preview: null, loading: true }))] }));
      if (skipped.length) toast(t("add.added", { n: fresh.length }), { description: t("add.skipped", { n: skipped.length }) + skipped.join(t("common.listSeparator")) });
      // Probe in small batches so the list fills in progressively.
      for (let i = 0; i < fresh.length; i += 4) {
        const batch = fresh.slice(i, i + 4);
        const entries = await api.probeFiles(batch);
        const byPath = new Map(entries.map((e) => [e.path, e]));
        set((st) => ({
          staged: st.staged.map((s) => {
            const e = byPath.get(s.path);
            return e ? { ...s, info: e.info, error: e.error, loading: false } : s;
          }),
        }));
      }
      syncSettings();
    },

    removeStaged: (path) => set((st) => ({ staged: st.staged.filter((s) => s.path !== path) })),
    clearStaged: () => set({ staged: [] }),

    async startStaged() {
      const { staged, settings } = get();
      if (!settings) return;
      const ready = staged.filter((s) => s.info);
      if (ready.length === 0) return;
      // Each job gets its own snapshot of the settings.
      const ids = await api.enqueue(ready.map((s) => ({ kind: "convert" as const, input: s.path, settings: clone(settings) })));
      set((st) => ({ staged: st.staged.filter((s) => !s.info) }));
      if (get().status?.paused) await api.setPaused(false);
      toast.success(t("queue.addedCount", { n: ids.length }));
    },

    setConfig(fn) {
      const cur = get().config;
      if (!cur) return;
      const next = clone(cur);
      fn(next);
      set({ config: next });
      setLocale(next.locale);
      applyTheme(next.theme);
      api.configSet(next).catch((e) => toast.error(String(e)));
    },

    async refreshPresets() {
      set({ presets: await api.presetsList() });
    },
  };
});

/** Jobs in queue order (stable reference per change). */
export const selectOrdered = (st: Store) => st.order.map((id) => st.jobs[id]).filter(Boolean);
