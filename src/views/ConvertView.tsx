import { useState } from "react";
import { toast } from "sonner";
import { FilePlus2, FolderPlus, FlaskConical, Film } from "lucide-react";
import { api, pickPaths, VIDEO_EXTENSIONS } from "../lib/api";
import { bytes, clock, codecLabel, fileName, fps, middleTruncate } from "../lib/format";
import { notice, t } from "../lib/i18n";
import { type Staged, useStore } from "../lib/store";
import { Button, Check, Chip } from "../components/ui";
import { Inspector } from "./Inspector";
import { QueueSection, StagedRowActions } from "./Queue";
import { TrialDialog } from "./Trial";

export function ConvertView() {
  const staged = useStore((st) => st.staged);
  const order = useStore((st) => st.order);
  const config = useStore((st) => st.config);
  const addPaths = useStore((st) => st.addPaths);
  const setConfig = useStore((st) => st.setConfig);
  const [trialOpen, setTrialOpen] = useState(false);

  const addFiles = async () => addPaths(await pickPaths({ multiple: true, filters: [{ name: "视频", extensions: VIDEO_EXTENSIONS }, { name: "所有文件", extensions: ["*"] }] }));
  const addFolder = async () => addPaths(await pickPaths({ directory: true, multiple: true }));
  const empty = staged.length === 0 && order.length === 0;

  return (
    <div className="convert">
      <div className="work">
        <div className="toolbar">
          <Button variant={empty ? "default" : "primary"} onClick={addFiles}>
            <FilePlus2 size={15} /> {t("add.files")}
          </Button>
          <Button onClick={addFolder}>
            <FolderPlus size={15} /> {t("add.folder")}
          </Button>
          <Check checked={config?.recursiveScan ?? false} onChange={(c) => setConfig((x) => void (x.recursiveScan = c))}>
            {t("add.recursive")}
          </Check>
          <span className="toolbar__grow" />
          {staged.some((s) => s.info) && (
            <Button variant="ghost" onClick={() => setTrialOpen(true)}>
              <FlaskConical size={15} /> {t("staged.trial")}
            </Button>
          )}
        </div>

        {empty ? (
          <div className="drop">
            <div>
              <Film size={34} strokeWidth={1.4} />
              <div className="drop__title">{t("add.dropTitle")}</div>
              <p className="drop__hint">{t("add.dropHint")}</p>
              <div className="row" style={{ justifyContent: "center" }}>
                <Button variant="primary" onClick={addFiles}>
                  {t("add.files")}
                </Button>
                <Button onClick={addFolder}>{t("add.folder")}</Button>
              </div>
            </div>
          </div>
        ) : (
          <>
            {staged.length > 0 && <StagedSection />}
            <QueueSection />
          </>
        )}
      </div>
      <Inspector />
      <TrialDialog open={trialOpen} onClose={() => setTrialOpen(false)} />
    </div>
  );
}

function StagedSection() {
  const staged = useStore((st) => st.staged);
  const clear = useStore((st) => st.clearStaged);
  const start = useStore((st) => st.startStaged);
  const settings = useStore((st) => st.settings);
  const ready = staged.filter((s) => s.info).length;
  const withIssues = staged.filter((s) => s.preview && !s.preview.ok && s.preview.issues.length).length;
  // Fixes offered for any staged file, applied to the shared settings in one click.
  const fixes = Array.from(new Set(staged.flatMap((s) => s.preview?.issues.flatMap((i) => i.fixes.map((f) => f.id)) ?? [])));

  const applyFix = async (fix: string) => {
    if (!settings) return;
    try {
      const next = await api.applyFix(settings, fix);
      useStore.getState().updateSettings((s) => Object.assign(s, next));
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <section aria-label={t("staged.title")}>
      <div className="section__head">
        <span className="section__title">{t("staged.title")}</span>
        <span className="section__meta">
          {staged.length} 个文件{withIssues ? `，${withIssues} 个需要处理` : ""}
        </span>
        <span className="grow" />
        <Button size="sm" variant="ghost" onClick={clear}>
          {t("staged.clear")}
        </Button>
      </div>
      <div className="list">
        {staged.map((s) => (
          <StagedRow key={s.path} s={s} />
        ))}
      </div>
      <div className="start-bar">
        <Button variant="primary" size="lg" disabled={ready === 0} onClick={() => start().catch((e) => toast.error(String(e)))}>
          {t("staged.start", { n: ready })}
        </Button>
        <span className="small muted grow">{t("staged.startHint")}</span>
      </div>
      {fixes.length > 0 && (
        <div className="row row--wrap" style={{ marginTop: 10 }}>
          <span className="small muted">一次处理全部：</span>
          {fixes.map((f) => (
            <Button key={f} size="sm" onClick={() => applyFix(f)}>
              {t(`fix.${f}`)}
            </Button>
          ))}
        </div>
      )}
    </section>
  );
}

function StagedRow({ s }: { s: Staged }) {
  const v = s.info?.video.find((x) => !x.attachedPic);
  const p = s.preview;
  const dot = s.loading ? "dot dot--run" : s.error ? "dot dot--bad" : p && !p.ok ? "dot dot--ask" : "dot dot--ok";
  return (
    <div className="item">
      <div className="item__main">
        <span className={dot} aria-hidden />
        <div style={{ minWidth: 0 }}>
          <div className="item__name" title={s.path}>
            {middleTruncate(fileName(s.path), 64)}
          </div>
          <div className="item__sub">
            {s.loading && <span>{t("staged.probing")}</span>}
            {s.error && <span className="bad-text">{notice("err", { code: s.error })}</span>}
            {s.info && (
              <span className="chips">
                {v && <Chip>{codecLabel(v.codec)}</Chip>}
                {v && (
                  <Chip>
                    {v.rotation % 180 === 90 ? `${v.height}×${v.width}` : `${v.width}×${v.height}`}
                  </Chip>
                )}
                {v?.fps ? <Chip>{v.vfr ? `≈${fps(v.fps)} 可变` : fps(v.fps)}</Chip> : null}
                {v?.bitDepth && v.bitDepth > 8 ? <Chip>{v.bitDepth}-bit</Chip> : null}
                {v && v.hdr !== "sdr" ? <Chip tone="run">{v.hdr === "pq" ? "HDR10/PQ" : "HLG"}</Chip> : null}
                <Chip>{clock(s.info.duration)}</Chip>
                <Chip>{bytes(s.info.size)}</Chip>
                <Chip>{s.info.audio.length ? `${s.info.audio.length} 条音轨` : "无音轨"}</Chip>
                {s.info.subtitles.length > 0 && <Chip>{s.info.subtitles.length} 条字幕</Chip>}
              </span>
            )}
          </div>
          {p?.ok && p.plannedOutput && (
            <div className="item__sub" title={p.plannedOutput}>
              <span>→ {middleTruncate(fileName(p.plannedOutput), 56)}</span>
              {p.summary?.estSize && p.summary.estReliability !== "unknown" ? <span>约 {bytes(p.summary.estSize)}</span> : null}
            </div>
          )}
        </div>
        <div className="item__actions">
          <StagedRowActions path={s.path} />
        </div>
      </div>
      {p && !p.ok && p.issues.length > 0 && (
        <div className="item__issues">
          {p.issues.map((iss, i) => (
            <div className="issue" key={i}>
              {notice("issue", iss)}
            </div>
          ))}
        </div>
      )}
      {p?.ok && p.warnings.length > 0 && (
        <div className="notes">
          {p.warnings.map((w, i) => (
            <span key={i}>· {notice("warn", w)}</span>
          ))}
        </div>
      )}
    </div>
  );
}
