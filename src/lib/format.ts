import { t } from "./i18n";

// Number formatting. Sizes follow the platform convention (macOS Finder uses
// 1000-based units, Windows Explorer 1024-based) so values match the file manager.

const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform || navigator.userAgent);
const BASE = isMac ? 1000 : 1024;

export function bytes(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = n;
  let i = 0;
  while (v >= BASE && i < units.length - 1) {
    v /= BASE;
    i++;
  }
  const digits = i === 0 ? 0 : v >= 100 ? 0 : 1;
  return `${v.toFixed(digits)} ${units[i]}`;
}

/** 83.4 → "1:23", 3725 → "1:02:05" */
export function clock(secs: number | null | undefined): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs) || secs < 0) return "—";
  const s = Math.floor(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return h > 0 ? `${h}:${mm}:${String(r).padStart(2, "0")}` : `${mm}:${String(r).padStart(2, "0")}`;
}

/** Timecode with tenths, for the trim view: "1:02:05.3" */
export function timecode(secs: number): string {
  const tenths = Math.floor((secs % 1) * 10);
  return `${clock(secs)}.${tenths}`;
}

/** Parse "1:23", "83.5", "1:02:05.3" → seconds */
export function parseTime(text: string): number | null {
  const parts = text.trim().split(":");
  if (parts.some((p) => p.trim() === "" || Number.isNaN(Number(p)))) return null;
  return parts.reduce((acc, p) => acc * 60 + Number(p), 0);
}

export function humanDuration(secs: number | null | undefined): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs)) return "—";
  if (secs < 60) return t("time.seconds", { n: Math.max(1, Math.round(secs)) });
  const m = Math.floor(secs / 60);
  const h = Math.floor(m / 60);
  if (h > 0) return t("time.hoursMinutes", { h, m: m % 60 });
  return t("time.minutesSeconds", { m, s: Math.round(secs % 60) });
}

export function percent(p: number): string {
  return `${Math.floor(Math.max(0, Math.min(1, p)) * 100)}%`;
}

export function ratioLabel(out: number, src: number): { smaller: boolean; text: string } {
  const r = out / src;
  return r <= 1 ? { smaller: true, text: `${Math.round((1 - r) * 100)}%` } : { smaller: false, text: `${Math.round((r - 1) * 100)}%` };
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function dirName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}

/** Keep both ends of long names visible: "旅行 vlog #1 (最终…东京塔.mp4" */
export function middleTruncate(s: string, max = 48): string {
  if (s.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep * 0.62);
  return `${s.slice(0, head)}…${s.slice(s.length - (keep - head))}`;
}

export const codecLabel = (c: string | null | undefined) =>
  ({ h264: "H.264", hevc: "HEVC", av1: "AV1", ffv1: "FFV1", vp9: "VP9", prores: "ProRes", mpeg4: "MPEG-4", mpeg2video: "MPEG-2" })[c ?? ""] ??
  (c ? c.toUpperCase() : "—");

export function fps(v: number | null | undefined): string {
  if (!v) return "—";
  return `${Number.isInteger(v) ? v : v.toFixed(3).replace(/0+$/, "")} fps`;
}
