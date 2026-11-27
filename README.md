# ClipRecipe

开源的视频批量处理桌面工具，基于 **Tauri 2 + React 19 + TypeScript + Rust + FFmpeg**，MIT 许可，免费使用、无需注册。

把一组处理步骤（去水印、裁剪、画中画、变速、调色……）配成一份“配方”，批量套用到整个视频队列；另有 34 个独立工具
（合并、分割、重组、提取、转换、字幕、封面等）。全部处理都由本机的 FFmpeg 完成，不依赖任何云服务。

## 功能

完整清单与每项的实现说明见 [docs/specs/](docs/specs/overview.md)（全部条目均已实现）。

- **批量流水线（18 个模块）**：去水印、加水印、画中画、文本、裁剪、分辨率、旋转翻转、画面调整（含 LUT）、宫格分屏、
  动态缩放、去头尾、加头尾与相框、变速、抽帧、重复帧移除、背景音、帧率、码率。数值参数支持范围值随机取样，
  可指定种子复现；**裂变**：每个视频按不同随机值输出多份。
- **预览**：批量处理页中栏常驻显示当前视频套用配方后的一帧并随参数自动刷新；单个模块或整条流水线可生成低清预览片段；区域框选 / 位置选取直接在视频画面上操作。
- **配方**：导入导出 JSON，拖入窗口即加载。
- **队列**：文件 / 文件夹导入（可递归）、拖放、排序、插队、并行处理、处理记录；同名字幕自动关联并烧录、
  同名图片作封面嵌入；退出时保存未完成的任务，下次启动恢复。
- **独立工具（34 个）**：合并 / 分割 / 文件夹合并 / 截取合并 / HLS 切片、镜像 / 转场 / 拆分 / 移除重组、简单混剪、
  定帧动画、定格播放、运镜、黑边去除、插帧、进度条、横竖互转、画面拼接、倒放、图转视频、GIF、批量截图、
  静止帧 / 静音移除、智能去头尾、改变时长、音视频提取与合成、音量音调、无损转换、语音生成字幕（本地 whisper.cpp）、
  字幕提取、字幕擦除、封面处理。
- **设置**：目标格式、重名处理、文件名去除、并行数、H.265、HDR 转 SDR 或保留 HDR、GPU 硬件编码（NVENC / QSV / AMF，
  失败自动回退软件编码，可设核显优先）、体积压缩、版权元数据、完成后打开文件夹 / 提示音 / 关机 / 退出、
  处理后删除源视频（默认移入回收站）或把输出移回源位置、托盘、检查更新与程序内自动更新（见下文“下载与更新”）。
- **桌面程序的用法**：深色界面，标题栏随界面配色（Windows 11），记住窗口位置与大小，只运行一个实例（再次打开会激活已有窗口，
  拖到程序图标上的文件直接导入），任务栏按钮显示处理进度，底部状态栏显示队列与运行状态。

### 快捷键

| 按键 | 作用 |
|---|---|
| Ctrl+O / Ctrl+Shift+O | 添加视频 / 添加文件夹 |
| Ctrl+Enter / Esc | 开始处理 / 停止处理（Esc 在有对话框时关闭对话框） |
| Ctrl+1 / 2 / 3、Ctrl+, | 切换到批量处理 / 工具箱 / 设置，打开设置 |
| F1 | 使用文档 |
| 队列中：↑ ↓ Home End、Shift / Ctrl + 点击、Ctrl+A、Delete、Enter | 移动与选择、全选、移除所选、播放 |

完整列表在“设置 → 关于”。

## FFmpeg

ClipRecipe 不包含也不分发 FFmpeg，而是把它作为独立程序调用。有两种方式：

