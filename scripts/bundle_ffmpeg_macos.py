#!/usr/bin/env python3
"""Copy an FFmpeg build (ffmpeg + ffprobe + every non-system dylib they load) into
src-tauri/resources/ffmpeg so the app never depends on PATH or Homebrew at runtime.
Load paths are rewritten to @loader_path and everything is ad-hoc re-signed.

    python3 scripts/bundle_ffmpeg_macos.py [ffmpeg_bin_dir]     (default: /opt/homebrew/bin)

The result inherits the minimum macOS version of the source build (Homebrew bottles
are built per OS release). To support older macOS, point this script at an FFmpeg
built with an older MACOSX_DEPLOYMENT_TARGET (any folder with ffmpeg + ffprobe works).
"""
import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = sys.argv[1] if len(sys.argv) > 1 else "/opt/homebrew/bin"
OUT = os.path.join(ROOT, "src-tauri", "resources", "ffmpeg")
LIB = os.path.join(OUT, "lib")


def run(*args: str) -> str:
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def deps(path: str) -> list[str]:
    lines = run("otool", "-L", path).splitlines()[1:]
    return [l.strip().split(" (")[0] for l in lines if l.strip()]


def is_system(dep: str) -> bool:
    return dep.startswith("/System/") or dep.startswith("/usr/lib/") or dep.startswith("@")


def main() -> None:
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(LIB)
    seen: dict[str, str] = {}  # basename -> real source path
    queue: list[str] = []

    def collect(binary: str) -> None:
        for dep in deps(binary):
            if is_system(dep):
                continue
            name = os.path.basename(dep)
            if name not in seen:
                real = os.path.realpath(dep)
                seen[name] = real
                dst = os.path.join(LIB, name)
                shutil.copy2(real, dst)
                os.chmod(dst, 0o755)
                queue.append(dst)

    for tool in ("ffmpeg", "ffprobe"):
        dst = os.path.join(OUT, tool)
        shutil.copy2(os.path.realpath(os.path.join(SRC, tool)), dst)
        os.chmod(dst, 0o755)
        collect(dst)
    i = 0
    while i < len(queue):
        collect(queue[i])
        i += 1

    def relink(path: str, prefix: str) -> None:
        for dep in deps(path):
            name = os.path.basename(dep)
            if name in seen and not dep.startswith("@"):
                subprocess.run(["install_name_tool", "-change", dep, f"{prefix}/{name}", path], check=True, capture_output=True)
        # Drop absolute rpaths pointing at the build machine.
        lines = run("otool", "-l", path).splitlines()
        for idx, line in enumerate(lines):
            if "LC_RPATH" in line:
                rp = lines[idx + 2].strip().split()[1]
                subprocess.run(["install_name_tool", "-delete_rpath", rp, path], capture_output=True)

    for name in seen:
        lib = os.path.join(LIB, name)
        subprocess.run(["install_name_tool", "-id", f"@loader_path/{name}", lib], check=True, capture_output=True)
        relink(lib, "@loader_path")
    for tool in ("ffmpeg", "ffprobe"):
        relink(os.path.join(OUT, tool), "@loader_path/lib")

    # install_name_tool invalidates signatures; ad-hoc sign (the app bundle is signed on top).
    for f in [os.path.join(LIB, n) for n in seen] + [os.path.join(OUT, "ffmpeg"), os.path.join(OUT, "ffprobe")]:
        subprocess.run(["codesign", "--force", "--sign", "-", "--timestamp=none", f], check=True, capture_output=True)

    # Refuse to ship anything that still points at the build machine.
    leftovers = []
    for f in [os.path.join(LIB, n) for n in seen] + [os.path.join(OUT, "ffmpeg"), os.path.join(OUT, "ffprobe")]:
        leftovers += [d for d in deps(f) if d.startswith("/opt/") or "/usr/local/" in d]
    if leftovers:
        sys.exit(f"error: unresolved absolute library paths remain: {sorted(set(leftovers))}")

    ff = os.path.join(OUT, "ffmpeg")
    version = run(ff, "-hide_banner", "-version").splitlines()[0]
    buildconf = subprocess.run([ff, "-hide_banner", "-buildconf"], capture_output=True, text=True).stdout
    minos = "unknown"
    lines = run("otool", "-l", ff).splitlines()
    for idx, line in enumerate(lines):
        if "LC_BUILD_VERSION" in line:
            for l in lines[idx : idx + 6]:
                if l.strip().startswith("minos"):
                    minos = l.split()[1]
            break
    with open(os.path.join(OUT, "VERSION.txt"), "w") as f:
        f.write(version + "\n")
    with open(os.path.join(OUT, "BUILDCONF.txt"), "w") as f:
        f.write(buildconf)
    with open(os.path.join(OUT, "LICENSE-NOTICE.txt"), "w") as f:
        f.write(
            f"""This folder contains {version} and the libraries it links against,
copied unmodified from the build at {SRC}.
Minimum macOS for these binaries: {minos}

FFmpeg is licensed under the GNU LGPL v2.1 or later. This build enables GPL components
(see BUILDCONF.txt: --enable-gpl / --enable-version3), so these FFmpeg binaries are
distributed under the GNU GPL (v3). FrameEase runs FFmpeg as a separate program and
does not link to it.

Corresponding source code:
  FFmpeg   https://ffmpeg.org/releases/ (version above)
  x264     https://code.videolan.org/videolan/x264
  x265     https://bitbucket.org/multicoreware/x265_git
  SVT-AV1  https://gitlab.com/AOMediaCodec/SVT-AV1
  dav1d    https://code.videolan.org/videolan/dav1d
  libvmaf  https://github.com/Netflix/vmaf
  Opus     https://opus-codec.org/
  LAME     https://lame.sourceforge.io/
  libvpx   https://chromium.googlesource.com/webm/libvpx
  OpenSSL  https://www.openssl.org/source/

Bundled libraries: {", ".join(sorted(seen))}
"""
        )
    size = sum(os.path.getsize(os.path.join(dp, n)) for dp, _, ns in os.walk(OUT) for n in ns)
    print(f"bundled {len(seen)} libraries into {OUT} ({size / 1e6:.1f} MB, minos {minos})")
    print(version)


if __name__ == "__main__":
    main()
