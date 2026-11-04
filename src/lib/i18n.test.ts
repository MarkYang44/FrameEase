import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import source from "./i18n.ts?raw";
import { checkName, checkValue, notice, presetName, setLocale, t } from "./i18n";
import { humanDuration } from "./format";

beforeAll(() => vi.stubGlobal("document", { documentElement: { lang: "" } }));
afterEach(() => setLocale("zh-CN"));

describe("English localization", () => {
  it("translates all existing UI and backend notice keys without a Chinese fallback", () => {
    const keys = [...source.matchAll(/^  "([\w.-]+)":/gm)].map((m) => m[1]);
    setLocale("en");
    for (const key of new Set(keys)) {
      expect(t(key), key).not.toMatch(/\p{Script=Han}/u);
      expect(t(key), key).not.toBe(key);
    }
    expect(keys.length).toBeGreaterThan(350);
  });

  it("interpolates notices and verification labels in English", () => {
    setLocale("en");
    expect(notice("issue", { code: "audioIncompatible", params: { track: "2", codec: "pcm", container: "MP4" } })).toBe("Audio track 2 (pcm) cannot be copied to MP4");
    expect(checkName("streamCopy:2")).toBe("Track 2 data hash");
    expect(notice("err", { code: "futureBackendError" })).toBe("futureBackendError");
  });

  it("uses English duration units and switches back to Chinese", () => {
    setLocale("en");
    expect(humanDuration(12)).toBe("12 sec");
    expect(humanDuration(83)).toBe("1 min 23 sec");
    expect(humanDuration(3725)).toBe("1 hr 2 min");
    setLocale("zh-CN");
    expect(humanDuration(83)).toBe("1 分 23 秒");
  });

  it("keeps dictionary keys and interpolation parameters aligned", () => {
    const dictionaries = [...source.matchAll(/^  "([\w.-]+)": ("(?:[^"\\]|\\.)*")/gm)];
    const halves = [dictionaries.slice(0, dictionaries.length / 2), dictionaries.slice(dictionaries.length / 2)];
    const params = (value: string) => [...value.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    expect(halves[0].map((m) => m[1])).toEqual(halves[1].map((m) => m[1]));
    halves[0].forEach((m, i) => expect(params(m[2]), m[1]).toEqual(params(halves[1][i][2])));
  });

  it("localizes the failed check names shown in job errors", () => {
    setLocale("en");
    expect(notice("err", { code: "verifyFailed", params: { checks: "pixFmt, streamCopy:2" } })).toBe("Output verification failed: Pixel format, Track 2 data hash");
  });

  it("translates saved verification diagnostics while preserving raw backend output", () => {
    setLocale("en");
    expect(checkValue("8 帧全部一致")).toBe("All 8 frames match");
    expect(checkValue("第 3 帧起不一致")).toBe("Frames differ starting at frame 3");
    expect(checkValue("完整解码无错误")).toBe("Full decode without errors");
    expect(checkValue("yuv420p10le")).toBe("yuv420p10le");
  });

  it("translates built-in presets without altering user-created names", () => {
    setLocale("en");
    expect(presetName({ id: "builtin-h264-balanced", name: "H.264 · 均衡（兼容性最好）", builtin: true })).toBe("H.264 · Balanced (compatible)");
    expect(presetName({ id: "user", name: "我的预设", builtin: false })).toBe("我的预设");
  });
});
