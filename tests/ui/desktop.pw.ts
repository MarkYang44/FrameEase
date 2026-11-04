import { expect, test, type Page } from "@playwright/test";
import type { useStore } from "../../src/lib/store";
import type { MediaInfo } from "../../src/lib/types";

declare global {
  interface Window { __fe: typeof useStore; }
}

// These are layout/localization checks against the labeled browser preview.
// Encoding correctness is covered separately by the Rust/FFmpeg integration tests.
async function open(page: Page) {
  await page.goto("/");
  await expect(page.locator(".ins__value")).toBeVisible();
}

async function language(page: Page, locale: "en" | "zh-CN") {
  await page.locator(".nav__item").last().click();
  await page.locator("select").filter({ has: page.locator('option[value="en"]') }).selectOption(locale);
  await expect(page.locator("html")).toHaveAttribute("lang", locale);
}

test("settings stays at the window bottom across views, sizes, and notifications", async ({ page }) => {
  await open(page);
  for (const [width, height] of [[1280, 820], [920, 620], [1440, 1000]]) {
    await page.setViewportSize({ width, height });
    for (const view of [3, 4, 1, 2, 0]) {
      await page.locator(".nav__item").nth(view).click();
      await expect.poll(() => page.locator(".nav").evaluate((el) => el.getBoundingClientRect().bottom)).toBe(height);
      const button = await page.locator(".nav__item").last().boundingBox();
      expect(button!.y + button!.height).toBeCloseTo(height - 14, 0);
    }
  }
  await page.evaluate(async () => {
    const { toast } = await import("/node_modules/.vite/deps/sonner.js");
    toast("Layout regression check");
  });
  await expect(page.locator(".nav")).toHaveCSS("height", "1000px");
});

