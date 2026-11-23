#!/usr/bin/env node
// 检查 src/components/ui/ 是否与 shadcn CLI 按 components.json（style：base-mira，Base UI 版的 Mira）生成的原版完全一致。
//
// 用法：npm run check:shadcn
//
// 做法：对 ui/ 里的每个组件运行 `shadcn add <组件> --diff <文件>`（预演，不写文件）：CLI 从注册表取当前风格的组件、
// 按本项目的别名与图标库转换后与本地文件比较，“(skip) No changes.” 即与原版一致。
// 有差异的文件打印 CLI 给出的 diff，并以退出码 1 结束。需要联网（ui.shadcn.com 注册表）。
// ui/ 下的文件不要手改，也不要用全局 CSS 覆盖它们的外观；主题变量用 CLI 的默认值（见 docs/PLAN.md“前端 · shadcn 组件与主题”）。

import fs from "node:fs";
import path from "node:path";
import { execFile } from "node:child_process";

const REPO = path.resolve(import.meta.dirname, "..");
const UI = path.join(REPO, "src", "components", "ui");
const CLI = path.join(REPO, "node_modules", "shadcn", "dist", "index.js");

const names = fs
  .readdirSync(UI)
  .filter((f) => f.endsWith(".tsx"))
  .map((f) => f.slice(0, -4))
  .sort();

function compare(name) {
  const file = `src/components/ui/${name}.tsx`;
  return new Promise((resolve) => {
    execFile(
      process.execPath,
      [CLI, "add", name, "--diff", file],
      { cwd: REPO, encoding: "utf8", maxBuffer: 16 * 1024 * 1024 },
      (error, stdout, stderr) => {
        const lines = `${stdout}`.split(/\r?\n/);
        const at = lines.findIndex((l) => /^├ /.test(l) && l.replace(/\\/g, "/").includes(file));
        if (at < 0) {
          resolve({ name, file, status: "error", detail: (stderr || error?.message || stdout).trim() });
          return;
        }
        const status = /\((\w+)\)/.exec(lines[at])?.[1] ?? "unknown";
        const detail = [];
        for (const l of lines.slice(at + 1)) {
          if (/^[├└]/.test(l)) break;
          detail.push(l);
        }
        resolve({ name, file, status, detail: detail.join("\n") });
      },
    );
  });
}

const results = [];
for (let i = 0; i < names.length; i += 7) {
  results.push(...(await Promise.all(names.slice(i, i + 7).map(compare))));
}

const style = JSON.parse(fs.readFileSync(path.join(REPO, "components.json"), "utf8")).style;
console.log(`shadcn 原版检查：${names.length} 个组件，style = ${style}`);
for (const r of results) console.log(`  ${r.status === "skip" ? "same" : "DIFF"}  ${r.file}`);
const differs = results.filter((r) => r.status !== "skip");
for (const r of differs) console.log(`\n${r.file}（${r.status}）：\n${r.detail}`);
console.log(differs.length ? `\n${differs.length} 个文件与原版不同` : "\n全部与原版一致");
process.exit(differs.length ? 1 : 0);
