# 测试记录

## 校验误报与不完整源文件（2026-11-09，v0.1.3）

用户反馈的一批视频全部以“输出校验未通过”结束：其中 5 个输出其实正确，属于校验误报；1 个是未下载完整的源文件。原因与修复见文末“测试中发现并已修复的问题”的最后三行。

| 检查 | 结果 |
| --- | --- |
| Rust 核心 | 169 项通过（136 单元 + 33 集成），新增 4 项回归：全范围源转 H.264 / HEVC（x264、x265、VideoToolbox）、编辑列表隐藏的开头帧、未下载完整的 MP4（添加时拒绝，同意后只处理可读部分）、不完整源的处理计划 |
| 新测试素材 | `scripts/gen-test-media.sh` 新增 `edit_list.mov`（编辑列表隐藏 15 帧）与 `incomplete_download.mp4`（文件头声明 10 秒，数据只剩 55%），均为合成内容 |
| 原始文件复测 | 6 个原本失败的文件按原设置用修复后的引擎重新处理：5 个直接完成并通过全部校验；不完整的 1 个在编码前被拦下，选择“只处理可读的部分”后完成并通过校验。输出用后即删 |
| 静态检查与前端 | `cargo clippy -p fe-core --tests -- -D warnings` 通过；前端类型检查与单元测试（含中英词典一致性）通过 |

## 界面与图标更新（2026-11-04）

本轮在同一台 macOS / Apple Silicon 机器上验证。新增浏览器测试使用明确标识的模拟预览和合成媒体信息，仅验证布局与翻译；真实编码正确性仍由 Rust / FFmpeg 集成测试负责。

```bash
npm test
npm run typecheck
npm run build
npx playwright install chromium
npm run test:ui
npm run icons
```

本机也可通过 `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` 指向已安装的 Chrome；未设置时使用 Playwright 安装的 Chromium。

| 检查 | 本轮结果与范围 |
| --- | --- |
| 前端单元测试 | 7 项通过；中英词典键与参数一致、英文无中文回退、提示与时间单位、历史校验文本、内置与用户预设名称 |
| 浏览器回归 | 5 项通过；跨页面与缩放时设置按钮保持底部、通知不分走导航高度、右栏长文本与英文控件不横向溢出、保持挂载的缓存队列行切换语言、载入媒体后的截取/轨道/信息页及复制报告 |
| 图标 | 原图副本与用户提供图片一致；PNG 尺寸正确；ICO 含 16/24/32/48/64/256 像素；ICNS 容器有效；打包 `.app` 内的 ICNS 与生成资源逐字节一致 |
| 前端与桌面构建 | 类型检查、前端生产构建与最终 macOS `.app` 打包通过 |
| Rust 核心 | 165 项通过（135 单元 + 30 集成）；新增空名称预设的本地化默认名回归，保留用户填写名称。沙箱内 VideoToolbox 不可用，宿主权限复核全部通过 |
| 静态检查 | `cargo clippy -p frameease --all-targets --features dev-bridge -- -D warnings` 通过 |
| Windows | 本轮更新了 ICO、PNG、安装包描述与 CI 检查，未在 Windows 上重新打包或实机运行 |

GitHub Actions 已增加前端单元测试与浏览器回归步骤；本轮尚未推送，新增步骤尚无远端运行结果。README 新截图来自英文浏览器预览，保留模拟数据提示。

