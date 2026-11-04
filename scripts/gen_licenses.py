#!/usr/bin/env python3
"""Write docs/THIRD_PARTY_LICENSES.md from the real dependency graph:
Rust crates linked into the app (cargo metadata, normal deps only) and npm
production packages (npm ls --omit=dev). FFmpeg is documented separately
because it ships as a separate program."""
import json
import os
import subprocess
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def rust() -> list[tuple[str, str, str]]:
    meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT, check=True, capture_output=True, text=True).stdout)
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [p["id"] for p in meta["packages"] if p["name"] == "frameease"]
    seen, stack = set(), list(roots)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for d in nodes[pid]["deps"]:
            # normal (runtime) dependencies only; build/dev tools are not shipped
            if any(k["kind"] in (None, "normal") for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    out = []
    for pid in seen:
        p = pkgs[pid]
        if p["source"] is None:
            continue  # our own crates
        out.append((p["name"], p["version"], p.get("license") or "see crate"))
    return sorted(out)


def npm() -> list[tuple[str, str, str]]:
    tree = json.loads(subprocess.run(["npm", "ls", "--omit=dev", "--all", "--json"], cwd=ROOT, capture_output=True, text=True).stdout or "{}")
    found = {}

    def walk(deps: dict) -> None:
        for name, info in (deps or {}).items():
            ver = info.get("version")
            if ver is None or info.get("missing"):
                continue  # optional peer dependency that is not installed (not shipped)
            if (name, ver) in found:
                continue
            lic = "?"
            pj = os.path.join(ROOT, "node_modules", name, "package.json")
            if os.path.exists(pj):
                with open(pj, encoding="utf-8") as f:
                    data = json.load(f)
                lic = data.get("license") or (data.get("licenses") or [{}])[0].get("type", "?")
            found[(name, ver)] = lic
            walk(info.get("dependencies"))

    walk(tree.get("dependencies"))
    return sorted((n, v, l) for (n, v), l in found.items())


def table(rows: list[tuple[str, str, str]]) -> str:
    by = defaultdict(int)
    for _, _, l in rows:
        by[l] += 1
    summary = "、".join(f"{l}（{n}）" for l, n in sorted(by.items(), key=lambda x: -x[1]))
    body = "\n".join(f"| {n} | {v} | {l} |" for n, v, l in rows)
    return f"按许可统计：{summary}\n\n| 名称 | 版本 | 许可 |\n|---|---|---|\n{body}\n"


def main() -> None:
    r, n = rust(), npm()
    notice_path = os.path.join(ROOT, "src-tauri", "resources", "ffmpeg", "LICENSE-NOTICE.txt")
    ffmpeg_notice = open(notice_path, encoding="utf-8").read() if os.path.exists(notice_path) else "（尚未打包 FFmpeg：运行 `npm run ffmpeg:bundle` 后重新生成本文件）"
    doc = f"""# 第三方组件与许可

本文件由 `python3 scripts/gen_licenses.py` 根据实际依赖图生成，请勿手工编辑。

## FFmpeg（随应用附带的独立程序）

FrameEase 以子进程方式调用 FFmpeg/ffprobe，不与其链接。随安装包附带的 FFmpeg 构建启用了 GPL 组件（x264、x265 等），因此这些二进制按 GNU GPL 分发；对应源码地址见下方说明。

Windows 安装包附带的是 BtbN/FFmpeg-Builds 的 GPL 共享库构建：`ffmpeg.exe`、`ffprobe.exe` 与 FFmpeg 自身的 DLL，x264 等其余库静态链接在这些 DLL 中。每个安装包的 `ffmpeg` 文件夹都附有所带构建的 `LICENSE-NOTICE.txt`、`BUILDCONF.txt` 与 `VERSION.txt`（Windows 版另附 GPL 全文 `LICENSE.txt`）；下方是生成本文件时所打包构建的说明。

```
{ffmpeg_notice.strip()}
```

## Rust crate（编译进应用的运行时依赖，共 {len(r)} 个）

{table(r)}
## npm 包（打包进界面的生产依赖，共 {len(n)} 个）

{table(n)}"""
    os.makedirs(os.path.join(ROOT, "docs"), exist_ok=True)
    with open(os.path.join(ROOT, "docs", "THIRD_PARTY_LICENSES.md"), "w", encoding="utf-8") as f:
        f.write(doc)
    print(f"rust crates: {len(r)}, npm packages: {len(n)}")


if __name__ == "__main__":
    main()
