# 参与贡献

感谢关注 ClipRecipe。

## 认领功能

1. 在 [docs/FEATURES.md](docs/FEATURES.md) 里找一个状态为 ⬜ 的功能，开 issue 说明要做哪个 ID。
2. 实现后把该功能在 `docs/FEATURES.md` 中的状态、以及 `src/features/registry.ts` 中的 `status` 一并更新。

## 约定

- 每个功能用它的 ID（如 `pipe.crop`、`tool.merge`）贯穿文档、前端注册表和后端代码。
- FFmpeg 命令行的拼装放在 Rust 侧，前端只传结构化参数。
- 数值参数默认支持范围值 `[min, max]`，随机取值在后端完成，便于“裂变”复现。
- 提交前运行 `npm run typecheck` 和 `cargo fmt && cargo clippy`（在 `src-tauri/` 下）。
- 不要提交 FFmpeg 二进制、原软件的截图或素材。
