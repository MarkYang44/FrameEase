// Mirrors of the Rust (serde camelCase) types exchanged with the backend.

export type VideoCodec = "h264" | "hevc" | "av1" | "ffv1";
export type Container = "mp4" | "mkv";
export type VideoMode = "lossy" | "lossless" | "copy";
export type Speed = "gpu" | "fast" | "balanced" | "slow";

export type RateControl =
  | { kind: "level" }
  | { kind: "native"; value: number }
  | { kind: "bitrate"; kbps: number; twoPass: boolean }
  | { kind: "targetSize"; megabytes: number; twoPass: boolean };

export interface Crop {
  left: number;
  top: number;
  right: number;
  bottom: number;
}
export type Scale = { kind: "maxHeight"; height: number } | { kind: "exact"; width: number; height: number };
export type Rotate = "cw90" | "ccw90" | "r180" | "flipH" | "flipV";

export interface Transforms {
  crop: Crop | null;
  scale: Scale | null;
  rotate: Rotate | null;
  fps: number | null;
}

export interface VideoSettings {
  codec: VideoCodec;
  encoder: string | null;
  mode: VideoMode;
  level: number;
  rate: RateControl;
  speed: Speed;
  transforms: Transforms;
  allowBitDepthReduction: boolean;
  allowChromaSubsampling: boolean;
  hwDecode: boolean;
}

export type AudioMode = "copy" | "transcode" | "none";
export type AudioCodec = "aac" | "opus" | "mp3" | "flac";
export type TrackSelection = "all" | "first" | "list";
export type IncompatiblePolicy = "ask" | "transcode" | "drop";

export interface AudioSettings {
  mode: AudioMode;
  codec: AudioCodec;
  bitrateKbps: number;
  tracks: TrackSelection;
  trackList: number[];
  onIncompatible: IncompatiblePolicy;
}

export type SubtitleMode = "none" | "keep" | "burn";
export type BurnSource = { kind: "embedded"; ordinal: number } | { kind: "external"; path: string };
export interface ExternalSubtitle {
  path: string;
  language: string | null;
}
export interface SubtitleSettings {
  mode: SubtitleMode;
  burn: BurnSource | null;
  external: ExternalSubtitle[];
}

export type OutputDir = "besideSource" | "custom";
export type ConflictPolicy = "rename" | "skip";
export interface OutputSettings {
  container: Container;
  dir: OutputDir;
  customDir: string | null;
  subfolder: string;
  nameTemplate: string;
  conflict: ConflictPolicy;
  faststart: boolean;
  keepMetadata: boolean;
  keepChapters: boolean;
}

export interface Trim {
  start: number;
  end: number | null;
}

export interface JobSettings {
  video: VideoSettings;
  audio: AudioSettings;
  subtitles: SubtitleSettings;
  output: OutputSettings;
  trim: Trim | null;
  accept: string[];
}

export type TrackKind = "audio" | "subtitle";
export interface MuxTrack {
  path: string;
  kind: TrackKind;
  language: string | null;
  title: string | null;
  default: boolean;
}
export interface MuxSpec {
  main: string;
  keepMainAudio: boolean;
  keepMainSubtitles: boolean;
  tracks: MuxTrack[];
  output: OutputSettings;
}
export interface ExtractSpec {
  input: string;
  streamIndex: number;
  output: OutputSettings;
}

export type JobSpec =
  | { kind: "convert"; input: string; settings: JobSettings }
  | ({ kind: "mux" } & MuxSpec)
  | ({ kind: "extract" } & ExtractSpec);

export type JobState = "waiting" | "needsInput" | "probing" | "encoding" | "muxing" | "verifying" | "done" | "cancelled" | "failed";

export interface Notice {
  code: string;
  params: Record<string, string>;
}
export interface Issue extends Notice {
  fixes: { id: string }[];
}
export interface JobError extends Notice {
  stage: JobState;
  detail: string | null;
}

