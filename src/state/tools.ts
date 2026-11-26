// 独立工具：各工具的参数与文件夹输入（记在本机 localStorage）、当前打开的工具、当前运行的进度。
// 工具定义按需加载（features/tools）：某个工具的定义加载后才把它保存的参数补全为当前结构（见 normalizeLoaded）。
// 运行事件来自后端 tool-event（见 state/events.ts），日志写进共用的处理记录（state/log.ts）。

import { create } from "zustand";
import {
  getToolDef,
  loadedToolDefs,
  loadToolDef,
  normalizeToolParams,
  subscribeToolDefs,
  toolDefaults,
} from "../features/tools";
import { TOOLS } from "../features/registry";
import type { JobEvent, TaskSpec, TaskState } from "../lib/backend";
import { cancelToolItem, startTool, stopTool, type ToolParams } from "../lib/tools";
import { requireFfmpeg } from "./ffmpeg";
import { useLog } from "./log";
import { useQueue, type QueueItem } from "./queue";
import { useView } from "./view";
import { useRecipe } from "./recipe";
import { useSettings } from "./settings";

/** 处理队列中的哪些视频。 */
export type VideoScope = "selected" | "all";

export interface ToolRunItem {
  id: string;
  label: string;
  inputs: string[];
  state: TaskState;
  percent: number;
  /** 第几步 / 共几步 */
  step: number;
  steps: number;
  speed: number;
  outputs: string[];
  error: string | null;
}

export interface ToolRun {
  toolId: string;
  items: ToolRunItem[];
  running: boolean;
  /** 本次运行开始前最后一条日志的 ID（之后的日志属于本次运行） */
  logFrom: number;
  summary: { done: number; failed: number; cancelled: number } | null;
}

interface ToolsState {
  /** 已加载定义的工具的参数（已补全为当前结构）；未加载的工具保存的参数在 savedParams 里原样保留 */
  params: Record<string, ToolParams>;
  folders: Record<string, Record<string, string>>;
  scope: VideoScope;
  /** 工具箱里当前选中的工具（工作区显示它） */
  openId: string | null;
  run: ToolRun | null;

  /** 选中这个工具并切到工具箱页（批量处理页的工具菜单用）；不给 scope 时按队列有无选中项决定处理范围 */
  open: (id: string, scope?: VideoScope) => void;
  /** 只在工具箱里选中工具（不改页面、不改处理范围） */
  select: (id: string) => void;
  setParams: (id: string, patch: Partial<ToolParams>) => void;
  resetParams: (id: string) => void;
  setFolder: (id: string, key: string, path: string) => void;
  setScope: (scope: VideoScope) => void;
  start: () => Promise<void>;
  stop: () => Promise<void>;
  cancelItem: (id: string) => Promise<void>;
  applyEvent: (event: JobEvent) => void;
}

const STORAGE_KEY = "cliprecipe.tools";

interface Saved {
  params?: Record<string, unknown>;
  folders?: Record<string, Record<string, string>>;
}

/** 本机保存的各工具参数（原样）：工具定义加载后再补全，写回时保留未加载工具的参数。 */
let savedParams: Record<string, unknown> = {};

function loadSaved(): { folders: Record<string, Record<string, string>> } {
  let folders: Record<string, Record<string, string>> = {};
  let saved: Saved = {};
  try {
    saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Saved;
  } catch {
    // 忽略：使用默认参数
  }
  if (saved.params && typeof saved.params === "object") savedParams = saved.params;
  if (saved.folders && typeof saved.folders === "object") folders = saved.folders;
  return { folders };
}

function persist(state: Pick<ToolsState, "params" | "folders">) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ params: { ...savedParams, ...state.params }, folders: state.folders }));
  } catch {
    // 忽略：仅本次生效
  }
}

export function toolName(id: string): string {
  return TOOLS.find((t) => t.id === id)?.name ?? id;
}

/** 工具是否可以运行（registry 中标记为 done）。 */
export function isToolReady(id: string): boolean {
  return TOOLS.find((t) => t.id === id)?.status === "done";
}

/** 按范围取队列中的视频（保持队列顺序，带队列序号）。 */
export function videosInScope(items: QueueItem[], selected: Set<string>, scope: VideoScope): TaskSpec[] {
  const tasks: TaskSpec[] = [];
  items.forEach((item, index) => {
    if (scope === "selected" && !selected.has(item.id)) return;
    if (item.probeError && !item.info) return;
    tasks.push({ id: item.id, path: item.path, sourceRoot: item.sourceRoot, info: item.info, number: index + 1 });
  });
  return tasks;
}

function patchItem(run: ToolRun, id: string, patch: Partial<ToolRunItem>): ToolRun {
  return { ...run, items: run.items.map((item) => (item.id === id ? { ...item, ...patch } : item)) };
}

const initial = loadSaved();

/** 为刚加载定义、还没有参数的工具补全保存的参数。 */
function normalizeLoaded() {
  const { params } = useTools.getState();
  const fresh = loadedToolDefs().filter((def) => !params[def.id]);
  if (fresh.length === 0) return;
  const added = Object.fromEntries(fresh.map((def) => [def.id, normalizeToolParams(def.id, savedParams[def.id])]));
  useTools.setState({ params: { ...params, ...added } });
}

