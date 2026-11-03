import { type CSSProperties, type PointerEvent, useEffect, useRef, useState } from "react";
import { api, fileSrc } from "../lib/api";
import { bytes, clock, fileName, middleTruncate } from "../lib/format";
import { notice, t } from "../lib/i18n";
import { useStore } from "../lib/store";
import type { TrialResult } from "../lib/types";
import { Button, Dialog, Field, Segmented, Slider, Spinner } from "../components/ui";

/** Before/after wipe: the result is clipped with clip-path inset, following the pointer. */
function Compare({ before, after }: { before: string | null; after: string | null }) {
  const ref = useRef<HTMLDivElement>(null);
  const [x, setX] = useState(50);
  const move = (e: PointerEvent) => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    setX(Math.max(0, Math.min(100, ((e.clientX - r.left) / r.width) * 100)));
  };
  return (
    <div
      ref={ref}
      className="compare"
      style={{ "--x": `${x}%` } as CSSProperties}
      onPointerDown={(e) => {
        (e.target as HTMLElement).setPointerCapture(e.pointerId);
        move(e);
      }}
      onPointerMove={(e) => {
        if (e.buttons === 1) move(e);
      }}
      role="slider"
      aria-label="原片与结果对比"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(x)}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") setX((v) => Math.max(0, v - 5));
        if (e.key === "ArrowRight") setX((v) => Math.min(100, v + 5));
      }}
    >
      {after && <img src={fileSrc(after)} alt={t("trial.result")} />}
      {before && <img className="compare__top" src={fileSrc(before)} alt={t("trial.original")} />}
      <div className="compare__bar" />
      <span className="compare__tag" style={{ left: 8 }}>
        {t("trial.original")}
      </span>
      <span className="compare__tag" style={{ right: 8 }}>
        {t("trial.result")}
      </span>
    </div>
  );
}

