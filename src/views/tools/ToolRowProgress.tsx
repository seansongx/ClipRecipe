// 队列行上的工具进度：独立工具处理到这个队列视频时（以及运行结束后），在行内显示该工具对它的状态与进度。
// 工作项 ID 等于队列任务 ID（逐个视频的工具），或工作项的输入路径包含这个视频（合并等多进一出的工具）。
// 与批量处理的进度互不影响：这一行带扳手图标和工具名。

import { Wrench } from "lucide-react";
import { Progress } from "@/components/ui/progress";
import Hint from "@/components/Hint";
import { cn } from "@/lib/utils";
import { toolName, useTools, type ToolRunItem } from "@/state/tools";
import { TOOL_STATE_TONE, toolItemPercent, toolItemStatus } from "./itemStatus";

/** 本次（或上次）工具运行中与这个队列视频对应的工作项；同一视频对应多项时优先正在处理的。 */
function useRowToolItem(queueId: string, path: string): ToolRunItem | undefined {
  return useTools((s) => {
    const matches = s.run?.items.filter((item) => item.id === queueId || item.inputs.includes(path));
    if (!matches || matches.length === 0) return undefined;
    return matches.find((item) => item.state === "running") ?? matches[0];
  });
}

export default function ToolRowProgress({ queueId, path }: { queueId: string; path: string }) {
  const item = useRowToolItem(queueId, path);
  const toolId = useTools((s) => s.run?.toolId);
  if (!item || !toolId) return null;
  const active = item.state === "running" || item.state === "done";
  return (
    <div className="mt-1.5">
      <Hint
        label={item.inputs.length > 1 ? `${item.label}（共 ${item.inputs.length} 个输入）` : null}
        side="bottom"
        align="start"
      >
        <div className="flex items-center gap-1 text-xs">
          <Wrench className="size-3 shrink-0 text-muted-foreground" />
          <span className="truncate text-muted-foreground">{toolName(toolId)}</span>
          <span className={cn("ml-auto shrink-0 tabular-nums", TOOL_STATE_TONE[item.state])}>{toolItemStatus(item)}</span>
        </div>
      </Hint>
      {active && (
        <Progress value={toolItemPercent(item)} className="mt-1" />
      )}
      {item.state === "failed" && item.error && (
        <p className="mt-1 line-clamp-2 text-xs text-destructive select-text">{item.error}</p>
      )}
    </div>
  );
}
