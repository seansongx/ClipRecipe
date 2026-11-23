// 流水线模块注册表：功能 ID → 前端定义（默认参数 + 表单）。
// 全部 18 个模块已在 M1 注册；后续实现某个模块只需修改它自己的 <模块>.tsx 文件。

import type { ModuleParams, Recipe } from "../../lib/backend";
import { MAX_FISSION, RECIPE_VERSION } from "../../lib/backend";
import { PIPELINE_MODULES } from "../registry";
import type { PipelineModuleDef } from "./types";

import addWatermark from "./add-watermark";
import adjust from "./adjust";
import bgm from "./bgm";
import bitrate from "./bitrate";
import crop from "./crop";
import dedupeFrames from "./dedupe-frames";
import dropFrames from "./drop-frames";
import fps from "./fps";
import grid from "./grid";
import introOutro from "./intro-outro";
import pip from "./pip";
import removeWatermark from "./remove-watermark";
import resolution from "./resolution";
import rotateFlip from "./rotate-flip";
import speed from "./speed";
import text from "./text";
import trim from "./trim";
import zoom from "./zoom";

const DEFS: PipelineModuleDef[] = [
  removeWatermark,
  addWatermark,
  pip,
  text,
  crop,
  resolution,
  rotateFlip,
  adjust,
  grid,
  zoom,
  trim,
  introOutro,
  speed,
  dropFrames,
  dedupeFrames,
  bgm,
  fps,
  bitrate,
];

export const PIPELINE_DEFS: Record<string, PipelineModuleDef> = Object.fromEntries(
  DEFS.map((def) => [def.id, def]),
);

// 开发时检查注册表与 registry.ts 是否一致
if (import.meta.env.DEV) {
  const missing = PIPELINE_MODULES.filter((m) => !PIPELINE_DEFS[m.id]).map((m) => m.id);
  if (missing.length > 0) console.error("流水线模块缺少前端定义：", missing);
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** 默认配方（与后端 `Recipe::default()` 一致：仅启用帧率，不裂变）。
 * `cargo test` 中的 tests/recipe_defaults.rs 会检查两边是否一致。 */
export function defaultRecipe(): Recipe {
  return {
    version: RECIPE_VERSION,
    fission: 1,
    modules: Object.fromEntries(DEFS.map((def) => [def.id, clone(def.defaults)])),
  };
}

/** 生成预览时不参与的模块（与后端 `pipeline::PREVIEW_EXCLUDED` 一致）。 */
export const PREVIEW_EXCLUDED: readonly string[] = [
  "pipe.speed",
  "pipe.bgm",
  "pipe.intro-outro",
  "pipe.fps",
  "pipe.bitrate",
];

/** 把裂变次数限制在 1..=MAX_FISSION 的整数。 */
export function clampFission(value: unknown): number {
  const n = typeof value === "number" && Number.isFinite(value) ? Math.round(value) : 1;
  return Math.min(MAX_FISSION, Math.max(1, n));
}

/** 把（可能来自旧版本或不完整的）配方补全为当前结构：缺失的模块与字段取默认值。 */
export function normalizeRecipe(input: unknown): Recipe {
  const recipe = defaultRecipe();
  if (!input || typeof input !== "object") return recipe;
  recipe.fission = clampFission((input as { fission?: unknown }).fission);
  const modules = (input as { modules?: unknown }).modules;
  if (!modules || typeof modules !== "object") return recipe;
  for (const def of DEFS) {
    const saved = (modules as Record<string, unknown>)[def.id];
    if (saved && typeof saved === "object") {
      recipe.modules[def.id] = { ...clone(def.defaults), ...(saved as Partial<ModuleParams>) };
    }
  }
  return recipe;
}
