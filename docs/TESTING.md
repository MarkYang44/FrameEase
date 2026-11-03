# 测试记录

## 本次测试环境

| 项目 | 值 |
|---|---|
| 日期 | 2026-11-03 |
| 系统 | macOS 27.0.1（Darwin 27），Apple M5，10 核，16 GB |
| 工具链 | Rust 1.99.1，Node.js 26.3.0，Tauri 2.11 |
| 后端 | FFmpeg 8.1.2（Homebrew 构建，经 `scripts/bundle_ffmpeg_macos.py` 重定位后的随包副本）；x264 r3222、x265 4.2、SVT-AV1 4.1.0、dav1d 1.5.3、libvmaf 3.1.0、VideoToolbox |
| 素材 | `scripts/gen-test-media.sh` 用 FFmpeg lavfi 合成（彩条 + 正弦音），**未读取任何个人视频** |

## 运行方式

```bash
npm run core:test
```

集成测试会自动生成 `test-media/`，优先使用 `src-tauri/resources/ffmpeg` 中附带的 FFmpeg（也可用环境变量 `FRAMEEASE_FFMPEG_DIR` 指定）。

## 结果

| 套件 | 数量 | 结果 | 说明 |
|---|---|---|---|
| 单元测试（`cargo test -p fe-core --lib`） | 134 | 全部通过 | 状态机、参数生成、编码器换算、文件安全、CSV、持久化、探测解析、能力解析、预设、停滞检测 |
| `tests/caps_detect.rs` | 4 | 全部通过 | 真实能力检测、VideoToolbox 实测、无损自检能发现隐式色度转换 |
| `tests/engine_e2e.rs` | 18 | 全部通过 | 真实 FFmpeg 端到端（见下表） |
| `tests/process_watchdog.rs` | 3 | 全部通过 | 卡死后端被识别并结束、正常进度不误判、快速退出不漏判 |
| `tests/trial.rs` | 4 | 全部通过 | 试压体积/速度/PSNR/SSIM/VMAF、取帧 |
| **合计** | **163** | **全部通过** | 单次完整运行约 15 秒 |
| 压力复测：端到端套件连续 15 轮 | 270 次执行 | 全部通过 | 其中 1 轮捕获到真实的 SVT-AV1 卡死并被自动重试恢复（该轮用时 27 秒，其余约 8 秒） |
| `cargo clippy`（两个 crate，含测试） | — | 无警告 | |
| `cargo check -p fe-core --tests --target x86_64-pc-windows-msvc` | — | 通过 | Windows 专用代码可编译；未在 Windows 上运行 |
| `tsc --noEmit` / `vite build` | — | 通过 | 主包 373 KB（未压缩），样式 21 KB |

## 需求覆盖（端到端测试）

| 需求 | 测试 |
|---|---|
| H.264 → AV1 → H.264 往返，检查实际编码/分辨率/音轨 | `h264_to_av1_to_h264_round_trip_is_verified` |
| 无损：x264、SVT-AV1、FFV1、x265 10-bit 逐帧像素校验 | `lossless_encoders_are_verified_frame_by_frame` |
| 中文/日文/空格/特殊符号路径（输入与输出目录） | `unicode_and_special_character_paths_work_end_to_end` |
| 无音轨、多音轨（语言保留）、字幕转换 | `no_audio_multi_audio_and_subtitles` |
| VFR 帧数不变、旋转按显示方向输出 | `vfr_keeps_every_frame_and_rotation_is_applied` |
| HDR10 元数据与 10-bit 保留；HDR→H.264 拒绝而非降级 | `hdr10_and_10bit_are_preserved` |
| 已有输出不覆盖（自动编号/跳过），输出不能覆盖源文件 | `existing_outputs_and_the_source_are_never_overwritten` |
| 取消正在编码的任务、清理临时文件、队列继续 | `cancelling_a_running_job_cleans_up_and_the_queue_continues` |
| 损坏/截断/不存在的输入只让该任务失败 | `corrupt_inputs_fail_individually` |
| 后端崩溃（SIGKILL）只让该任务失败 | `crashed_backend_fails_only_that_job` |
| 后端缺失（断开后端） | `missing_backend_is_reported_per_job_without_crashing` |
| **结果 CSV 被锁定时队列不中断，解锁后补写（回归）** | `locked_results_csv_never_stops_the_queue`（Windows 用独占共享模式，macOS 用只读文件模拟） |
| 并发任务隔离（输出、日志、线程预算） | `parallel_jobs_are_isolated` |
| 重启恢复：中断任务从头再来，已完成任务不重跑，临时文件清理 | `restart_recovers_unfinished_jobs_without_rerunning_finished_ones` |
| 音轨不兼容时等待用户选择，选择后完成 | `incompatible_audio_waits_for_a_decision` |
| 流复制重新封装、关键帧截取、精确截取、轨道提取、轨道合并 | `remux_trim_extract_and_mux` |
| 暂停、排序 | `paused_queue_holds_jobs_and_respects_reordering` |
| 后端卡死被识别、结束并自动重试一次 | `a_hung_backend_is_killed_and_the_step_retried_once` |

