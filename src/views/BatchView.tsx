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
import { FfmpegMissingBanner } from "@/components/FfmpegDownload";
import Hint from "@/components/Hint";
import OutputPicker from "@/components/OutputPicker";
import { folderFromDrop } from "@/components/useFolderDrop";
import { isPointInElement, isRecipeFile, isTauri, onDragDrop } from "@/lib/backend";
import { KEYS } from "@/lib/shortcuts";
import { addFolder, addVideos, openPaths } from "@/state/open";
import { useQueue, type SortKey } from "@/state/queue";
import { useRecipe } from "@/state/recipe";
import { useSettings } from "@/state/settings";
import { useTools } from "@/state/tools";
import LogPanel from "./batch/LogPanel";
import QueueList from "./batch/QueueList";
import RecipePanel from "./batch/RecipePanel";
import { RemoveConfirmDialog, requestRemove } from "./batch/RemoveConfirm";
import { QueueToolsMenu } from "./tools/ToolMenus";

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
    <div className="flex flex-wrap items-center gap-0.5 px-3 py-1.5">
      <Hint label="全选（列表获得焦点时）" keys={KEYS.selectAll}>
        <Button variant="ghost" size="sm" onClick={selectAll} disabled={items.length === 0}>
          全选
        </Button>
      </Hint>
      <Button variant="ghost" size="sm" onClick={invertSelection} disabled={items.length === 0}>
        反选
      </Button>
      <Hint label="从队列移除所选（不删除文件）" keys={KEYS.remove} disabledTrigger>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => requestRemove({ kind: "ids", ids: [...selected] })}
          disabled={none}
         
        >
          删除所选{selected.size > 0 && ` (${selected.size})`}
        </Button>
      </Hint>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => requestRemove({ kind: "clear" })}
        disabled={items.length === 0}
       
      >
        <Trash2 /> 清空
      </Button>
      <Separator orientation="vertical" className="mx-1 h-4 data-vertical:self-center" />
      <Hint label="插队：移到最前（处理中时优先处理）" disabledTrigger>
        <Button variant="ghost" size="sm" onClick={moveSelectedToTop} disabled={none}>
          <ArrowUpToLine /> 置顶
        </Button>
      </Hint>
      <Hint label="上移" disabledTrigger>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="上移"
          onClick={() => moveSelected(-1)}
          disabled={none}
        >
          <ArrowUp />
        </Button>
      </Hint>
      <Hint label="下移" disabledTrigger>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="下移"
          onClick={() => moveSelected(1)}
          disabled={none}
        >
          <ArrowDown />
        </Button>
      </Hint>
      <DropdownMenu>
        <DropdownMenuTrigger asChild disabled={items.length < 2}>
          <Button variant="ghost" size="sm">
            <ArrowDownUp /> 排序
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-auto">
          {SORT_OPTIONS.map((o) => (
            <DropdownMenuItem key={o.value} onSelect={() => sortBy(o.key, o.descending)}>
              {o.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <QueueToolsMenu />
      <span className="ml-auto truncate pl-2 text-muted-foreground">
        Ctrl / Shift + 点击多选 · 双击播放 · 右键更多 · 拖动把手排序
      </span>
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
        openPaths(event.paths);
      }),
    [outputBox],
  );
  return hover;
}

export default function BatchView() {
  const items = useQueue((s) => s.items);
  const running = useQueue((s) => s.running);
  const importing = useQueue((s) => s.importing);
  const { start, stop } = useQueue.getState();
  const recursive = useSettings((s) => s.settings.recursive);
  const outputBox = useRef<HTMLDivElement>(null);
  const hover = useWindowDrop(outputBox);

  return (
    <div className="flex h-full">
      <section className="relative flex min-w-0 flex-1 flex-col">
        <header className="flex h-11 shrink-0 items-center gap-2 border-b px-4">
          <h1 className="text-sm font-semibold">视频队列</h1>
          <div className="ml-auto flex gap-1.5">
            <Hint label="添加视频" keys={KEYS.addVideos}>
              <Button variant="outline" size="sm" onClick={() => void addVideos()} disabled={!isTauri || !!importing}>
                <Plus /> 添加视频
              </Button>
            </Hint>
            <Hint
              label={recursive ? "包含子目录（可在设置中修改）" : "不含子目录（可在设置中修改）"}
              keys={KEYS.addFolder}
            >
              <Button variant="outline" size="sm" onClick={() => void addFolder()} disabled={!isTauri || !!importing}>
                <FolderPlus /> 添加文件夹
              </Button>
            </Hint>
          </div>
        </header>

        <FfmpegMissingBanner />
        <QueueToolbar />

        <div className="relative flex min-h-0 flex-1 flex-col px-3 pb-2">
          <QueueList hovering={hover?.target === "queue" && !hover.recipe} />
          {hover?.target === "queue" && hover.recipe && (
            <div className="pointer-events-none absolute inset-x-3 top-0 bottom-2 grid place-items-center rounded-md border border-dashed border-primary bg-card/90">
              <p className="flex items-center gap-2">
                <FileCog className="size-4 text-primary" /> 松开以加载配方
              </p>
            </div>
          )}
        </div>

        <LogPanel />

        <footer className="flex items-center gap-3 border-t bg-card px-4 py-2">
          <OutputPicker boxRef={outputBox} dropTarget={hover?.target === "output"} />
          {running ? (
            <Hint label="停止处理" keys={KEYS.stop}>
              <Button variant="outline" onClick={() => void stop()} className="w-28 shrink-0">
                <Square /> 停止
              </Button>
            </Hint>
          ) : (
            <Hint label="按当前配方处理队列" keys={KEYS.start} disabledTrigger>
              <Button
                onClick={() => void start(useRecipe.getState().recipe)}
                disabled={!isTauri || items.length === 0 || !!importing}
                className="w-28 shrink-0"
              >
                <Play /> 开始处理
              </Button>
            </Hint>
          )}
        </footer>
      </section>

      <RecipePanel />
      <RemoveConfirmDialog />
    </div>
  );
}
