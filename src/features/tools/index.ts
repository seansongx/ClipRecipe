// 独立工具注册表：功能 ID → 前端定义（默认参数 + 输入声明 + 表单）。
// 全部 34 个工具已在 M3 注册；实现某个工具只需修改它自己的 <工具>.tsx 文件。
//
// 每个工具的 <工具>.tsx 是单独的代码块，按需加载（打开对话框、队列“工具”菜单、开始运行时）：
// - React 组件里用 useToolDef(id) / useToolDefs(ids)（加载完成前为 undefined，加载后组件自动刷新）；
// - 其它地方用 loadToolDef(id)（异步）或 getToolDef(id)（已加载时同步返回）。
// 只有本目录下一层的 *.tsx 算工具模块；shared/ 里是工具表单共用的部件，不是工具。

import { useEffect, useState, useSyncExternalStore } from "react";
import { TOOLS } from "../registry";
import type { ToolParams } from "../../lib/tools";
import type { ToolDef } from "./types";

type ToolModule = { default: ToolDef };

/** 各工具模块的加载函数，键为 "./<工具>.tsx"。 */
const LOADERS = import.meta.glob<ToolModule>("./*.tsx");

const moduleKey = (id: string) => `./${id.replace(/^tool\./, "")}.tsx`;

/** 已加载的定义。 */
const loaded: Record<string, ToolDef> = {};
const pending = new Map<string, Promise<ToolDef | undefined>>();
const listeners = new Set<() => void>();
/** 每加载一个定义加一，供 useSyncExternalStore 判断是否变化 */
let loadedVersion = 0;

/**
 * 已加载的工具定义（旧名称，保留兼容）。注意：工具按需加载，这里只有已经加载过的；
 * 需要某个工具时用 loadToolDef / useToolDef。
 */
export const TOOL_DEFS: Readonly<Record<string, ToolDef>> = loaded;

// 开发时检查注册表与 registry.ts 是否一致
if (import.meta.env.DEV) {
  const missing = TOOLS.filter((t) => !LOADERS[moduleKey(t.id)]).map((t) => t.id);
  if (missing.length > 0) console.error("独立工具缺少前端定义：", missing);
}

/** 订阅“有新的工具定义加载完成”；返回取消订阅的函数。 */
export function subscribeToolDefs(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 已加载的全部定义。 */
export function loadedToolDefs(): ToolDef[] {
  return Object.values(loaded);
}

/** 已加载时同步返回定义，否则 undefined（不会触发加载）。 */
export function getToolDef(id: string): ToolDef | undefined {
  return loaded[id];
}

/** 加载某个工具的定义（只加载一次）；没有这个工具时为 undefined。加载失败时 reject，之后可以重试。 */
export function loadToolDef(id: string): Promise<ToolDef | undefined> {
  const cached = loaded[id];
  if (cached) return Promise.resolve(cached);
  const inFlight = pending.get(id);
  if (inFlight) return inFlight;
  const loader = LOADERS[moduleKey(id)];
  if (!loader) return Promise.resolve(undefined);
  const promise = loader()
    .then((mod) => {
      const def = mod.default;
      if (def.id !== id) console.error(`独立工具 ${moduleKey(id)} 的 id 为 ${def.id}，应为 ${id}`);
      loaded[id] = def;
      loadedVersion += 1;
      listeners.forEach((listener) => listener());
      return def;
    })
    .finally(() => pending.delete(id));
  pending.set(id, promise);
  return promise;
}

/** 预先加载若干工具（忽略失败，用到时会再试）。 */
export function preloadToolDefs(ids: readonly string[]): void {
  for (const id of ids) loadToolDef(id).catch(() => {});
}

const getLoadedVersion = () => loadedVersion;

/** 组件里取某个工具的定义：自动加载，加载完成后刷新；加载失败时给出 error，可调用 retry 重试。 */
export function useToolDef(id: string | null | undefined): {
  def: ToolDef | undefined;
  error: string | null;
  retry: () => void;
} {
  const def = useSyncExternalStore(subscribeToolDefs, () => (id ? loaded[id] : undefined));
  const [error, setError] = useState<{ id: string; message: string } | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!id || loaded[id]) return;
    let alive = true;
    setError(null);
    loadToolDef(id).catch((e: unknown) => {
      if (alive) setError({ id, message: String(e) });
    });
    return () => {
      alive = false;
    };
  }, [id, attempt]);
  return {
    def,
    error: error && error.id === id ? error.message : null,
    retry: () => setAttempt((n) => n + 1),
  };
}

/**
 * 组件里取若干工具的定义（按 ids 顺序，只含已加载的）。`load` 为 true 时挂载后加载其余的；
 * `loading` 表示还有没加载完的。
 */
export function useToolDefs(ids: readonly string[], load = true): { defs: ToolDef[]; loading: boolean } {
  useSyncExternalStore(subscribeToolDefs, getLoadedVersion);
  useEffect(() => {
    if (load) preloadToolDefs(ids);
  }, [ids, load]);
  const defs = ids.map((id) => loaded[id]).filter((def): def is ToolDef => !!def);
  return { defs, loading: defs.length < ids.filter((id) => LOADERS[moduleKey(id)]).length };
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

/** 某个工具的默认参数（深拷贝）。需要该工具已加载，否则为 {}。 */
export function toolDefaults(id: string): ToolParams {
  const def = loaded[id];
  return def ? clone(def.defaults) : {};
}

/**
 * 把保存的（可能来自旧版本的）参数补全为当前结构：缺失的字段取默认值，多余的字段丢弃。
 * 需要该工具已加载，否则为 {}。
 */
export function normalizeToolParams(id: string, saved: unknown): ToolParams {
  const params = toolDefaults(id);
  if (!saved || typeof saved !== "object") return params;
  for (const key of Object.keys(params)) {
    const value = (saved as Record<string, unknown>)[key];
    if (value !== undefined && typeof value === typeof params[key]) params[key] = value;
  }
  return params;
}

export type ToolDeclaration = {
  implemented: boolean;
  defaults: ToolParams;
  inputs: { videos: boolean; folders: string[] };
  preview: boolean;
};

/**
 * 全部工具的状态、默认参数与输入声明（会加载全部工具模块）。
 * scripts/dump-recipe-defaults.mjs 输出给 cargo test（tests/recipe_defaults.rs）比对。
 */
export async function toolDeclarations(): Promise<Record<string, ToolDeclaration>> {
  const modules = await Promise.all(Object.values(LOADERS).map((load) => load()));
  return Object.fromEntries(
    modules.map(({ default: def }) => [
      def.id,
      {
        implemented: TOOLS.some((t) => t.id === def.id && t.status === "done"),
        defaults: clone(def.defaults),
        inputs: { videos: def.inputs.videos, folders: def.inputs.folders.map((f) => f.key) },
        preview: !!def.preview,
      },
    ]),
  );
}