下方为此前核心功能验证的历史记录。

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
| `tests/caps_detect.rs` | 5 | 全部通过 | 真实能力检测、VideoToolbox 实测、无损自检能发现隐式色度转换、同一目录中并发自检互不干扰且不留文件 |
| `tests/engine_e2e.rs` | 18 | 全部通过 | 真实 FFmpeg 端到端（见下表） |
| `tests/process_watchdog.rs` | 3 | 全部通过 | 卡死后端被识别并结束、正常进度不误判、快速退出不漏判 |
| `tests/trial.rs` | 4 | 全部通过 | 试压体积/速度/PSNR/SSIM/VMAF、取帧 |
| **合计** | **164** | **全部通过** | 单次完整运行约 25 秒 |
| 压力复测：端到端套件连续 15 轮 | 270 次执行 | 全部通过 | 其中 1 轮捕获到真实的 SVT-AV1 卡死并被自动重试恢复（该轮用时 27 秒，其余约 8 秒） |
| `cargo clippy`（两个 crate，含测试） | — | 无警告 | |
| `cargo check -p fe-core --tests --target x86_64-pc-windows-msvc` | — | 通过 | Windows 专用代码可编译 |
| GitHub Actions：`macos-15`（Apple Silicon）、`windows-latest`（Windows Server 2025） | — | 通过 | 生成测试素材、全部核心测试、类型检查、打包（macOS `.dmg`+`.app` 41 MB；Windows NSIS+MSI 起初共 226 MB，见下一行），见[运行 41811801335](https://github.com/MarkYang44/FrameEase/actions/runs/41811801335)。同一提交的下一次运行在 Windows 上暴露了无损自检的并发问题（见下表），已修复 |
| GitHub Actions：Windows 改用 FFmpeg 共享库构建 | — | 通过 | 全部核心测试与打包通过；NSIS 安装包 59.7 MB、MSI 81.9 MB（CI 产物合计 141 MB，静态构建时 226 MB）。两个安装包解包后随包 FFmpeg 文件齐全，`ffmpeg.exe`/`ffprobe.exe` 在 PATH 只含系统目录时也能从自身文件夹启动。与同一版本（n8.1.3-32-g760d10603e）的静态构建相比，编码器、解码器、滤镜、封装/解封装、协议、比特流滤镜、硬件加速与像素格式列表完全相同，配置参数只差 `--enable-shared --disable-static`。见[运行 41816866579](https://github.com/MarkYang44/FrameEase/actions/runs/41816866579)与[对比运行 41816866452](https://github.com/MarkYang44/FrameEase/actions/runs/41816866452) |
| `tsc --noEmit` / `vite build` | — | 通过 | 主包 375 KB（未压缩），样式 21 KB |

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
| 全范围（JPEG range）源转 H.264 / HEVC（含 VideoToolbox）保持全范围并通过校验 | `full_range_sources_keep_their_range_and_verify` |
| MOV 编辑列表隐藏的开头帧不算作丢帧 | `frames_hidden_by_an_edit_list_are_not_counted_as_lost` |
| 未下载完整的 MP4 在导入时识别并拒绝；用户同意后只处理可读部分 | `an_unfinished_download_is_flagged_and_its_readable_part_can_be_processed` |

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
- 浮层（菜单、提示）：在 1280×820 与最小窗口 920×620 下逐一测量检查器中全部 15 个提示和各个菜单的位置，全部完整显示在窗口内；队列底部任务的菜单自动翻到上方；“⋯”预设菜单的方向键、Esc、Tab 操作正常；面板滚动时菜单跟随按钮，按钮滚出视野即关闭。
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
| 生成测试素材的脚本先切换到输出目录、再用调用方给的相对路径运行 FFmpeg，CI 的所有任务都在这一步失败 | GitHub Actions 首次运行 | 脚本先把两个路径转成绝对路径；CI 步骤失败时把输出写成注解，不登录也能看到原因 |
| 无损自检并发运行时，同一像素格式的自检共用一个源片段；一个自检重写它时另一个可能正在读取，健康的编码器会偶发被判为“未通过无损验证” | Windows CI：同一提交一次通过、一次 x264 被判未验证 | 每次自检使用独立目录并在结束后删除；能力缓存版本升为 2，旧缓存中可能的误判会重新检测；回归测试 `concurrent_self_tests_in_one_folder_all_pass_and_clean_up` |
| 菜单与提示被可滚动的面板（检查器、队列）裁切，例如“⋯”预设菜单左侧被遮住 | 用户反馈 | 菜单与提示渲染到页面顶层、按窗口定位：一侧放不下时翻到另一侧，靠边时沿边平移，从最靠近按钮的一点展开 |
| 全范围（JPEG range）的手机视频转 H.264 / HEVC 后校验失败：“像素格式”预期 `yuv420p`，实际 `yuvj420p`。输出其实如实保留了全范围，只是 FFmpeg 把全范围 8-bit H.264/HEVC 记作 `yuvj420p`，而色彩范围另有单独的检查 | 用户反馈：手机 App 录制的全范围 HEVC 视频（VideoToolbox 编码） | 比较像素格式时把 `yuvj…` 视同 `yuv…`，范围仍由“色彩范围”检查把关；回归测试覆盖 x264、x265 与 VideoToolbox |
| 带编辑列表的 MOV 校验“帧数”失败（预期 1081，实际 1066）：文件里有 15 个被编辑列表隐藏、播放器和 FFmpeg 都不显示的开头帧，旧的计数把它们也算进去了 | 用户反馈：手机 App 导出的 MOV | 计数时排除被解复用器标记为丢弃的包（只数实际显示的帧）；测试素材增加 `edit_list.mov` |
| 未下载完整的 MP4（文件头声明 19:51，数据只到 11:12）照常编码，最后才以“时长”校验失败告终，看不出原因 | 用户反馈：一个下载的文件 | 导入时检查 MP4/MOV 的数据是否到达声明的结尾，不完整则在开始前提示“源文件不完整”，并提供“只处理可读的部分”；测试素材增加 `incomplete_download.mp4` |

## 未验证 / 未覆盖

- Windows：只在 GitHub Actions 的 Windows Server 2025 运行器上测试和打包，未在实体 Windows 10/11 电脑上安装运行。
- macOS Intel：没有可随包附带的 Intel 版 FFmpeg（Homebrew 已停止提供 Intel Mac 的 FFmpeg 预编译包），CI 暂不构建 Intel 版本；需要选定 FFmpeg 来源（例如带校验和的第三方静态构建，或在 CI 中自行编译）。
- NVENC、Quick Sync、AMF、libaom、rav1e、libass 文字字幕烧录：本机 FFmpeg/硬件不具备，代码路径只在运行时能力检测通过后启用，未实测。
- 杜比视界、HLG、隔行素材、超长视频（小时级）、网络磁盘与 exFAT 的回退重命名路径：无本地素材或环境，未实测。
- 原生窗口截图（见上）。
