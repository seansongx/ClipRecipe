<!-- project-docs:rules:start -->
## 文档

文档索引在 `docs/README.md`。写或改文档前先读它。

- 文档只能放在 `docs/specs/`（功能与需求）、`docs/plans/`（进行中的计划）、`docs/decisions/`（决策记录）和 `docs/architecture.md`，其他位置不新建 `.md`。
- 写之前先搜 `docs/`，已有就原地更新；一个事实只写一处，别处用链接。
- 需求和功能是同一个文件：需求阶段写进 `docs/specs/` 并标 `status: planned`，做完改成 `shipped` 并把内容改成实际行为。
- 不写工作总结、改动说明、分析报告：这些写进 commit message 或直接在回复里说。
- 计划写在 `docs/plans/`，完成后把结论并入 spec 或架构文档，然后删除计划文件。
- 改代码让文档失效时，同一次提交里改文档。
- 代码里能看出来的（参数、字段、目录结构）不写进文档。
- Bug 和任务提 issue，不写进文档。
- 改完文档运行 `node scripts/check-docs.mjs --index` 更新索引，再运行 `node scripts/check-docs.mjs --check`。
<!-- project-docs:rules:end -->