每个成功任务都经过 `verify.rs` 的全部检查（实际编码、分辨率、像素格式、色彩、帧数、时长、轨道、复制轨道哈希、完整解码或逐帧比对），测试断言 `verify.ok` 且输出文件真实存在。

## 无损自检结果（本机）

| 编码器 | yuv420p | yuv420p10le | yuv444p |
|---|---|---|---|
| x264（`-qp 0`） | ✓ | ✓ | ✓（按需自检） |
| x265（`lossless=1`） | ✓ | ✓ | — |
| SVT-AV1（`lossless=1`） | ✓ | ✓ | ✗ 不支持 4:4:4，FFmpeg 会隐式降为 4:2:0，自检正确判为失败 |
| FFV1 | ✓ | ✓ | — |
| SVT-AV1 `CRF 0` | ✗ 不是无损（与需求中的警告一致） | | |

## 界面验证

- 浏览器面板 + 模拟数据：检查深/浅色、最小窗口 920×640、长文件名、密集队列、各种任务状态的布局（`docs/screenshots/07-*`、`08-*`）。
- **开发桥接 + 真实后端**（调试版 `--features dev-bridge`）：真实探测与问题提示、开始处理、按任务修正（转 AAC、改用 SVT-AV1）、真实进度时间线、试压指标与滑动对比、截取关键帧与起止画面、媒体信息、设置页真实能力表（`docs/screenshots/01-*` 至 `06-*`）。
- 原生窗口：调试版与发布版都已实际启动，启动检测成功；**本环境没有屏幕录制权限，未能对原生 WKWebView 窗口截图**。

## 打包验证（macOS Apple Silicon）

- `npm run app:build` 生成 `FrameEase.app`（42 MB，其中 FFmpeg 38 MB）与 `FrameEase_0.1.0_aarch64.dmg`（18 MB），签名为 ad-hoc（未用开发者证书签名或公证）。
- 通过 LaunchServices 启动发布版：确认使用 `.app` 内的 FFmpeg（来源 `bundled`），首启能力检测成功，8 组无损自检通过，VideoToolbox 解码实测通过。
- 内存（`footprint`，空闲）：应用进程 28 MB + WebKit 内容进程 62 MB + GPU 进程 17 MB + 网络进程 6 MB，约 113 MB。

## 测试中发现并已修复的问题

| 问题 | 如何发现 | 修复 |
|---|---|---|
| 视频重编码时，复制的音轨没有加入哈希校验 | 单元测试 `h264_to_av1_uses_svt_crf_and_copies_audio` | 重编码路径也登记音轨哈希比对 |
| 关键帧截取的期望时长按所选起点计算，校验误判 | 端到端测试 | 期望时长改按实际关键帧起点计算（先补回归断言） |
| 分段控件在带提示的选项上滑块位置为 0 | 界面实测 | 改为相对控件本身测量位置 |
| 进度播放头用 `left` 过渡，停在起点 | 界面实测（采样位置） | 改用 `transform` 平移，与进度条同步 |
| 设置摘要被下方控件覆盖 | 界面截图 | 调整层级 |
| 并发保存队列共用临时文件名，可能写坏或保存失败 | 代码审查 → 并发测试复现 | 每次保存使用唯一临时名，引擎串行化保存 |
| **SVT-AV1 4.1 在高并发下偶发内部死锁**（采样显示所有流水线线程阻塞在 `svt_block_on_semaphore`），任务停在“编码中” | 全量测试偶发超时 → 进程状态监控（非僵尸、仍存活）→ 对照实验（软解也会卡、x264 不会、限制 `lp` 降低概率）→ `sample` 采样确认 | 1）SVT-AV1 始终显式设置并行级别；2）停滞检测：进度计数 5 分钟不变即结束进程（libaom 15 分钟）；3）同一步骤自动重试一次并在任务上提示；二次卡死才判失败 |
| 发布构建因 Tauri npm 包与 crate 次版本不一致而失败 | 发布构建 | 锁定 npm 包到与 crate 相同的次版本 |

## 未验证 / 未覆盖

- Windows 10/11 与 macOS Intel：未实机运行；CI 已配置但未在 GitHub 上运行过。
- NVENC、Quick Sync、AMF、libaom、rav1e、libass 文字字幕烧录：本机 FFmpeg/硬件不具备，代码路径只在运行时能力检测通过后启用，未实测。
- 杜比视界、HLG、隔行素材、超长视频（小时级）、网络磁盘与 exFAT 的回退重命名路径：无本地素材或环境，未实测。
- 原生窗口截图（见上）。
