// 任务栏按钮上的进度（Windows，Tauri setProgressBar）：批量处理或独立工具运行时显示总进度；
// 结束（含停止）时清除；有失败时显示红色的出错状态，直到用户回到窗口（获得焦点或点击）。

import { isTauri, setTaskbarProgress, type TaskbarProgressState } from "../lib/backend";
import { useQueue, type QueueItem } from "./queue";
import { useTools, type ToolRunItem } from "./tools";

const FINISHED = new Set(["done", "failed", "cancelled"]);

/** 本次运行的总进度（0～100）：已结束的任务算 100（完成后被移出队列的也算），处理中的按百分比。 */
export function batchPercent(runIds: string[], items: QueueItem[]): number {
  if (runIds.length === 0) return 0;
  const byId = new Map(items.map((i) => [i.id, i]));
  let sum = 0;
  for (const id of runIds) {
    const item = byId.get(id);
    if (!item || FINISHED.has(item.state)) sum += 100;
    else if (item.state === "running") sum += Math.max(0, Math.min(100, item.percent));
  }
  return sum / runIds.length;
}

export function toolPercent(items: ToolRunItem[]): number {
  if (items.length === 0) return 0;
  const sum = items.reduce((acc, i) => acc + (FINISHED.has(i.state) ? 100 : Math.max(0, Math.min(100, i.percent))), 0);
  return sum / items.length;
}

/** 开始同步任务栏进度，返回停止函数。 */
export function syncTaskbarProgress(): () => void {
  if (!isTauri) return () => {};
  let failed = false;
  let last = "";

  const push = (state: TaskbarProgressState, progress?: number) => {
    const value = progress === undefined ? undefined : Math.round(progress);
    const key = `${state}:${value ?? ""}`;
    if (key === last) return;
    last = key;
    void setTaskbarProgress(state, value).catch(() => {});
  };

  const update = () => {
    const queue = useQueue.getState();
    const run = useTools.getState().run;
    if (queue.running) push("normal", batchPercent(queue.runIds, queue.items));
    else if (run?.running) push(run.items.length === 0 ? "indeterminate" : "normal", run.items.length ? toolPercent(run.items) : undefined);
    else if (failed) push("error", 100);
    else push("none");
  };

  const stopQueue = useQueue.subscribe((state, prev) => {
    if (state.running && !prev.running) failed = false;
    if (!state.running && prev.running) failed = (state.lastRun?.failed ?? 0) > 0;
    if (state.items !== prev.items || state.running !== prev.running) update();
  });
  const stopTools = useTools.subscribe((state, prev) => {
    const now = state.run;
    const before = prev.run;
    if (now?.running && !before?.running) failed = false;
    if (now && !now.running && before?.running) failed = (now.summary?.failed ?? 0) > 0;
    if (now !== before) update();
  });
  // 回到窗口后清除出错状态
  const acknowledge = () => {
    if (!failed) return;
    failed = false;
    update();
  };
  window.addEventListener("focus", acknowledge);
  window.addEventListener("pointerdown", acknowledge, true);
  update();
  return () => {
    stopQueue();
    stopTools();
    window.removeEventListener("focus", acknowledge);
    window.removeEventListener("pointerdown", acknowledge, true);
  };
}
