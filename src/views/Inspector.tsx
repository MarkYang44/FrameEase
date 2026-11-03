import { AudioLines, FileVideo2, FolderOpen, Gauge, Info, Ruler, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { api, pickPaths, pickSavePath } from "../lib/api";
import { bytes, codecLabel, fileName } from "../lib/format";
import { notice, t } from "../lib/i18n";
import { useStore } from "../lib/store";
import type { AudioCodec, JobSettings, Rotate, Speed, VideoCodec, VideoMode } from "../lib/types";
import { Button, Check, Disclosure, Field, Menu, Segmented, Slider, Tooltip } from "../components/ui";

const LEVELS = { high: 25, balanced: 50, small: 75 } as const;

function qualityPreset(s: JobSettings): string {
  if (s.video.rate.kind !== "level") return "custom";
  const hit = Object.entries(LEVELS).find(([, v]) => v === s.video.level);
  return hit ? hit[0] : "custom";
}

export function Inspector() {
  const settings = useStore((st) => st.settings);
  const choice = useStore((st) => st.choice);
  const caps = useStore((st) => st.caps);
  const status = useStore((st) => st.status);
  const update = useStore((st) => st.updateSettings);
  if (!settings) return null;
  const v = settings.video;
  const lossy = v.mode === "lossy";
  const copy = v.mode === "copy";
  const gpuOk = (choice?.gpuCodecs ?? []).includes(v.codec) && lossy;
  const libass = caps?.filters.includes("subtitles") ?? false;

  const setCodec = (c: VideoCodec) =>
    update((s) => {
      s.video.codec = c;
      s.video.encoder = null;
      if (c === "ffv1") {
        s.video.mode = "lossless";
        s.output.container = "mkv";
      }
      if (s.video.speed === "gpu" && !(choice?.gpuCodecs ?? []).includes(c)) s.video.speed = "balanced";
    });

  const setMode = (m: VideoMode) =>
    update((s) => {
      s.video.mode = m;
      if (m !== "lossy" && s.video.speed === "gpu") s.video.speed = "balanced";
      if (m === "lossless" && s.subtitles.mode === "burn") s.subtitles.mode = "keep";
    });

  return (
    <aside className="inspector" aria-label="输出设置">
      <div className="ins__body">
        <PresetRow />

        <div className="ins__group">
          <div className="ins__label">{t("ins.codec")}</div>
          <Segmented
            label={t("ins.codec")}
            value={v.codec}
            onChange={setCodec}
            options={[
              { value: "h264", label: "H.264", disabled: copy, hint: "兼容性最好，几乎所有设备都能播放" },
              { value: "hevc", label: "HEVC", disabled: copy, hint: "同画质下比 H.264 小约 30–50%，较新的设备支持" },
              { value: "av1", label: "AV1", disabled: copy, hint: "压缩率最高，编码较慢，较新的设备才能硬件解码" },
              { value: "ffv1", label: "FFV1", disabled: copy, hint: "无损存档格式，只能放在 MKV 中，体积大、兼容性有限" },
            ]}
          />
          {copy && <p className="ins__explain">流复制会保留原视频的编码，不使用这里的编码选择。</p>}
        </div>

        <div className="ins__group">
          <div className="ins__label">{t("ins.mode")}</div>
          <Segmented
            label={t("ins.mode")}
            value={v.mode}
            onChange={setMode}
            options={[
              { value: "lossy", label: t("mode.lossy"), disabled: v.codec === "ffv1", hint: v.codec === "ffv1" ? "FFV1 只用于无损" : undefined },
              { value: "lossless", label: t("mode.lossless") },
              { value: "copy", label: t("mode.copy") },
            ]}
          />
          <p className="ins__explain">{t(`mode.${v.mode}Hint`)}</p>
        </div>

        {lossy && <QualityGroup />}

        {!copy && (
          <div className="ins__group">
            <div className="ins__label">{t("ins.speed")}</div>
            <Segmented<Speed>
              label={t("ins.speed")}
              value={v.speed}
              onChange={(sp) =>
                update((s) => {
                  s.video.speed = sp;
                  s.video.encoder = null;
                })
              }
              options={[
                { value: "gpu", label: t("speed.gpu"), disabled: !gpuOk, hint: !gpuOk ? (lossy ? t("speed.gpuNone", { codec: codecLabel(v.codec) }) : "无损模式只使用经过验证的 CPU 编码器") : "使用显卡/媒体引擎编码：速度最快，同画质下体积稍大" },
                { value: "fast", label: t("speed.fast") },
                { value: "balanced", label: t("speed.balanced") },
                { value: "slow", label: t("speed.slow"), hint: "编码更慢，同画质下体积更小" },
              ]}
            />
            {choice?.issue ? (
              <p className="ins__explain bad-text">{notice("issue", choice.issue)}</p>
            ) : (
              choice?.label && (
                <p className="ins__explain">
                  {t("speed.uses", { label: choice.label })}（{choice.hardware ? t("speed.hardware") : t("speed.cpuThreads", { n: status?.threadsPerJob ?? "—" })}）
                </p>
              )
            )}
          </div>
        )}

        <AudioGroup />
        <SubtitleGroup libass={libass} />
        <OutputGroup />

        <Disclosure title={t("ins.advanced")}>
          <Advanced />
        </Disclosure>
      </div>
      <Summary />
    </aside>
  );
}

function PresetRow() {
  const presets = useStore((st) => st.presets);
  const presetId = useStore((st) => st.presetId);
  const settings = useStore((st) => st.settings)!;
  const applyPreset = useStore((st) => st.applyPreset);
  const refresh = useStore((st) => st.refreshPresets);
  const current = presets.find((p) => p.id === presetId);

  const save = async () => {
    const name = window.prompt(t("ins.presetName"), current && !current.builtin ? current.name : "");
    if (!name) return;
    try {
      const p = await api.presetSave(name, settings);
      await refresh();
      useStore.setState({ presetId: p.id });
      toast.success(`已保存预设“${p.name}”`);
    } catch (e) {
      toast.error(String(e));
    }
  };
  const importP = async () => {
    const [path] = await pickPaths({ multiple: false, filters: [{ name: "FrameEase 预设", extensions: ["json"] }] });
    if (!path) return;
    try {
      const p = await api.presetImport(path);
      await refresh();
      applyPreset(p);
      toast.success(`已导入预设“${p.name}”`);
    } catch (e) {
      toast.error(`无法导入：${String(e)}`);
    }
  };
  const exportP = async () => {
    if (!current) return;
    const path = await pickSavePath(`${current.name}.frameease-preset.json`, [{ name: "FrameEase 预设", extensions: ["json"] }]);
    if (!path) return;
    try {
      await api.presetExport(current.id, path);
      toast.success("已导出预设");
    } catch (e) {
      toast.error(String(e));
    }
  };
  const del = async () => {
    if (!current || current.builtin) return;
    await api.presetDelete(current.id);
    await refresh();
    useStore.setState({ presetId: null });
  };

  return (
    <div className="row">
      <Menu
        trigger={({ toggle, open }) => (
          <Button className="grow" onClick={toggle} aria-haspopup="menu" aria-expanded={open} style={{ justifyContent: "space-between" }}>
            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{current?.name ?? t("ins.presetCustom")}</span>
            <span className="muted">▾</span>
          </Button>
        )}
        items={[
          { label: "内置预设", heading: true },
          ...presets.filter((p) => p.builtin).map((p) => ({ label: p.name, checked: p.id === presetId, onSelect: () => applyPreset(p) })),
          ...(presets.some((p) => !p.builtin) ? [{ label: "我的预设", heading: true }] : []),
          ...presets.filter((p) => !p.builtin).map((p) => ({ label: p.name, checked: p.id === presetId, onSelect: () => applyPreset(p) })),
        ]}
      />
      <Menu
        align="right"
        trigger={({ toggle }) => (
          <Button onClick={toggle} aria-label="预设操作">
            ⋯
          </Button>
        )}
        items={[
          { label: t("ins.savePreset"), onSelect: save },
          { label: t("ins.importPreset"), onSelect: importP },
          { label: t("ins.exportPreset"), onSelect: exportP, disabled: !current },
          { separator: true, label: "" },
          { label: t("ins.deletePreset"), onSelect: del, disabled: !current || current.builtin },
        ]}
      />
    </div>
  );
}

function QualityGroup() {
  const s = useStore((st) => st.settings)!;
  const choice = useStore((st) => st.choice);
  const update = useStore((st) => st.updateSettings);
  const preset = qualityPreset(s);
  const q = choice?.quality;
  const rate = s.video.rate;
  return (
    <div className="ins__group">
      <div className="ins__label">{t("ins.quality")}</div>
      <Segmented
        label={t("ins.quality")}
        value={preset}
        onChange={(p) =>
          update((x) => {
            if (p === "custom") return;
            x.video.level = LEVELS[p as keyof typeof LEVELS];
            x.video.rate = { kind: "level" };
          })
        }
        options={[
          { value: "high", label: t("q.high") },
          { value: "balanced", label: t("q.balanced") },
          { value: "small", label: t("q.small") },
          { value: "custom", label: t("q.custom"), disabled: preset !== "custom" },
        ]}
      />
      {rate.kind === "level" || rate.kind === "native" ? (
        <>
          <div className="ins__readout">
            <span className="muted">{t("q.level")}</span>
            {q && choice?.label && (
              <Tooltip text={t("q.nativeHint", { encoder: choice.label })}>
                <span className="ins__value" tabIndex={0}>
                  {choice.label} {q.kind} {Number.isInteger(q.value) ? q.value : q.value.toFixed(1)} <Info size={12} style={{ verticalAlign: -1 }} className="muted" />
                </span>
              </Tooltip>
            )}
          </div>
          <Slider
            label={t("q.level")}
            min={0}
            max={100}
            value={s.video.level}
            onChange={(n) =>
              update((x) => {
                x.video.level = n;
                x.video.rate = { kind: "level" };
              })
            }
          />
          <div className="ticks" aria-hidden>
            <span>画质优先</span>
            <span>体积优先</span>
          </div>
          <p className="ins__explain">{t("q.sizeUnknown")}</p>
        </>
      ) : (
        <p className="ins__explain">
          {rate.kind === "bitrate" ? `目标码率 ${rate.kbps} kbps` : `目标体积 ${rate.megabytes} MB`}（在“高级设置”中修改）。体积按码率估算，实际会有偏差。
        </p>
      )}
    </div>
  );
}

function AudioGroup() {
  const s = useStore((st) => st.settings)!;
  const caps = useStore((st) => st.caps);
  const update = useStore((st) => st.updateSettings);
  const a = s.audio;
  const codecs: { value: AudioCodec; label: string; encoder: string }[] = [
    { value: "aac", label: "AAC", encoder: "aac" },
    { value: "opus", label: "Opus", encoder: "libopus" },
    { value: "mp3", label: "MP3", encoder: "libmp3lame" },
    { value: "flac", label: "FLAC（无损）", encoder: "flac" },
  ];
  return (
    <div className="ins__group">
      <div className="ins__label">{t("ins.audio")}</div>
      <Segmented
        label={t("ins.audio")}
        value={a.mode}
        onChange={(m) => update((x) => void (x.audio.mode = m))}
        options={[
          { value: "copy", label: t("audio.copy"), hint: "原样保留音频数据，不损失音质，完成后用数据哈希校验" },
          { value: "transcode", label: t("audio.transcode") },
          { value: "none", label: t("audio.none") },
        ]}
      />
      {a.mode === "transcode" && (
        <div className="row">
          <select className="select grow" aria-label={t("audio.codec")} value={a.codec} onChange={(e) => update((x) => void (x.audio.codec = e.target.value as AudioCodec))}>
            {codecs.map((c) => (
              <option key={c.value} value={c.value} disabled={caps ? !caps.audioEncoders.includes(c.encoder) : false}>
                {c.label}
              </option>
            ))}
          </select>
          <select
            className="select"
            aria-label={t("audio.bitrate")}
            value={a.bitrateKbps}
            disabled={a.codec === "flac"}
            onChange={(e) => update((x) => void (x.audio.bitrateKbps = Number(e.target.value)))}
          >
            {[96, 128, 160, 192, 256, 320].map((k) => (
              <option key={k} value={k}>
                {k} kbps
              </option>
            ))}
          </select>
        </div>
      )}
      {a.mode !== "none" && (
        <div className="row">
          <select className="select grow" aria-label={t("audio.tracks")} value={a.tracks} onChange={(e) => update((x) => void (x.audio.tracks = e.target.value as "all" | "first"))}>
            <option value="all">{t("audio.tracksAll")}</option>
            <option value="first">{t("audio.tracksFirst")}</option>
          </select>
          {a.mode === "copy" && (
            <Tooltip text={t("audio.onIncompatible")}>
              <select
                className="select"
                aria-label={t("audio.onIncompatible")}
                value={a.onIncompatible}
                onChange={(e) => update((x) => void (x.audio.onIncompatible = e.target.value as "ask" | "transcode" | "drop"))}
              >
                <option value="ask">{t("audio.ask")}</option>
                <option value="transcode">{t("audio.toAac")}</option>
                <option value="drop">{t("audio.dropTrack")}</option>
              </select>
            </Tooltip>
          )}
        </div>
      )}
    </div>
  );
}

function SubtitleGroup({ libass }: { libass: boolean }) {
  const s = useStore((st) => st.settings)!;
  const update = useStore((st) => st.updateSettings);
  const sub = s.subtitles;
  const reencode = s.video.mode === "lossy";
  const addExternal = async () => {
    const paths = await pickPaths({ multiple: true, filters: [{ name: "字幕", extensions: ["srt", "ass", "ssa", "vtt", "sup"] }] });
    if (paths.length) update((x) => x.subtitles.external.push(...paths.map((p) => ({ path: p, language: null }))));
  };
  const burnExternal = async () => {
    const [p] = await pickPaths({ multiple: false, filters: [{ name: "字幕", extensions: ["srt", "ass", "ssa", "vtt", "sup"] }] });
    if (p) update((x) => void (x.subtitles.burn = { kind: "external", path: p }));
  };
  return (
    <div className="ins__group">
      <div className="ins__label">{t("ins.subtitles")}</div>
      <Segmented
        label={t("ins.subtitles")}
        value={sub.mode}
        onChange={(m) =>
          update((x) => {
            x.subtitles.mode = m;
            if (m === "burn" && !x.subtitles.burn) x.subtitles.burn = { kind: "embedded", ordinal: 0 };
          })
        }
        options={[
          { value: "keep", label: t("subs.keep"), hint: "软字幕：保留在文件里，播放时可开关。MP4 中文字字幕会转为 mov_text" },
          { value: "none", label: t("subs.none") },
          { value: "burn", label: t("subs.burn"), disabled: !reencode, hint: reencode ? "把字幕画进画面，无法再关闭" : "烧录需要有损重新编码" },
        ]}
      />
      {sub.mode === "keep" && (
        <>
          {sub.external.map((e, i) => (
            <div className="row small" key={e.path}>
              <span className="grow" title={e.path} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {fileName(e.path)}
              </span>
              <input
                className="input"
                style={{ width: 64 }}
                placeholder={t("mux.language")}
                value={e.language ?? ""}
                onChange={(ev) => update((x) => void (x.subtitles.external[i].language = ev.target.value || null))}
              />
              <button className="icon-btn" aria-label="移除" onClick={() => update((x) => void x.subtitles.external.splice(i, 1))}>
                ×
              </button>
            </div>
          ))}
          <Button size="sm" variant="ghost" onClick={addExternal} style={{ alignSelf: "flex-start" }}>
            {t("subs.addExternal")}
          </Button>
        </>
      )}
      {sub.mode === "burn" && (
        <>
          <div className="row">
            <select
              className="select grow"
              aria-label="烧录来源"
              value={sub.burn?.kind === "external" ? "external" : String(sub.burn?.kind === "embedded" ? sub.burn.ordinal : 0)}
              onChange={(e) => {
                if (e.target.value === "external") burnExternal();
                else update((x) => void (x.subtitles.burn = { kind: "embedded", ordinal: Number(e.target.value) }));
              }}
            >
              {[0, 1, 2, 3].map((n) => (
                <option key={n} value={n}>
                  内嵌字幕第 {n + 1} 条
                </option>
              ))}
              <option value="external">{sub.burn?.kind === "external" ? fileName(sub.burn.path) : "外挂字幕文件…"}</option>
            </select>
          </div>
          {!libass && <p className="ins__explain bad-text">{t("subs.burnNoLibass")}</p>}
        </>
      )}
    </div>
  );
}

function OutputGroup() {
  const s = useStore((st) => st.settings)!;
  const update = useStore((st) => st.updateSettings);
  const o = s.output;
  const choose = async () => {
    const [dir] = await pickPaths({ directory: true, multiple: false });
    if (dir)
      update((x) => {
        x.output.dir = "custom";
        x.output.customDir = dir;
      });
  };
  return (
    <div className="ins__group">
      <div className="ins__label">{t("ins.output")}</div>
      <Segmented
        label={t("out.container")}
        value={o.container}
        onChange={(c) => update((x) => void (x.output.container = c))}
        options={[
          { value: "mp4", label: "MP4", disabled: s.video.codec === "ffv1" && s.video.mode !== "copy", hint: "兼容性最好" },
          { value: "mkv", label: "MKV", hint: "几乎能放任何轨道，适合多音轨/字幕/无损" },
        ]}
      />
      <div className="row">
        <select
          className="select grow"
          aria-label={t("out.location")}
          value={o.dir}
          onChange={(e) => {
            if (e.target.value === "custom") choose();
            else update((x) => void (x.output.dir = "besideSource"));
          }}
        >
          <option value="besideSource">{t("out.beside", { name: o.subfolder || "同一" })}</option>
          <option value="custom">{o.customDir ? o.customDir : t("out.custom")}</option>
        </select>
        <Button onClick={choose} aria-label={t("out.choose")}>
          <FolderOpen size={15} />
        </Button>
      </div>
      <Field label={t("out.name")} hint={t("out.nameHint")}>
        <input className="input" value={o.nameTemplate} onChange={(e) => update((x) => void (x.output.nameTemplate = e.target.value))} spellCheck={false} />
      </Field>
      <div className="row">
        <span className="small muted">{t("out.conflict")}</span>
        <div className="grow">
          <Segmented
            label={t("out.conflict")}
            value={o.conflict}
            onChange={(c) => update((x) => void (x.output.conflict = c))}
            options={[
              { value: "rename", label: t("out.rename"), hint: "生成“名字 (2).mp4”，从不覆盖已有文件" },
              { value: "skip", label: t("out.skip"), hint: "已有同名输出时跳过该文件" },
            ]}
          />
        </div>
      </div>
    </div>
  );
}

function Advanced() {
  const s = useStore((st) => st.settings)!;
  const choice = useStore((st) => st.choice);
  const update = useStore((st) => st.updateSettings);
  const v = s.video;
  const lossy = v.mode === "lossy";
  const tf = v.transforms;
  const rate = v.rate;
  return (
    <>
      {lossy && (
        <Field label={t("adv.rate")}>
          <Segmented
            label={t("adv.rate")}
            value={rate.kind === "native" ? "level" : rate.kind}
            onChange={(k) =>
              update((x) => {
                if (k === "level") x.video.rate = { kind: "level" };
                if (k === "bitrate") x.video.rate = { kind: "bitrate", kbps: 4000, twoPass: false };
                if (k === "targetSize") x.video.rate = { kind: "targetSize", megabytes: 100, twoPass: false };
              })
            }
            options={[
              { value: "level", label: t("rate.level") },
              { value: "bitrate", label: t("rate.bitrate"), disabled: choice ? !choice.bitrate : false },
              { value: "targetSize", label: t("rate.size"), disabled: choice ? !choice.bitrate : false },
            ]}
          />
          {(rate.kind === "level" || rate.kind === "native") && choice?.quality && (
            <div className="row small">
              <span className="muted grow">{t("rate.native", { kind: choice.quality.kind })}</span>
              <input
                className="input input--num"
                type="number"
                min={choice.quality.min}
                max={choice.quality.max}
                step={1}
                value={Number.isInteger(choice.quality.value) ? choice.quality.value : choice.quality.value.toFixed(1)}
                onChange={(e) => {
                  const value = Number(e.target.value);
                  if (Number.isFinite(value)) update((x) => void (x.video.rate = { kind: "native", value }));
                }}
              />
            </div>
          )}
          {rate.kind === "bitrate" && (
            <div className="row small">
              <span className="muted grow">平均码率</span>
              <input className="input input--num" type="number" min={100} value={rate.kbps} onChange={(e) => update((x) => void (x.video.rate = { ...rate, kbps: Math.max(50, Number(e.target.value)) }))} />
              <span className="muted">kbps</span>
            </div>
          )}
          {rate.kind === "targetSize" && (
            <div className="row small">
              <span className="muted grow">整个文件</span>
              <input className="input input--num" type="number" min={1} value={rate.megabytes} onChange={(e) => update((x) => void (x.video.rate = { ...rate, megabytes: Math.max(1, Number(e.target.value)) }))} />
              <span className="muted">MB</span>
            </div>
          )}
          {(rate.kind === "bitrate" || rate.kind === "targetSize") && (
            <Check checked={rate.twoPass} disabled={!choice?.twoPass} onChange={(c) => update((x) => void (x.video.rate = { ...rate, twoPass: c }))}>
              {choice?.twoPass ? t("rate.twoPass") : t("rate.twoPassNo", { encoder: choice?.label ?? "" })}
            </Check>
          )}
        </Field>
      )}

      {v.mode !== "copy" && choice && choice.options.length > 0 && (
        <Field label={t("adv.encoder")}>
          <select
            className="select"
            value={v.encoder ?? ""}
            onChange={(e) =>
              update((x) => {
                x.video.encoder = e.target.value || null;
                if (e.target.value && x.video.speed === "gpu") x.video.speed = "balanced";
              })
            }
          >
            <option value="">{t("adv.encoderAuto")}</option>
            {choice.options.map((o) => (
              <option key={o.name} value={o.name} disabled={!o.usable || (v.mode === "lossless" && o.hardware)}>
                {o.label}（{o.hardware ? "GPU" : "CPU"}）{o.usable ? "" : " 不可用"}
              </option>
            ))}
          </select>
        </Field>
      )}

      {lossy && (
        <>
          <Field label={t("adv.resolution")}>
            <select
              className="select"
              value={tf.scale?.kind === "maxHeight" ? String(tf.scale.height) : ""}
              onChange={(e) => update((x) => void (x.video.transforms.scale = e.target.value ? { kind: "maxHeight", height: Number(e.target.value) } : null))}
            >
              <option value="">{t("adv.keep")}</option>
              {[2160, 1440, 1080, 720, 480].map((h) => (
                <option key={h} value={h}>
                  {t("adv.maxHeight", { h })}
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("adv.fps")}>
            <select className="select" value={tf.fps ?? ""} onChange={(e) => update((x) => void (x.video.transforms.fps = e.target.value ? Number(e.target.value) : null))}>
              <option value="">{t("adv.fpsKeep")}</option>
              {[60, 50, 30, 25, 24].map((f) => (
                <option key={f} value={f}>
                  {f} fps
                </option>
              ))}
            </select>
          </Field>
          <Field label={t("adv.rotate")}>
            <select className="select" value={tf.rotate ?? ""} onChange={(e) => update((x) => void (x.video.transforms.rotate = (e.target.value || null) as Rotate | null))}>
              <option value="">{t("adv.none")}</option>
              <option value="cw90">顺时针 90°</option>
              <option value="ccw90">逆时针 90°</option>
              <option value="r180">180°</option>
              <option value="flipH">水平翻转</option>
              <option value="flipV">垂直翻转</option>
            </select>
          </Field>
          <Field label={t("adv.crop")} hint="按显示方向，从四边各裁去的像素；奇数会向下取偶">
            <div className="row">
              {(["left", "top", "right", "bottom"] as const).map((k) => (
                <input
                  key={k}
                  className="input input--num"
                  style={{ width: "25%" }}
                  type="number"
                  min={0}
                  aria-label={{ left: "左", top: "上", right: "右", bottom: "下" }[k]}
                  placeholder={{ left: "左", top: "上", right: "右", bottom: "下" }[k]}
                  value={tf.crop?.[k] || ""}
                  onChange={(e) =>
                    update((x) => {
                      const c = x.video.transforms.crop ?? { left: 0, top: 0, right: 0, bottom: 0 };
                      c[k] = Math.max(0, Number(e.target.value) || 0);
                      x.video.transforms.crop = c.left + c.top + c.right + c.bottom > 0 ? c : null;
                    })
                  }
                />
              ))}
            </div>
          </Field>
          <Field label={t("adv.colorSafety")} hint={t("adv.colorSafetyHint")}>
            <Check checked={v.allowBitDepthReduction} onChange={(c) => update((x) => void (x.video.allowBitDepthReduction = c))}>
              {t("adv.allowBitDepth")}
            </Check>
            <Check checked={v.allowChromaSubsampling} onChange={(c) => update((x) => void (x.video.allowChromaSubsampling = c))}>
              {t("adv.allowChroma")}
            </Check>
          </Field>
          <Check checked={v.hwDecode} onChange={(c) => update((x) => void (x.video.hwDecode = c))}>
            {t("adv.hwDecode")}
          </Check>
        </>
      )}
      {s.output.container === "mp4" && (
        <Check checked={s.output.faststart} onChange={(c) => update((x) => void (x.output.faststart = c))}>
          {t("adv.faststart")}
        </Check>
      )}
      <Check
        checked={s.output.keepMetadata && s.output.keepChapters}
        onChange={(c) =>
          update((x) => {
            x.output.keepMetadata = c;
            x.output.keepChapters = c;
          })
        }
      >
        {t("adv.keepMeta")}
      </Check>
    </>
  );
}

function Summary() {
  const s = useStore((st) => st.settings)!;
  const choice = useStore((st) => st.choice);
  const staged = useStore((st) => st.staged);
  const v = s.video;
  const q = choice?.quality;
  const estimates = staged.map((x) => x.preview?.summary).filter(Boolean);
  const reliable = estimates.length > 0 && estimates.every((e) => e!.estReliability !== "unknown" && e!.estSize);
  const total = reliable ? estimates.reduce((a, e) => a + (e!.estSize ?? 0), 0) : null;
  const quality =
    v.rate.kind === "bitrate"
      ? `${v.rate.kbps} kbps`
      : v.rate.kind === "targetSize"
        ? `目标 ${v.rate.megabytes} MB`
        : q
          ? `${choice?.label} ${q.kind} ${Number.isInteger(q.value) ? q.value : q.value.toFixed(1)}`
          : "—";
  const where = s.output.dir === "custom" && s.output.customDir ? s.output.customDir : `源文件旁的“${s.output.subfolder}”`;
  const a = s.audio;
  return (
    <div className="summary" aria-live="polite">
      <div className="ins__label">{t("ins.summary")}</div>
      <div className="summary__line">
        <ShieldCheck size={14} className="ok-text" />
        <span>{t("sum.keepSource")}</span>
      </div>
      <div className="summary__line">
        <FileVideo2 size={14} />
        <span>
          输出 <strong>{v.mode === "copy" ? "原视频编码" : codecLabel(v.codec)}</strong>，封装为 <strong>{s.output.container.toUpperCase()}</strong>
        </span>
      </div>
      <div className="summary__line">
        <Gauge size={14} />
        <span>
          {v.mode === "lossy" && (
            <>
              <strong>有损压缩</strong>，{quality}
            </>
          )}
          {v.mode === "lossless" && <strong>{t("sum.lossless")}</strong>}
          {v.mode === "copy" && <strong>{t("sum.copy")}</strong>}
        </span>
      </div>
      <div className="summary__line">
        <Ruler size={14} />
        <span>{total ? t("sum.sizeEstimate", { size: bytes(total) }) : v.mode === "lossless" ? "体积：通常明显大于原文件" : t("sum.sizeUnknown")}</span>
      </div>
      <div className="summary__line">
        <AudioLines size={14} />
        <span>{a.mode === "copy" ? t("sum.audioCopy") : a.mode === "none" ? t("sum.audioNone") : t("sum.audioTranscode", { codec: a.codec.toUpperCase(), kbps: a.codec === "flac" ? "无损" : a.bitrateKbps })}</span>
      </div>
      <div className="summary__line">
        <FolderOpen size={14} />
        <span style={{ overflowWrap: "anywhere" }}>{t("sum.where", { where })}</span>
      </div>
    </div>
  );
}
