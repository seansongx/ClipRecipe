import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Check, ChevronDown, ChevronRight, Copy, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import Hint from "@/components/Hint";
import { useCopied } from "@/lib/clipboard";
import { formatTime } from "@/lib/format";
import { cn, scrollViewport } from "@/lib/utils";
import { copyLog } from "@/state/feedback";
import { useLog, type LogEntry } from "@/state/log";

const LEVEL_TONE = {
  debug: "text-muted-foreground",
  info: "text-foreground",
  warn: "text-warning",
  error: "text-destructive",
} as const;

const LEVEL_LABEL = { debug: "调试", info: "信息", warn: "警告", error: "错误" } as const;

/**
 * 处理记录面板：可折叠，自动滚动到最新，默认隐藏调试信息（FFmpeg 命令行）。
 * `filter` 只显示部分记录（如独立工具对话框只显示本次运行的记录）；`className` 调整外边距与高度。
 */
export default function LogPanel({
  filter,
  className,
  bodyClassName = "h-36",
}: {
  filter?: (entry: LogEntry) => boolean;
  className?: string;
  /** 记录列表的高度 */
  bodyClassName?: string;
} = {}) {
  const allEntries = useLog((s) => s.entries);
  const entries = useMemo(() => (filter ? allEntries.filter(filter) : allEntries), [allEntries, filter]);
  const clear = useLog((s) => s.clear);
  const [open, setOpen] = useState(true);
  const [showDebug, setShowDebug] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const debugId = useId();
  const [copied, markCopied] = useCopied();

  const visible = useMemo(
    () => (showDebug ? entries : entries.filter((e) => e.level !== "debug")),
    [entries, showDebug],
  );
  const problems = entries.filter((e) => e.level === "error").length;

  useEffect(() => {
    const el = scrollViewport(scroller.current);
    if (el) el.scrollTop = el.scrollHeight;
  }, [visible.length, open]);

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      render={<section className={cn("mx-3 mb-2 flex flex-col rounded-md border bg-card", className)} />}
    >
      <header className="flex items-center gap-2 px-1.5 py-0.5">
        <CollapsibleTrigger render={<Button variant="ghost" size="sm" />}>
          {open ? <ChevronDown /> : <ChevronRight />}
          处理记录
          {/* 次要文字；按钮展开 / 悬停时是 muted 底，muted 文字在上面不够 4.5:1，改为继承按钮的前景色 */}
          <span className="font-normal text-muted-foreground group-hover/button:text-inherit group-aria-expanded/button:text-inherit">
            {visible.length} 条{problems > 0 && `，${problems} 个错误`}
          </span>
        </CollapsibleTrigger>
        <div className="ml-auto flex items-center gap-1.5">
          <Checkbox id={debugId} checked={showDebug} onCheckedChange={(v) => setShowDebug(v === true)} />
          <Label htmlFor={debugId}>
            显示命令行
          </Label>
        </div>
        {/* 反馈问题时用：总是带上环境信息与命令行（不管“显示命令行”有没有勾） */}
        <Hint label={copied ? "已复制" : "复制记录（含环境信息与命令行）"}>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="复制记录"
            onClick={() => void copyLog(entries).then((ok) => ok && markCopied())}
            disabled={entries.length === 0}
          >
            {copied ? <Check /> : <Copy />}
          </Button>
        </Hint>
        <Hint label="清空记录">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="清空记录"
            onClick={clear}
            disabled={entries.length === 0}
          >
            <Trash2 />
          </Button>
        </Hint>
      </header>
      <CollapsibleContent>
        <ScrollArea ref={scroller} className={cn("border-t", bodyClassName)}>
          <div className="px-3 py-1.5 font-mono text-xs leading-5 select-text">
            {visible.length === 0 ? (
              <p className="text-muted-foreground">暂无记录</p>
            ) : (
              visible.map((entry) => (
                <div key={entry.id} className={`flex gap-2 ${LEVEL_TONE[entry.level]}`}>
                  <span className="shrink-0 text-muted-foreground tabular-nums">{formatTime(entry.time)}</span>
                  <span className="shrink-0">{LEVEL_LABEL[entry.level]}</span>
                  <span className="min-w-0 break-all whitespace-pre-wrap">{entry.message}</span>
                </div>
              ))
            )}
          </div>
        </ScrollArea>
      </CollapsibleContent>
    </Collapsible>
  );
}
