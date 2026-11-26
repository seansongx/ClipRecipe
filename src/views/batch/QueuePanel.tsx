import { useMemo, useState } from "react";
import { ArrowDown, ArrowUp, ArrowUpToLine, ChevronDown, FileCog, FolderPlus, MoreHorizontal, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import Hint from "@/components/Hint";
import { isTauri } from "@/lib/backend";
import { formatDuration } from "@/lib/format";
import { useShortcutLabels } from "@/lib/shortcuts";
import { addFolder, addVideos } from "@/state/open";
import { useQueue, type SortKey } from "@/state/queue";
import { useSettings } from "@/state/settings";
import { QueueToolsMenu } from "../tools/ToolMenus";
import QueueList, { matchesFilter, type StatusFilter } from "./QueueList";
import { requestRemove } from "./RemoveConfirm";
import type { DropHover } from "./useWindowDrop";

const SORT_OPTIONS: { value: string; label: string; key: SortKey; descending: boolean }[] = [
  { value: "name-asc", label: "名称 A→Z", key: "name", descending: false },
  { value: "name-desc", label: "名称 Z→A", key: "name", descending: true },
  { value: "duration-asc", label: "时长 短→长", key: "duration", descending: false },
  { value: "duration-desc", label: "时长 长→短", key: "duration", descending: true },
  { value: "size-asc", label: "大小 小→大", key: "size", descending: false },
  { value: "size-desc", label: "大小 大→小", key: "size", descending: true },
];

const FILTERS: { value: StatusFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "pending", label: "待处理" },
  { value: "done", label: "完成" },
  { value: "failed", label: "失败" },
];

/** 标题行：“添加”下拉菜单（添加视频 / 添加文件夹）。 */
function AddMenu() {
  const importing = useQueue((s) => s.importing);
  const recursive = useSettings((s) => s.settings.recursive);
  const keys = useShortcutLabels();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger disabled={!isTauri || !!importing} render={<Button variant="outline" size="sm" />}>
        <Plus /> 添加 <ChevronDown />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-auto min-w-48">
        <DropdownMenuItem onClick={() => void addVideos()}>
          <Plus /> 添加视频
          <DropdownMenuShortcut>{keys.addVideos}</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => void addFolder()}>
          <FolderPlus /> 添加文件夹
          <DropdownMenuShortcut>{keys.addFolder}</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuLabel>
            {recursive ? "包含子目录（可在设置中修改）" : "不含子目录（可在设置中修改）"}
          </DropdownMenuLabel>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 标题行：“更多”菜单（全选、反选、排序、清空）。 */
