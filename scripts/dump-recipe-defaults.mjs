// 输出前端默认配方（features/pipeline 中各模块 defaults 汇总后的 defaultRecipe()）为 JSON。
//
// 用途：src-tauri/tests/recipe_defaults.rs 运行本脚本，把结果与后端 Recipe::default() 对比，
// 防止前后端默认参数不一致。也可手动运行：node scripts/dump-recipe-defaults.mjs
// 加参数 `settings` 时输出前端 DEFAULT_SETTINGS（与后端 Settings::default() 对比）。
// 加参数 `tools` 时改为输出全部独立工具的默认参数与输入声明（与后端 tools::default_params()、
// tools::input_declarations() 对比）：node scripts/dump-recipe-defaults.mjs tools
//
// 借助 Vite 的 SSR 模块加载直接执行 TS/TSX 源码（不做类型检查，也不需要先构建）。

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
// 独立的缓存目录，避免与正在运行的 dev server 互相干扰
const cacheDir = mkdtempSync(join(tmpdir(), "cliprecipe-defaults-"));

let server;
try {
  server = await createServer({
    root,
    configFile: false,
    logLevel: "silent",
    cacheDir,
    appType: "custom",
    // 与 vite.config.ts / tsconfig.json 的 @/ 别名一致
    resolve: { alias: { "@": join(root, "src") } },
    server: { middlewareMode: true, hmr: false, ws: false, watch: null },
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  if (process.argv[2] === "settings") {
    // 设置：lib/backend.ts 的 DEFAULT_SETTINGS（与后端 Settings::default() 对比）
    const mod = await server.ssrLoadModule("/src/lib/backend.ts");
    process.stdout.write(JSON.stringify(mod.DEFAULT_SETTINGS));
  } else if (process.argv[2] === "tools") {
    // 独立工具：每个工具的 defaults 与 inputs（features/tools/index.ts 的 toolDeclarations()，
    // 异步：工具模块按需加载，这里一次加载全部）
    const mod = await server.ssrLoadModule("/src/features/tools/index.ts");
    process.stdout.write(JSON.stringify(await mod.toolDeclarations()));
  } else {
    const mod = await server.ssrLoadModule("/src/features/pipeline/index.ts");
    process.stdout.write(JSON.stringify(mod.defaultRecipe()));
  }
} finally {
  await server?.close();
  rmSync(cacheDir, { recursive: true, force: true });
}