test("the inspector wraps long values and cannot pan horizontally", async ({ page }) => {
  await open(page);
  await page.setViewportSize({ width: 920, height: 620 });
  await page.getByRole("button", { name: "高级设置", exact: true }).click();
  await page.evaluate(async () => {
    const st = window.__fe.getState();
    window.__fe.setState({ choice: { ...st.choice!, label: "hardware_encoder_".repeat(16) } });
  });
  await expect(page.locator(".ins__value")).toContainText("hardware_encoder_".repeat(16));
  await expect.poll(() => page.locator(".inspector").evaluate((el) => el.scrollWidth - el.clientWidth)).toBe(0);
  await page.locator(".inspector").evaluate((el) => { el.scrollLeft = 1000; el.scrollTop = 500; });
  expect(await page.locator(".inspector").evaluate((el) => el.scrollLeft)).toBe(0);
  expect(await page.locator(".inspector").evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
});

test("switching language refreshes unchanged memoized queue rows", async ({ page }) => {
  await open(page);
  const done = page.locator('.item[data-state="done"]');
  await page.getByRole("button", { name: "详情", exact: true }).first().click();
  // Keep the row mounted, with unchanged job props, while changing the locale.
  await page.evaluate(() => window.__fe.getState().setConfig((config) => { config.locale = "en"; }));
  await expect(done).toContainText("Verified");
  await expect(done).toContainText("Elapsed 5 min 12 sec");
  await expect(done).toContainText("Track 1 data hash");
  await page.evaluate(() => window.__fe.getState().setConfig((config) => { config.locale = "zh-CN"; }));
  await expect(done).toContainText("已校验");
  await expect(done).toContainText("用时 5 分 12 秒");
  await expect(done).toContainText("轨道 1 数据哈希");
});

test("all inspector modes and preset menus fit in an English minimum-size window", async ({ page }) => {
  await open(page);
  await language(page, "en");
  await page.setViewportSize({ width: 920, height: 620 });
  await page.locator(".nav__item").first().click();
  const inspector = page.locator(".inspector");
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
  for (const mode of ["Lossless", "Stream copy", "Lossy"]) {
    await inspector.getByRole("radio", { name: mode, exact: true }).click();
    await expect.poll(() => inspector.evaluate((el) => el.scrollWidth - el.clientWidth)).toBe(0);
    const text = await inspector.innerText();
    // This folder is persisted user data and deliberately keeps its original name.
    expect(text.replaceAll("FrameEase 输出", "output")).not.toMatch(/\p{Script=Han}/u);
  }
  await inspector.getByRole("button", { name: /Current settings/ }).click();
  await expect(page.getByRole("menu")).toContainText("H.264 · Balanced (compatible)");
  await expect(page.getByRole("menu")).not.toContainText("均衡");
  await page.keyboard.press("Escape");
  // The closing menu stays mounted for its exit animation on slower machines.
  await expect(page.getByRole("menu")).toHaveCount(0);
  await inspector.getByRole("button", { name: "Preset actions" }).click();
  await expect(page.getByRole("menu")).toBeVisible();
  const box = await page.getByRole("menu").boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(920);
});

test("media details, trimming, and track tools use English after a file is loaded", async ({ page, context }) => {
  await open(page);
  await language(page, "en");
  const media: MediaInfo = {
    path: "/samples/input.mp4", size: 123456, formatName: "mov,mp4", formatLongName: "MP4",
    duration: 30, startTime: 0, bitRate: 1000000, attachments: 0, dataStreams: 0, chapters: 1, tags: {},
    video: [{
      index: 0, codec: "h264", codecLong: "H.264", profile: "High", level: 41,
      width: 1920, height: 1080, sar: "1:1", dar: "16:9", pixFmt: "yuv420p", bitDepth: 8,
      chroma: "420", alpha: false, rgb: false, colorRange: "tv", colorSpace: "bt709",
      colorTransfer: "bt709", colorPrimaries: "bt709", hdr: "sdr", masteringDisplay: null,
      contentLight: null, dolbyVision: null, rotation: 0, rFrameRate: null, avgFrameRate: null,
      fps: 30, vfr: false, fieldOrder: "progressive", interlaced: false, nbFrames: 900,
      duration: 30, bitRate: 800000, attachedPic: false, default: true, language: null, title: null,
    }],
    audio: [{ index: 1, codec: "aac", codecLong: "AAC", profile: "LC", channels: 2,
      channelLayout: "stereo", sampleRate: 48000, sampleFmt: "fltp", bitRate: 192000,
      duration: 30, language: "eng", title: "Studio voice", default: true }],
    subtitles: [{ index: 2, codec: "mov_text", language: "eng", title: "Captions", default: true, forced: false, bitmap: false }],
  };
  await page.evaluate(async (media) => {
    const url = performance.getEntriesByType("resource").map((r) => r.name).find((url) => new URL(url).pathname === "/src/lib/api.ts");
    const { api } = await import(url!);
    api.probeFiles = async (paths: string[]) => paths.map((path) => ({ path, info: { ...media, path }, error: null }));
    api.keyframes = async () => [0, 10, 20];
    api.extractFrame = async () => "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
  }, media);
  for (const view of [1, 2, 3]) {
    await page.locator(".nav__item").nth(view).click();
    await page.evaluate(() => window.__fe.getState().dropHandler?.(["/samples/input.mp4"]));
    await expect(page.locator(".page__inner")).toContainText("input.mp4");
    expect(await page.locator(".page__inner").innerText()).not.toMatch(/\p{Script=Han}/u);
    if (view === 2) {
      await page.getByRole("tab", { name: "Extract tracks" }).click();
      await page.evaluate(() => window.__fe.getState().dropHandler?.(["/samples/input.mp4"]));
      await expect(page.locator(".table")).toContainText("2 channels");
      expect(await page.locator(".page__inner").innerText()).not.toMatch(/\p{Script=Han}/u);
    }
  }
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.getByRole("button", { name: "Copy as text" }).click();
  const report = await page.evaluate(() => navigator.clipboard.readText());
  expect(report).toContain("Audio #1: aac 2 channels");
  expect(report).not.toMatch(/\p{Script=Han}/u);
});
