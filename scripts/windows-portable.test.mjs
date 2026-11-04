import { afterEach, describe, expect, it } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stageWindowsPortable } from "./stage-windows-portable.mjs";

const roots = [];
afterEach(() => roots.splice(0).forEach((root) => rmSync(root, { recursive: true, force: true })));

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "frameease-portable-"));
  roots.push(root);
  const write = (file, text) => {
    const path = join(root, file);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, text);
  };
  write("src-tauri/tauri.conf.json", JSON.stringify({ version: "0.1.1" }));
  write("target/release/frameease.exe", "app binary");
  write("src-tauri/resources/ffmpeg/ffmpeg.exe", "encoder binary");
  write("src-tauri/resources/ffmpeg/ffprobe.exe", "probe binary");
  write("src-tauri/resources/ffmpeg/avcodec-62.dll", "codec library");
  write("src-tauri/resources/ffmpeg/LICENSE.txt", "FFmpeg license");
  write("src-tauri/resources/ffmpeg/LICENSE-NOTICE.txt", "FFmpeg notice");
  write("src-tauri/resources/ffmpeg/BUILDCONF.txt", "FFmpeg build configuration");
  write("src-tauri/resources/ffmpeg/VERSION.txt", "FFmpeg version");
  write("docs/THIRD_PARTY_LICENSES.md", "third-party notices");
  write("docs/WINDOWS_PORTABLE.txt", "bilingual portable instructions");
  write("LICENSE", "project license");
  return { root, write };
}

describe("Windows portable distribution staging", () => {
  it("keeps the executable and all FFmpeg DLLs/notices together", () => {
    const { root } = fixture();
    const result = stageWindowsPortable(root);
    expect(result.version).toBe("0.1.1");
    expect(result.zip).toMatch(/FrameEase_0\.1\.1_windows-x64-portable\.zip$/);
    expect(readFileSync(join(result.directory, "FrameEase.exe"), "utf8")).toBe("app binary");
    expect(readFileSync(join(result.directory, "ffmpeg/avcodec-62.dll"), "utf8")).toBe("codec library");
    expect(readFileSync(join(result.directory, "ffmpeg/LICENSE-NOTICE.txt"), "utf8")).toBe("FFmpeg notice");
    expect(readFileSync(join(result.directory, "THIRD_PARTY_LICENSES.md"), "utf8")).toBe("third-party notices");
    expect(readFileSync(join(result.directory, "README.txt"), "utf8")).toBe("bilingual portable instructions");
    expect(existsSync(join(result.directory, "LICENSE"))).toBe(true);
  });

  it("carries the WebView2 loader when the build emits one", () => {
    const { root, write } = fixture();
    write("target/release/WebView2Loader.dll", "loader");
    const result = stageWindowsPortable(root);
    expect(readFileSync(join(result.directory, "WebView2Loader.dll"), "utf8")).toBe("loader");
  });

  it("rejects incomplete FFmpeg resources instead of producing a broken package", () => {
    const { root } = fixture();
    rmSync(join(root, "src-tauri/resources/ffmpeg/ffprobe.exe"));
    expect(() => stageWindowsPortable(root)).toThrow(/ffprobe.exe/);
  });

  it("rejects a shared FFmpeg folder that is missing its DLLs", () => {
    const { root } = fixture();
    rmSync(join(root, "src-tauri/resources/ffmpeg/avcodec-62.dll"));
    expect(() => stageWindowsPortable(root)).toThrow(/DLL/);
  });

  it("removes stale output files when rebuilding the same version", () => {
    const { root } = fixture();
    const result = stageWindowsPortable(root);
    writeFileSync(join(result.directory, "old.dll"), "stale");
    stageWindowsPortable(root);
    expect(existsSync(join(result.directory, "old.dll"))).toBe(false);
  });
});
