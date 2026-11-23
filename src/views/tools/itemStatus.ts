// 工具工作项的状态文字与颜色（工具对话框的逐项列表、队列行上的工具进度共用）。

import type { ToolRunItem } from "@/state/tools";

const STATE_LABEL: Record<ToolRunItem["state"], string> = {
  waiting: "等待中",
  running: "处理中",
  done: "完成",
  failed: "失败",
  cancelled: "已取消",
};

export const TOOL_STATE_TONE: Record<ToolRunItem["state"], string> = {
  waiting: "text-muted-foreground",
  running: "text-primary",
  done: "text-success",
  failed: "text-destructive",
  cancelled: "text-muted-foreground",
};

/** 如“处理中 42% · 第 1/2 步 · 3.1x”“完成 · 3 个输出”。 */
export function toolItemStatus(item: ToolRunItem): string {
  let status = STATE_LABEL[item.state];
  if (item.state === "running") {
    status += ` ${item.percent.toFixed(0)}%`;
    if (item.steps > 1 && item.step > 0) status += ` · 第 ${item.step}/${item.steps} 步`;
    if (item.speed > 0) status += ` · ${item.speed.toFixed(1)}x`;
  }
  if (item.state === "done" && item.outputs.length !== 1) status += ` · ${item.outputs.length} 个输出`;
  return status;
}

/** 进度条的值（0～100）。 */
export const toolItemPercent = (item: ToolRunItem) => Math.max(0, Math.min(100, item.percent));
