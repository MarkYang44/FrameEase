<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="88" alt="FrameEase 图标">
</p>

<h1 align="center">FrameEase</h1>

<p align="center"><strong>本地视频压缩与转码工具</strong><br>H.264 / HEVC / AV1 压缩、逐帧校验的无损重编码与批量队列，全部在本机完成。</p>

<p align="center"><strong>简体中文</strong> · <a href="README.en.md">English</a></p>

<p align="center">
  <a href="https://github.com/MarkYang44/FrameEase/releases/latest"><img src="https://img.shields.io/github/v/release/MarkYang44/FrameEase?label=Release&amp;color=3457D5" alt="最新版本"></a>
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20macOS-59636E" alt="Windows 与 macOS">
  <img src="https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&amp;logoColor=white" alt="Tauri 2">
  <img src="https://img.shields.io/badge/FFmpeg-8.1-007808?logo=ffmpeg&amp;logoColor=white" alt="FFmpeg 8.1">
  <a href="https://github.com/MarkYang44/FrameEase/actions/workflows/build.yml"><img src="https://github.com/MarkYang44/FrameEase/actions/workflows/build.yml/badge.svg" alt="构建状态"></a>
</p>

FrameEase 是一款运行在本机的视频压缩、转码与批量处理工具，目标是“接近小丸工具箱，但更可靠”：你选的输出格式不会被偷换，无损就是逐帧像素一致，批量队列不会因为某个坏文件或被 Excel 占用的 CSV 而中断，原文件永远不会被覆盖。界面基于 Tauri 2 与 React，处理由随安装包附带的 FFmpeg 8.1 完成。

**下载即用：Windows 10/11（x64）与 macOS（Apple Silicon）安装包已内置 FFmpeg，无需另行安装或配置 PATH。**

