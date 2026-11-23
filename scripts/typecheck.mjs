#!/usr/bin/env node
// TypeScript 类型检查（npm run typecheck，npm run build 的第一步）。
//
// 与直接运行 `tsc --noEmit` 相同，只有一处例外：src/components/ui/ 下的“声明了但没用”（TS6133）不算错误。
// 那里是 shadcn CLI 生成的原版，不许手改（npm run check:shadcn 核对），而 Base UI 版的个别组件留着用不到的导入
// （例如 scroll-area.tsx 的 `import * as React`），会撞上本项目的 noUnusedLocals。业务代码仍按 noUnusedLocals 检查。

import path from "node:path";
import { spawnSync } from "node:child_process";

const REPO = path.resolve(import.meta.dirname, "..");
const TSC = path.join(REPO, "node_modules", "typescript", "bin", "tsc");

const run = spawnSync(process.execPath, [TSC, "--noEmit", "--pretty", "false"], { cwd: REPO, encoding: "utf8" });
if (run.error) throw run.error;

// 每条诊断以“文件(行,列): error TSxxxx:”开头，后面缩进的行属于同一条
const diagnostics = [];
for (const line of `${run.stdout}${run.stderr}`.split(/\r?\n/)) {
  if (!line.trim()) continue;
  if (/^\s/.test(line) && diagnostics.length) diagnostics[diagnostics.length - 1] += `\n${line}`;
  else diagnostics.push(line);
}
const isStockUnused = (d) => /^src[\\/]components[\\/]ui[\\/][^(]+\(\d+,\d+\): error TS6133:/.test(d);
const errors = diagnostics.filter((d) => !isStockUnused(d));
const ignored = diagnostics.length - errors.length;

for (const e of errors) console.error(e);
if (ignored) console.log(`（忽略 src/components/ui/ 原版组件里 ${ignored} 处未使用的声明）`);
if (errors.length || (run.status !== 0 && diagnostics.length === 0)) process.exit(run.status || 1);
