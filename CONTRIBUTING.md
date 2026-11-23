# 参与贡献

感谢关注 ClipRecipe。架构约定、验收标准、模块 / 工具的编写指南都在 [docs/PLAN.md](docs/PLAN.md)，动手前请先读一遍。

## 从哪里开始

- [docs/FEATURES.md](docs/FEATURES.md) 列出了全部功能及状态（目前均已实现）。修 bug、改进现有功能、补测试都欢迎；
  新增功能请先开 issue 说明，并约定一个功能 ID。
- 每个功能用它的 ID（如 `pipe.crop`、`tool.merge`、`set.hdr`）贯穿文档、前端注册表（`src/features/registry.ts`）和后端代码。
- 新增流水线模块 / 独立工具时只改 PLAN.md 指南里列出的那几个文件；完成后更新 registry.ts 的 `status` 与 FEATURES.md 的状态。

## 约定（详见 PLAN.md）

- FFmpeg 命令行只在 Rust 侧拼装，前端只传结构化参数。
- 数值参数支持范围值 `{ min, max }`，随机取值在后端用可指定的种子完成，便于“裂变”复现与测试。
- 参数结构体用 serde（`camelCase`、`#[serde(default)]`，都有 `Default`）；前端 `defaults` 必须与后端 `Default` 一致，
  设置项的 `DEFAULT_SETTINGS` 必须与 `Settings::default()` 一致（`cargo test` 会自动检查）。
- 界面文字用中文；控件一律用 shadcn/ui（`npx shadcn@latest add <组件>` 添加），不要手写 `<button>` / `<input>` /
  `<select>` / `<textarea>` 等原生控件；颜色用 `index.css` 的主题变量，深浅色都要正常。
- 每个模块 / 工具都要有单元测试（断言生成的滤镜或命令行）和集成测试（`src-tauri/tests/`，用 `lavfi` 现场生成素材真实运行）。
  不提交媒体文件、FFmpeg 二进制或原软件的截图素材。

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

集成测试需要 FFmpeg（`PATH` 或环境变量 `CLIPRECIPE_FFMPEG`）与 Node.js（`npm install` 之后）。
