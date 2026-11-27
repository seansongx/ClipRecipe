# 应用内下载 FFmpeg

- **固定清单**：开发依赖 `@seankit/ffmpeg`、`@seankit/ffprobe`（package.json 中写死精确版本）各带一份 `manifest.json`
  （每个平台的 tgz 地址——npmmirror 在前、npmjs 在后、内容相同——以及 `size`、`sha256`、包内路径 `binary`、`binarySha256`、
  `binarySize`、Rust 目标三元组、别名）。`npm run sync-ffmpeg-manifest`（`scripts/sync-ffmpeg-manifest.mjs`）把两份原样合并为
  `src-tauri/resources/ffmpeg-manifest.json` 并提交；`ffmpeg/manifest.rs` 用 `include_str!` 内嵌、解析并校验（只允许 HTTPS、
  哈希格式、包内路径安全、两个工具版本一致、版本号可作目录名）。**程序只下载内嵌清单列出的文件**，运行时从不联网取清单。
  `manifest::tests::committed_manifest_matches_installed_npm_packages` 在提交的清单与 node_modules 中的包不一致时失败
  （没有 node_modules 时打印原因并跳过）。平台：先按编译目标三元组（build.rs 写入 `CLIPRECIPE_TARGET`）匹配 `rustTargets`，
  再按 `win32-x64` 式的平台键，最后按别名（Windows on ARM → x64）；都没有时给出中文说明，界面不显示下载按钮。
- **升级 FFmpeg**：`npm install --save-dev --save-exact @seankit/ffmpeg@<新版本> @seankit/ffprobe@<新版本>` →
  `npm run sync-ffmpeg-manifest` → 用新的 node_modules 跑一遍 `cargo test`（会用到新构建）并检查 essentials 缺少的功能是否变化 →
  提交 package.json、package-lock.json 与清单。已下载旧版本的用户在设置页看到“可更新”（旧版本仍然可用），点击后下载新版本，
  成功后删除旧版本目录。
- **查找顺序**（`ffmpeg::locate_detailed`，每次调用重新查找、不缓存）：`CLIPRECIPE_FFMPEG` → 托管副本（固定版本；只有更旧的
  版本时也使用并标记 `outdated`）→ 程序所在目录 → `PATH` → 仅调试构建与测试：仓库 `node_modules/@seankit/ffmpeg-<平台>/bin`
  与 `ffprobe-<平台>/bin`（两个目录，`npm install` 即可运行程序与测试）。`ffmpeg_status` 报告 `source`（env / managed /
  bundled / path / dev / missing）、版本、`whisper`（有无 whisper 滤镜）与 `managed`（能否下载、大小、已下载的版本、可否更新）。