[下载安装](#下载与安装) · [功能概览](#功能概览) · [常见问题](#常见问题) · [从源码构建](#从源码构建) · [架构说明](docs/ARCHITECTURE.md) · [测试记录](docs/TESTING.md) · [问题反馈](https://github.com/MarkYang44/FrameEase/issues)

## 界面与实际使用

**转码队列 · 真实进度**

每个任务都显示转换方向、实际编码参数、体积变化与校验结果；编码中的任务显示进度、倍速、帧率与剩余时间。右侧“将会这样处理”汇总当前设置的实际效果。

![FrameEase 转码队列：已完成并通过校验的任务与正在编码的任务](docs/screenshots/03-queue-encoding-timeline.jpg)

<details>
<summary><strong>导入检查 · 冲突在开始前提示</strong></summary>

导入时逐个探测：HDR 不能用所选编码器保留、音轨不能直接复制到目标容器、源文件损坏等问题会在开始前标出，并给出可点的修正选项。

![FrameEase 待处理列表：HDR、PCM 音轨与损坏文件的提示](docs/screenshots/01-staging-real-issues.jpg)

</details>

<details>
<summary><strong>试压 · 先看效果再决定</strong></summary>

对一小段做真实编码，给出体积外推、编码速度与 VMAF / SSIM / PSNR，并可左右滑动对比原片与结果。

![FrameEase 试压：片段体积、整片估计、VMAF、SSIM、PSNR 与滑动对比](docs/screenshots/04-trial-metrics-compare.jpg)

</details>

<details>
<summary><strong>截取 · 关键帧快速截取与精确截取</strong></summary>

快速模式流复制、零画质损失，并如实告诉你起点实际落在哪个关键帧；精确模式重新编码。

![FrameEase 截取片段：起止画面预览与关键帧提示](docs/screenshots/05-trim-keyframes.jpg)

</details>

<details>
<summary><strong>设置 · 实测的能力检测</strong></summary>

FFmpeg 版本、许可与来源，以及每个编码器是否可用、是否通过实际编码测试、是否支持 10-bit。

![FrameEase 设置：FFmpeg 后端信息与编码器能力检测结果](docs/screenshots/06-settings-real-caps.jpg)

</details>

截图来自开发桥接：浏览器页面连接正在运行的调试版应用，由真实 FFmpeg 处理 `scripts/gen-test-media.sh` 生成的合成素材，不含任何个人视频；页面由 Chromium 渲染，与原生窗口可能有细微差别。

## 功能概览

| 模块 | 能力 |
| --- | --- |
| 有损压缩 | H.264、HEVC、AV1 相互转换；高画质 / 均衡 / 小体积 + 0–100 压缩程度，按编码器换算为各自的 CRF / CQ / q:v 并直接显示；目标码率、目标体积与两遍编码 |
| 无损重编码 | 只开放启动自检（逐帧像素哈希比对）通过的“编码器 + 像素格式”组合；每个任务完成后再与原片逐帧比对，不一致即判失败；FFV1 无损存档；CRF 0 不算无损 |
| 画质与格式保真 | 默认保持分辨率与帧率（含可变帧率）；HDR 不会当 SDR 输出，10-bit 不会降为 8-bit，4:4:4 不会降为 4:2:0；不支持的组合直接拒绝并给出修正选项 |
| 音频与字幕 | 音频直接复制（完成后数据哈希校验）、转码为 AAC / Opus / MP3 / FLAC 或移除；字幕可保留、把外挂字幕文件加为软字幕，或烧录进画面 |
| 批量队列 | 文件、多选、拖放与文件夹导入；每个任务保存独立的设置快照；排序、暂停、取消与重试；重启后恢复队列 |
| 硬件加速 | Apple VideoToolbox、NVIDIA NVENC、Intel Quick Sync、AMD AMF；只有实测编码通过的编码器才会启用；CPU 编码与 GPU 编码 / 流复制分开排队 |
| 工具 | 试压与画质指标、关键帧快速截取与精确截取、封装合并与轨道提取、媒体信息、预设（内置、保存、导入、导出） |

CPU 编码会限制每个任务的线程数（x264 `-threads`、x265 `pools`、SVT-AV1 `lp`），并按内存与磁盘空间决定何时开始下一个任务；默认“自动”并行时只同时运行 1 个 CPU 编码任务。

## 下载与安装

### 1. 选择安装包

前往 [Releases](https://github.com/MarkYang44/FrameEase/releases/latest) 下载对应系统的安装包，并可用同页的 `SHA256SUMS.txt` 校验文件完整性。

| 系统 | 安装包 | 说明 |
| --- | --- | --- |
| Windows 10/11 x64 | `FrameEase_<版本>_x64-setup.exe`（推荐） | 安装到当前用户，无需管理员权限 |
| Windows 10/11 x64 | `FrameEase_<版本>_x64_en-US.msi` | 适合按机器部署；安装需要管理员权限 |
| macOS · Apple Silicon | `FrameEase_<版本>_aarch64.dmg` | 需要 macOS 15 或更高版本；打开后把 FrameEase 拖进“应用程序” |

暂不提供 Intel Mac 版本：Homebrew 已不再为 Intel Mac 提供 FFmpeg 预编译包，目前没有可随包附带的 FFmpeg。

### 2. 首次打开

当前安装包没有使用付费的开发者证书签名，系统会在首次打开时拦截：

- **macOS：** 双击打开一次，出现“无法验证”提示后，前往“系统设置 → 隐私与安全性”，在页面底部点“仍要打开”。如果提示“已损坏，无法打开”，在终端执行 `xattr -dr com.apple.quarantine /Applications/FrameEase.app` 后再打开。
- **Windows：** 如果出现“Windows 已保护你的电脑”，点“更多信息 → 仍要运行”。系统缺少 WebView2 运行时（少数 Windows 10）时，安装程序会下载并显示 WebView2 安装界面，需要联网。

### 3. 开始使用

1. 把视频拖进窗口，或点“添加视频 / 添加文件夹”。
2. 在右侧选择输出编码与处理方式（有损压缩、无损重编码或流复制），确认“将会这样处理”中的摘要。
3. 点“开始处理”。结果默认保存到源文件旁的 `FrameEase 输出` 文件夹，原文件保持不变。

> 拿不准压缩程度时，先用“试压”对一小段做真实编码，查看体积外推与画质指标后再处理整片。

## 输出与数据安全

- **原文件不动：** 只写入带任务编号的临时文件，校验通过后以“不覆盖”方式原子重命名提交；输出路径与源文件相同会被拒绝；重名时自动编号为 `名称 (2).mp4`，或按设置跳过。
- **按实际结果校验：** 重新探测输出的编码、分辨率、像素格式、旋转、色彩与 HDR 元数据、帧数、时长和音轨 / 字幕；直接复制的轨道比对数据哈希，并完整解码一遍。不符合预期即判失败，不靠扩展名判断。
- **队列不会被单个任务拖垮：** 坏文件、后端崩溃、校验失败都只影响该任务；后端卡死（进度 5 分钟不变）会被结束并以相同设置自动重试一次；结果 CSV 被占用时先暂存，稍后补写。
- **空间与退出保护：** 开始前检查磁盘空间，编码中空间不足会停止该任务；有任务进行时关闭应用会先询问。
- **重启恢复：** 已完成且已校验的任务不会重跑；被中断的任务从头重新处理，不宣称从中断处续编。

## 数据与隐私

| 位置 | 内容 |
| --- | --- |
| 源文件旁的 `FrameEase 输出`，或自定义文件夹 | 输出的视频 |
| 应用数据目录 | 队列 `queue.json`、设置 `config.json`、预设 `presets.json`、能力检测缓存 `capabilities.json`、结果记录 `FrameEase 结果记录.csv` |
| 应用数据目录下的 `logs/` | 每个任务的 FFmpeg 日志，保留 30 天 |
| 应用缓存目录 | 编码临时目录 `work/` 与预览帧 `previews/`，启动时清理 |

应用数据目录：macOS 为 `~/Library/Application Support/com.frameease.desktop/`，Windows 为 `%APPDATA%\com.frameease.desktop\`；缓存目录分别为 `~/Library/Caches/com.frameease.desktop/` 与 `%LOCALAPPDATA%\com.frameease.desktop\`。

FrameEase 不需要账号，不上传文件，也不收集使用数据；所有处理都由本机的 FFmpeg 完成。

## 平台与验证状态

| 平台 | 状态 |
| --- | --- |
| macOS · Apple Silicon | 在 macOS 27（M5）上实机开发与测试：164 项 Rust 测试全部通过（其中 30 项调用真实 FFmpeg 或真实子进程），端到端套件连续 15 轮压力复测无失败；桌面应用已实际运行。 |
| Windows 10/11 x64 | 尚未在实体 Windows 电脑上验证。GitHub Actions 的 Windows Server 2025 运行器上，核心测试（含 Excel 式文件独占锁、中文 / 日文 / 特殊字符路径、取消与重启恢复）与 NSIS / MSI 打包均已通过，并逐一解包核对两个安装包中的 FFmpeg。 |
| macOS · Intel | 暂不提供：没有可随包附带的 Intel 版 FFmpeg（见上文）。代码本身没有平台假设。 |

详细的测试范围、发现并修复的问题以及尚未覆盖的场景见 [docs/TESTING.md](docs/TESTING.md)。

## 已知限制

- macOS 安装包附带的 Homebrew FFmpeg 不含 libass / libaom / rav1e：**文字字幕烧录不可用**（界面会说明原因，图形字幕可以烧录），AV1 只能用 SVT-AV1。Windows 安装包附带的 BtbN 构建包含这些库，但相关功能尚未实测。
- Apple 平台没有 AV1 硬件编码器，相应 GPU 选项会如实禁用。NVENC、Quick Sync、AMF 只在能力检测实测通过后启用，尚未在真实显卡上验证。
- HDR：有损重编码只允许 x265 与 SVT-AV1（已验证保留 HDR10 元数据）；未实现 HDR → SDR 色调映射；杜比视界动态元数据会丢失（需用户确认），Profile 5 直接拒绝。
- 重新开始的任务从头编码，不支持从中断帧续编；快速截取的起点只能落在关键帧（未实现“智能剪切”）。
- SVT-AV1 4.1 在多路高并发时偶发内部死锁（属编码库问题）。FrameEase 显式限制其并行级别、检测停滞并自动重试一次；默认设置下触发概率很低，但无法根除。
- 试压的体积外推与画质指标只代表所选片段。
- 英文界面目前只覆盖外壳部分文字，其余显示中文。

## 常见问题

| 问题 | 处理办法 |
| --- | --- |
| macOS 提示“无法验证”或“已损坏” | 按[首次打开](#2-首次打开)的步骤放行；这是未购买开发者证书签名所致 |
| Windows SmartScreen 拦截安装程序 | 点“更多信息 → 仍要运行” |
| 启动后提示找不到 FFmpeg 或后端出错 | 确认安装目录中的 `ffmpeg` 文件夹没有被安全软件隔离；也可以在“设置 → FFmpeg 后端 → 更换文件夹…”中指定一个同时包含 ffmpeg 与 ffprobe 的文件夹 |
| 某个编码器或 GPU 选项不可选 | 该编码器没有通过实际编码测试（硬件、驱动或 FFmpeg 不支持）；更新驱动后可在“设置”中点“重新检测” |
| 任务停在“需要处理” | 设置与源文件冲突（例如 HDR 转 H.264、音轨无法直接复制到 MP4）；点任务上给出的修正选项，或改用建议的容器 |
| 结果 CSV 被 Excel 打开时队列会不会停 | 不会。记录会先暂存，关闭 Excel 后自动补写 |
| 想完全保留原始画质 | 选“无损重编码”（逐帧校验）或“流复制”（不重新编码，只改封装） |

## 从源码构建

### 1. 准备环境

| 依赖 | 用途 |
| --- | --- |
| Node.js 20+ | 构建界面 |
| Rust stable（[rustup](https://rustup.rs/)） | 构建核心与应用外壳 |
| Xcode Command Line Tools（macOS）/ MSVC 生成工具与 WebView2（Windows） | 原生编译 |
| FFmpeg 8.1 | 随应用附带的处理后端，见下一步 |

### 2. 获取源码与 FFmpeg

```bash
git clone https://github.com/MarkYang44/FrameEase.git
cd FrameEase
npm install
```

把 FFmpeg 放进应用资源目录 `src-tauri/resources/ffmpeg`：

```bash
# macOS：复制 Homebrew 的 ffmpeg@8 及其依赖库，并改为相对路径加载
brew install ffmpeg@8
npm run ffmpeg:bundle -- "$(brew --prefix ffmpeg@8)/bin"
```

```powershell
# Windows：下载 BtbN 的 FFmpeg 8.1 GPL 共享库构建
pwsh ./scripts/fetch-ffmpeg-windows.ps1
```

开发时如果资源目录中没有 FFmpeg，应用会查找系统中的 ffmpeg（例如 Homebrew）。

### 3. 运行、测试与打包

```bash
npm run app:dev      # 开发运行
npm run core:test    # 核心测试（自动在 test-media/ 生成合成素材，不读取个人视频）
npm run app:build    # 打包：macOS 生成 .app / .dmg，Windows 生成 NSIS / MSI
```

产物位于 `target/release/bundle/`。

### 关于随包 FFmpeg

- **最低系统版本由随包 FFmpeg 决定。** 应用本身只要求 macOS 11 / Windows 10，但 Homebrew 的 FFmpeg 按构建它的系统版本编译：在 macOS 26 上打包会要求 26.0+，CI 在 `macos-15` 上打包则要求 15.0+。打包脚本会按随包库的实际要求写入最低版本；要支持更老的 macOS，可把任意兼容的 FFmpeg 目录交给 `python3 scripts/bundle_ffmpeg_macos.py <目录>`。
- **Windows 使用共享库构建。** `ffmpeg.exe`、`ffprobe.exe` 与同一文件夹里 FFmpeg 自身的 DLL 一起随包；x264、x265、SVT-AV1、libaom、dav1d、libass 等库静态链接在这些 DLL 中，整套编解码器只随包一份（静态构建的两个 exe 各带一份）。Windows 会先从程序所在文件夹加载 DLL，应用又以完整路径启动 FFmpeg，因此不受 PATH 影响。
- **许可：** 随包 FFmpeg 启用了 GPL 组件，按 GPL 分发；应用以独立进程调用 FFmpeg，不与其链接。

## 开发与验证

| 目录 | 职责 |
| --- | --- |
| `crates/fe-core` | 全部业务逻辑：探测、能力检测、参数生成、队列与状态机、输出校验、文件安全；不依赖 Tauri，可单独测试 |
| `src-tauri` | Tauri 外壳：命令与事件桥接、启动检测（能力结果按 FFmpeg 指纹缓存）、退出保护 |
| `src` | React 19 + TypeScript + Vite 界面（zustand、lucide、sonner） |
| `scripts` | FFmpeg 打包、测试素材生成、第三方许可清单、CI 辅助脚本 |
| `.github/workflows/build.yml` | macOS 与 Windows 的测试、打包和发布 |

模块划分、状态机与安全提交流程见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

CI 在 `macos-15`（Apple Silicon）与 `windows-latest` 上生成测试素材、运行全部核心测试与类型检查并打包；Windows 还会解包两个安装包，核对随包 FFmpeg 文件齐全、能从解包出的目录直接启动。推送 `v*` 版本标签时，CI 会把这些安装包与 SHA-256 校验和发布到 GitHub Releases。

`npm run app:dev:bridge` 以 `dev-bridge` 特性启动调试版，同时在 `127.0.0.1:1430` 提供命令与事件；在浏览器打开 `http://localhost:1420/?bridge` 即可用同一界面驱动真实后端，用于自动化界面检查。发布构建不包含此功能。

## 许可与致谢

FrameEase 源码以 [MIT 许可](LICENSE)发布。随安装包附带的 FFmpeg 按 GNU GPL v3 分发，各组件与 Rust / npm 依赖的许可见 [docs/THIRD_PARTY_LICENSES.md](docs/THIRD_PARTY_LICENSES.md)。

感谢 [FFmpeg](https://ffmpeg.org/)、[x264](https://code.videolan.org/videolan/x264)、[x265](https://bitbucket.org/multicoreware/x265_git)、[SVT-AV1](https://gitlab.com/AOMediaCodec/SVT-AV1)、[dav1d](https://code.videolan.org/videolan/dav1d)、[Tauri](https://tauri.app/)、[React](https://react.dev/)、[Lucide](https://lucide.dev/)，以及提供 Windows 版 FFmpeg 构建的 [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds)。

<p align="center">Designed by Mark Yang</p>