export interface LiveStats {
  fps: number | null;
  speed: number | null;
  outTime: number | null;
  sizeBytes: number | null;
  etaSecs: number | null;
  pass: string | null;
}

export interface Check {
  name: string;
  ok: boolean;
  expected: string;
  actual: string;
}
export interface VerifyReport {
  ok: boolean;
  checks: Check[];
}

export interface PlanSummary {
  kind: string;
  video: string;
  encoder: string | null;
  quality: string | null;
  lossless: boolean;
  reencodeVideo: boolean;
  audio: string;
  subtitles: string;
  container: string;
  estSize: number | null;
  estReliability: "exact" | "estimate" | "unknown" | string;
  threads: number | null;
  hw: boolean;
  estMemoryMb: number | null;
  estDiskBytes: number | null;
}

export interface SourceSummary {
  container: string;
  video: string | null;
  width: number | null;
  height: number | null;
  fps: number | null;
  duration: number | null;
  size: number;
  audioTracks: number;
  subtitleTracks: number;
  hdr: boolean;
  bitDepth: number | null;
}

export interface Job {
  id: string;
  spec: JobSpec;
  name: string;
  state: JobState;
  progress: number;
  stats: LiveStats;
  source: SourceSummary | null;
  plan: PlanSummary | null;
  plannedOutput: string | null;
  output: string | null;
  outputSize: number | null;
  tempFiles: string[];
  issues: Issue[];
  warnings: Notice[];
  error: JobError | null;
  verify: VerifyReport | null;
  attempts: number;
  interrupted: boolean;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  elapsedMs: number;
  logPath: string | null;
  pid: number | null;
}

export type Concurrency = { kind: "auto" } | { kind: "fixed"; n: number };

export interface QueueStatus {
  paused: boolean;
  running: number;
  waiting: number;
  needsInput: number;
  done: number;
  failed: number;
  concurrency: Concurrency;
  cpuSlots: number;
  lightSlots: number;
  threadsPerJob: number;
  blocked: string | null;
  order: string[];
  recovered: number;
}

export interface ManifestStatus {
  path: string;
  pending: number;
  lastError: string | null;
}

export interface EncoderCaps {
  name: string;
  hardware: boolean;
  usable: boolean;
  qualityMode: boolean;
  pixFmts: string[];
  error: string | null;
}
export interface LosslessCheck {
  encoder: string;
  pixFmt: string;
  ok: boolean;
  detail: string;
}
export interface CpuInfo {
  logical: number;
  physical: number | null;
  performance: number | null;
  brand: string;
  arch: string;
}
export interface Capabilities {
  ffmpeg: { ffmpeg: string; ffprobe: string; source: "custom" | "bundled" | "portable" | "system" };
  version: string;
  license: string;
  encoders: EncoderCaps[];
  audioEncoders: string[];
  decoders: string[];
  filters: string[];
  muxers: string[];
  hwaccels: string[];
  hwDecode: string[];
  lossless: LosslessCheck[];
  cpu: CpuInfo;
  memoryMb: number;
  detectedAt: number;
}

