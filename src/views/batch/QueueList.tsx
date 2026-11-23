import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { Captions, Film, GripVertical, Image, Info, Play, RefreshCw, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import Hint from "@/components/Hint";
import { COVER_EXTENSIONS, isTauri, pickFile, SUBTITLE_EXTENSIONS } from "@/lib/backend";
import { dirName, formatBitrate, formatDuration, formatFps, formatSize } from "@/lib/format";
import { KEYS } from "@/lib/shortcuts";
import { cn, scrollViewport } from "@/lib/utils";
import { usePreview } from "@/state/preview";
import { useQueue, type QueueItem } from "@/state/queue";
import { QueueToolsSubmenu } from "../tools/ToolMenus";
import ToolRowProgress from "../tools/ToolRowProgress";
import MediaInfoDialog from "./MediaInfoDialog";
import { requestRemove } from "./RemoveConfirm";

const STATE_LABEL: Record<QueueItem["state"], string> = {
  idle: "未处理",
  waiting: "等待中",
  running: "处理中",
  done: "完成",
  failed: "失败",
  cancelled: "已取消",
};

const STATE_TONE: Record<QueueItem["state"], string> = {
  idle: "text-muted-foreground",
  waiting: "text-muted-foreground",
  running: "text-primary",
  done: "text-success",
  failed: "text-destructive",
  cancelled: "text-muted-foreground",
};

// 行内按钮只在悬停该行（或键盘聚焦）时出现
const rowButton = "opacity-0 focus-visible:opacity-100 group-hover:opacity-100";

const rowDomId = (id: string) => `queue-row-${id}`;

function mediaSummary(item: QueueItem): string {
  const info = item.info;
  if (!info) return item.probeError ?? "未读取媒体信息";
  const v = info.video;
  const parts: string[] = [];
  if (v) {
    const [w, h] = v.rotation % 180 === 90 ? [v.height, v.width] : [v.width, v.height];
    parts.push(`${w}×${h}`, formatFps(v.fps));
  }
  parts.push(formatDuration(info.duration));
  parts.push(formatBitrate(v?.bitRate ?? info.bitRate));
  const codecs = [v?.codec, info.audio[0]?.codec].filter(Boolean).join(" / ");
  if (codecs) parts.push(codecs);
  parts.push(formatSize(info.size));
  return parts.filter(Boolean).join(" · ");
}

function StreamBadge({ title, children }: { title: string; children: string }) {
  return (
    <Hint label={title}>
      <Badge variant="outline" aria-label={title}>
        {children}
      </Badge>
    </Hint>
  );
}

/** 关联的字幕 / 封面（queue.sidecar）：处理时烧录 / 嵌入。 */
function AttachmentBadge({ title, children }: { title: string; children: string }) {
  return (
    <Hint label={title}>
      <Badge variant="secondary" aria-label={title}>
        {children}
      </Badge>
    </Hint>
  );
}

/** 队列行右键菜单的“字幕与封面”子菜单：设置 / 清除所选视频的关联文件，全部重载。 */
function SidecarSubmenu() {
  const { selected, setAttachment, reloadSidecars } = useQueue.getState();
  const ids = () => [...selected];
  const choose = async (kind: "subtitle" | "cover") => {
    const path =
      kind === "subtitle"
        ? await pickFile("字幕", SUBTITLE_EXTENSIONS)
        : await pickFile("图片", COVER_EXTENSIONS);
    if (path) setAttachment(ids(), kind, path);
  };
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <Captions /> 字幕与封面
      </ContextMenuSubTrigger>
      <ContextMenuSubContent className="min-w-40">
        <ContextMenuItem disabled={!isTauri} onSelect={() => void choose("subtitle")}>
          <Captions /> 设置字幕…
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => setAttachment(ids(), "subtitle", null)}>
          清除字幕
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem disabled={!isTauri} onSelect={() => void choose("cover")}>
          <Image /> 设置封面…
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => setAttachment(ids(), "cover", null)}>
          清除封面
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem disabled={!isTauri} onSelect={() => void reloadSidecars()}>
          <RefreshCw /> 全部重载（同名字幕与封面）
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

function OutputLine({ item }: { item: QueueItem }) {
  if (item.state !== "done") return null;
  if (item.outputs.length > 1) {
    const folder = dirName(item.outputs[0] ?? "");
    return (
      <Hint label={item.outputs.join("\n")} side="bottom" align="start">
        <p className="mt-0.5 truncate text-xs text-muted-foreground select-text">
          → {item.outputs.length} 个输出：{folder}
        </p>
      </Hint>
    );
  }
  if (!item.output) return null;
  return (
    <Hint label={item.output} side="bottom" align="start">
      <p className="mt-0.5 truncate text-xs text-muted-foreground select-text">→ {item.output}</p>
    </Hint>
  );
}

