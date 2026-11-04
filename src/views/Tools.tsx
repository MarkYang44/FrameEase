import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { Copy, FileVideo2 } from "lucide-react";
import { api, fileSrc, pickPaths, VIDEO_EXTENSIONS } from "../lib/api";
import { bytes, clock, codecLabel, fileName, fps, parseTime, timecode } from "../lib/format";
import { notice, t } from "../lib/i18n";
import { clone, useStore } from "../lib/store";
import type { JobSpec, MediaInfo, MuxTrack } from "../lib/types";
import { Button, Check, Field, Segmented, Slider } from "../components/ui";

const LANGS = [
  ["chi", "language.chi"],
  ["eng", "language.eng"],
  ["jpn", "language.jpn"],
  ["kor", "language.kor"],
  ["fre", "language.fre"],
  ["ger", "language.ger"],
  ["spa", "language.spa"],
  ["rus", "language.rus"],
  ["und", "language.und"],
];

/** One media file chosen by picker or drop, with its probe result. */
function useSingleFile() {
  const [path, setPath] = useState<string | null>(null);
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async (p: string) => {
    setPath(p);
    setInfo(null);
    setError(null);
    try {
      const [e] = await api.probeFiles([p]);
      if (e.info) setInfo(e.info);
      else setError(e.error ?? "inputCorrupt");
    } catch (err) {
      setError(String(err));
    }
  }, []);
  // Drops onto the window go to this page while it is open.
  useEffect(() => {
    useStore.setState({ dropHandler: (paths) => paths[0] && load(paths[0]) });
    return () => useStore.setState({ dropHandler: null });
  }, [load]);
  const pick = async () => {
    const [p] = await pickPaths({ multiple: false, filters: [{ name: t("common.video"), extensions: VIDEO_EXTENSIONS }, { name: t("common.allFiles"), extensions: ["*"] }] });
    if (p) load(p);
  };
  return { path, info, error, pick, load };
}

function FilePickRow({ path, info, error, pick, label }: { path: string | null; info: MediaInfo | null; error: string | null; pick: () => void; label: string }) {
  return (
    <div className="row row--wrap">
      <Button onClick={pick}>
        <FileVideo2 size={15} /> {label}
      </Button>
      {path && (
        <span className="small" title={path} style={{ overflowWrap: "anywhere" }}>
          {fileName(path)}
          {info && <span className="muted"> ({clock(info.duration)}, {bytes(info.size)})</span>}
        </span>
      )}
      {error && <span className="small bad-text">{t("info.readFailed", { error: notice("err", { code: error }) })}</span>}
    </div>
  );
}