export const useTools = create<ToolsState>((set, get) => ({
  params: {},
  folders: initial.folders,
  scope: "all",
  openId: null,
  run: null,

  open: (id, scope) => {
    const fallback = useQueue.getState().selected.size > 0 ? "selected" : "all";
    set({ openId: id, scope: scope ?? fallback });
    useView.getState().setView("toolbox");
    // 工作区挂载前就开始加载工具定义（失败时工作区会显示错误并可重试）
    loadToolDef(id).catch(() => {});
  },
  select: (id) => {
    set({ openId: id });
    loadToolDef(id).catch(() => {});
  },

  setParams: (id, patch) => {
    const params = { ...get().params, [id]: { ...(get().params[id] ?? toolDefaults(id)), ...patch } };
    set({ params });
    persist(get());
  },
  resetParams: (id) => {
    set({ params: { ...get().params, [id]: toolDefaults(id) } });
    persist(get());
  },
  setFolder: (id, key, path) => {
    const folders = { ...get().folders, [id]: { ...(get().folders[id] ?? {}), [key]: path } };
    set({ folders });
    persist(get());
  },
  setScope: (scope) => set({ scope }),

  start: async () => {
    const openId = get().openId;
    if (!openId || get().run?.running) return;
    if (!(await requireFfmpeg("运行工具"))) return;
    const log = useLog.getState();
    const name = toolName(openId);
    let def = getToolDef(openId);
    if (!def) {
      // 工作区在定义加载完成前不会启用“开始”；这里只是兜底
      try {
        def = await loadToolDef(openId);
      } catch (e) {
        log.add("error", `${name}：无法加载工具——${String(e)}`);
        return;
      }
      if (get().run?.running || get().openId !== openId) return;
    }
    const { params, folders, scope } = get();
    if (!def || !isToolReady(openId)) {
      log.add("warn", `${name}：该工具尚未实现`);
      return;
    }
    const { items, selected } = useQueue.getState();
    const videos = def.inputs.videos ? videosInScope(items, selected, scope) : [];
    if (def.inputs.videos && def.inputs.folders.length === 0 && videos.length === 0) {
      log.add("warn", `${name}：${scope === "selected" ? "没有选中的视频" : "队列为空"}`);
      return;
    }
    const chosen = folders[openId] ?? {};
    const missing = def.inputs.folders.find((slot) => !chosen[slot.key]?.trim());
    if (missing) {
      log.add("warn", `${name}：请选择${missing.label}`);
      return;
    }
    const { settings } = useSettings.getState();
    const entries = useLog.getState().entries;
    const logFrom = entries[entries.length - 1]?.id ?? 0;
    set({ run: { toolId: openId, items: [], running: true, logFrom, summary: null } });
    try {
      const response = await startTool({
        tool: openId,
        params: params[openId] ?? toolDefaults(openId),
        videos,
        folders: chosen,
        settings,
        outputDir: settings.outputDir,
        bitrate: useRecipe.getState().recipe.modules["pipe.bitrate"] ?? null,
      });
      log.add("debug", `[${name}] 随机种子：${response.seed}`);
      const current = get().run;
      if (!current || current.toolId !== openId) return;
      // 事件可能先于返回值到达：保留已收到的状态
      const known = new Map(current.items.map((item) => [item.id, item]));
      set({
        run: {
          ...current,
          items: response.items.map((item) => ({
            state: "waiting" as TaskState,
            percent: 0,
            step: 0,
            steps: 1,
            speed: 0,
            outputs: [],
            error: null,
            ...known.get(item.id),
            ...item,
          })),
        },
      });
    } catch (e) {
      log.add("error", `${name}：无法开始——${String(e)}`);
      set({ run: null });
    }
  },

  stop: async () => {
    try {
      await stopTool();
    } catch (e) {
      useLog.getState().add("error", `停止失败：${String(e)}`);
    }
  },

  cancelItem: async (id) => {
    try {
      await cancelToolItem(id);
    } catch (e) {
      useLog.getState().add("error", `取消失败：${String(e)}`);
    }
  },

  applyEvent: (event) => {
    const run = get().run;
    const prefix = run ? `[${toolName(run.toolId)}] ` : "";
    switch (event.type) {
      case "taskState": {
        if (!run) return;
        const patch: Partial<ToolRunItem> = { state: event.state };
        if (event.state === "running") patch.percent = 0;
        if (event.state === "done") patch.percent = 100;
        if (event.state === "done" || event.state === "failed" || event.state === "cancelled") {
          patch.outputs = event.outputs ?? [];
        }
        if (event.state === "failed") patch.error = event.error;
        const exists = run.items.some((item) => item.id === event.taskId);
        set({
          run: exists
            ? patchItem(run, event.taskId, patch)
            : {
                ...run,
                items: [
                  ...run.items,
                  {
                    id: event.taskId,
                    label: event.taskId,
                    inputs: [],
                    state: event.state,
                    percent: 0,
                    step: 0,
                    steps: 1,
                    speed: 0,
                    outputs: [],
                    error: null,
                    ...patch,
                  },
                ],
              },
        });
        break;
      }
      case "progress":
        if (!run) return;
        set({
          run: patchItem(run, event.taskId, {
            percent: event.percent,
            speed: event.speed,
            step: event.round,
            steps: event.rounds,
          }),
        });
        break;
      case "log":
        useLog.getState().add(event.level, `${prefix}${event.message}`, event.taskId);
        break;
      case "runFinished":
        if (!run) return;
        set({
          run: {
            ...run,
            running: false,
            summary: { done: event.done, failed: event.failed, cancelled: event.cancelled },
          },
        });
        break;
    }
  },
}));

subscribeToolDefs(normalizeLoaded);
normalizeLoaded();