export interface Rational {
  num: number;
  den: number;
}
export interface VideoStream {
  index: number;
  codec: string;
  codecLong: string;
  profile: string | null;
  level: number | null;
  width: number;
  height: number;
  sar: string | null;
  dar: string | null;
  pixFmt: string | null;
  bitDepth: number | null;
  chroma: string | null;
  alpha: boolean;
  rgb: boolean;
  colorRange: string | null;
  colorSpace: string | null;
  colorTransfer: string | null;
  colorPrimaries: string | null;
  hdr: "sdr" | "pq" | "hlg";
  masteringDisplay: { maxLuminance: number; minLuminance: number } | null;
  contentLight: { maxCll: number; maxFall: number } | null;
  dolbyVision: { profile: number; blCompatId: number } | null;
  rotation: number;
  rFrameRate: Rational | null;
  avgFrameRate: Rational | null;
  fps: number | null;
  vfr: boolean;
  fieldOrder: string | null;
  interlaced: boolean;
  nbFrames: number | null;
  duration: number | null;
  bitRate: number | null;
  attachedPic: boolean;
  default: boolean;
  language: string | null;
  title: string | null;
}
export interface AudioStream {
  index: number;
  codec: string;
  codecLong: string;
  profile: string | null;
  channels: number;
  channelLayout: string | null;
  sampleRate: number | null;
  sampleFmt: string | null;
  bitRate: number | null;
  duration: number | null;
  language: string | null;
  title: string | null;
  default: boolean;
}
export interface SubtitleStream {
  index: number;
  codec: string;
  language: string | null;
  title: string | null;
  default: boolean;
  forced: boolean;
  bitmap: boolean;
}
export interface MediaInfo {
  path: string;
  size: number;
  formatName: string;
  formatLongName: string;
  duration: number | null;
  startTime: number | null;
  bitRate: number | null;
  video: VideoStream[];
  audio: AudioStream[];
  subtitles: SubtitleStream[];
  attachments: number;
  dataStreams: number;
  chapters: number;
  tags: Record<string, string>;
}

export interface ProbeEntry {
  path: string;
  info: MediaInfo | null;
  error: string | null;
}

export interface ScanResult {
  files: string[];
  skippedTemp: number;
  skippedOutputDirs: number;
  skippedNonVideo: number;
  errors: string[];
}

export interface PreviewResult {
  path: string;
  ok: boolean;
  issues: Issue[];
  warnings: Notice[];
  summary: PlanSummary | null;
  plannedOutput: string | null;
  error: string | null;
}

export interface QualityInfo {
  kind: "CRF" | "CQ" | "q:v" | "ICQ" | "QP";
  value: number;
  min: number;
  max: number;
  lowerIsBetter: boolean;
  atBest: number;
  atSmallest: number;
}
export interface EncoderOption {
  name: string;
  label: string;
  hardware: boolean;
  usable: boolean;
  reason: string | null;
}
export interface EncoderChoice {
  encoder: string | null;
  label: string | null;
  hardware: boolean;
  quality: QualityInfo | null;
  twoPass: boolean;
  bitrate: boolean;
  losslessCapable: boolean;
  issue: Issue | null;
  options: EncoderOption[];
  gpuCodecs: string[];
}

export interface TrialResult {
  output: string;
  start: number;
  segmentSecs: number;
  encodeSecs: number;
  bytes: number;
  sourceSegmentBytes: number | null;
  estFullBytes: number | null;
  speed: number | null;
  psnr: number | null;
  ssim: number | null;
  vmaf: number | null;
  summary: PlanSummary;
  warnings: Notice[];
}

export interface Preset {
  id: string;
  name: string;
  settings: JobSettings;
  builtin: boolean;
}

export interface AppConfig {
  ffmpegDir: string | null;
  concurrency: Concurrency;
  defaults: JobSettings;
  recursiveScan: boolean;
  theme: "system" | "light" | "dark" | string;
  locale: string;
}

export type InitStatus = { kind: "pending" } | { kind: "noBackend" } | { kind: "backendError"; message: string } | { kind: "ready" };

export interface AppInfo {
  version: string;
  dataDir: string;
  platform: string;
  arch: string;
}

export interface Bootstrap {
  init: InitStatus;
  caps: Capabilities | null;
  config: AppConfig;
  jobs: Job[];
  status: QueueStatus | null;
  manifest: ManifestStatus | null;
  presets: Preset[];
  app: AppInfo;
}

export const ACTIVE_STATES: JobState[] = ["probing", "encoding", "muxing", "verifying"];
export const isActive = (s: JobState) => ACTIVE_STATES.includes(s);
export const isTerminal = (s: JobState) => s === "done" || s === "cancelled" || s === "failed";