export function TrialDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const allStaged = useStore((st) => st.staged);
  const settings = useStore((st) => st.settings);
  const staged = allStaged.filter((s) => s.info);
  const [path, setPath] = useState<string | null>(null);
  const [start, setStart] = useState(0);
  const [duration, setDuration] = useState("10");
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<TrialResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [at, setAt] = useState(0);
  const [frames, setFrames] = useState<{ before: string | null; after: string | null }>({ before: null, after: null });

  const file = staged.find((s) => s.path === path) ?? staged[0];
  const total = file?.info?.duration ?? 0;
  const hdr = file?.info?.video.some((v) => v.hdr !== "sdr");

  useEffect(() => {
    if (open && file && path !== file.path) {
      setPath(file.path);
      setStart(Math.floor(total * 0.3));
      setResult(null);
    }
  }, [open, file, path, total]);

  useEffect(() => {
    if (!result || !file) return;
    let alive = true;
    const timer = setTimeout(async () => {
      try {
        const [before, after] = await Promise.all([
          api.extractFrame(file.path, result.start + at, 1280, "src"),
          api.extractFrame(result.output, at, 1280, "trial"),
        ]);
        if (alive) setFrames({ before, after });
      } catch {
        /* preview frames are best effort */
      }
    }, 120);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [result, at, file]);

  const run = async () => {
    if (!file || !settings) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setFrames({ before: null, after: null });
    try {
      const r = await api.trialRun({ path: file.path, settings, start, duration: Number(duration), metrics: true });
      setResult(r);
      setAt(r.segmentSecs / 2);
    } catch (e) {
      const code = String(e);
      if (code !== "cancelled") setError(notice("issue", { code }));
    } finally {
      setRunning(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={() => {
        if (running) api.trialCancel();
        onClose();
      }}
      title={t("trial.title")}
      wide
    >
      {!file ? (
        <p className="dialog__body">{t("trial.none")}</p>
      ) : (
        <div className="block" style={{ gap: 14 }}>
          <div className="row row--wrap" style={{ alignItems: "flex-end", gap: 14 }}>
            <Field label={t("trial.file")}>
              <select className="select" value={file.path} onChange={(e) => setPath(e.target.value)} style={{ maxWidth: 300 }}>
                {staged.map((s) => (
                  <option key={s.path} value={s.path}>
                    {middleTruncate(fileName(s.path), 40)}
                  </option>
                ))}
              </select>
            </Field>
            <div className="grow" style={{ minWidth: 200 }}>
              <Field label={`${t("trial.start")} ${clock(start)}`}>
                <Slider label={t("trial.start")} min={0} max={Math.max(0, Math.floor(total - 1))} value={start} onChange={setStart} />
              </Field>
            </div>
            <Field label={t("trial.duration")}>
              <Segmented
                label={t("trial.duration")}
                value={duration}
                onChange={setDuration}
                options={[
                  { value: "5", label: "5 秒" },
                  { value: "10", label: "10 秒" },
                  { value: "20", label: "20 秒" },
                ]}
              />
            </Field>
            {running ? (
              <Button onClick={() => api.trialCancel()}>{t("trial.cancel")}</Button>
            ) : (
              <Button variant="primary" onClick={run}>
                {t("trial.run")}
              </Button>
            )}
          </div>

          {running && (
            <div className="row">
              <Spinner /> <span className="muted">{t("trial.running")}</span>
            </div>
          )}
          {error && <div className="issue issue--bad">{error}</div>}

          {result && (
            <>
              <div className="metric-grid">
                <div className="metric">
                  <div className="metric__v">{bytes(result.bytes)}</div>
                  <div className="metric__l">
                    {t("trial.size")}（{result.segmentSecs.toFixed(0)} 秒
                    {result.sourceSegmentBytes ? `，原片约 ${bytes(result.sourceSegmentBytes)}` : ""}）
                  </div>
                </div>
                {result.estFullBytes !== null && (
                  <div className="metric">
                    <div className="metric__v">≈ {bytes(result.estFullBytes)}</div>
                    <div className="metric__l">{t("trial.estimate")}（估计）</div>
                  </div>
                )}
                {result.speed !== null && (
                  <div className="metric">
                    <div className="metric__v">{result.speed.toFixed(2)}×</div>
                    <div className="metric__l">
                      {t("trial.speed")}（用时 {result.encodeSecs.toFixed(1)} 秒）
                    </div>
                  </div>
                )}
                {result.vmaf !== null && (
                  <div className="metric">
                    <div className="metric__v">{result.vmaf.toFixed(1)}</div>
                    <div className="metric__l">VMAF</div>
                  </div>
                )}
                {result.ssim !== null && (
                  <div className="metric">
                    <div className="metric__v">{result.ssim.toFixed(4)}</div>
                    <div className="metric__l">SSIM</div>
                  </div>
                )}
                {result.psnr !== null && (
                  <div className="metric">
                    <div className="metric__v">{result.psnr.toFixed(2)} dB</div>
                    <div className="metric__l">PSNR</div>
                  </div>
                )}
              </div>
              <p className="small muted" style={{ margin: 0 }}>
                {t("trial.estimateHint")} {result.summary.lossless ? "无损结果以完成后的逐帧校验为准。" : t("trial.metricsHint")}
              </p>
              <Field label={`${t("trial.compare")}：片段内 ${clock(at)}`}>
                <Slider label={t("trial.compare")} min={0} max={Math.max(0.1, result.segmentSecs - 0.1)} step={0.1} value={at} onChange={setAt} />
              </Field>
              <Compare before={frames.before} after={frames.after} />
              {hdr && <p className="small muted">{t("trial.hdrNote")}</p>}
            </>
          )}
        </div>
      )}
      <div className="dialog__actions" style={{ marginTop: 16 }}>
        <Button
          onClick={() => {
            if (running) api.trialCancel();
            onClose();
          }}
        >
          关闭
        </Button>
      </div>
    </Dialog>
  );
}
