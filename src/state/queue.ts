// 视频队列与任务进度。队列由前端维护；开始处理时把任务交给后端，后端通过 job-event 回报。
//
// 分层：本文件与 selection / log / settings 是最底层的状态。模块表单经 RegionPicker → selection 依赖它，
// 所以这里不能导入 state/recipe 或 features/*，否则会形成 features/pipeline → 表单 → … → features/pipeline
// 的循环，求值顺序一变就报 “Cannot access 'DEFS' before initialization”。需要配方时由调用方传入。

import { create } from "zustand";
import {
  cancelTask,
  findSidecars,
  importPaths,
  restoreQueue,
  saveQueue,
  prioritizeTask,
  startJobs,
  stopJobs,
  type ImportProgress,
  type ImportResult,
  type JobEvent,
  type MediaInfo,
  type Recipe,
  type SavedTask,
  type TaskSpec,
  type TaskState,
} from "../lib/backend";
import { fileName, pathKey } from "../lib/format";
import { useLog } from "./log";
import { useSettings } from "./settings";

export interface QueueItem {
  id: string;
  path: string;
  name: string;
  sourceRoot: string | null;
  info: MediaInfo | null;
  /** 导入时探测失败的原因 */
  probeError: string | null;
  state: TaskState | "idle";
  percent: number;
  speed: number;
  output: string | null;
  /** 全部输出（裂变时有多个） */
  outputs: string[];
  /** 正在处理第几次裂变 / 共几次 */
  round: number;
  rounds: number;
  error: string | null;
  /** 关联的字幕（处理时烧录进画面）与封面（嵌入输出），queue.sidecar */
  subtitle: string | null;
  cover: string | null;
}

export type AttachmentKind = "subtitle" | "cover";

export type SortKey = "name" | "duration" | "size";

export interface RunStats {
  done: number;
  failed: number;
  cancelled: number;
}

interface QueueState {
  items: QueueItem[];
  selected: Set<string>;
  /** 正在导入时的进度；total 为 0 表示正在展开文件夹 */
  importing: ImportProgress | null;
  running: boolean;
  lastRun: RunStats | null;
  /** 本次运行已完成的输出（“完成后打开输出文件夹”用；完成后移除的任务也算在内） */
  runOutputs: string[];

  addPaths: (paths: string[]) => Promise<void>;
  addImported: (result: ImportResult) => number;
  /** 设置 / 清除所选视频关联的字幕或封面。 */
  setAttachment: (ids: string[], kind: AttachmentKind, path: string | null) => void;
  /** 全部重载：重新查找每个视频的同名字幕与封面（找不到的清除）。 */
  reloadSidecars: () => Promise<void>;
  /** 读取上次保存的未完成任务（set.keep-queue）。 */
  restore: () => Promise<void>;
  remove: (ids: string[]) => void;
  clear: () => void;
  select: (id: string, mode: "single" | "toggle") => void;
  selectAll: () => void;
  invertSelection: () => void;
  clearSelection: () => void;
  setImportProgress: (progress: ImportProgress) => void;

  /** 把一个视频移到 index 位置（拖拽排序）。 */
  moveTo: (id: string, index: number) => void;
  /** 上移 / 下移所选（delta 为 -1 / 1）。 */
  moveSelected: (delta: -1 | 1) => void;
  /** 插队：把所选移到最前；处理中时等待的任务也会被后端提前。 */
  moveSelectedToTop: () => void;
  sortBy: (key: SortKey, descending: boolean) => void;

  /** 按给定配方处理队列（配方由调用方传入，见文件开头的分层说明）。 */
  start: (recipe: Recipe) => Promise<void>;
  stop: () => Promise<void>;
  cancel: (id: string) => Promise<void>;
  applyEvent: (event: JobEvent) => void;
}

let nextId = 1;
const newId = () => `t${Date.now().toString(36)}-${nextId++}`;

function patchItem(items: QueueItem[], id: string, patch: Partial<QueueItem>): QueueItem[] {
  return items.map((item) => (item.id === id ? { ...item, ...patch } : item));
}