function MoreMenu() {
  const count = useQueue((s) => s.items.length);
  const { selectAll, invertSelection, sortBy } = useQueue.getState();
  const keys = useShortcutLabels();
  return (
    <DropdownMenu>
      <Hint label="更多">
        <DropdownMenuTrigger
          disabled={count === 0}
          render={<Button variant="ghost" size="icon-sm" aria-label="更多" />}
        >
          <MoreHorizontal />
        </DropdownMenuTrigger>
      </Hint>
      <DropdownMenuContent align="end" className="w-auto min-w-40">
        <DropdownMenuItem onClick={selectAll}>
          全选
          <DropdownMenuShortcut>{keys.selectAll}</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={invertSelection}>反选</DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger disabled={count < 2}>排序</DropdownMenuSubTrigger>
          <DropdownMenuSubContent className="w-auto min-w-32">
            {SORT_OPTIONS.map((o) => (
              <DropdownMenuItem key={o.value} onClick={() => sortBy(o.key, o.descending)}>
                {o.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={() => requestRemove({ kind: "clear" })}>
          <Trash2 /> 清空
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** 选中后才出现的操作条（列表下方）。 */
function SelectionBar() {
  const selected = useQueue((s) => s.selected);
  const { moveSelected, moveSelectedToTop } = useQueue.getState();
  const keys = useShortcutLabels();
  if (selected.size === 0) return null;
  return (
    <div className="m-2 mt-1 flex shrink-0 items-center gap-0.5 rounded-xl border bg-secondary py-1.5 pr-1.5 pl-3">
      <span className="mr-auto text-muted-foreground">已选 {selected.size} 个</span>
      <Hint label="插队：移到最前（处理中时优先处理）">
        <Button variant="ghost" size="icon-sm" aria-label="置顶" onClick={moveSelectedToTop}>
          <ArrowUpToLine />
        </Button>
      </Hint>
      <Hint label="上移">
        <Button variant="ghost" size="icon-sm" aria-label="上移" onClick={() => moveSelected(-1)}>
          <ArrowUp />
        </Button>
      </Hint>
      <Hint label="下移">
        <Button variant="ghost" size="icon-sm" aria-label="下移" onClick={() => moveSelected(1)}>
          <ArrowDown />
        </Button>
      </Hint>
      <QueueToolsMenu />
      <Hint label="从队列移除所选（不删除文件）" keys={keys.remove}>
        <Button
          variant="ghost"
          size="sm"
          className="text-[color-mix(in_oklab,var(--destructive)_80%,var(--foreground))] hover:text-[color-mix(in_oklab,var(--destructive)_80%,var(--foreground))]"
          onClick={() => requestRemove({ kind: "ids", ids: [...selected] })}
        >
          <Trash2 /> 移除
        </Button>
      </Hint>
    </div>
  );
}

/** 批量处理页左栏：队列标题与添加菜单、状态筛选、队列列表、选中后的操作条。 */
export default function QueuePanel({ hover }: { hover: DropHover }) {
  const items = useQueue((s) => s.items);
  const [filter, setFilter] = useState<StatusFilter>("all");

  const counts = useMemo(
    () => ({
      all: items.length,
      pending: items.filter((i) => matchesFilter(i, "pending")).length,
      done: items.filter((i) => matchesFilter(i, "done")).length,
      failed: items.filter((i) => matchesFilter(i, "failed")).length,
    }),
    [items],
  );
  const totalDuration = items.reduce((sum, i) => sum + (i.info?.duration ?? 0), 0);

  return (
    <section className="flex w-80 shrink-0 flex-col border-r bg-card">
      <header className="flex h-12 shrink-0 items-center gap-2 pr-3 pl-4">
        <h1 className="text-sm font-semibold">视频队列</h1>
        {items.length > 0 && (
          <span className="truncate text-muted-foreground">
            {items.length} 个{totalDuration > 0 && ` · 共 ${formatDuration(totalDuration)}`}
          </span>
        )}
        <div className="ml-auto flex shrink-0 items-center gap-1">
          <AddMenu />
          <MoreMenu />
        </div>
      </header>
      {items.length > 0 && (
        <div className="shrink-0 px-3 pb-2.5">
          <ToggleGroup
            variant="pill"
            size="sm"
            spacing={1}
            aria-label="按状态筛选"
            value={[filter]}
            onValueChange={(next) => {
              const chosen = FILTERS.find((f) => f.value === next[0]);
              if (chosen) setFilter(chosen.value);
            }}
          >
            {FILTERS.map((f) => (
              <ToggleGroupItem key={f.value} value={f.value}>
                {f.label} {counts[f.value]}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>
      )}
      <div className="relative flex min-h-0 flex-1 flex-col px-2 pb-2">
        <QueueList hovering={hover?.target === "queue" && !hover.recipe} filter={filter} />
        {hover?.target === "queue" && hover.recipe && (
          <div className="pointer-events-none absolute inset-x-2 top-0 bottom-2 grid place-items-center rounded-md border border-dashed border-primary bg-card/90">
            <p className="flex items-center gap-2">
              <FileCog className="size-4 text-primary-ink" /> 松开以加载配方
            </p>
          </div>
        )}
      </div>
      <SelectionBar />
    </section>
  );
}