function stateText(item: QueueItem): string {
  let text = STATE_LABEL[item.state];
  if (item.state === "running") {
    text += ` ${item.percent.toFixed(0)}%`;
    if (item.rounds > 1 && item.round > 0) text += ` · 第 ${item.round}/${item.rounds} 次`;
    if (item.speed > 0) text += ` · ${item.speed.toFixed(1)}x`;
  }
  if (item.state === "done" && item.outputs.length > 1) text += ` · ${item.outputs.length} 个`;
  if ((item.state === "failed" || item.state === "cancelled") && item.outputs.length > 0) {
    text += `（已输出 ${item.outputs.length} 个）`;
  }
  return text;
}

function Row({
  item,
  index,
  selected,
  active,
  dropMark,
  onRowClick,
  onGripDown,
  onShowInfo,
}: {
  item: QueueItem;
  index: number;
  selected: boolean;
  /** 键盘焦点所在的行 */
  active: boolean;
  dropMark: "before" | "after" | null;
  onRowClick: (e: ReactMouseEvent, item: QueueItem) => void;
  onGripDown: (e: ReactPointerEvent, id: string) => void;
  onShowInfo: (item: QueueItem) => void;
}) {
  const { select, cancel } = useQueue.getState();
  const isActive = item.state === "running" || item.state === "waiting";
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <li
          id={rowDomId(item.id)}
          role="option"
          aria-selected={selected}
          data-row-index={index}
          data-active={active || undefined}
          onClick={(e) => onRowClick(e, item)}
          onContextMenu={() => {
            // 右键未选中的行时改为只选中它，菜单里的操作针对所选视频
            if (!selected) select(item.id, "single");
          }}
          onDoubleClick={() => usePreview.getState().playSource(item)}
          className={cn(
            "group relative flex items-start gap-2 py-1.5 pr-2 pl-1",
            selected
              ? "bg-primary/15 before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary"
              : "hover:bg-muted/60",
            "group-focus-visible/list:data-active:outline group-focus-visible/list:data-active:-outline-offset-1 group-focus-visible/list:data-active:outline-ring",
          )}
        >
          {dropMark && (
            <span
              className={cn("pointer-events-none absolute inset-x-0 h-0.5 bg-primary", dropMark === "before" ? "-top-px" : "-bottom-px")}
            />
          )}
          <Hint label="拖动调整顺序">
            <span
              aria-label="拖动调整顺序"
              onPointerDown={(e) => onGripDown(e, item.id)}
              onClick={(e) => e.stopPropagation()}
              className="mt-0.5 cursor-grab touch-none text-muted-foreground opacity-40 hover:opacity-100 active:cursor-grabbing"
            >
              <GripVertical className="size-4" />
            </span>
          </Hint>
          <Checkbox
            checked={selected}
            tabIndex={-1}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onCheckedChange={() => select(item.id, "toggle")}
            aria-label={`选择 ${item.name}`}
            className="mt-0.5"
          />
          <span className="mt-px w-6 text-right text-xs text-muted-foreground tabular-nums">{index + 1}</span>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <Hint label={item.path} side="bottom" align="start">
                <p className="truncate" data-path={item.path}>
                  {item.name}
                </p>
              </Hint>
              {item.subtitle && <AttachmentBadge title={`烧录字幕：${item.subtitle}`}>CC</AttachmentBadge>}
              {item.cover && <AttachmentBadge title={`嵌入封面：${item.cover}`}>FC</AttachmentBadge>}
              {!item.subtitle && item.info && item.info.subtitles.length > 0 && (
                <StreamBadge title="文件内带字幕轨">CC</StreamBadge>
              )}
              {!item.cover && item.info?.hasCover && <StreamBadge title="文件内带封面">FC</StreamBadge>}
            </div>
            <p className={cn("truncate text-xs", item.probeError ? "text-destructive select-text" : "text-muted-foreground")}>
              {mediaSummary(item)}
            </p>
            {(item.state === "running" || item.state === "done") && (
              <Progress
                value={Math.max(0, Math.min(100, item.percent))}
                className={cn(
                  "mt-1 h-1",
                  item.state === "done" && "[&>[data-slot=progress-indicator]]:bg-muted-foreground/60",
                )}
              />
            )}
            {item.state === "failed" && item.error && (
              <p className="mt-1 line-clamp-2 text-xs text-destructive select-text">{item.error}</p>
            )}
            <OutputLine item={item} />
            <ToolRowProgress queueId={item.id} path={item.path} />
          </div>
          <span className={cn("mt-px shrink-0 text-xs tabular-nums", STATE_TONE[item.state])}>{stateText(item)}</span>
          <div className="-my-0.5 flex shrink-0" onDoubleClick={(e) => e.stopPropagation()}>
            <Hint label="播放">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="播放"
                tabIndex={-1}
                onClick={(e) => {
                  e.stopPropagation();
                  usePreview.getState().playSource(item);
                }}
                className={rowButton}
              >
                <Play />
              </Button>
            </Hint>
            <Hint label="媒体信息">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="媒体信息"
                tabIndex={-1}
                onClick={(e) => {
                  e.stopPropagation();
                  onShowInfo(item);
                }}
                className={rowButton}
              >
                <Info />
              </Button>
            </Hint>
            <Hint label={isActive ? "取消" : "移除"}>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={isActive ? "取消" : "移除"}
                tabIndex={-1}
                onClick={(e) => {
                  e.stopPropagation();
                  if (isActive) void cancel(item.id);
                  else requestRemove({ kind: "ids", ids: [item.id] });
                }}
                className={rowButton}
              >
                <X />
              </Button>
            </Hint>
          </div>
        </li>
      </ContextMenuTrigger>
      <ContextMenuContent className="min-w-44">
        <ContextMenuItem onSelect={() => usePreview.getState().playSource(item)}>
          <Play /> 播放
          <ContextMenuShortcut>Enter</ContextMenuShortcut>
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => onShowInfo(item)}>
          <Info /> 媒体信息
        </ContextMenuItem>
        <QueueToolsSubmenu />
        <SidecarSubmenu />
        <ContextMenuSeparator />
        <ContextMenuItem
          variant="destructive"
          onSelect={() => {
            if (isActive) void cancel(item.id);
            else requestRemove({ kind: "ids", ids: [...useQueue.getState().selected] });
          }}
        >
          <X /> {isActive ? "取消处理" : "移除所选"}
          {!isActive && <ContextMenuShortcut>{KEYS.remove}</ContextMenuShortcut>}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** 拖动排序：按住把手拖到目标位置。窗口启用了系统拖放，HTML5 拖放不可用，所以用指针事件。 */
