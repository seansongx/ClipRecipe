// 从批量队列打开独立工具：队列工具栏的“工具”下拉菜单、队列行右键菜单里的“独立工具”子菜单。
// 只列出处理队列视频、且已实现的工具；打开时处理范围为“选中的视频”（没有选中时为全部）。
// 工具定义按需加载：菜单打开（或指针移到按钮上）时才加载已实现工具的定义，以便知道哪些处理队列视频。

import { Wrench } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Spinner } from "@/components/ui/spinner";
import { TOOLS, groupBy, type Feature } from "@/features/registry";
import { preloadToolDefs, useToolDefs } from "@/features/tools";
import { useQueue } from "@/state/queue";
import { useTools } from "@/state/tools";

/** 已实现的工具（不论输入类型）。 */
const DONE_TOOLS = TOOLS.filter((t) => t.status === "done");
const DONE_TOOL_IDS = DONE_TOOLS.map((t) => t.id);

/** 能对队列视频使用的已实现工具，按分组；挂载时加载这些工具的定义。 */
function useQueueToolGroups(): { groups: [string, Feature[]][]; loading: boolean } {
  const { defs, loading } = useToolDefs(DONE_TOOL_IDS);
  const videoTools = new Set(defs.filter((def) => def.inputs.videos).map((def) => def.id));
  return { groups: groupBy(DONE_TOOLS.filter((t) => videoTools.has(t.id))), loading };
}

function openFromQueue(id: string) {
  const scope = useQueue.getState().selected.size > 0 ? "selected" : "all";
  useTools.getState().open(id, scope);
}

const preload = () => preloadToolDefs(DONE_TOOL_IDS);

function LoadingLabel() {
  return (
    <span className="flex items-center gap-1.5 px-2 py-1.5 text-xs text-muted-foreground">
      <Spinner className="size-3" /> 加载中…
    </span>
  );
}

function QueueToolsMenuItems() {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  const { groups, loading } = useQueueToolGroups();
  return (
    <>
      {/* Base UI 的菜单标签（GroupLabel）必须放在 Group 里 */}
      <DropdownMenuGroup>
        <DropdownMenuLabel>
          {selected.size > 0 ? `处理选中的 ${selected.size} 个视频` : `处理全部 ${items.length} 个视频`}
        </DropdownMenuLabel>
      </DropdownMenuGroup>
      {groups.map(([group, tools]) => (
        <DropdownMenuGroup key={group}>
          <DropdownMenuSeparator />
          <DropdownMenuLabel>{group}</DropdownMenuLabel>
          {tools.map((tool) => (
            <DropdownMenuItem key={tool.id} onClick={() => openFromQueue(tool.id)}>
              {tool.name}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
      ))}
      {loading && <LoadingLabel />}
      {!loading && groups.length === 0 && (
        <p className="px-2 py-1.5 text-xs text-muted-foreground">没有可处理队列视频的工具</p>
      )}
    </>
  );
}

/** 队列工具栏上的“工具”下拉菜单。 */
export function QueueToolsMenu({ className }: { className?: string }) {
  const items = useQueue((s) => s.items);
  if (DONE_TOOLS.length === 0) return null;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={items.length === 0}
        render={<Button variant="ghost" size="sm" className={className} onPointerEnter={preload} onFocus={preload} />}
      >
        <Wrench /> 工具
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-auto min-w-40">
        <QueueToolsMenuItems />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 队列行右键菜单中的“独立工具”子菜单（处理当前选中的视频）。只在右键菜单打开时挂载。 */
export function QueueToolsSubmenu() {
  const { groups, loading } = useQueueToolGroups();
  if (DONE_TOOLS.length === 0 || (!loading && groups.length === 0)) return null;
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <Wrench /> 独立工具
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="min-w-36">
        {groups.map(([group, tools], index) => (
          <ContextMenuGroup key={group}>
            {index > 0 && <ContextMenuSeparator />}
            <ContextMenuLabel>{group}</ContextMenuLabel>
            {tools.map((tool) => (
              <ContextMenuItem key={tool.id} onClick={() => openFromQueue(tool.id)}>
                {tool.name}
              </ContextMenuItem>
            ))}
          </ContextMenuGroup>
        ))}
        {loading && <LoadingLabel />}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}
