# ClipRecipe

开源的视频批量处理桌面工具，基于 **Tauri 2 + React + FFmpeg**。

把一组处理步骤（裁剪、水印、画中画、变速、调色……）配成一份“配方”，批量套用到整个视频队列；另有合并、分割、提取、转换等独立工具。

> 项目处于起步阶段：目前只有脚手架和功能规划，处理功能尚未实现。

## 功能规划

完整清单见 [docs/FEATURES.md](docs/FEATURES.md)：

- **批量流水线**：去/加水印、裁剪、画中画、去/加头尾、变速、文本、背景音、画面调整、宫格分屏、分辨率、旋转翻转、帧率、抽帧、动态缩放、码率、裂变输出。
- **独立工具**：合并、分割、场景重组、黑边去除、静音移除、音视频提取、无损转换、HLS 切片、GIF、倒放、字幕提取、图转视频等 30 余项。
- **其它**：配方导入导出、GPU 硬件加速、即时预览、深浅色主题。

## 开发

环境要求：

- Node.js 20+
- Rust（stable）与 [Tauri 2 的系统依赖](https://tauri.app/start/prerequisites/)（Windows 需要 MSVC 生成工具和 WebView2）
- FFmpeg / FFprobe 在 `PATH` 中，或用环境变量 `CLIPRECIPE_FFMPEG` 指定 ffmpeg 可执行文件

```bash
npm install
npm run tauri dev     # 开发
npm run tauri build   # 打包
```

只调试界面时可以直接 `npm run dev`，在浏览器里打开（原生能力不可用）。

## 目录结构

```
src/                 前端（React + TypeScript + Tailwind）
  features/          功能注册表：流水线模块与独立工具的元数据
  views/             页面：批量处理、工具箱、设置
  lib/               与后端通信的封装
src-tauri/           后端（Rust）
  src/commands.rs    暴露给前端的命令
  src/ffmpeg.rs      FFmpeg 定位与调用
docs/FEATURES.md     功能清单与进度
```

## 参与贡献

见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可

[MIT](LICENSE)。FFmpeg 是独立项目，不随本仓库分发，使用时请遵守其许可条款。
