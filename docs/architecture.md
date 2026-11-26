# 架构

功能范围见 [specs/overview.md](specs/overview.md)。本文是架构文档的入口：全局约定写在这里，各子系统的约定、编写指南与实现说明在 `architecture/` 下。

| 文档 | 内容 |
|---|---|
| [architecture/frontend.md](architecture/frontend.md) | 前端约定：shadcn/ui 的用法、主题 token、桌面应用体验约定、平台来源、开发环境的调试面板、界面的端到端检查 |
| [architecture/batch.md](architecture/batch.md) | 批量处理：流水线模块的编写指南，任务调度、流水线级功能、设置与收尾的实现说明 |
| [architecture/tools.md](architecture/tools.md) | 独立工具：框架、编写指南与实现说明 |
| [architecture/ffmpeg.md](architecture/ffmpeg.md) | 应用内下载 FFmpeg |
| [architecture/update.md](architecture/update.md) | 自动更新与发布的实现说明 |

目录结构见 [README.md](../README.md#目录结构)；提交前的验收命令与测试要求见 [CONTRIBUTING.md](../CONTRIBUTING.md#提交前)；发布流程见 [RELEASING.md](RELEASING.md)。

## 后端约定（`src-tauri/src/`）

- FFmpeg 命令行只在后端拼装，前端只传结构化参数。
- 所有随机取值（范围值、目录随机素材、裂变）在后端用**可指定种子**的随机数完成，保证单元测试可复现。
- 参数结构体用 serde，字段 `camelCase`，都有 `Default`；配方文件带版本号。
- 模块之间互不依赖；应用顺序在 `pipeline/mod.rs` 里集中定义。
- 新增模块/工具不应需要改动别的模块文件。公共注册点（模块列表、命令注册）已一次性把全部 ID 占位好，新增时只填自己的文件。

## 前端约定（`src/`）

见 [architecture/frontend.md](architecture/frontend.md)。