function useDragReorder(container: RefObject<HTMLDivElement | null>) {
  const [dragId, setDragId] = useState<string | null>(null);
  const [drop, setDrop] = useState<{ index: number; mark: "before" | "after" } | null>(null);

  const targetAt = (x: number, y: number) => {
    const row = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-row-index]");
    if (!row) return null;
    const index = Number(row.dataset.rowIndex);
    const box = row.getBoundingClientRect();
    return { index, mark: y < box.top + box.height / 2 ? ("before" as const) : ("after" as const) };
  };

  const onGripDown = (e: ReactPointerEvent, id: string) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    setDragId(id);
    const move = (ev: PointerEvent) => {
      setDrop(targetAt(ev.clientX, ev.clientY));
      // 靠近边缘时自动滚动
      const viewport = scrollViewport(container.current);
      const box = viewport?.getBoundingClientRect();
      if (viewport && box) {
        if (ev.clientY < box.top + 24) viewport.scrollBy(0, -12);
        else if (ev.clientY > box.bottom - 24) viewport.scrollBy(0, 12);
      }
    };
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const target = targetAt(ev.clientX, ev.clientY);
      const { items, moveTo } = useQueue.getState();
      const from = items.findIndex((i) => i.id === id);
      if (target && from >= 0) {
        let to = target.mark === "before" ? target.index : target.index + 1;
        if (to > from) to -= 1;
        if (to !== from) moveTo(id, to);
      }
      setDragId(null);
      setDrop(null);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return { dragId, drop, onGripDown };
}

/**
 * 键盘操作（列表获得焦点时，像系统的列表控件）：↑↓ / Home / End 移动并选中，Shift 扩展选择，Ctrl 只移动焦点；
 * 空格切换选中，Enter 播放，Ctrl+A 全选，Delete 移除所选。点击：单选；Ctrl 加选；Shift 连续选择。
 */
