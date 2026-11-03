import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { Copy, FileVideo2 } from "lucide-react";
import { api, fileSrc, pickPaths, VIDEO_EXTENSIONS } from "../lib/api";
import { bytes, clock, codecLabel, fileName, fps, parseTime, timecode } from "../lib/format";
import { t } from "../lib/i18n";
import { clone, useStore } from "../lib/store";
import type { JobSpec, MediaInfo, MuxTrack } from "../lib/types";
import { Button, Check, Field, Segmented, Slider } from "../components/ui";

const LANGS = [
  ["chi", "中文"],
  ["eng", "英语"],
  ["jpn", "日语"],
  ["kor", "韩语"],
  ["fre", "法语"],
  ["ger", "德语"],
  ["spa", "西班牙语"],
  ["rus", "俄语"],
  ["und", "未指定"],
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
    const [p] = await pickPaths({ multiple: false, filters: [{ name: "视频", extensions: VIDEO_EXTENSIONS }, { name: "所有文件", extensions: ["*"] }] });
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
          {info && <span className="muted">（{clock(info.duration)}，{bytes(info.size)}）</span>}
        </span>
      )}
      {error && <span className="small bad-text">无法读取：{error}</span>}
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
    s.output.nameTemplate = `${s.output.nameTemplate.replace("{name}", "{name}_片段")}`;
    try {
      await api.enqueue([{ kind: "convert", input: path, settings: s }]);
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success("已加入队列");
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
                <div className="frame">{frames.a ? <img src={fileSrc(frames.a)} alt="起点画面" /> : "…"}</div>
              </div>
              <div className="block">
                <Field label={`${t("trim.end")} ${timecode(end)}`}>
                  <Slider label={t("trim.end")} min={0} max={duration} step={0.1} value={end} onChange={(v) => setEnd(Math.max(v, start + 0.1))} />
                  <TimeInput value={end} onCommit={(v) => setEnd(Math.min(duration, Math.max(v, start + 0.1)))} />
                </Field>
                <div className="frame">{frames.b ? <img src={fileSrc(frames.b)} alt="终点画面" /> : "…"}</div>
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
                  ? t("trim.fastHint", { kf: timecode(kf), d: `${(start - kf).toFixed(1)} 秒` })
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
      aria-label="时间（时:分:秒）"
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
    const paths = await pickPaths({ multiple: true, filters: [{ name: kind === "audio" ? "音频" : "字幕", extensions: exts }] });
    setTracks((ts) => [...ts, ...paths.map((p) => ({ path: p, kind, language: null, title: null, default: false }))]);
  };
  const run = async () => {
    if (!path || !settings) return;
    const spec: JobSpec = { kind: "mux", main: path, keepMainAudio: keepAudio, keepMainSubtitles: keepSubs, tracks, output: { ...settings.output, container, nameTemplate: "{name}_合并" } };
    try {
      await api.enqueue([spec]);
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success("已加入队列");
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
            {t("mux.keepAudio")}（{info.audio.length} 条）
          </Check>
          <Check checked={keepSubs} onChange={setKeepSubs}>
            {t("mux.keepSubs")}（{info.subtitles.length} 条）
          </Check>
          {tracks.length > 0 && (
            <table className="table">
              <thead>
                <tr>
                  <th>文件</th>
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
                      {tr.kind === "audio" ? "音频" : "字幕"}：{fileName(tr.path)}
                    </td>
                    <td>
                      <select className="select" value={tr.language ?? "und"} onChange={(e) => setTracks((ts) => ts.map((x, j) => (j === i ? { ...x, language: e.target.value } : x)))}>
                        {LANGS.map(([code, name]) => (
                          <option key={code} value={code}>
                            {name}
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
                      <button className="icon-btn" aria-label="移除" onClick={() => setTracks((ts) => ts.filter((_, j) => j !== i))}>
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
          <Field label="容器">
            <div style={{ width: 200 }}>
              <Segmented
                label="容器"
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
        ...info.video.filter((v) => !v.attachedPic).map((v) => ({ index: v.index, kind: "视频", desc: `${codecLabel(v.codec)} ${v.width}×${v.height} ${fps(v.fps)}` })),
        ...info.audio.map((a) => ({ index: a.index, kind: "音频", desc: `${a.codec} ${a.channels} 声道 ${a.language ?? ""} ${a.title ?? ""}` })),
        ...info.subtitles.map((s) => ({ index: s.index, kind: "字幕", desc: `${s.codec}${s.bitmap ? "（图形）" : ""} ${s.language ?? ""} ${s.title ?? ""}` })),
      ]
    : [];
  const run = async () => {
    if (!path || !settings) return;
    try {
      await api.enqueue(chosen.map((i) => ({ kind: "extract" as const, input: path, streamIndex: i, output: settings.output })));
      if (useStore.getState().status?.paused) await api.setPaused(false);
      toast.success(`已加入队列 ${chosen.length} 个任务`);
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
                <th>类型</th>
                <th>内容</th>
              </tr>
            </thead>
            <tbody>
              {streams.map((s) => (
                <tr key={s.index}>
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`轨道 ${s.index}`}
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

function infoText(m: MediaInfo): string {
  const lines = [`文件：${m.path}`, `容器：${m.formatLongName || m.formatName}，${bytes(m.size)}，时长 ${clock(m.duration)}，总码率 ${m.bitRate ? Math.round(m.bitRate / 1000) + " kbps" : "—"}`];
  m.video.forEach((v) =>
    lines.push(
      `视频 #${v.index}：${codecLabel(v.codec)} ${v.profile ?? ""} ${v.width}×${v.height}，${fps(v.fps)}${v.vfr ? "（可变帧率）" : ""}，${v.pixFmt ?? "?"}${v.bitDepth ? ` ${v.bitDepth}-bit` : ""}，色彩 ${v.colorPrimaries ?? "?"}/${v.colorTransfer ?? "?"}/${v.colorSpace ?? "?"} ${v.colorRange ?? ""}${v.hdr !== "sdr" ? `，HDR ${v.hdr.toUpperCase()}` : ""}${v.rotation ? `，旋转 ${v.rotation}°` : ""}${v.attachedPic ? "（封面）" : ""}`,
    ),
  );
  m.audio.forEach((a) => lines.push(`音频 #${a.index}：${a.codec} ${a.channels} 声道 ${a.sampleRate ?? "?"} Hz ${a.bitRate ? Math.round(a.bitRate / 1000) + " kbps" : ""} ${a.language ?? ""} ${a.title ?? ""}`));
  m.subtitles.forEach((s) => lines.push(`字幕 #${s.index}：${s.codec}${s.bitmap ? "（图形）" : ""} ${s.language ?? ""} ${s.title ?? ""}`));
  if (m.chapters) lines.push(`章节：${m.chapters}`);
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
                  .catch(() => toast.error("无法写入剪贴板"))
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
                <dt>格式</dt>
                <dd>{info.formatLongName || info.formatName}</dd>
                <dt>大小</dt>
                <dd>{bytes(info.size)}</dd>
                <dt>时长</dt>
                <dd>{clock(info.duration)}</dd>
                <dt>总码率</dt>
                <dd>{info.bitRate ? `${Math.round(info.bitRate / 1000)} kbps` : "—"}</dd>
                <dt>章节 / 附件</dt>
                <dd>
                  {info.chapters} / {info.attachments}
                </dd>
              </dl>
            </section>
            {info.video.map((v) => (
              <section className="block" key={v.index}>
                <div className="block__title">
                  {t("info.video")} #{v.index}
                  {v.attachedPic ? "（封面）" : ""}
                </div>
                <dl className="kv selectable">
                  <dt>编码</dt>
                  <dd>
                    {codecLabel(v.codec)} {v.profile ?? ""} {v.level ? `Level ${v.level}` : ""}
                  </dd>
                  <dt>分辨率</dt>
                  <dd>
                    {v.width}×{v.height}
                    {v.sar && v.sar !== "1:1" ? `（SAR ${v.sar}，DAR ${v.dar}）` : ""}
                    {v.rotation ? `，${t("info.rotation", { deg: v.rotation })}` : ""}
                  </dd>
                  <dt>帧率</dt>
                  <dd>
                    {fps(v.fps)}，{v.vfr ? t("info.vfr") : t("info.cfr")}
                    {v.nbFrames ? `，${v.nbFrames} 帧` : ""}
                    {v.interlaced ? `，${t("info.interlaced")}（${v.fieldOrder}）` : ""}
                  </dd>
                  <dt>像素格式</dt>
                  <dd>
                    {v.pixFmt ?? "—"}
                    {v.bitDepth ? `，${v.bitDepth}-bit` : ""}
                    {v.chroma ? `，${v.chroma === "gray" ? "灰度" : v.chroma.split("").join(":")}` : ""}
                    {v.alpha ? "，含透明通道" : ""}
                  </dd>
                  <dt>色彩</dt>
                  <dd>
                    色域 {v.colorPrimaries ?? "未标注"}，传递 {v.colorTransfer ?? "未标注"}，矩阵 {v.colorSpace ?? "未标注"}，范围 {v.colorRange === "pc" ? "全范围 (pc)" : v.colorRange === "tv" ? "有限范围 (tv)" : "未标注"}
                  </dd>
                  <dt>HDR</dt>
                  <dd>
                    {v.hdr === "sdr" ? "SDR" : v.hdr === "pq" ? "HDR10 (PQ)" : "HLG"}
                    {v.masteringDisplay ? `，母版亮度 ${v.masteringDisplay.maxLuminance} / ${v.masteringDisplay.minLuminance} nit` : ""}
                    {v.contentLight ? `，MaxCLL ${v.contentLight.maxCll}，MaxFALL ${v.contentLight.maxFall}` : ""}
                    {v.dolbyVision ? `，杜比视界 Profile ${v.dolbyVision.profile}` : ""}
                  </dd>
                  <dt>码率</dt>
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
                  <dt>编码</dt>
                  <dd>
                    {a.codec} {a.profile ?? ""}
                  </dd>
                  <dt>声道</dt>
                  <dd>
                    {a.channels}（{a.channelLayout ?? "?"}），{a.sampleRate ?? "?"} Hz
                  </dd>
                  <dt>码率</dt>
                  <dd>{a.bitRate ? `${Math.round(a.bitRate / 1000)} kbps` : "—"}</dd>
                  <dt>语言 / 标题</dt>
                  <dd>
                    {a.language ?? "—"} / {a.title ?? "—"}
                    {a.default ? "（默认）" : ""}
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
                          {s.bitmap ? "（图形）" : ""}
                        </td>
                        <td>{s.language ?? "—"}</td>
                        <td>{s.title ?? ""}</td>
                        <td>
                          {s.default ? "默认 " : ""}
                          {s.forced ? "强制" : ""}
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
