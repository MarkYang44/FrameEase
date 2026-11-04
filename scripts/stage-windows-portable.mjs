import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../", import.meta.url));

/** Stage only the files needed to run the release executable outside an installer. */
export function stageWindowsPortable(root = projectRoot) {
  const version = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
  if (!/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(version)) throw new Error("Invalid portable package version");
  const release = join(root, "target/release");
  const ffmpeg = join(root, "src-tauri/resources/ffmpeg");
  const files = [
    [join(release, "frameease.exe"), "FrameEase.exe"],
    [join(root, "LICENSE"), "LICENSE"],
    [join(root, "docs/THIRD_PARTY_LICENSES.md"), "THIRD_PARTY_LICENSES.md"],
    [join(root, "docs/WINDOWS_PORTABLE.txt"), "README.txt"],
  ];
  const required = ["ffmpeg.exe", "ffprobe.exe", "LICENSE.txt", "LICENSE-NOTICE.txt", "VERSION.txt", "BUILDCONF.txt"];
  for (const path of [...files.map(([path]) => path), ...required.map((name) => join(ffmpeg, name))]) {
    if (!existsSync(path) || !statSync(path).isFile()) throw new Error(`Required portable file is missing: ${path}`);
  }
  if (!readdirSync(ffmpeg).some((name) => /\.dll$/i.test(name))) throw new Error("The shared FFmpeg build is missing its DLLs");

  const loader = join(release, "WebView2Loader.dll");
  if (existsSync(loader)) files.push([loader, basename(loader)]);
  const name = `FrameEase_${version}_windows-x64-portable`;
  const directory = join(release, "bundle/portable", name);
  rmSync(directory, { recursive: true, force: true });
  mkdirSync(directory, { recursive: true });
  for (const [source, target] of files) cpSync(source, join(directory, target));
  cpSync(ffmpeg, join(directory, "ffmpeg"), { recursive: true });
  return { version, directory, zip: `${directory}.zip` };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.stdout.write(`${JSON.stringify(stageWindowsPortable())}\n`);
}
