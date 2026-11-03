import { useState } from "react";
import { toast } from "sonner";
import { api, pickPaths } from "../lib/api";
import { codecLabel } from "../lib/format";
import { t } from "../lib/i18n";
import { useStore } from "../lib/store";
import type { EncoderCaps } from "../lib/types";
import { Button, Dialog, Segmented, Spinner } from "../components/ui";

const codecOf = (name: string) =>
  name === "ffv1" ? "ffv1" : name.includes("264") ? "h264" : name.includes("265") || name.startsWith("hevc") ? "hevc" : name.includes("av1") ? "av1" : "other";

const ENC_LABEL: Record<string, string> = {
  libx264: "x264",
  libx265: "x265",
  libsvtav1: "SVT-AV1",
  "libaom-av1": "libaom",
  ffv1: "FFV1",
  h264_videotoolbox: "VideoToolbox",
  hevc_videotoolbox: "VideoToolbox",
  h264_nvenc: "NVENC",
  hevc_nvenc: "NVENC",
  av1_nvenc: "NVENC",
  h264_qsv: "Quick Sync",
  hevc_qsv: "Quick Sync",
  av1_qsv: "Quick Sync",
  h264_amf: "AMF",
  hevc_amf: "AMF",
  av1_amf: "AMF",
};

export function SettingsView() {
  const caps = useStore((st) => st.caps);
  const config = useStore((st) => st.config);
  const app = useStore((st) => st.app);
  const manifest = useStore((st) => st.manifest);
  const status = useStore((st) => st.status);
  const setConfig = useStore((st) => st.setConfig);
  const [busy, setBusy] = useState(false);

  const redetect = async () => {
    setBusy(true);
    try {
      const c = await api.redetectCaps();
      useStore.setState({ caps: c });
      toast.success("检测完成");
    } catch (e) {
      toast.error(String(e));
    } finally {
      setBusy(false);
    }
  };
  const changeBackend = async () => {
    const [dir] = await pickPaths({ directory: true, multiple: false });
    if (!dir) return;
    try {
      const r = await api.setFfmpegDir(dir);
      toast.success(r === "restartRequired" ? t("set.restart") : "已切换");
    } catch (e) {
      toast.error(String(e));
    }
  };

  const groups = ["h264", "hevc", "av1", "ffv1"].map((c) => ({ codec: c, list: (caps?.encoders ?? []).filter((e) => codecOf(e.name) === c) }));

  return (
    <div className="page">
      <div className="page__inner">
        <h1 className="page__title">{t("nav.settings")}</h1>

        <section className="block">
          <div className="block__title">{t("set.backend")}</div>
          {caps ? (
            <dl className="kv selectable">
              <dt>{t("set.version")}</dt>
              <dd>FFmpeg {caps.version}</dd>
              <dt>{t("set.license")}</dt>
              <dd>{caps.license}</dd>
              <dt>{t("set.source")}</dt>
              <dd>
                {t(`src.${caps.ffmpeg.source}`)}：{caps.ffmpeg.ffmpeg}
              </dd>
            </dl>
          ) : (
            <p className="muted">—</p>
          )}
          <div className="row">
            <Button onClick={changeBackend}>{t("set.changeBackend")}</Button>
            <Button onClick={redetect} disabled={busy}>
              {busy ? t("set.redetecting") : t("set.redetect")}
            </Button>
          </div>
        </section>

        {caps && (
          <section className="block">
            <div className="block__title">{t("set.hardware")}</div>
            <dl className="kv">
              <dt>{t("set.cpu")}</dt>
              <dd>
                {caps.cpu.brand || caps.cpu.arch}，{caps.cpu.logical} 线程{caps.cpu.performance ? `（${caps.cpu.performance} 个性能核）` : ""}
              </dd>
              <dt>{t("set.memory")}</dt>
              <dd>{(caps.memoryMb / 1024).toFixed(0)} GB</dd>
              <dt>{t("set.hwDecode")}</dt>
              <dd>{caps.hwDecode.length ? caps.hwDecode.map((h) => h.replace(":", " · ")).join("，") : "无（已实际测试）"}</dd>
            </dl>
            <table className="table">
              <thead>
                <tr>
                  <th>输出编码</th>
                  <th>{t("set.encoders")}</th>
                  <th>类型</th>
                  <th>状态</th>
                </tr>
              </thead>
              <tbody>
                {groups.map((g) =>
                  g.list.length === 0 ? (
                    <tr key={g.codec}>
                      <td>{codecLabel(g.codec)}</td>
                      <td colSpan={3} className="muted">
                        当前 FFmpeg 不包含
                      </td>
                    </tr>
                  ) : (
                    g.list.map((e: EncoderCaps, i) => (
                      <tr key={e.name}>
                        <td>{i === 0 ? codecLabel(g.codec) : ""}</td>
                        <td>
                          {ENC_LABEL[e.name] ?? e.name} <span className="muted">{e.name}</span>
                        </td>
                        <td>{e.hardware ? "GPU / 媒体引擎" : "CPU"}</td>
                        <td>
                          {e.usable ? (
                            <span className="ok-text">
                              {t("set.available")}
                              {e.hardware ? `，${t("set.verifiedEncode")}${e.qualityMode ? "" : "（仅码率模式）"}` : ""}
                              {e.pixFmts.some((p) => p.includes("10")) ? "，支持 10-bit" : ""}
                            </span>
                          ) : (
                            <span className="bad-text" title={e.error ?? ""}>
                              {t("set.unavailable")}
                            </span>
                          )}
                        </td>
                      </tr>
                    ))
                  ),
                )}
              </tbody>
            </table>
            {caps.encoders.filter((e) => e.hardware && codecOf(e.name) === "av1").length === 0 && (
              <p className="small muted" style={{ margin: 0 }}>
                此 FFmpeg 中没有 AV1 硬件编码器；AV1 使用 CPU 编码（SVT-AV1）。
              </p>
            )}
            <div className="block__title" style={{ fontSize: "var(--fs)" }}>
              {t("set.lossless")}
            </div>
            <p className="small muted" style={{ margin: 0 }}>
              {t("set.losslessHint")}
            </p>
            <table className="table">
              <tbody>
                {caps.lossless.map((c) => (
                  <tr key={c.encoder + c.pixFmt}>
                    <td>{ENC_LABEL[c.encoder] ?? c.encoder}</td>
                    <td>{c.pixFmt}</td>
                    <td className={c.ok ? "ok-text" : "bad-text"}>{c.ok ? "像素一致，已开放" : "未通过，已禁用"}</td>
                    <td className="muted">{c.detail}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        )}

        <section className="block">
          <div className="block__title">{t("set.queue")}</div>
          <p className="small muted" style={{ margin: 0 }}>
            {t("queue.autoHint", { cpu: status?.cpuSlots ?? 1, threads: status?.threadsPerJob ?? "—", light: status?.lightSlots ?? 1 })}
            开始每个任务前会检查可用内存与磁盘空间，编码中磁盘将满时会自动停止该任务。
          </p>
        </section>

        <section className="block">
          <div className="block__title">{t("set.manifest")}</div>
          <p className="small selectable" style={{ margin: 0, overflowWrap: "anywhere" }}>
            {manifest?.path ?? "—"}
          </p>
          {manifest && manifest.pending > 0 && <p className="small bad-text">{t("queue.manifestPending", { n: manifest.pending })}</p>}
          <div className="row">
            <Button disabled={!manifest} onClick={() => manifest && api.reveal(manifest.path).catch((e) => toast.error(String(e)))}>
              {t("set.openManifest")}
            </Button>
          </div>
        </section>

        {config && (
          <section className="block">
            <div className="block__title">{t("set.appearance")}</div>
            <div className="row">
              <span className="small muted" style={{ width: 60 }}>
                {t("set.theme")}
              </span>
              <div style={{ width: 300 }}>
                <Segmented
                  label={t("set.theme")}
                  value={config.theme === "light" || config.theme === "dark" ? config.theme : "system"}
                  onChange={(v) => setConfig((c) => void (c.theme = v))}
                  options={[
                    { value: "system", label: t("theme.system") },
                    { value: "light", label: t("theme.light") },
                    { value: "dark", label: t("theme.dark") },
                  ]}
                />
              </div>
            </div>
            <div className="row">
              <span className="small muted" style={{ width: 60 }}>
                {t("set.language")}
              </span>
              <select className="select" value={config.locale} onChange={(e) => setConfig((c) => void (c.locale = e.target.value))}>
                <option value="zh-CN">简体中文</option>
                <option value="en">English（部分）</option>
              </select>
            </div>
          </section>
        )}

        <section className="block">
          <div className="block__title">{t("set.about")}</div>
          <dl className="kv selectable">
            <dt>FrameEase</dt>
            <dd>
              {app?.version} · {app?.platform} {app?.arch}
            </dd>
            <dt>{t("set.dataDir")}</dt>
            <dd>{app?.dataDir}</dd>
          </dl>
          <p className="small muted" style={{ margin: 0 }}>
            {t("set.licensesHint")}
          </p>
        </section>
      </div>
    </div>
  );
}

export function SetupView() {
  const init = useStore((st) => st.init);
  const choose = async () => {
    const [dir] = await pickPaths({ directory: true, multiple: false });
    if (!dir) return;
    try {
      await api.setFfmpegDir(dir);
    } catch (e) {
      toast.error(String(e));
    }
  };
  return (
    <div className="center-page">
      <div className="center-page__box">
        {init.kind === "pending" && (
          <>
            <div style={{ display: "grid", placeItems: "center", marginBottom: 14 }}>
              <Spinner />
            </div>
            <div className="drop__title">{t("setup.detecting")}</div>
            <p className="muted small">{t("setup.detectingHint")}</p>
          </>
        )}
        {(init.kind === "noBackend" || init.kind === "backendError") && (
          <>
            <div className="drop__title">{init.kind === "noBackend" ? t("setup.noBackend") : t("setup.backendError")}</div>
            {init.kind === "backendError" && <p className="small bad-text selectable">{init.message}</p>}
            <p className="muted small">{t("setup.noBackendHint")}</p>
            <Button variant="primary" onClick={choose}>
              {t("setup.chooseFolder")}
            </Button>
          </>
        )}
      </div>
    </div>
  );
}

export function CloseDialog() {
  const n = useStore((st) => st.closeRequest);
  const [quitting, setQuitting] = useState(false);
  return (
    <Dialog open={n !== null} onClose={() => useStore.setState({ closeRequest: null })} title={t("close.title", { n: n ?? 0 })}>
      <p className="dialog__body">{t("close.body")}</p>
      <div className="dialog__actions">
        <Button data-autofocus onClick={() => useStore.setState({ closeRequest: null })}>
          {t("close.keep")}
        </Button>
        <Button
          variant="primary"
          disabled={quitting}
          onClick={async () => {
            setQuitting(true);
            await api.quitApp().catch(() => setQuitting(false));
          }}
        >
          {quitting ? "正在停止…" : t("close.quit")}
        </Button>
      </div>
    </Dialog>
  );
}