export const useQueue = create<QueueState>((set, get) => ({
  items: [],
  selected: new Set(),
  importing: null,
  running: false,
  lastRun: null,
  runOutputs: [],

  addPaths: async (paths) => {
    if (paths.length === 0 || get().importing) return;
    const log = useLog.getState();
    set({ importing: { done: 0, total: 0 } });
    try {
      const { recursive, autoSubtitle, autoCover } = useSettings.getState().settings;
      const result = await importPaths(paths, recursive, { subtitle: autoSubtitle, cover: autoCover });
      const added = get().addImported(result);
      const parts = [`导入 ${added} 个视频`];
      if (result.items.length > added) parts.push(`${result.items.length - added} 个已在队列中`);
      if (result.skipped > 0) parts.push(`跳过 ${result.skipped} 个非视频文件`);
      log.add("info", parts.join("，"));
      for (const error of result.errors) log.add("warn", `无法导入：${error}`);
    } catch (e) {
      log.add("error", `导入失败：${String(e)}`);
    } finally {
      set({ importing: null });
    }
  },

  addImported: (result) => {
    const existing = new Set(get().items.map((item) => pathKey(item.path)));
    const fresh: QueueItem[] = [];
    for (const item of result.items) {
      const key = pathKey(item.path);
      if (existing.has(key)) continue;
      existing.add(key);
      if (item.error) useLog.getState().add("warn", `${fileName(item.path)}：${item.error}`);
      fresh.push({
        id: newId(),
        path: item.path,
        name: fileName(item.path),
        sourceRoot: item.sourceRoot,
        info: item.info,
        probeError: item.error,
        state: "idle",
        percent: 0,
        speed: 0,
        output: null,
        outputs: [],
        round: 0,
        rounds: 1,
        error: null,
        subtitle: item.subtitle ?? null,
        cover: item.cover ?? null,
      });
    }
    set({ items: [...get().items, ...fresh] });
    return fresh.length;
  },

  setAttachment: (ids, kind, path) => {
    const pick = new Set(ids);
    set({
      items: get().items.map((item) => (pick.has(item.id) ? { ...item, [kind]: path } : item)),
    });
  },

  reloadSidecars: async () => {
    const items = get().items;
    if (items.length === 0) return;
    try {
      const found = await findSidecars(
        items.map((i) => i.path),
        { subtitle: true, cover: true },
      );
      const byId = new Map(items.map((item, i) => [item.id, found[i]]));
      set({
        items: get().items.map((item) => {
          const hit = byId.get(item.id);
          return hit ? { ...item, subtitle: hit.subtitle, cover: hit.cover } : item;
        }),
      });
      const subs = found.filter((f) => f.subtitle).length;
      const covers = found.filter((f) => f.cover).length;
      useLog.getState().add("info", `全部重载：找到同名字幕 ${subs} 个、同名封面 ${covers} 个`);
    } catch (e) {
      useLog.getState().add("error", `重载同名字幕 / 封面失败：${String(e)}`);
    }
  },

  restore: async () => {
    const log = useLog.getState();
    try {
      const restored = await restoreQueue();
      for (const path of restored.missing) log.add("warn", `上次未完成的任务已不存在，已移除：${path}`);
      for (const note of restored.notes) log.add("warn", note);
      const added = get().addImported({ items: restored.items, skipped: 0, errors: [] });
      if (added > 0) log.add("info", `已恢复上次未完成的 ${added} 个任务`);
    } catch (e) {
      log.add("warn", `无法恢复上次的任务：${String(e)}`);
    }
  },

  remove: (ids) => {
    const drop = new Set(ids.filter((id) => get().items.find((i) => i.id === id)?.state !== "running"));
    const selected = new Set([...get().selected].filter((id) => !drop.has(id)));
    set({ items: get().items.filter((item) => !drop.has(item.id)), selected });
  },

  clear: () => {
    const running = get().items.filter((i) => i.state === "running" || i.state === "waiting");
    set({ items: running, selected: new Set() });
  },

  select: (id, mode) => {
    const selected = new Set(mode === "toggle" ? get().selected : []);
    if (mode === "toggle" && selected.has(id)) selected.delete(id);
    else selected.add(id);
    set({ selected });
  },
  selectAll: () => set({ selected: new Set(get().items.map((i) => i.id)) }),
  invertSelection: () => {
    const current = get().selected;
    set({ selected: new Set(get().items.filter((i) => !current.has(i.id)).map((i) => i.id)) });
  },
  clearSelection: () => set({ selected: new Set() }),
  setImportProgress: (progress) => {
    if (get().importing) set({ importing: progress });
  },

  moveTo: (id, index) => {
    const items = [...get().items];
    const from = items.findIndex((i) => i.id === id);
    if (from < 0) return;
    const [moved] = items.splice(from, 1);
    items.splice(Math.max(0, Math.min(index, items.length)), 0, moved!);
    set({ items });
  },

  moveSelected: (delta) => {
    const { selected } = get();
    const items = [...get().items];
    // 向上移动时从前往后处理，向下时从后往前，保证多选整体移动
    const order = items.map((_, i) => i);
    if (delta > 0) order.reverse();
    for (const i of order) {
      const j = i + delta;
      const a = items[i];
      const b = items[j];
      if (!a || !b || !selected.has(a.id) || selected.has(b.id)) continue;
      items[i] = b;
      items[j] = a;
    }
    set({ items });
  },

  moveSelectedToTop: () => {
    const { items, selected, running } = get();
    const picked = items.filter((i) => selected.has(i.id));
    if (picked.length === 0) return;
    set({ items: [...picked, ...items.filter((i) => !selected.has(i.id))] });
    if (running) {
      // 后端每次把一个任务移到最前：倒序提交，最终顺序与界面一致
      const waiting = picked.filter((i) => i.state === "waiting").reverse();
      for (const item of waiting) {
        prioritizeTask(item.id).catch((e) => useLog.getState().add("error", `插队失败：${String(e)}`));
      }
    }
  },

  sortBy: (key, descending) => {
    const value = (item: QueueItem): number => {
      if (key === "duration") return item.info?.duration ?? -1;
      return item.info?.size ?? -1;
    };
    const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });
    const items = [...get().items].sort((a, b) => {
      const order = key === "name" ? collator.compare(a.name, b.name) : value(a) - value(b);
      return descending ? -order : order;
    });
    set({ items });
  },

  start: async (recipe) => {
    const { items, running } = get();
    if (running) return;
    const log = useLog.getState();
    const { settings } = useSettings.getState();

    const tasks: TaskSpec[] = [];
    items.forEach((item, index) => {
      if (item.probeError && !item.info) return;
      if (settings.skipDone && item.state === "done") return;
      tasks.push({
        id: item.id,
        path: item.path,
        sourceRoot: item.sourceRoot,
        info: item.info,
        number: index + 1,
        subtitle: item.subtitle,
        cover: item.cover,
      });
    });
    if (tasks.length === 0) {
      log.add("warn", items.length === 0 ? "队列为空" : "没有需要处理的视频（已完成的会被跳过）");
      return;
    }
    const ids = new Set(tasks.map((t) => t.id));
    set({
      running: true,
      lastRun: null,
      runOutputs: [],
      items: items.map((item) =>
        ids.has(item.id)
          ? {
              ...item,
              state: "waiting",
              percent: 0,
              speed: 0,
              output: null,
              outputs: [],
              round: 0,
              rounds: recipe.fission,
              error: null,
            }
          : item,
      ),
    });
    try {
      const { seed } = await startJobs({ tasks, recipe, settings, outputDir: settings.outputDir });
      log.add("debug", `随机种子：${seed}`);
    } catch (e) {
      log.add("error", `无法开始处理：${String(e)}`);
      set({
        running: false,
        items: get().items.map((item) => (ids.has(item.id) ? { ...item, state: "idle" } : item)),
      });
    }
  },

  stop: async () => {
    try {
      await stopJobs();
    } catch (e) {
      useLog.getState().add("error", `停止失败：${String(e)}`);
    }
  },

  cancel: async (id) => {
    try {
      await cancelTask(id);
    } catch (e) {
      useLog.getState().add("error", `取消失败：${String(e)}`);
    }
  },

  applyEvent: (event) => {
    switch (event.type) {
      case "taskState": {
        const patch: Partial<QueueItem> = { state: event.state };
        if (event.state === "done") {
          patch.percent = 100;
          patch.output = event.output;
        }
        if (event.state === "done" || event.state === "failed" || event.state === "cancelled") {
          patch.outputs = event.outputs ?? [];
        }
        if (event.state === "failed") patch.error = event.error;
        if (event.state === "running") patch.percent = 0;
        let items = patchItem(get().items, event.taskId, patch);
        if (event.state === "done") {
          set({ runOutputs: [...get().runOutputs, ...(event.outputs ?? [])] });
          // set.remove-task：完成后从队列移除
          if (useSettings.getState().settings.removeFinished) {
            items = items.filter((item) => item.id !== event.taskId);
            const selected = new Set(get().selected);
            selected.delete(event.taskId);
            set({ selected });
          }
        }
        set({ items });
        break;
      }
      case "progress":
        set({
          items: patchItem(get().items, event.taskId, {
            percent: event.percent,
            speed: event.speed,
            round: event.round,
            rounds: event.rounds,
          }),
        });
        break;
      case "log":
        useLog.getState().add(event.level, event.message, event.taskId);
        break;
      case "runFinished":
        set({
          running: false,
          lastRun: { done: event.done, failed: event.failed, cancelled: event.cancelled },
        });
        break;
    }
  },
}));

/** 需要保存的任务：未完成的（已完成的不保存）。 */
export function unfinishedTasks(items: QueueItem[]): SavedTask[] {
  return items
    .filter((item) => item.state !== "done")
    .map((item) => ({
      path: item.path,
      sourceRoot: item.sourceRoot,
      info: item.info,
      subtitle: item.subtitle,
      cover: item.cover,
    }));
}

let persistTimer: ReturnType<typeof setTimeout> | undefined;

/** set.keep-queue：队列变化时（防抖）保存未完成的任务；关闭该设置时删除保存的内容。返回取消订阅的函数。 */
export function persistQueue(): () => void {
  const save = () => {
    clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      const keep = useSettings.getState().settings.keepQueue;
      saveQueue(keep ? unfinishedTasks(useQueue.getState().items) : []).catch((e) =>
        useLog.getState().add("warn", `保存未完成任务失败：${String(e)}`),
      );
    }, 800);
  };
  const stopQueue = useQueue.subscribe((state, prev) => {
    if (state.items !== prev.items) save();
  });
  const stopSettings = useSettings.subscribe((state, prev) => {
    if (state.settings.keepQueue !== prev.settings.keepQueue) save();
  });
  return () => {
    stopQueue();
    stopSettings();
  };
}