export function TrimView() {
  const { path, info, error, pick } = useSingleFile();
  const settings = useStore((st) => st.settings);
  const [start, setStart] = useState(0);
  const [end, setEnd] = useState(0);
  const [mode, setMode] = useState<"fast" | "precise">("fast");
  const [keyframes, setKeyframes] = useState<number[]>([]);
  const [frames, setFrames] = useState<{ a: string | null; b: string | null }>({ a: null, b: null });
  const duration = info?.duration ?? 0;

  useEffect(() => {
    if (!info || !path) return;
    setStart(0);
    setEnd(Math.floor(duration * 10) / 10);
    setKeyframes([]);
    api.keyframes(path).then(setKeyframes).catch(() => setKeyframes([]));
  }, [info, path, duration]);

  useEffect(() => {
    if (!path || !info) return;
    let alive = true;
    const timer = setTimeout(async () => {
      try {
        const [a, b] = await Promise.all([api.extractFrame(path, start, 640, "in"), api.extractFrame(path, Math.max(0, end - 0.05), 640, "out")]);
        if (alive) setFrames({ a, b });
      } catch {
        /* best effort */
      }
    }, 220);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [path, info, start, end]);

  const kf = useMemo(() => keyframes.filter((k) => k <= start + 1e-6).pop() ?? null, [keyframes, start]);
  const visibleKf = useMemo(() => (keyframes.length > 400 ? keyframes.filter((_, i) => i % Math.ceil(keyframes.length / 400) === 0) : keyframes), [keyframes]);

  const add = async () => {
    if (!path || !settings) return;
    const s = clone(settings);
    s.trim = { start, end: end < duration - 0.05 ? end : null };
    if (mode === "fast") s.video.mode = "copy";
    else if (s.video.mode === "copy") s.video.mode = "lossy";
    s.output.nameTemplate = `${s.output.nameTemplate.replace("{name}", t("trim.nameTemplate"))}`;
    try {
      await api.enqueue([{ kind: "convert", input: path, settings: s }]);
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success(t("queue.added"));
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <div className="page">
      <div className="page__inner">
        <h1 className="page__title">{t("trim.title")}</h1>
        <FilePickRow path={path} info={info} error={error} pick={pick} label={t("trim.pick")} />
        {!path && <p className="muted">{t("trim.dropHint")}</p>}
        {info && (
          <>
            <div className="range-strip" aria-label={t("trim.keyframes")}>
              <div className="range-strip__sel" style={{ left: `${(start / duration) * 100}%`, width: `${((end - start) / duration) * 100}%` }} />
              {visibleKf.map((k) => (
                <span key={k} className="range-strip__kf" style={{ left: `${(k / duration) * 100}%` }} />
              ))}
            </div>
            <div className="frame-pair">
              <div className="block">
                <Field label={`${t("trim.start")} ${timecode(start)}`}>
                  <Slider label={t("trim.start")} min={0} max={duration} step={0.1} value={start} onChange={(v) => setStart(Math.min(v, end - 0.1))} />
                  <TimeInput value={start} onCommit={(v) => setStart(Math.max(0, Math.min(v, end - 0.1)))} />
                </Field>
                <div className="frame">{frames.a ? <img src={fileSrc(frames.a)} alt={t("trim.startFrame")} /> : "…"}</div>
              </div>
              <div className="block">
                <Field label={`${t("trim.end")} ${timecode(end)}`}>
                  <Slider label={t("trim.end")} min={0} max={duration} step={0.1} value={end} onChange={(v) => setEnd(Math.max(v, start + 0.1))} />
                  <TimeInput value={end} onCommit={(v) => setEnd(Math.min(duration, Math.max(v, start + 0.1)))} />
                </Field>
                <div className="frame">{frames.b ? <img src={fileSrc(frames.b)} alt={t("trim.endFrame")} /> : "…"}</div>
              </div>
            </div>
            <Field label={t("trim.mode")}>
              <Segmented
                label={t("trim.mode")}
                value={mode}
                onChange={setMode}
                options={[
                  { value: "fast", label: t("trim.fast") },
                  { value: "precise", label: t("trim.precise") },
                ]}
              />
            </Field>
            <p className="small muted" style={{ margin: 0 }}>
              {mode === "fast"
                ? kf !== null && start - kf > 0.05
                  ? t("trim.fastHint", { kf: timecode(kf), d: t("time.seconds", { n: (start - kf).toFixed(1) }) })
                  : t("trim.fastHintExact")
                : t("trim.preciseHint")}
            </p>
            <div>
              <Button variant="primary" size="lg" onClick={add}>
                {t("trim.add")}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function TimeInput({ value, onCommit }: { value: number; onCommit: (v: number) => void }) {
  const [text, setText] = useState(timecode(value));
  useEffect(() => setText(timecode(value)), [value]);
  return (
    <input
      className="input"
      style={{ width: 120 }}
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        const v = parseTime(text);
        if (v === null) setText(timecode(value));
        else onCommit(v);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
      }}
      aria-label={t("trim.timeLabel")}
    />
  );
}

export function MuxView() {
  const [tab, setTab] = useState<"mux" | "extract">("mux");
  return (
    <div className="page">
      <div className="page__inner">
        <h1 className="page__title">{t("mux.title")}</h1>
        <div className="tabs" role="tablist">
          <button className="tab" role="tab" aria-selected={tab === "mux"} onClick={() => setTab("mux")}>
            {t("mux.tabMux")}
          </button>
          <button className="tab" role="tab" aria-selected={tab === "extract"} onClick={() => setTab("extract")}>
            {t("mux.tabExtract")}
          </button>
        </div>
        {tab === "mux" ? <MuxTab /> : <ExtractTab />}
      </div>
    </div>
  );
}

function MuxTab() {
  const { path, info, error, pick } = useSingleFile();
  const settings = useStore((st) => st.settings);
  const [tracks, setTracks] = useState<MuxTrack[]>([]);
  const [keepAudio, setKeepAudio] = useState(true);
  const [keepSubs, setKeepSubs] = useState(true);
  const [container, setContainer] = useState<"mkv" | "mp4">("mkv");

  const addTracks = async (kind: "audio" | "subtitle") => {
    const exts = kind === "audio" ? ["m4a", "aac", "mp3", "ac3", "eac3", "flac", "opus", "wav", "mka", "dts", "mp4", "mkv"] : ["srt", "ass", "ssa", "vtt", "sup", "mks"];
    const paths = await pickPaths({ multiple: true, filters: [{ name: kind === "audio" ? t("common.audio") : t("common.subtitles"), extensions: exts }] });
    setTracks((ts) => [...ts, ...paths.map((p) => ({ path: p, kind, language: null, title: null, default: false }))]);
  };
  const run = async () => {
    if (!path || !settings) return;
    const spec: JobSpec = { kind: "mux", main: path, keepMainAudio: keepAudio, keepMainSubtitles: keepSubs, tracks, output: { ...settings.output, container, nameTemplate: t("mux.nameTemplate") } };
    try {
      await api.enqueue([spec]);
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success(t("queue.added"));
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <div className="block">
      <FilePickRow path={path} info={info} error={error} pick={pick} label={t("mux.main")} />
      {info && (
        <>
          <Check checked={keepAudio} onChange={setKeepAudio}>
            {t("mux.keepAudio")} ({info.audio.length})
          </Check>
          <Check checked={keepSubs} onChange={setKeepSubs}>
            {t("mux.keepSubs")} ({info.subtitles.length})
          </Check>
          {tracks.length > 0 && (
            <table className="table">
              <thead>
                <tr>
                  <th>{t("common.file")}</th>
                  <th>{t("mux.language")}</th>
                  <th>{t("mux.trackTitle")}</th>
                  <th>{t("mux.default")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {tracks.map((tr, i) => (
                  <tr key={tr.path + i}>
                    <td title={tr.path}>
                      {tr.kind === "audio" ? t("common.audio") : t("common.subtitles")}: {fileName(tr.path)}
                    </td>
                    <td>
                      <select className="select" value={tr.language ?? "und"} onChange={(e) => setTracks((ts) => ts.map((x, j) => (j === i ? { ...x, language: e.target.value } : x)))}>
                        {LANGS.map(([code, name]) => (
                          <option key={code} value={code}>
                            {t(name)}
                          </option>
                        ))}
                      </select>
                    </td>
                    <td>
                      <input className="input" value={tr.title ?? ""} onChange={(e) => setTracks((ts) => ts.map((x, j) => (j === i ? { ...x, title: e.target.value || null } : x)))} />
                    </td>
                    <td>
                      <input type="checkbox" checked={tr.default} onChange={(e) => setTracks((ts) => ts.map((x, j) => (j === i ? { ...x, default: e.target.checked } : x)))} />
                    </td>
                    <td>
                      <button className="icon-btn" aria-label={t("common.remove")} onClick={() => setTracks((ts) => ts.filter((_, j) => j !== i))}>
                        ×
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <div className="row">
            <Button onClick={() => addTracks("audio")}>{t("mux.addAudio")}</Button>
            <Button onClick={() => addTracks("subtitle")}>{t("mux.addSubtitle")}</Button>
          </div>
          <Field label={t("out.container")}>
            <div style={{ width: 200 }}>
              <Segmented
                label={t("out.container")}
                value={container}
                onChange={setContainer}
                options={[
                  { value: "mkv", label: "MKV" },
                  { value: "mp4", label: "MP4" },
                ]}
              />
            </div>
          </Field>
          <p className="small muted" style={{ margin: 0 }}>
            {t("mux.hint")}
          </p>
          <div>
            <Button variant="primary" size="lg" onClick={run} disabled={tracks.length === 0 && keepAudio && keepSubs}>
              {t("mux.run")}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

function ExtractTab() {
  const { path, info, error, pick } = useSingleFile();
  const settings = useStore((st) => st.settings);
  const [chosen, setChosen] = useState<number[]>([]);
  useEffect(() => setChosen([]), [path]);
  const streams = info
    ? [
        ...info.video.filter((v) => !v.attachedPic).map((v) => ({ index: v.index, kind: t("common.video"), desc: `${codecLabel(v.codec)} ${v.width}×${v.height} ${fps(v.fps)}` })),
        ...info.audio.map((a) => ({ index: a.index, kind: t("common.audio"), desc: `${a.codec} ${t("info.channelCount", { n: a.channels })} ${a.language ?? ""} ${a.title ?? ""}` })),
        ...info.subtitles.map((s) => ({ index: s.index, kind: t("common.subtitles"), desc: `${s.codec}${s.bitmap ? t("info.bitmap") : ""} ${s.language ?? ""} ${s.title ?? ""}` })),
      ]
    : [];
  const run = async () => {
    if (!path || !settings) return;
    try {
      await api.enqueue(chosen.map((i) => ({ kind: "extract" as const, input: path, streamIndex: i, output: settings.output })));
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success(t("queue.addedCount", { n: chosen.length }));
    } catch (e) {
      toast.error(String(e));
    }
  };
  return (
    <div className="block">
      <FilePickRow path={path} info={info} error={error} pick={pick} label={t("ext.pick")} />
      {info && (
        <>
          <table className="table">
            <thead>
              <tr>
                <th />
                <th>#</th>
                <th>{t("common.type")}</th>
                <th>{t("common.contents")}</th>
              </tr>
            </thead>
            <tbody>
              {streams.map((s) => (
                <tr key={s.index}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={t("info.trackLabel", { n: s.index })}
                      checked={chosen.includes(s.index)}
                      onChange={(e) => setChosen((c) => (e.target.checked ? [...c, s.index] : c.filter((x) => x !== s.index)))}
                    />
                  </td>
                  <td>{s.index}</td>
                  <td>{s.kind}</td>
                  <td>{s.desc}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="small muted" style={{ margin: 0 }}>
            {t("ext.hint")}
          </p>
          <div>
            <Button variant="primary" size="lg" disabled={chosen.length === 0} onClick={run}>
              {t("ext.run", { n: chosen.length })}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

export function infoText(m: MediaInfo): string {
  const lines = [
    `${t("common.file")}: ${m.path}`,
    `${t("info.container")}: ${m.formatLongName || m.formatName}, ${bytes(m.size)}, ${t("info.duration")}: ${clock(m.duration)}, ${t("info.totalBitrate")}: ${m.bitRate ? Math.round(m.bitRate / 1000) + " kbps" : "—"}`,
  ];
  m.video.forEach((v) => lines.push(
    `${t("info.video")} #${v.index}: ${codecLabel(v.codec)} ${v.profile ?? ""} ${v.width}×${v.height}, ${fps(v.fps)} (${v.vfr ? t("info.vfr") : t("info.cfr")}), ${v.pixFmt ?? "?"}${v.bitDepth ? ` ${v.bitDepth}-bit` : ""}, ${t("info.color")}: ${v.colorPrimaries ?? "?"}/${v.colorTransfer ?? "?"}/${v.colorSpace ?? "?"} ${v.colorRange ?? ""}${v.hdr !== "sdr" ? `, HDR ${v.hdr.toUpperCase()}` : ""}${v.rotation ? `, ${t("info.rotation", { deg: v.rotation })}` : ""}${v.attachedPic ? t("info.cover") : ""}`,
  ));
  m.audio.forEach((a) => lines.push(`${t("info.audio")} #${a.index}: ${a.codec} ${t("info.channelCount", { n: a.channels })} ${a.sampleRate ?? "?"} Hz ${a.bitRate ? Math.round(a.bitRate / 1000) + " kbps" : ""} ${a.language ?? ""} ${a.title ?? ""}`));
  m.subtitles.forEach((s) => lines.push(`${t("info.subtitles")} #${s.index}: ${s.codec}${s.bitmap ? t("info.bitmap") : ""} ${s.language ?? ""} ${s.title ?? ""}`));
  if (m.chapters) lines.push(t("info.chapters", { n: m.chapters }));
  return lines.join("\n");
}

export function InfoView() {
  const { path, info, error, pick } = useSingleFile();
  return (
    <div className="page">
      <div className="page__inner">
        <h1 className="page__title">{t("info.title")}</h1>
        <div className="row">
          <FilePickRow path={path} info={info} error={error} pick={pick} label={t("info.pick")} />
          <span className="grow" />
          {info && (
            <Button
              variant="ghost"
              onClick={() =>
                navigator.clipboard
                  .writeText(infoText(info))
                  .then(() => toast.success(t("info.copied")))
                  .catch(() => toast.error(t("info.clipboardFailed")))
              }
            >
              <Copy size={14} /> {t("info.copy")}
            </Button>
          )}
        </div>
        {!path && <p className="muted">{t("info.dropHint")}</p>}
        {info && (
          <>
            <section className="block">
              <div className="block__title">{t("info.container")}</div>
              <dl className="kv selectable">
                <dt>{t("info.format")}</dt>
                <dd>{info.formatLongName || info.formatName}</dd>
                <dt>{t("info.size")}</dt>
                <dd>{bytes(info.size)}</dd>
                <dt>{t("info.duration")}</dt>
                <dd>{clock(info.duration)}</dd>
                <dt>{t("info.totalBitrate")}</dt>
                <dd>{info.bitRate ? `${Math.round(info.bitRate / 1000)} kbps` : "—"}</dd>
                <dt>{t("info.chaptersAttachments")}</dt>
                <dd>
                  {info.chapters} / {info.attachments}
                </dd>
              </dl>
            </section>
            {info.video.map((v) => (
              <section className="block" key={v.index}>
                <div className="block__title">
                  {t("info.video")} #{v.index}
                  {v.attachedPic ? t("info.cover") : ""}
                </div>
                <dl className="kv selectable">
                  <dt>{t("info.codec")}</dt>
                  <dd>
                    {codecLabel(v.codec)} {v.profile ?? ""} {v.level ? `Level ${v.level}` : ""}
                  </dd>
                  <dt>{t("info.dimensions")}</dt>
                  <dd>
                    {v.width}×{v.height}
                    {v.sar && v.sar !== "1:1" ? ` (SAR ${v.sar}, DAR ${v.dar})` : ""}
                    {v.rotation ? `, ${t("info.rotation", { deg: v.rotation })}` : ""}
                  </dd>
                  <dt>{t("info.frameRate")}</dt>
                  <dd>
                    {fps(v.fps)}, {v.vfr ? t("info.vfr") : t("info.cfr")}
                    {v.nbFrames ? t("info.frameCount", { n: v.nbFrames }) : ""}
                    {v.interlaced ? `, ${t("info.interlaced")} (${v.fieldOrder})` : ""}
                  </dd>
                  <dt>{t("info.pixelFormat")}</dt>
                  <dd>
                    {v.pixFmt ?? "—"}
                    {v.bitDepth ? `, ${v.bitDepth}-bit` : ""}
                    {v.chroma ? `, ${v.chroma === "gray" ? t("info.gray") : v.chroma.split("").join(":")}` : ""}
                    {v.alpha ? t("info.alpha") : ""}
                  </dd>
                  <dt>{t("info.color")}</dt>
                  <dd>
                    {t("info.colors", { primaries: v.colorPrimaries ?? t("info.unspecified"), transfer: v.colorTransfer ?? t("info.unspecified"), matrix: v.colorSpace ?? t("info.unspecified"), range: v.colorRange === "pc" ? t("info.fullRange") : v.colorRange === "tv" ? t("info.limitedRange") : t("info.unspecified") })}
                  </dd>
                  <dt>HDR</dt>
                  <dd>
                    {v.hdr === "sdr" ? "SDR" : v.hdr === "pq" ? "HDR10 (PQ)" : "HLG"}
                    {v.masteringDisplay ? t("info.mastering", { max: v.masteringDisplay.maxLuminance, min: v.masteringDisplay.minLuminance }) : ""}
                    {v.contentLight ? `, MaxCLL ${v.contentLight.maxCll}, MaxFALL ${v.contentLight.maxFall}` : ""}
                    {v.dolbyVision ? t("info.dolbyVision", { profile: v.dolbyVision.profile }) : ""}
                  </dd>
                  <dt>{t("info.bitrate")}</dt>
                  <dd>{v.bitRate ? `${Math.round(v.bitRate / 1000)} kbps` : "—"}</dd>
                </dl>
              </section>
            ))}
            {info.audio.map((a) => (
              <section className="block" key={a.index}>
                <div className="block__title">
                  {t("info.audio")} #{a.index}
                </div>
                <dl className="kv selectable">
                  <dt>{t("info.codec")}</dt>
                  <dd>
                    {a.codec} {a.profile ?? ""}
                  </dd>
                  <dt>{t("info.channels")}</dt>
                  <dd>
                    {a.channels} ({a.channelLayout ?? "?"}), {a.sampleRate ?? "?"} Hz
                  </dd>
                  <dt>{t("info.bitrate")}</dt>
                  <dd>{a.bitRate ? `${Math.round(a.bitRate / 1000)} kbps` : "—"}</dd>
                  <dt>{t("info.languageTitle")}</dt>
                  <dd>
                    {a.language ?? "—"} / {a.title ?? "—"}
                    {a.default ? t("info.default") : ""}
                  </dd>
                </dl>
              </section>
            ))}
            {info.subtitles.length > 0 && (
              <section className="block">
                <div className="block__title">{t("info.subtitles")}</div>
                <table className="table selectable">
                  <tbody>
                    {info.subtitles.map((s) => (
                      <tr key={s.index}>
                        <td>#{s.index}</td>
                        <td>
                          {s.codec}
                          {s.bitmap ? t("info.bitmap") : ""}
                        </td>
                        <td>{s.language ?? "—"}</td>
                        <td>{s.title ?? ""}</td>
                        <td>
                          {s.default ? t("info.defaultFlag") : ""}
                          {s.forced ? t("info.forced") : ""}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            )}
          </>
        )}
      </div>
    </div>
  );
}
