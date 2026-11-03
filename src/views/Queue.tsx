import { memo, useState } from "react";
import { toast } from "sonner";
import { ChevronDown, FileText, FolderSearch, MoreHorizontal, Play, RotateCcw, Square, X } from "lucide-react";
import { api } from "../lib/api";
import { bytes, clock, codecLabel, fileName, humanDuration, middleTruncate, percent, ratioLabel } from "../lib/format";
import { checkName, notice, t } from "../lib/i18n";
import { useShallow } from "zustand/react/shallow";
import { selectOrdered, useStore } from "../lib/store";
import { type Job, isActive } from "../lib/types";
import { Button, Chip, HoldButton, IconButton, Menu } from "../components/ui";

const dotClass = (j: Job) =>
  ({
    probing: "dot dot--run",
    encoding: "dot dot--run",
    muxing: "dot dot--run",
    verifying: "dot dot--run",
    done: "dot dot--ok",
    failed: "dot dot--bad",
    needsInput: "dot dot--ask",
    cancelled: "dot dot--muted",
    waiting: "dot",
  })[j.state];

function targetChips(j: Job) {
  const p = j.plan;
  const spec = j.spec;
  if (spec.kind === "extract") return [<Chip key="k">提取轨道 #{spec.streamIndex}</Chip>];
  if (spec.kind === "mux") return [<Chip key="k">合并轨道</Chip>, <Chip key="c">{spec.output.container.toUpperCase()}</Chip>];
  const s = spec.settings;
  const chips = [];
  const from = j.source?.video ? codecLabel(j.source.video) : null;
  const to = s.video.mode === "copy" ? (from ?? "原编码") : codecLabel(s.video.codec);
  chips.push(
    <Chip key="codec" tone="accent">
      {from ? `${from} → ${to}` : to}
    </Chip>,
  );
  chips.push(<Chip key="ctr">{s.output.container.toUpperCase()}</Chip>);
  if (s.video.mode === "lossless") chips.push(<Chip key="m" tone="ok">无损</Chip>);
  else if (s.video.mode === "copy") chips.push(<Chip key="m">流复制</Chip>);
  else if (p?.quality) chips.push(<Chip key="q">{qualityText(p.quality, p.encoder)}</Chip>);
  if (s.trim) chips.push(<Chip key="t">截取 {clock(s.trim.start)}–{s.trim.end !== null ? clock(s.trim.end) : "结尾"}</Chip>);
  if (p?.hw) chips.push(<Chip key="hw">GPU</Chip>);
  if (s.audio.mode === "none") chips.push(<Chip key="a">无音频</Chip>);
  else if (s.audio.mode === "transcode") chips.push(<Chip key="a">音频 {s.audio.codec.toUpperCase()}</Chip>);
  return chips;
}

export function qualityText(q: string, encoder: string | null) {
  const [k, v] = q.split(":");
  const enc = encoder ? `${encoder.replace(/^lib/, "")} ` : "";
  if (k === "bitrate") return `${v} kbps`;
  if (k === "size") return `目标 ${v} MB`;
  if (k === "lossless" || k === "copy") return k;
  const label = { crf: "CRF", cq: "CQ", q: "q:v", icq: "ICQ", qp: "QP" }[k] ?? k;
  return `${enc}${label} ${v}`;
}

const JobRow = memo(function JobRow({ job, index, total }: { job: Job; index: number; total: number }) {
  const [open, setOpen] = useState(false);
  const [log, setLog] = useState<string | null>(null);
  const active = isActive(job.state);
  const run = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
    } catch (e) {
      toast.error(String(e));
    }
  };
  const showLog = () => run(async () => setLog(await api.readLog(job.id)));
  const dur = job.source?.duration ?? null;
  const fillTone = job.state === "verifying" ? "timeline__fill timeline__fill--verify" : job.state === "done" ? "timeline__fill timeline__fill--done" : "timeline__fill";
  const showTimeline = active || (job.state === "waiting" && job.progress > 0);
  const out = job.output ?? job.plannedOutput;

  return (
    <div className="item" data-state={job.state}>
      <div className="item__main">
        <span className={dotClass(job)} aria-hidden />
        <div style={{ minWidth: 0 }}>
          <div className="item__name" title={job.spec.kind === "convert" ? job.spec.input : job.name}>
            {middleTruncate(job.name, 64)}
          </div>
          <div className="item__sub">
            <span className={job.state === "failed" ? "bad-text" : job.state === "needsInput" ? "" : ""}>{t(`state.${job.state}`)}</span>
            {job.interrupted && job.state === "waiting" && <Chip tone="ask">{t("job.interrupted")}</Chip>}
            <span className="chips">{targetChips(job)}</span>
          </div>
        </div>
        <div className="item__actions">
          {job.state === "done" && out && (
            <>
              <Chip tone="ok">✓ {t("job.verified")}</Chip>
              <IconButton label={t("job.open")} onClick={() => run(() => api.openPath(out))}>
                <Play size={15} />
              </IconButton>
              <IconButton label={t("job.reveal")} onClick={() => run(() => api.reveal(out))}>
                <FolderSearch size={15} />
              </IconButton>
            </>
          )}
          {(active || job.state === "waiting" || job.state === "needsInput") && (
            <IconButton label={t("job.cancel")} onClick={() => run(() => api.cancel(job.id))}>
              <Square size={13} />
            </IconButton>
          )}
          {(job.state === "failed" || job.state === "cancelled") && (
            <IconButton label={t("job.retry")} onClick={() => run(() => api.retry(job.id))}>
              <RotateCcw size={15} />
            </IconButton>
          )}
          <IconButton label={t("job.details")} aria-expanded={open} onClick={() => setOpen((o) => !o)}>
            <ChevronDown size={15} style={{ transform: open ? "rotate(180deg)" : undefined, transition: "transform 160ms cubic-bezier(0.23,1,0.32,1)" }} />
          </IconButton>
          <Menu
            align="right"
            trigger={({ toggle }) => (
              <button type="button" className="icon-btn" aria-label="更多操作" onClick={toggle}>
                <MoreHorizontal size={16} />
              </button>
            )}
            items={[
              { label: t("job.moveTop"), onSelect: () => run(() => api.move(job.id, 0)), disabled: job.state !== "waiting" || index === 0 },
              { label: t("job.moveUp"), onSelect: () => run(() => api.move(job.id, index - 1)), disabled: job.state !== "waiting" || index === 0 },
              { label: t("job.moveDown"), onSelect: () => run(() => api.move(job.id, index + 1)), disabled: job.state !== "waiting" || index === total - 1 },
              { label: t("job.moveBottom"), onSelect: () => run(() => api.move(job.id, total - 1)), disabled: job.state !== "waiting" || index === total - 1 },
              { separator: true, label: "" },
              { label: t("job.log"), onSelect: () => {
                  setOpen(true);
                  showLog();
                }, disabled: !job.logPath },
              { label: t("job.remove"), onSelect: () => run(() => api.remove([job.id])) },
            ]}
          />
        </div>
      </div>

      {showTimeline && (
        <>
          <div className="timeline" aria-hidden>
            <div className="timeline__track">
              <div className={fillTone} style={{ transform: `scaleX(${job.progress})` }} />
            </div>
            <div className="timeline__ticks" />
            <div className="timeline__playhead" style={{ transform: `translateX(${job.progress * 100}%)` }}>
              <div className="timeline__head" />
            </div>
          </div>
          <div className="stats" role="status">
            <span>
              <strong>{percent(job.progress)}</strong>
            </span>
            {job.state === "encoding" && job.stats.outTime !== null && dur !== null && (
              <span>
                {clock(job.stats.outTime)} / {clock(dur)}
              </span>
            )}
            {job.stats.pass && <span>{t("job.pass", { p: job.stats.pass })}</span>}
            {job.stats.speed !== null && job.state === "encoding" && <span>{t("job.speed", { x: job.stats.speed.toFixed(2) })}</span>}
            {job.stats.fps !== null && job.state === "encoding" && <span>{Math.round(job.stats.fps)} fps</span>}
            {job.stats.etaSecs !== null && <span>{t("job.eta", { t: humanDuration(job.stats.etaSecs) })}</span>}
            {job.state === "verifying" && <span>正在检查输出文件</span>}
          </div>
        </>
      )}

      {job.state === "done" && (
        <div className="stats">
          {job.outputSize !== null && (
            <span>
              <strong>{bytes(job.outputSize)}</strong>
            </span>
          )}
          {job.outputSize !== null && job.source?.size ? (
            (() => {
              const r = ratioLabel(job.outputSize, job.source.size);
              return <span className={r.smaller ? "ok-text" : ""}>{r.smaller ? t("job.saved", { p: r.text }) : t("job.grew", { p: r.text })}</span>;
            })()
          ) : null}
          <span>{t("job.elapsed", { t: humanDuration(job.elapsedMs / 1000) })}</span>
          {out && (
            <span className="muted" title={out}>
              {middleTruncate(fileName(out), 40)}
            </span>
          )}
        </div>
      )}

      {job.state === "needsInput" && job.issues.length > 0 && (
        <div className="item__issues">
          {job.issues.map((iss, i) => (
            <div className="issue" key={i}>
              <span>{notice("issue", iss)}</span>
              {iss.fixes.length > 0 && (
                <div className="issue__fixes">
                  {iss.fixes.map((f) => (
                    <Button key={f.id} size="sm" onClick={() => run(() => api.resolve(job.id, f.id))}>
                      {t(`fix.${f.id}`)}
                    </Button>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {job.state === "failed" && job.error && (
        <div className="item__issues">
          <div className="issue issue--bad">
            <span>{notice("err", job.error)}</span>
            <div className="issue__fixes">
              <Button size="sm" onClick={() => run(() => api.retry(job.id))}>
                <RotateCcw size={13} /> {t("job.retry")}
              </Button>
              {job.logPath && (
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setOpen(true);
                    showLog();
                  }}
                >
                  <FileText size={13} /> {t("job.log")}
                </Button>
              )}
            </div>
          </div>
        </div>
      )}

      {job.state === "cancelled" && job.error && <div className="notes">{notice("err", job.error)}</div>}

      {job.warnings.length > 0 && (job.state === "done" || job.state === "waiting" || active) && (
        <div className="notes">
          {job.warnings.map((w, i) => (
            <span key={i}>· {notice("warn", w)}</span>
          ))}
        </div>
      )}

      {open && (
        <div className="details selectable">
          {job.verify && (
            <>
              <div className="ins__label" style={{ marginBottom: 6 }}>
                {t("job.checks")}
              </div>
              <div className="checks">
                {job.verify.checks.map((c) => (
                  <div key={c.name} style={{ display: "contents" }}>
                    <span className={c.ok ? "checks__ok" : "checks__bad"}>{c.ok ? "✓" : "✗"}</span>
                    <span>{checkName(c.name)}</span>
                    <span className="muted">
                      {c.ok ? c.actual : `期望 ${c.expected}，实际 ${c.actual}`}
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
          <dl className="kv" style={{ marginTop: job.verify ? 10 : 0 }}>
            <dt>{t("job.source")}</dt>
            <dd>{job.spec.kind === "convert" ? job.spec.input : job.spec.kind === "mux" ? job.spec.main : job.spec.input}</dd>
            {out && (
              <>
                <dt>{t("job.output")}</dt>
                <dd>{out}</dd>
              </>
            )}
            {job.plan?.encoder && (
              <>
                <dt>编码器</dt>
                <dd>
                  {job.plan.encoder}
                  {job.plan.threads ? `，${job.plan.threads} 线程` : ""}
                </dd>
              </>
            )}
            {job.attempts > 1 && (
              <>
                <dt>尝试</dt>
                <dd>{t("job.attempts", { n: job.attempts })}</dd>
              </>
            )}
          </dl>
          {job.error?.detail && <pre className="log">{job.error.detail}</pre>}
          {log !== null && <pre className="log">{log || "（日志为空）"}</pre>}
          {log === null && job.logPath && (
            <Button size="sm" variant="ghost" onClick={showLog} style={{ marginTop: 8 }}>
              <FileText size={13} /> {t("job.log")}
            </Button>
          )}
        </div>
      )}
    </div>
  );
});

export function QueueSection() {
  const jobs = useStore(useShallow(selectOrdered));
  const status = useStore((st) => st.status);
  const manifest = useStore((st) => st.manifest);
  const doneCount = jobs.filter((j) => j.state === "done").length;
  const conc = status?.concurrency;
  const setConc = (n: number | null) => api.setConcurrency(n === null ? { kind: "auto" } : { kind: "fixed", n }).catch((e) => toast.error(String(e)));

  return (
    <section aria-label={t("queue.title")}>
      <div className="section__head">
        <span className="section__title">{t("queue.title")}</span>
        <span className="section__meta">
          {status && status.running > 0 ? t("queue.running", { n: status.running }) : status?.paused && jobs.length ? t("queue.paused") : ""}
          {status && status.waiting > 0 ? `，${status.waiting} 个等待` : ""}
        </span>
        <span className="grow" />
        {jobs.length > 0 && (
          <>
            <Menu
              align="right"
              trigger={({ toggle }) => (
                <Button size="sm" variant="ghost" onClick={toggle}>
                  {t("queue.concurrency")}：{conc?.kind === "fixed" ? conc.n : t("queue.concurrencyAuto")}
                </Button>
              )}
              items={[
                {
                  label: t("queue.autoHint", { cpu: status?.cpuSlots ?? 1, threads: status?.threadsPerJob ?? "—", light: status?.lightSlots ?? 1 }),
                  heading: true,
                },
                { label: t("queue.concurrencyAuto"), checked: conc?.kind === "auto", onSelect: () => setConc(null) },
                ...[1, 2, 3, 4].map((n) => ({ label: `${n} 个同时进行`, checked: conc?.kind === "fixed" && conc.n === n, onSelect: () => setConc(n) })),
              ]}
            />
            <Button size="sm" onClick={() => api.setPaused(!status?.paused).catch((e) => toast.error(String(e)))}>
              {status?.paused ? t("queue.resume") : t("queue.pause")}
            </Button>
            <HoldButton disabled={doneCount === 0} onConfirm={() => api.clearFinished().catch((e) => toast.error(String(e)))}>
              {t("queue.clearDone")}
            </HoldButton>
          </>
        )}
      </div>

      {status && status.recovered > 0 && status.paused && (
        <div className="banner banner--ask" style={{ marginBottom: 10 }}>
          <span className="grow">{t("queue.recovered", { n: status.recovered })}</span>
          <Button size="sm" variant="primary" onClick={() => api.setPaused(false)}>
            {t("queue.resumeRecovered")}
          </Button>
        </div>
      )}
      {status?.blocked === "memory" && (
        <div className="banner banner--ask" style={{ marginBottom: 10 }}>
          {t("queue.blockedMemory")}
        </div>
      )}
      {manifest && manifest.pending > 0 && (
        <div className="banner banner--ask" style={{ marginBottom: 10 }}>
          <span className="grow">{t("queue.manifestPending", { n: manifest.pending })}</span>
          <Button size="sm" onClick={() => api.manifestFlush().then((m) => useStore.setState({ manifest: m }))}>
            {t("queue.manifestRetry")}
          </Button>
        </div>
      )}

      {jobs.length === 0 ? (
        <p className="muted small">{t("queue.empty")}</p>
      ) : (
        <div className="list">
          {jobs.map((j, i) => (
            <JobRow key={j.id} job={j} index={i} total={jobs.length} />
          ))}
        </div>
      )}
    </section>
  );
}

export function StagedRowActions({ path }: { path: string }) {
  const remove = useStore((st) => st.removeStaged);
  return (
    <IconButton label={t("staged.remove")} onClick={() => remove(path)}>
      <X size={15} />
    </IconButton>
  );
}
