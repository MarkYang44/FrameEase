English | [简体中文](README.zh-CN.md)

<img src="src-tauri/icons/128x128@2x.png" alt="FrameEase app icon" width="104">

# FrameEase

A local video compression and conversion toolkit for **macOS and Windows**, powered by FFmpeg.

Compress a batch, compare a short trial, trim a clip, or reorganize tracks. FrameEase keeps source files intact and checks outputs before marking jobs complete.

[![Build](https://github.com/MarkYang44/FrameEase/actions/workflows/build.yml/badge.svg)](https://github.com/MarkYang44/FrameEase/actions/workflows/build.yml)
![macOS and Windows](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-555)
[![MIT License](https://img.shields.io/badge/license-MIT-3457D5)](LICENSE)

[Download v0.1.3](https://github.com/MarkYang44/FrameEase/releases/tag/v0.1.3) · [Quick start](#quick-start) · [Development](#development) · [Documentation](#documentation)

## Overview

FrameEase combines H.264, HEVC, AV1, and FFV1 workflows with a persistent batch queue. Choose lossy compression, verified lossless re-encoding, or stream copy explicitly; unsupported combinations are reported with suggested fixes.

English and Simplified Chinese interfaces, light and dark themes, keyboard controls, and reduced-motion support are included. Video processing runs on your computer.

## Screenshots

Browser previews with synthetic queue data, captured from the current interface. These show layout and language support; they are not encoding benchmarks or native-window captures.

<table>
  <tr>
    <td width="50%"><strong>Batch conversion</strong><br><a href="docs/screenshots/en/convert.png"><img src="docs/screenshots/en/convert.png" alt="English conversion queue and output settings in dark mode" width="600"></a></td>
    <td width="50%"><strong>Settings and capabilities</strong><br><a href="docs/screenshots/en/settings.png"><img src="docs/screenshots/en/settings.png" alt="English settings with detected encoders and lossless self-test results" width="600"></a></td>
  </tr>
</table>

## Core features

| Workflow | What it does |
| --- | --- |
| **Compress and convert** | H.264, HEVC, and AV1; quality presets, encoder-specific quality values, target bitrate or file size, and two-pass encoding where supported. |
| **Lossless encoding** | Enable only encoder/pixel-format combinations that pass self-tests, then compare every decoded output frame with the source. FFV1 archival output uses MKV. |
| **Stream copy** | Remux or select tracks without re-encoding. Copied tracks are checked by their data hashes. |
| **Trial encode** | Encode a short segment, compare images, and inspect available PSNR, SSIM, or VMAF metrics. Size estimates describe the selected segment only. |
| **Trim and manage tracks** | Keyframe-based fast trimming or precise re-encoded trimming; merge audio/subtitles and extract individual tracks. |
| **Batch queue** | Files, folders, drag and drop, optional recursive scanning, per-job settings, reordering, pause, cancel, retry, and restart recovery. |
| **Inspect and reuse** | Detailed media information, copyable reports, built-in presets, and custom preset import/export. |

Output files are written to temporary paths, verified, and committed without overwriting existing files. Interrupted jobs restart from the beginning. A failed job or an occupied results CSV does not stop the rest of the queue.

Hardware encoding and decoding are enabled only after runtime tests. Available codecs and filters depend on your FFmpeg build and hardware.

## Installation and compatibility

Download the installers from [GitHub Releases](https://github.com/MarkYang44/FrameEase/releases/latest). Each release includes `SHA256SUMS.txt` for integrity checks. Development build artifacts are also available from successful [GitHub Actions runs](https://github.com/MarkYang44/FrameEase/actions/workflows/build.yml).

| Platform | Availability and validation |
| --- | --- |
| **macOS · Apple Silicon** | Developed and tested locally; CI builds `.app` and `.dmg` packages. The minimum macOS version depends on the bundled FFmpeg libraries. The local bundle configuration currently requires macOS 26.0+. |
| **Windows 10/11 · x64** | NSIS/MSI packaging and core tests have passed on Windows CI. Physical Windows desktop validation remains pending. WebView2 is required; the installer offers setup if it is missing. |
| **macOS · Intel** | No packaged build in the current CI workflow; native validation remains pending. A compatible Intel FFmpeg distribution is needed for source builds. |

For Windows without a FrameEase installation, download `FrameEase_<version>_windows-x64-portable.zip`, extract the **entire** archive, and double-click **FrameEase.exe**. Keep the `ffmpeg` folder beside it. The ZIP includes FFmpeg and its DLLs, but uses the system WebView2 Runtime. Settings and queues stay in the current user's application-data folder, as with the installed version. [Portable instructions](docs/WINDOWS_PORTABLE.txt).

On macOS, open the DMG and copy FrameEase to Applications. On Windows, run the `.exe` installer for a current-user installation, or use `.msi` for a per-machine installation. The installers include FFmpeg when built with the bundle configuration.

The shell's configured macOS floor is 11.0; this is **not** a compatibility promise for packages with newer FFmpeg dependencies. CI derives its package floor from the bundled executable. See [testing and platform notes](docs/TESTING.md).

### First launch

Packages are not signed with a paid developer certificate. On macOS, after the first blocked launch, use **System Settings → Privacy & Security → Open Anyway**. On Windows, SmartScreen may offer **More info → Run anyway**. Windows installations without WebView2 require an internet connection to install that runtime.

## Quick start

1. Open FrameEase and let it detect FFmpeg and supported encoders.
2. Add videos or a folder, or drop them into the window.
3. Choose the output codec, processing mode, quality, and destination on the right.
4. Resolve any compatibility notices. Use **Trial encode** to compare a short segment before committing to a batch.
5. Click **Start processing**. Open or reveal results after verification finishes.

Switch language at **Settings → Appearance → Language**. Use **Trim**, **Tracks**, and **Info** for the other tools.

| Shortcut | Action |
| --- | --- |
| `⌘/Ctrl + O` | Add videos |
| `⌘/Ctrl + Shift + O` | Add a folder |
| `⌘/Ctrl + ,` | Open settings |
| `⌘/Ctrl + Enter` | Start staged files from Convert |

## Processing limits

- Lossy encoding can produce larger files and adds generation loss. Lossless re-encoding usually increases size and cannot restore detail already lost.
- HDR-to-SDR tone mapping is not implemented. Unsupported HDR combinations are rejected; re-encoding Dolby Vision removes dynamic metadata after confirmation, and Profile 5 is rejected.
- Text subtitle burn-in requires libass. Codec, hardware, filter, and metric availability varies by FFmpeg build.
- Fast trimming starts at a keyframe. Interrupted jobs do not resume from an intermediate frame.
- SVT-AV1 can stall under high parallel load. FrameEase limits threads, detects stalled processes, and retries once; this does not eliminate the upstream issue.

## Privacy

Media is processed locally without automatic media uploads. Queue state, settings, previews, logs, and the results CSV are stored locally. Reports and logs can contain file paths and media metadata.

## Development

Use **Node.js 22.12+**, **Rust 1.89+**, and FFmpeg/ffprobe. Native builds also need Xcode Command Line Tools on macOS or MSVC Build Tools and WebView2 on Windows.

```bash
npm ci
npm run app:dev
```

FFmpeg lookup order: custom folder → bundled resources → portable folder beside the executable → system PATH. Select another backend in Settings when needed.

```bash
npm test                 # Translation and formatting regressions
npm run typecheck
npm run build
npm run core:test        # Rust tests with synthetic FFmpeg media
npx playwright install chromium
npm run test:ui          # Browser layout and localization regressions
```

Browser tests use the labeled preview with synthetic data; they do not verify video encoding. `npm run dev` opens the same preview. The optional debug bridge, started with `npm run app:dev:bridge`, connects `http://localhost:1420/?bridge` to the actual debug backend; release builds exclude it.

<details>
<summary>Build desktop packages</summary>

macOS, with a compatible FFmpeg installation:

```bash
npm run ffmpeg:bundle
npm run app:build
```

Windows, in PowerShell:

```powershell
pwsh ./scripts/fetch-ffmpeg-windows.ps1
$env:STATIC_VCRUNTIME = "true"
npm run app:build
npm run windows:portable
```

Packages are written to `target/release/bundle/`. Ensure the declared macOS minimum matches the libraries you bundle.

Pushing a matching `v*` version tag runs both platform builds, checks the Windows installers, and publishes the installers and Windows no-install ZIP with SHA-256 checksums and `docs/releases/<tag>.md` as the release notes.

The icon source is `assets/app-icon.png`. Run `npm run icons` to regenerate desktop PNG, ICNS, ICO, and Windows tile assets with Tauri.

</details>

## Documentation

The detailed engineering documents are currently primarily in Chinese.

| Document | Contents |
| --- | --- |
| [Architecture](docs/ARCHITECTURE.md) | Rust engine, command bridge, UI responsibilities, and output safety |
| [Testing](docs/TESTING.md) | Test commands, recorded results, platform validation, and limitations |
| [Third-party licenses](docs/THIRD_PARTY_LICENSES.md) | FFmpeg and bundled dependency notices |

## License

FrameEase source code is licensed under [MIT](LICENSE). FFmpeg and its libraries retain their own licenses; bundled GPL-enabled builds are distributed under their applicable terms. See [third-party notices](docs/THIRD_PARTY_LICENSES.md).
