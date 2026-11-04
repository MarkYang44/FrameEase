import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const require = createRequire(import.meta.url);
const cli = join(dirname(require.resolve("@tauri-apps/cli/package.json")), "tauri.js");
const generated = mkdtempSync(join(tmpdir(), "frameease-icons-"));
try {
  const result = spawnSync(process.execPath, [cli, "icon", join(root, "assets/app-icon.png"), "--output", generated], { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Icon generation failed (${result.status})`);
  // Desktop assets only; Tauri also generates mobile directories by default.
  for (const entry of readdirSync(generated, { withFileTypes: true })) {
    if (entry.isFile()) copyFileSync(join(generated, entry.name), join(root, "src-tauri/icons", entry.name));
  }
} finally {
  rmSync(generated, { recursive: true, force: true });
}
