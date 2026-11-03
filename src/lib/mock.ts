// Browser-only stand-in used to check layouts in a plain web view (the window
// shows a banner saying so). It never runs encodes and is not used by tests.
import type { Bootstrap, Job, JobSettings, JobState, PreviewResult, ProbeEntry, QueueStatus } from "./types";

type Listener = (payload: unknown) => void;
const listeners = new Map<string, Set<Listener>>();
const emit = (event: string, payload: unknown) => listeners.get(event)?.forEach((l) => l(payload));

export function listen(event: string, cb: Listener): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(cb);
  return () => listeners.get(event)?.delete(cb);
}

const settings = (): JobSettings => ({
  video: {
    codec: "h264",
    encoder: null,
    mode: "lossy",
    level: 50,
    rate: { kind: "level" },
    speed: "balanced",
    transforms: { crop: null, scale: null, rotate: null, fps: null },
    allowBitDepthReduction: false,
    allowChromaSubsampling: false,
    hwDecode: true,
  },
  audio: { mode: "copy", codec: "aac", bitrateKbps: 192, tracks: "all", trackList: [], onIncompatible: "ask" },
  subtitles: { mode: "keep", burn: null, external: [] },
  output: { container: "mp4", dir: "besideSource", customDir: null, subfolder: "FrameEase 输出", nameTemplate: "{name}_{codec}", conflict: "rename", faststart: true, keepMetadata: true, keepChapters: true },
  trim: null,
  accept: [],
});

const src = (w: number, h: number, dur: number, size: number) => ({ container: "mov,mp4", video: "h264", width: w, height: h, fps: 30, duration: dur, size, audioTracks: 1, subtitleTracks: 0, hdr: false, bitDepth: 8 });

function job(id: string, name: string, state: JobState, extra: Partial<Job> = {}, codec: JobSettings["video"]["codec"] = "av1"): Job {
  const s = settings();
  s.video.codec = codec;
  return {
    id,
    spec: { kind: "convert", input: `/Users/demo/视频/${name}`, settings: s },
    name,
    state,
    progress: 0,
    stats: { fps: null, speed: null, outTime: null, sizeBytes: null, etaSecs: null, pass: null },
    source: src(1920, 1080, 252, 486_000_000),
    plan: { kind: "convert", video: codec, encoder: codec === "av1" ? "libsvtav1" : "libx264", quality: codec === "av1" ? "crf:34" : "crf:23", lossless: false, reencodeVideo: true, audio: "copy", subtitles: "keep", container: "mp4", estSize: null, estReliability: "unknown", threads: 10, hw: false, estMemoryMb: 1600, estDiskBytes: null },
    plannedOutput: `/Users/demo/视频/FrameEase 输出/${name.replace(/\.\w+$/, "")}_${codec}.mp4`,
    output: null,
    outputSize: null,
    tempFiles: [],
    issues: [],
    warnings: [],
    error: null,
    verify: null,
    attempts: 1,
    interrupted: false,
    createdAt: Date.now(),
    startedAt: null,
    finishedAt: null,
    elapsedMs: 0,
    logPath: "/tmp/x.log",
    pid: null,
    ...extra,
  };
}

const jobs: Job[] = [
  job("j1", "旅行 vlog #1 (最终版) & 東京タワー 'test'.mp4", "done", {
    progress: 1,
    output: "/Users/demo/视频/FrameEase 输出/旅行 vlog #1 (最终版) & 東京タワー 'test'_av1.mp4",
    outputSize: 141_000_000,
    elapsedMs: 312_000,
    verify: {
      ok: true,
      checks: [
        { name: "readable", ok: true, expected: "", actual: "mov,mp4,m4a,3gp,3g2,mj2" },
        { name: "videoCodec", ok: true, expected: "av1", actual: "av1" },
        { name: "dimensions", ok: true, expected: "1920×1080", actual: "1920×1080" },
        { name: "frameCount", ok: true, expected: "7560", actual: "7560" },
        { name: "duration", ok: true, expected: "252.00s", actual: "252.02s" },
        { name: "streamCopy:1", ok: true, expected: "packet sha256", actual: "identical" },
        { name: "decode", ok: true, expected: "", actual: "ok" },
      ],
    },
  }),
  job("j2", "家庭聚会 2026-10-01 全程录像 · 客厅机位 (4K HDR) 未剪辑版本 最终最终版.mov", "encoding", {
    progress: 0.43,
    stats: { fps: 63, speed: 2.1, outTime: 108, sizeBytes: 54_000_000, etaSecs: 84, pass: null },
  }),
  job("j3", "猫の動画 [AV1] 100%.mkv", "verifying", { progress: 0.93 }, "h264"),
  job("j4", "录音棚采访.mov", "needsInput", {
    issues: [{ code: "audioIncompatible", params: { track: "1", codec: "pcm_s16le", container: "mp4" }, fixes: [{ id: "audioTranscode" }, { id: "audioDrop" }, { id: "useMkv" }] }],
  }),
  job("j5", "损坏的文件.mp4", "failed", {
    error: { code: "inputCorrupt", params: {}, stage: "probing", detail: "[mov,mp4,m4a,3gp,3g2,mj2 @ 0x7f] moov atom not found\nfile:/Users/demo/视频/损坏的文件.mp4: Invalid data found when processing input" },
  }),
  job("j6", "毕业典礼.mp4", "cancelled"),
  job("j7", "第10集.mkv", "waiting", { interrupted: true }),
  job("j8", "第11集.mkv", "waiting"),
  job("j9", "第12集.mkv", "waiting"),
];