- **应用内下载（最简单）**：启动时没有找到 FFmpeg，程序会直接开始下载，批量处理页顶部的提示里显示进度，可以取消；
  取消或失败后不会再自动重试，点提示里的“下载 FFmpeg”即可重新下载。也可以在
  设置 → 关于 → 运行环境 中下载、更新或删除。下载的是 npm 包 [`@seankit/ffmpeg`](https://www.npmjs.com/package/@seankit/ffmpeg) /
  [`@seankit/ffprobe`](https://www.npmjs.com/package/@seankit/ffprobe) 中未经修改的上游静态构建（当前为 FFmpeg 9.0.2，
  Windows 上是 gyan.dev 的 essentials 构建，两个压缩包合计约 75 MB），先从 npmmirror、失败时从 npmjs 下载，
  按程序内置的大小与 SHA-256 校验后放到本机数据目录（Windows：`%LOCALAPPDATA%\com.xiaofany.cliprecipe\ffmpeg\<版本>\`），
  下载后立即生效、无需重启。只在启动时缺少 FFmpeg（自动下载一次）或点击按钮时才会联网；请求只是普通的 GET，不带任何用户信息；会使用环境变量
  `HTTPS_PROXY` / `NO_PROXY` 设置的代理。这些 FFmpeg 构建按 GPL-3.0-or-later 授权。
- **自行安装**：例如 Windows 上 `winget install Gyan.FFmpeg`（full 构建），让 `ffmpeg` / `ffprobe` 在 `PATH` 中，
  或设置环境变量 `CLIPRECIPE_FFMPEG` 指向 ffmpeg 可执行文件（ffprobe 需在同一目录）。

查找顺序：`CLIPRECIPE_FFMPEG` → 应用内下载的副本 → 程序所在目录 → `PATH`。设置 → 关于 中可以看到实际使用的 FFmpeg、
来源与可用的硬件编码器。

部分功能依赖 FFmpeg 的编译选项：字幕烧录需要 libass，HDR 转 SDR 需要 zimg（`zscale`），这些应用内下载的构建都有。
它**没有**的：`whisper` 滤镜——“语音生成字幕”的内置模式会在表单里提前说明，请改用外部的 `whisper-cli`，
或通过 `PATH` / `CLIPRECIPE_FFMPEG` 使用带 whisper 的完整构建（FFmpeg 8.0 起，如 gyan.dev full）；
另外少了 libdav1d（AV1 改用 libaom 解码，较慢）、SVT-AV1 / rav1e 编码器、frei0r、libplacebo、OpenCL / Vulkan 滤镜等，
ClipRecipe 不使用它们。

**语音生成字幕的模型**：程序不会自动下载任何东西。请从 whisper.cpp 的 Hugging Face 仓库
（<https://huggingface.co/ggerganov/whisper.cpp>）下载 ggml 格式的模型（如 `ggml-base.bin`），在工具里选择它。

## 下载与更新

从 [GitHub Releases](https://github.com/seansongx/ClipRecipe/releases) 下载：Windows 10 / 11 用 `*_x64-setup.exe`
（NSIS 安装包，安装到当前用户），Linux 用 `*.AppImage` 或 `*.deb`。暂不提供 macOS 版本。
安装包没有 Windows 代码签名，首次运行时 SmartScreen 可能提示“已保护你的电脑”，点“更多信息”→“仍要运行”即可。

**自动更新**：启动时（设置 → 通用 → 启动时检查更新，可关闭）或手动检查 GitHub 上的最新发布。Windows 安装包版与
AppImage 版发现新版本时显示版本号与更新说明，确认后在程序内下载（可取消）、校验签名、安装并重启；deb 安装的版本、
开发版，以及下载或校验失败时，改为在浏览器中打开发布页。检查更新只访问 GitHub（Release 里的 `latest.json`，
必要时 `api.github.com` 的公开发布信息），请求是不带用户信息的普通 GET；确认更新之后才从 GitHub Release 下载安装包。
发布流程见 [docs/RELEASING.md](docs/RELEASING.md)，各版本的变化见 [CHANGELOG.md](CHANGELOG.md)。

## 构建与运行

环境要求：

- Node.js 20+
- Rust（stable）与 [Tauri 2 的系统依赖](https://tauri.app/start/prerequisites/)（Windows 需要 MSVC 生成工具和 WebView2）
- FFmpeg：开发时不必另装——`npm install` 会装上开发依赖 `@seankit/ffmpeg` / `@seankit/ffprobe`，调试构建与
  `cargo test` 找不到别的 FFmpeg 时自动使用 `node_modules` 里的这份（设置了 `CLIPRECIPE_FFMPEG` 或 `PATH` 里有时以它们为准）

```bash
npm install
npm run tauri dev     # 开发运行
npm run tauri build   # 打包安装程序（Windows：NSIS；Linux：AppImage 与 deb）
```

只调试界面时可以 `npm run dev` 后在浏览器打开 <http://localhost:1420>（原生能力不可用，设置不会保存）。

## 测试

验收命令见 [CONTRIBUTING.md](CONTRIBUTING.md#提交前)。

`cargo test` 中的集成测试用 FFmpeg 的 `lavfi` 现场生成素材并真实运行，需要 FFmpeg 可用（`npm install` 后自动使用
`node_modules` 里的 FFmpeg 9.0.2 essentials；也可用 `PATH` 或 `CLIPRECIPE_FFMPEG` 指定别的构建，两者都应全部通过），
默认值一致性检查（`tests/recipe_defaults.rs`）需要 Node.js 并已 `npm install`。仓库里不包含任何媒体文件。
测试不访问网络：下载器的测试用进程内的本机 HTTP 服务器。

## 目录结构

```
src/                      前端（React + TypeScript + Tailwind v4 + shadcn/ui）
  components/ui/          shadcn/ui 组件（CLI 生成，风格 Mira，已按本应用的设计改过）
  components/             业务控件（范围输入、素材来源、区域框选、颜色、字体……）
  features/pipeline/      每个流水线模块的参数表单
  features/tools/         每个独立工具的参数表单
  features/registry.ts    功能元数据（ID、名称、分组、状态）
  state/                  队列、配方、设置、预览、工具运行等状态
  views/                  页面：批量处理、工具箱、设置
  lib/backend.ts          与后端通信的命令、事件与类型
  lib/utils.ts            cn()（合并 className）、scrollViewport()
  lib/platform.ts         平台来源（全应用只从这里读平台）
  lib/shortcuts.ts        快捷键的定义、显示文字与按键匹配
  debug/                  开发环境的调试面板（生产构建不含）
src-tauri/src/            后端（Rust）
  ffmpeg/                 FFmpeg 定位、探测、运行与进度、编码器探测；manifest.rs / managed.rs 应用内下载
src-tauri/resources/      ffmpeg-manifest.json：应用内下载的固定清单（npm run sync-ffmpeg-manifest 生成）
  model/                  范围值、素材来源、配方、设置
  pipeline/               配方 → FFmpeg 命令；modules/ 每个模块一个文件；steps.rs 字幕 / 封面 / HDR 等流水线级步骤
  jobs/                   导入、输出命名、调度执行、收尾（移回 / 删除源）、队列保存
  tools/                  独立工具框架与 34 个工具；common/ 为工具共用的引擎
  commands/               暴露给前端的命令
  desktop.rs              主窗口：首帧后显示、标题栏配色、任务栏进度、单实例、窗口位置
src-tauri/tests/          集成测试
docs/README.md            文档索引
docs/specs/               功能清单与说明（队列、流水线模块、独立工具、设置）
docs/architecture.md      架构约定的入口；architecture/ 下是各子系统的约定、模块 / 工具编写指南与实现说明
docs/RELEASING.md         发布流程（版本号、CHANGELOG、tag、草稿 Release、updater 签名密钥）
.github/workflows/        CI（前端检查，Windows / Linux 上的 fmt、clippy、test）与发布流水线
```

## 已知限制

- 只在 Windows 上做过完整测试；代码保持跨平台（macOS / Linux 的托盘、关机、回收站走各自的系统接口，未逐一验证）。
- 程序内自动更新只支持 Windows 安装包版与 AppImage 版；deb 安装的版本与自行编译的程序只提示并打开发布页。
- 只发布 Windows 与 Linux 版本；安装包没有 Windows 代码签名（首次运行会有 SmartScreen 提示）。
- 字幕擦除是本地近似处理（修补 / 模糊 / 遮盖指定区域），不会自动识别字幕位置。
- 改变视频时长只改容器里的时长元数据，FFmpeg 系的播放器仍显示真实时长。
- 双击播放源视频时不叠加关联的字幕（预览与处理结果中有）；ass / ssa 字幕保留文件自带样式，设置中的字幕样式只作用于 srt / vtt。
- 变速适应（设置 → 高级）只对“音视频合成”和“截取合并”生效，其它工具没有目标时长。
- 倒放按段处理（默认 3 秒一段），很长的视频会产生较多临时文件、耗时较长。

## 参与贡献

见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

[MIT](LICENSE)。FFmpeg 是独立项目，不随本仓库或安装包分发（应用内下载在本机缺少 FFmpeg 时从 npm 获取上游构建，
并作为独立程序运行），使用时请遵守其许可条款（LGPL / GPL，取决于构建方式；应用内下载的构建为 GPL-3.0-or-later）。