function useListKeyboard(list: RefObject<HTMLUListElement | null>) {
  const items = useQueue((s) => s.items);
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = items.some((i) => i.id === activeId) ? activeId : null;

  useEffect(() => {
    if (active) document.getElementById(rowDomId(active))?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const onRowClick = (e: ReactMouseEvent, item: QueueItem) => {
    const { select } = useQueue.getState();
    if (e.shiftKey) select(item.id, "range");
    else select(item.id, e.ctrlKey || e.metaKey ? "toggle" : "single");
    setActiveId(item.id);
    list.current?.focus({ preventScroll: true });
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLUListElement>) => {
    if (e.target !== e.currentTarget) return;
    const queue = useQueue.getState();
    const all = queue.items;
    if (all.length === 0) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const index = Math.max(0, all.findIndex((i) => i.id === active));
    const moveTo = (next: number) => {
      const target = all[Math.max(0, Math.min(all.length - 1, next))];
      if (!target) return;
      setActiveId(target.id);
      if (e.shiftKey) queue.select(target.id, "range");
      else if (!ctrl) queue.select(target.id, "single");
    };
    const current = all[index];
    switch (e.key) {
      case "ArrowDown":
        moveTo(active ? index + 1 : 0);
        break;
      case "ArrowUp":
        moveTo(active ? index - 1 : 0);
        break;
      case "Home":
        moveTo(0);
        break;
      case "End":
        moveTo(all.length - 1);
        break;
      case "PageDown":
        moveTo(index + 10);
        break;
      case "PageUp":
        moveTo(index - 10);
        break;
      case " ":
        if (current) {
          queue.select(current.id, "toggle");
          setActiveId(current.id);
        }
        break;
      case "Enter":
        if (current && !ctrl) usePreview.getState().playSource(current);
        else return;
        break;
      case "Delete":
        if (queue.selected.size === 0) return;
        requestRemove({ kind: "ids", ids: [...queue.selected] });
        break;
      case "a":
      case "A":
        if (!ctrl || e.shiftKey || e.altKey) return;
        queue.selectAll();
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  const onFocus = () => {
    if (active) return;
    const { items: all, selected } = useQueue.getState();
    const first = all.find((i) => selected.has(i.id)) ?? all[0];
    if (first) setActiveId(first.id);
  };

  return { active, onRowClick, onKeyDown, onFocus };
}

export default function QueueList({ hovering }: { hovering: boolean }) {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  const importing = useQueue((s) => s.importing);
  const container = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const { dragId, drop, onGripDown } = useDragReorder(container);
  const { active, onRowClick, onKeyDown, onFocus } = useListKeyboard(list);
  const [infoItem, setInfoItem] = useState<QueueItem | null>(null);

  return (
    <div
      className={cn(
        "relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-md border bg-card transition-colors",
        hovering && "border-dashed border-primary bg-muted",
        dragId && "cursor-grabbing",
      )}
    >
      {importing && (
        <div className="border-b px-3 py-1.5 text-xs text-muted-foreground">
          <div className="flex items-center gap-2">
            <Spinner className="size-3.5" aria-label="导入中" />
            {importing.total > 10 ? `正在导入 ${importing.done} / ${importing.total}` : "正在导入…"}
          </div>
          {importing.total > 10 && <Progress value={(importing.done / importing.total) * 100} className="mt-1 h-1" />}
        </div>
      )}
      {items.length === 0 ? (
        <div className="grid flex-1 place-items-center text-muted-foreground">
          <div className="flex items-center gap-3">
            <Film className="size-6 shrink-0 opacity-50" />
            <div>
              <p className="text-foreground">把视频或文件夹拖到这里</p>
              <p className="text-xs">
                {isTauri
                  ? "或用“添加视频”（Ctrl+O）、“添加文件夹”（Ctrl+Shift+O）；拖入配方文件（.json）可加载参数"
                  : "当前在浏览器中预览，导入功能需要在桌面应用中使用"}
              </p>
            </div>
          </div>
        </div>
      ) : (
        <ScrollArea ref={container} className="min-h-0 flex-1">
          <ul
            ref={list}
            role="listbox"
            aria-label="视频队列"
            aria-multiselectable
            aria-activedescendant={active ? rowDomId(active) : undefined}
            tabIndex={0}
            onKeyDown={onKeyDown}
            onFocus={onFocus}
            className="group/list divide-y outline-none"
          >
            {items.map((item, index) => (
              <Row
                key={item.id}
                item={item}
                index={index}
                selected={selected.has(item.id)}
                active={item.id === active}
                dropMark={dragId && drop?.index === index && dragId !== item.id ? drop.mark : null}
                onRowClick={onRowClick}
                onGripDown={onGripDown}
                onShowInfo={setInfoItem}
              />
            ))}
          </ul>
        </ScrollArea>
      )}
      {infoItem && (
        <MediaInfoDialog
          item={items.find((i) => i.id === infoItem.id) ?? infoItem}
          onClose={() => setInfoItem(null)}
        />
      )}
    </div>
  );
}
