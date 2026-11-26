# 参与贡献

感谢关注 ClipRecipe。架构约定、模块 / 工具的编写指南都从 [docs/architecture.md](docs/architecture.md) 进入，动手前请先读一遍。

## 从哪里开始

- [docs/specs/](docs/specs/overview.md) 列出了全部功能（目前均已实现）。修 bug、改进现有功能、补测试都欢迎；
  新增功能请先开 issue 说明，并约定一个功能 ID。
- 每个功能用它的 ID（如 `pipe.crop`、`tool.merge`、`set.hdr`）贯穿文档、前端注册表（`src/features/registry.ts`）和后端代码。
- 新增流水线模块 / 独立工具时只改编写指南（[流水线模块](docs/architecture/batch.md)、[独立工具](docs/architecture/tools.md)）里列出的那几个文件；
  完成后更新 registry.ts 的 `status`，并在 `docs/specs/` 对应文件的表里加上该功能。

## 约定（详见 [docs/architecture.md](docs/architecture.md)）

- FFmpeg 命令行只在 Rust 侧拼装，前端只传结构化参数。
- 数值参数支持范围值 `{ min, max }`，随机取值在后端用可指定的种子完成，便于“裂变”复现与测试。
- 参数结构体用 serde（`camelCase`、`#[serde(default)]`，都有 `Default`）；前端 `defaults` 必须与后端 `Default` 一致，
  设置项的 `DEFAULT_SETTINGS` 必须与 `Settings::default()` 一致（`cargo test` 会自动检查）。
- 界面文字用中文；控件一律用 shadcn/ui（`npx shadcn@latest add <组件>` 添加），不要手写 `<button>` / `<input>` /
  `<select>` / `<textarea>` 等原生控件；颜色用 `index.css` 的主题变量，深浅色都要正常。shadcn 风格是 Mira，
  `src/components/ui/` 是 CLI 生成的原版，不要手改（`npm run check:shadcn` 核对）；外观只改 `index.css` 的主题变量，
  组件尺寸用自带的 `size` 变体，不给组件写改外观的类（详见 [docs/architecture/frontend.md](docs/architecture/frontend.md)）。
- 每个模块 / 工具都要有单元测试（断言生成的滤镜或命令行）和集成测试（`src-tauri/tests/`，用 `lavfi` 现场生成素材真实运行）。
  不提交媒体文件、FFmpeg 二进制或原软件的截图素材。
- 不留 `todo!()`、`unimplemented!()`、空实现或被注释掉的测试；做不到的点在交付说明里明确列出。

## 提交前

在仓库根目录与 `src-tauri/` 下运行验收命令，全部通过后再提交：

```bash
npm run typecheck
npm run build
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

推送到 `main` 或提交 PR 时，GitHub Actions（`.github/workflows/ci.yml`）会在 Windows 与 Linux 上跑同样的检查，
外加 `npm run check:shadcn` 与版本号一致性检查（`npm run release:check`）。

集成测试需要 FFmpeg 与 Node.js。`npm install` 会装上开发依赖 `@seankit/ffmpeg` / `@seankit/ffprobe`
（FFmpeg 9.0.2 essentials），`cargo test` 与调试构建找不到别的 FFmpeg 时自动使用 `node_modules` 里的这份，不必另装；
也可以用 `PATH` 或环境变量 `CLIPRECIPE_FFMPEG` 指定其它构建（例如带 whisper / flite 的 full 构建）。改动与 FFmpeg
构建有关的代码时，请分别用这两种 FFmpeg 跑一遍 `cargo test`。

升级应用内下载的 FFmpeg 版本见 [docs/architecture/ffmpeg.md](docs/architecture/ffmpeg.md)（改 npm 包版本后运行 `npm run sync-ffmpeg-manifest`，
提交生成的 `src-tauri/resources/ffmpeg-manifest.json`；不一致时 `cargo test` 会失败）。端到端冒烟测试 `npm run e2e`
同样直接使用 `node_modules` 里的 FFmpeg。

## 发布

版本号、CHANGELOG、打 tag 与草稿 Release 的步骤见 [docs/RELEASING.md](docs/RELEASING.md)。用户可见的改动请顺手在
[CHANGELOG.md](CHANGELOG.md) 顶部记一笔（没有对应版本时新建 `## [未发布]` 小节，发布时改为版本号与日期）。