- **托管目录**：`<应用本地数据目录>/ffmpeg/<FFmpeg 版本>/`（Windows：`%LOCALAPPDATA%\com.xiaofany.cliprecipe\ffmpeg\9.0.2\`），
  ffmpeg 与 ffprobe 在同一目录，另有 `receipt.json`（版本、平台、实际下载地址、tgz 与可执行文件的哈希和大小、安装时间）。
  设置了 `CLIPRECIPE_CONFIG_DIR` 时改为 `<它>/ffmpeg`；`cargo test` 不登记根目录，因此测试永远不碰真实的数据目录。
  版本目录要有安装记录、版本与目录名一致、两个可执行文件的大小与记录相符才算完整；以点开头的临时目录不算。
- **下载与安装**（`ffmpeg/managed.rs`，由 `download_ffmpeg` 调用：启动时自动一次，见下方“界面”，其余只在用户点击时）：按清单顺序尝试地址，失败换下一个；
  边下载边写入 `<根目录>/.staging-*` 并计算 SHA-256，超过清单大小立即停止，结束时必须正好相等；**先**核对 tgz 的 SHA-256，
  一致后才解压；只解出 `package/<binary>` 这一个条目，其它条目不写入磁盘；任何条目是绝对路径 / 盘符 / 含 `..` / 链接或特殊文件 /
  大于可执行文件，整个压缩包拒绝；核对可执行文件的 SHA-256 与大小；Unix 上设置 0755；两个工具都通过后写安装记录，把临时目录整体改名为
  版本目录（已有同版本时先移开、成功后删除、失败移回），成功后删除旧版本。失败、取消都删除临时目录，下次安装时清理残留
  （一小时内其它进程的临时目录保留）。未经校验的文件不会出现在版本目录中，也就不会被找到、执行。删除（`remove_managed_ffmpeg`）
  先改名再删除：正在使用的程序在 Windows 上无法改名，因此不会删掉一半；处理或下载进行中时拒绝。下载与删除同一时间只允许一个。
- **网络**（`net.rs`，检查更新共用）：只发普通 GET，`User-Agent: ClipRecipe`，不带任何用户标识；正式构建只允许 HTTPS
  （ureq `https_only`，跳转到 http 也拒绝）；代理按环境变量 `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`，`NO_PROXY` 中的主机与本机地址直连
  （此前检查更新不读代理，现在两者一致）；连接超时 15 秒，30 秒收不到任何数据判定卡住并换下一个地址。取消在两次读取之间生效，
  网络卡住时最多等到读取超时。
- **新依赖**：`sha2`、`flate2`（此前已是间接依赖）、`tar`（`default-features = false`，只用其读取器，不用 `unpack`）；
  开发依赖 `@seankit/ffmpeg`、`@seankit/ffprobe`。
- **命令与事件**（`commands/ffmpeg.rs`）：`download_ffmpeg`（返回 `{ cancelled, status }`，失败时返回错误）、`cancel_ffmpeg_download`、
  `remove_managed_ffmpeg`、`redetect_ffmpeg`；进度事件 `ffmpeg-download`（阶段、工具、来源主机、已下载 / 总字节、速度）。
  安装或删除后清空编码器探测缓存；因为 `locate` 不缓存，新副本立即生效，无需重启。没有新增 capability，CSP 不变（下载在 Rust 中进行）。
- **自动下载**：启动时检测完成（`state/events.ts`）后，没有可用的 FFmpeg 且本平台支持应用内下载，就直接开始下载，不等用户点击
  （`autoDownloadIfMissing`，处理记录里记一条“未找到 FFmpeg，开始自动下载 …”）。每次启动只自动尝试一次：用户取消、下载失败、
  之后在设置里删除，都不会再自动重下，改由批量处理页提示或设置页里的按钮手动触发；更新到新版本也只在点击时进行。
- **界面**：`state/ffmpeg.ts`（状态、下载进度、自动下载、`requireFfmpeg`：导入 / 开始处理 / 运行工具前发现没有 FFmpeg 时在处理记录里指向下载入口并
  重新显示提示）、`components/FfmpegDownload.tsx`（下载 / 进度 / 速度 / 取消 / 失败重试、下载说明、批量处理页“未找到 FFmpeg”提示：
  下载中显示进度、标题改为“正在下载 FFmpeg…”且不能关闭，其余时候可关闭）；设置 → 关于 → 运行环境：来源、版本、路径、重新检测、下载 / 更新 / 删除（AlertDialog 确认）。
- **essentials 构建缺少的功能**（与 gyan.dev 9.1 full 对比 `-filters` / `-encoders` / `-decoders`）：`whisper`（语音生成字幕的内置引擎：
  `ffmpeg_status.whisper` 为 false 时表单提前说明，运行时报错指向 whisper-cli 或完整构建）、`flite`（只有测试用来合成语音：
  没有时真实识别的测试打印原因跳过）、libdav1d（AV1 改用 libaom 解码）、SVT-AV1 / rav1e / libjxl 等编码器、frei0r、libplacebo、
  OpenCL / Vulkan 滤镜、bs2b / sofalizer 等——代码都没有用到。rubberband、zscale、libass、drawtext、vidstab、NVENC / QSV / AMF、
  libx264 / libx265 / libmp3lame / libopus 都有。
- **ffprobe 9.0 与 HLS 测试**：ffprobe 9.0.2 在 `-show_entries stream=…` 时对带节目的容器（MPEG-TS）还输出 `programs[].streams`，
  CSV 输出因此把每条流列两次（9.1 不会）。`tool.hls` 的输出本身在两个版本下相同且正确（每个分片一条 H.264、一条 AAC），
  所以只改了测试的探测：`tool_support::probe_entries` / `stream_count` 改为读 JSON 顶层的 `streams`。
- **端到端测试**：`npm run e2e` 以 `--features e2e-hooks` 构建调试版。这个特性（只能用于调试构建，否则 `compile_error!`）多出两个
  环境变量：`CLIPRECIPE_E2E_FFMPEG_MANIFEST`（用测试清单代替内嵌清单，允许 `http://` 本机地址）与 `CLIPRECIPE_E2E_ISOLATE_FFMPEG=1`
  （不查找程序目录、PATH、node_modules）。正式构建里这些代码不存在，所以无法被触发。`[ffmpeg]` 场景用
  `scripts/e2e-ffmpeg-server.mjs` 的本机假服务器（把 node_modules 里的真 ffmpeg / ffprobe 打成 npm 结构的 tgz，第一个地址返回 503；
  先慢速发送，以便看到自动开始的下载并取消）走完整界面流程：启动后不点击即自动下载、下载中导入被拦截 → 取消（临时文件清理、
  不会自己再开始、关闭提示后导入仍被拦截）→ 点击重新下载，完成后立即可导入 → 设置页删除（不会自动重下）。
