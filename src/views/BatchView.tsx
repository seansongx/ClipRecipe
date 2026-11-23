import { useEffect, useRef, useState, type RefObject } from "react";
import {
  ArrowDown,
  ArrowDownUp,
  ArrowUp,
  ArrowUpToLine,
  FileCog,
  FolderPlus,
  Play,
  Plus,
  Square,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Separator } from "@/components/ui/separator";
import Hint from "@/components/Hint";
import OutputPicker from "@/components/OutputPicker";
import { folderFromDrop } from "@/components/useFolderDrop";
import { isPointInElement, isRecipeFile, isTauri, onDragDrop, pickFolder, pickVideos } from "@/lib/backend";
import { useLog } from "@/state/log";
import { useQueue, type SortKey } from "@/state/queue";
import { useRecipe } from "@/state/recipe";
import { useSettings } from "@/state/settings";
import { useTools } from "@/state/tools";
import LogPanel from "./batch/LogPanel";
import QueueList from "./batch/QueueList";
import RecipePanel from "./batch/RecipePanel";
import { RemoveConfirmDialog, requestRemove } from "./batch/RemoveConfirm";
import { QueueToolsMenu } from "./tools/ToolMenus";

const toolbarButton = "h-7 px-2 text-xs font-normal text-muted-foreground";

function RunSummary() {
  const items = useQueue((s) => s.items);
  const running = useQueue((s) => s.running);
  const lastRun = useQueue((s) => s.lastRun);
  const count = (state: string) => items.filter((i) => i.state === state).length;
  if (running) {
    return (
      <span className="text-xs text-muted-foreground tabular-nums">
        处理中 {count("running")} · 等待 {count("waiting")} · 完成 {count("done")}
        {count("failed") > 0 && ` · 失败 ${count("failed")}`}
      </span>
    );
  }
  if (lastRun) {
    return (
      <span className="text-xs text-muted-foreground tabular-nums">
        上次：完成 {lastRun.done} · 失败 {lastRun.failed} · 取消 {lastRun.cancelled}
      </span>
    );
  }
  return null;
}

const SORT_OPTIONS: { value: string; label: string; key: SortKey; descending: boolean }[] = [
  { value: "name-asc", label: "名称 A→Z", key: "name", descending: false },
  { value: "name-desc", label: "名称 Z→A", key: "name", descending: true },
  { value: "duration-asc", label: "时长 短→长", key: "duration", descending: false },
  { value: "duration-desc", label: "时长 长→短", key: "duration", descending: true },
  { value: "size-asc", label: "大小 小→大", key: "size", descending: false },
  { value: "size-desc", label: "大小 大→小", key: "size", descending: true },
];

