# ClipRecipe

开源的视频批量处理桌面工具，基于 **Tauri 2 + React 19 + TypeScript + Rust + FFmpeg**，MIT 许可，免费使用、无需注册。

把一组处理步骤（去水印、裁剪、画中画、变速、调色……）配成一份“配方”，批量套用到整个视频队列；另有 34 个独立工具
（合并、分割、重组、提取、转换、字幕、封面等）。全部处理都由本机的 FFmpeg 完成，不依赖任何云服务。

## 功能

完整清单与每项的实现说明见 [docs/FEATURES.md](docs/FEATURES.md)（全部条目均已实现）。

- **批量流水线（18 个模块）**：去水印、加水印、画中画、文本、裁剪、分辨率、旋转翻转、画面调整（含 LUT）、宫格分屏、
  动态缩放、去头尾、加头尾与相框、变速、抽帧、重复帧移除、背景音、帧率、码率。数值参数支持范围值随机取样，
  可指定种子复现；**裂变**：每个视频按不同随机值输出多份。
- **预览**：单个模块或整条流水线生成低清预览片段；区域框选 / 位置选取直接在视频画面上操作。
- **配方**：导入导出 JSON，拖入窗口即加载。
- **队列**：文件 / 文件夹导入（可递归）、拖放、排序、插队、并行处理、处理记录；同名字幕自动关联并烧录、
  同名图片作封面嵌入；退出时保存未完成的任务，下次启动恢复。
- **独立工具（34 个）**：合并 / 分割 / 文件夹合并 / 截取合并 / HLS 切片、镜像 / 转场 / 拆分 / 移除重组、简单混剪、
  定帧动画、定格播放、运镜、黑边去除、插帧、进度条、横竖互转、画面拼接、倒放、图转视频、GIF、批量截图、
  静止帧 / 静音移除、智能去头尾、改变时长、音视频提取与合成、音量音调、无损转换、语音生成字幕（本地 whisper.cpp）、
  字幕提取、字幕擦除、封面处理。
- **设置**：目标格式、重名处理、文件名去除、并行数、H.265、HDR 转 SDR 或保留 HDR、GPU 硬件编码（NVENC / QSV / AMF，
  失败自动回退软件编码，可设核显优先）、体积压缩、版权元数据、完成后打开文件夹 / 提示音 / 关机 / 退出、
  处理后删除源视频（默认移入回收站）或把输出移回源位置、托盘、深浅色主题、检查更新（只提示并打开 GitHub 发布页）。

## FFmpeg

ClipRecipe 不包含也不分发 FFmpeg。请自行安装 FFmpeg（建议 7.0 以上的 full 构建，例如 Windows 上的
`winget install Gyan.FFmpeg`），并让 `ffmpeg` / `ffprobe` 在 `PATH` 中，或设置环境变量 `CLIPRECIPE_FFMPEG`
指向 ffmpeg 可执行文件（ffprobe 需在同一目录）。设置 → 关于 中可以看到检测结果与可用的硬件编码器。

部分功能依赖 FFmpeg 的编译选项：字幕烧录需要 libass，HDR 转 SDR 需要 zimg（`zscale`），
“语音生成字幕”的内置模式需要带 whisper 滤镜的构建（FFmpeg 8.0 起），否则改用外部的 `whisper-cli`。

**语音生成字幕的模型**：程序不会自动下载任何东西。请从 whisper.cpp 的 Hugging Face 仓库
（<https://huggingface.co/ggerganov/whisper.cpp>）下载 ggml 格式的模型（如 `ggml-base.bin`），在工具里选择它。

## 构建与运行

环境要求：

- Node.js 20+
- Rust（stable）与 [Tauri 2 的系统依赖](https://tauri.app/start/prerequisites/)（Windows 需要 MSVC 生成工具和 WebView2）
- FFmpeg（见上）

```bash
npm install
npm run tauri dev     # 开发运行
npm run tauri build   # 打包安装程序
```

只调试界面时可以 `npm run dev` 后在浏览器打开 <http://localhost:1420>（原生能力不可用，设置不会保存）。

## 测试

验收命令（[docs/PLAN.md](docs/PLAN.md#验收标准)）：

```bash
npm run typecheck
npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` 中的集成测试用 FFmpeg 的 `lavfi` 现场生成素材并真实运行，需要 FFmpeg 可用（`PATH` 或 `CLIPRECIPE_FFMPEG`），
默认值一致性检查（`tests/recipe_defaults.rs`）需要 Node.js 并已 `npm install`。仓库里不包含任何媒体文件。

## 目录结构

```
src/                      前端（React + TypeScript + Tailwind v4 + shadcn/ui）
  components/ui/          shadcn/ui 组件（CLI 生成）
  components/             业务控件（范围输入、素材来源、区域框选、颜色、字体……）
  features/pipeline/      每个流水线模块的参数表单
  features/tools/         每个独立工具的参数表单
  features/registry.ts    功能元数据（ID、名称、分组、状态）
  state/                  队列、配方、设置、预览、工具运行等状态
  views/                  页面：批量处理、工具箱、设置
  lib/backend.ts          与后端通信的命令、事件与类型
src-tauri/src/            后端（Rust）
  ffmpeg/                 FFmpeg 定位、探测、运行与进度、编码器探测
  model/                  范围值、素材来源、配方、设置
  pipeline/               配方 → FFmpeg 命令；modules/ 每个模块一个文件；steps.rs 字幕 / 封面 / HDR 等流水线级步骤
  jobs/                   导入、输出命名、调度执行、收尾（移回 / 删除源）、队列保存
  tools/                  独立工具框架与 34 个工具；common/ 为工具共用的引擎
  commands/               暴露给前端的命令
src-tauri/tests/          集成测试
docs/FEATURES.md          功能清单与状态
docs/PLAN.md              架构约定、验收标准、模块 / 工具编写指南、各里程碑的实现说明
```

## 已知限制

- 只在 Windows 上做过完整测试；代码保持跨平台（macOS / Linux 的托盘、关机、回收站走各自的系统接口，未逐一验证）。
- 检查更新只提示并打开发布页，不会自动下载或替换程序。
- 字幕擦除是本地近似处理（修补 / 模糊 / 遮盖指定区域），不会自动识别字幕位置。
- 改变视频时长只改容器里的时长元数据，FFmpeg 系的播放器仍显示真实时长。
- 双击播放源视频时不叠加关联的字幕（预览与处理结果中有）；ass / ssa 字幕保留文件自带样式，设置中的字幕样式只作用于 srt / vtt。
- 变速适应（设置 → 高级）只对“音视频合成”和“截取合并”生效，其它工具没有目标时长。
- 倒放按段处理（默认 3 秒一段），很长的视频会产生较多临时文件、耗时较长。

## 参与贡献

见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

[MIT](LICENSE)。FFmpeg 是独立项目，不随本仓库分发，使用时请遵守其许可条款（LGPL / GPL，取决于构建方式）。