const status = (): QueueStatus => ({
  paused: false,
  running: jobs.filter((j) => ["probing", "encoding", "muxing", "verifying"].includes(j.state)).length,
  waiting: jobs.filter((j) => j.state === "waiting").length,
  needsInput: 1,
  done: 1,
  failed: 1,
  concurrency: { kind: "auto" },
  cpuSlots: 1,
  lightSlots: 1,
  threadsPerJob: 10,
  blocked: null,
  order: jobs.map((j) => j.id),
  recovered: 0,
});

// A slow, smooth tick so progress-bar motion can be judged in the preview.
setInterval(() => {
  const j = jobs[1];
  j.progress = j.progress >= 0.99 ? 0.05 : j.progress + 0.004;
  j.stats = { ...j.stats, outTime: j.progress * 252, etaSecs: Math.max(1, (1 - j.progress) * 150) };
  emit("fe://job", { ...j });
}, 250);

const bootstrap = (): Bootstrap => ({
  init: { kind: "ready" },
  caps: {
    ffmpeg: { ffmpeg: "/Applications/FrameEase.app/Contents/Resources/ffmpeg/ffmpeg", ffprobe: "", source: "bundled" },
    version: "8.1.2",
    license: "GPL v3",
    encoders: [
      { name: "libx264", hardware: false, usable: true, qualityMode: true, pixFmts: ["yuv420p", "yuv420p10le"], error: null },
      { name: "libx265", hardware: false, usable: true, qualityMode: true, pixFmts: ["yuv420p", "yuv420p10le"], error: null },
      { name: "libsvtav1", hardware: false, usable: true, qualityMode: true, pixFmts: ["yuv420p", "yuv420p10le"], error: null },
      { name: "ffv1", hardware: false, usable: true, qualityMode: true, pixFmts: ["yuv420p"], error: null },
      { name: "h264_videotoolbox", hardware: true, usable: true, qualityMode: true, pixFmts: ["nv12", "yuv420p"], error: null },
      { name: "hevc_videotoolbox", hardware: true, usable: true, qualityMode: true, pixFmts: ["nv12", "p010le"], error: null },
    ],
    audioEncoders: ["aac", "libopus", "flac", "libmp3lame"],
    decoders: [],
    filters: ["scale", "libvmaf", "psnr", "ssim"],
    muxers: [],
    hwaccels: ["videotoolbox"],
    hwDecode: ["videotoolbox:h264", "videotoolbox:hevc"],
    lossless: [
      { encoder: "libx264", pixFmt: "yuv420p", ok: true, detail: "8 frames identical" },
      { encoder: "libsvtav1", pixFmt: "yuv420p", ok: true, detail: "8 frames identical" },
    ],
    cpu: { logical: 10, physical: 10, performance: 4, brand: "Apple M5", arch: "aarch64" },
    memoryMb: 16384,
    detectedAt: Date.now(),
  },
  config: { ffmpegDir: null, concurrency: { kind: "auto" }, defaults: settings(), recursiveScan: false, theme: "system", locale: "zh-CN" },
  jobs,
  status: status(),
  manifest: { path: "/Users/demo/Library/Application Support/com.frameease.app/FrameEase 结果记录.csv", pending: 0, lastError: null },
  presets: [{ id: "builtin-h264-balanced", name: "H.264 · 均衡（兼容性最好）", settings: settings(), builtin: true }],
  app: { version: "0.1.0", dataDir: "/Users/demo/Library/Application Support/com.frameease.app", platform: "macos", arch: "aarch64" },
});

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  switch (cmd) {
    case "bootstrap":
      return bootstrap() as T;
    case "describe_settings": {
      const s = args!.settings as JobSettings;
      const enc = { h264: ["libx264", "x264", 23], hevc: ["libx265", "x265", 25], av1: ["libsvtav1", "SVT-AV1", 34], ffv1: ["ffv1", "FFV1", 0] }[s.video.codec];
      return {
        encoder: enc[0],
        label: s.video.speed === "gpu" ? "Apple VideoToolbox" : enc[1],
        hardware: s.video.speed === "gpu",
        quality: { kind: "CRF", value: Number(enc[2]) + Math.round((s.video.level - 50) / 7), min: 0, max: 51, lowerIsBetter: true, atBest: 16, atSmallest: 30 },
        twoPass: s.video.codec !== "av1",
        bitrate: true,
        losslessCapable: true,
        issue: null,
        options: [{ name: enc[0], label: enc[1], hardware: false, usable: true, reason: null }],
        gpuCodecs: ["h264", "hevc"],
      } as T;
    }
    case "scan_inputs":
      return { files: args!.paths as string[], skippedTemp: 0, skippedOutputDirs: 0, skippedNonVideo: 0, errors: [] } as T;
    case "probe_files":
      return (args!.paths as string[]).map((p): ProbeEntry => ({ path: p, info: null, error: "inputUnreadable" })) as T;
    case "plan_preview":
      return [] as PreviewResult[] as T;
    case "queue_snapshot":
      return { jobs, status: status() } as T;
    default:
      return undefined as T;
  }
}