function QueueToolbar() {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  const { selectAll, invertSelection, moveSelected, moveSelectedToTop, sortBy } = useQueue.getState();
  const none = selected.size === 0;

  return (
    <div className="flex flex-wrap items-center gap-0.5 px-4 pt-2">
      <Button variant="ghost" size="xs" onClick={selectAll} disabled={items.length === 0} className={toolbarButton}>
        全选
      </Button>
      <Button variant="ghost" size="xs" onClick={invertSelection} disabled={items.length === 0} className={toolbarButton}>
        反选
      </Button>
      <Button variant="ghost" size="xs" onClick={() => requestRemove({ kind: "ids", ids: [...selected] })} disabled={none} className={toolbarButton}>
        删除所选{selected.size > 0 && ` (${selected.size})`}
      </Button>
      <Button
        variant="ghost"
        size="xs"
        onClick={() => requestRemove({ kind: "clear" })}
        disabled={items.length === 0}
        className={toolbarButton}
      >
        <Trash2 /> 清空
      </Button>
      <Separator orientation="vertical" className="mx-1 h-4 data-vertical:self-center" />
      <Button
        variant="ghost"
        size="xs"
        onClick={moveSelectedToTop}
        disabled={none}
        title="插队：移到最前（处理中时优先处理）"
        className={toolbarButton}
      >
        <ArrowUpToLine /> 置顶
      </Button>
      <Hint label="上移">
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="上移"
          onClick={() => moveSelected(-1)}
          disabled={none}
          className="size-7 text-muted-foreground"
        >
          <ArrowUp />
        </Button>
      </Hint>
      <Hint label="下移">
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="下移"
          onClick={() => moveSelected(1)}
          disabled={none}
          className="size-7 text-muted-foreground"
        >
          <ArrowDown />
        </Button>
      </Hint>
      <DropdownMenu>
        <DropdownMenuTrigger asChild disabled={items.length < 2}>
          <Button variant="ghost" size="xs" className={toolbarButton}>
            <ArrowDownUp /> 排序
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-auto">
          {SORT_OPTIONS.map((o) => (
            <DropdownMenuItem key={o.value} onSelect={() => sortBy(o.key, o.descending)} className="text-xs">
              {o.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <QueueToolsMenu className={toolbarButton} />
      <span className="ml-auto text-[11px] text-muted-foreground">Ctrl + 点击多选 · 双击播放 · 右键更多 · 拖动把手排序</span>
    </div>
  );
}

type DropHover = { target: "queue" | "output"; recipe: boolean } | null;

/** 窗口级拖放：配方文件 → 加载配方；落在输出位置上 → 设为输出文件夹；其余 → 导入视频。 */
function useWindowDrop(outputBox: RefObject<HTMLDivElement | null>): DropHover {
  const [hover, setHover] = useState<DropHover>(null);
  const draggingRecipe = useRef(false);

  useEffect(
    () =>
      onDragDrop((event) => {
        // 工具对话框打开时拖放由对话框处理（其中的文件夹输入、输出位置），这里不导入视频
        if (event.type === "leave" || useTools.getState().openId) {
          setHover(null);
          return;
        }
        if (event.type === "enter") {
          draggingRecipe.current = event.paths.length > 0 && event.paths.every(isRecipeFile);
        }
        const overOutput = isPointInElement(outputBox.current, event.x, event.y);
        const target = overOutput ? "output" : "queue";
        if (event.type !== "drop") {
          setHover({ target, recipe: draggingRecipe.current });
          return;
        }
        setHover(null);
        if (overOutput) {
          void folderFromDrop(event.paths).then((dir) => {
            if (dir) useSettings.getState().update({ outputDir: dir });
          });
          return;
        }
        const recipes = event.paths.filter(isRecipeFile);
        const videos = event.paths.filter((p) => !isRecipeFile(p));
        if (recipes.length > 0) {
          void useRecipe.getState().importFile(recipes[0]);
          if (recipes.length > 1) useLog.getState().add("warn", "一次只能加载一个配方文件，已加载第一个");
        }
        if (videos.length > 0) void useQueue.getState().addPaths(videos);
      }),
    [outputBox],
  );
  return hover;
}

export default function BatchView() {
  const items = useQueue((s) => s.items);
  const running = useQueue((s) => s.running);
  const importing = useQueue((s) => s.importing);
  const { addPaths, start, stop } = useQueue.getState();
  const recursive = useSettings((s) => s.settings.recursive);
  const outputBox = useRef<HTMLDivElement>(null);
  const hover = useWindowDrop(outputBox);

  return (
    <div className="flex h-full">
      <section className="relative flex min-w-0 flex-1 flex-col">
        <header className="flex flex-wrap items-center gap-2 border-b px-5 py-3">
          <h1 className="text-base font-semibold">视频队列</h1>
          <span className="text-muted-foreground">{items.length} 个</span>
          <div className="ml-auto flex gap-2">
            <Button
              variant="outline"
              onClick={() => void pickVideos().then(addPaths)}
              disabled={!isTauri || !!importing}
            >
              <Plus /> 添加视频
            </Button>
            <Button
              variant="outline"
              onClick={() =>
                void pickFolder().then((dir) => {
                  if (dir) void addPaths([dir]);
                })
              }
              disabled={!isTauri || !!importing}
              title={recursive ? "包含子目录（可在设置中修改）" : "不含子目录（可在设置中修改）"}
            >
              <FolderPlus /> 添加文件夹
            </Button>
          </div>
        </header>

        <QueueToolbar />

        <div className="relative flex min-h-0 flex-1 flex-col p-4 pt-2">
          <QueueList hovering={hover?.target === "queue" && !hover.recipe} />
          {hover?.target === "queue" && hover.recipe && (
            <div className="pointer-events-none absolute inset-4 top-2 grid place-items-center rounded-xl border-2 border-dashed border-primary bg-card/90">
              <p className="flex items-center gap-2">
                <FileCog className="size-5 text-primary" /> 松开以加载配方
              </p>
            </div>
          )}
        </div>

        <LogPanel />

        <footer className="flex items-start gap-3 border-t bg-card px-5 py-3">
          <OutputPicker boxRef={outputBox} dropTarget={hover?.target === "output"} />
          <div className="flex shrink-0 flex-col items-end gap-1">
            {running ? (
              <Button variant="outline" onClick={() => void stop()} className="px-4">
                <Square /> 停止
              </Button>
            ) : (
              <Button
                onClick={() => void start(useRecipe.getState().recipe)}
                disabled={!isTauri || items.length === 0 || !!importing}
                title={isTauri ? undefined : "需要在桌面应用中使用"}
                className="px-4"
              >
                <Play /> 开始处理
              </Button>
            )}
            <RunSummary />
          </div>
        </footer>
      </section>

      <RecipePanel />
      <RemoveConfirmDialog />
    </div>
  );
}
