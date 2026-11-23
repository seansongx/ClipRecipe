// 独立工具对话框：左侧参数表单，右侧处理对象（队列视频 / 文件夹）、输出位置、逐项进度与处理记录。
// 从工具箱或批量队列（工具栏“工具”菜单、行右键菜单）打开；关闭对话框不会中断运行。
//
// 结构：ToolDialog（外壳：标题与说明，不依赖工具定义）→ ToolDialogBody（按需加载工具定义，加载中 / 失败的状态）
// → LoadedToolBody（拿到定义 def 与参数 value 之后的全部内容：参数表单、处理对象、输出位置、进度、底栏）。
// 需要工具定义的功能（如预览：def.preview 为 true 时底栏有“预览”按钮）加在 LoadedToolBody 里。
// 文件夹可以拖到“处理对象”的文件夹输入框和输出位置上；对话框打开时主窗口不会把拖入的文件导入队列。

import { useCallback, useMemo } from "react";
import { CircleAlert, Eye, Play, RotateCcw, Square, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import ChoiceGroup from "@/components/ChoiceGroup";
import FolderInput from "@/components/FolderInput";
import Hint from "@/components/Hint";
import OutputPicker from "@/components/OutputPicker";
import { useFolderDrop } from "@/components/useFolderDrop";
import type { BitrateParams } from "@/features/pipeline/bitrate";
import { TOOLS, type Feature } from "@/features/registry";
import { toolDefaults, useToolDef } from "@/features/tools";
import type { ToolDef } from "@/features/tools/types";
import { isTauri, type Range } from "@/lib/backend";
import { dirName, fileName } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { LogEntry } from "@/state/log";
import { useQueue } from "@/state/queue";
import { useRecipe } from "@/state/recipe";
import { usePreview } from "@/state/preview";
import { useSettings } from "@/state/settings";
import { isToolReady, useTools, type ToolRunItem, type VideoScope } from "@/state/tools";
import LogPanel from "../batch/LogPanel";
import { TOOL_STATE_TONE, toolItemPercent, toolItemStatus } from "./itemStatus";

/** 拖放目标“输出位置”的键；文件夹输入的键为 `folder:<键>`。 */
const OUTPUT_DROP = "output";
const folderDrop = (key: string) => `folder:${key}`;

function rangeText(range: Range | undefined, unit: string): string {
  if (!range) return "";
  return range.min === range.max ? `${range.min}${unit}` : `${range.min}～${range.max}${unit}`;
}

/** 工具沿用批量处理的码率设置（pipe.bitrate），这里说明当前是什么。 */
function useBitrateNote(): string {
  const bitrate = useRecipe((s) => s.recipe.modules["pipe.bitrate"]) as Partial<BitrateParams> | undefined;
  if (!bitrate?.enabled) return "沿用源视频码率，过低时提高到保证画质的下限";
  return bitrate.mode === "fixed" ? `固定 ${rangeText(bitrate.kbps, " kbps")}` : `源码率 × ${rangeText(bitrate.multiplier, "")}`;
}

function SectionTitle({ children }: { children: string }) {
  return <h3 className="mb-1.5 text-xs font-medium text-muted-foreground">{children}</h3>;
}

/** 处理对象：队列视频（选中 / 全部）与工具声明的文件夹。 */
function Inputs({
  def,
  dropBox,
  dropHover,
}: {
  def: ToolDef;
  dropBox: ReturnType<typeof useFolderDrop>["dropBox"];
  dropHover: string | null;
}) {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  const scope = useTools((s) => s.scope);
  const folders = useTools((s) => s.folders[def.id]);
  const { setScope, setFolder } = useTools.getState();
  const selectedCount = items.filter((i) => selected.has(i.id)).length;
  return (
    <div className="space-y-2">
      <SectionTitle>处理对象</SectionTitle>
      {def.inputs.videos && (
        <div className="flex flex-wrap items-center gap-2">
          <ChoiceGroup<VideoScope>
            label="处理范围"
            value={scope}
            onChange={setScope}
            options={[
              { value: "selected", label: `队列中选中的 ${selectedCount} 个` },
              { value: "all", label: `队列中全部 ${items.length} 个` },
            ]}
          />
          {(scope === "selected" ? selectedCount : items.length) === 0 && (
            <span className="flex items-center gap-1 text-[11px] text-warning">
              <CircleAlert className="size-3" /> {items.length === 0 ? "队列为空，请先在“批量处理”中添加视频" : "没有选中的视频"}
            </span>
          )}
        </div>
      )}
      {def.inputs.folders.map((slot) => (
        <FolderInput
          key={slot.key}
          label={slot.label}
          value={folders?.[slot.key] ?? ""}
          onChange={(path) => setFolder(def.id, slot.key, path)}
          placeholder="选择、粘贴或拖入文件夹"
          boxRef={dropBox(folderDrop(slot.key))}
          dropTarget={dropHover === folderDrop(slot.key)}
        />
      ))}
    </div>
  );
}

function ItemRow({ item, active }: { item: ToolRunItem; active: boolean }) {
  const cancelItem = useTools((s) => s.cancelItem);
  const running = item.state === "running" || item.state === "waiting";
  return (
    <li className="group flex items-start gap-2 px-3 py-2">
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs" title={item.inputs.join("\n")}>
          {item.label}
        </p>
        {(item.state === "running" || item.state === "done") && (
          <Progress
            value={toolItemPercent(item)}
            className={cn("mt-1 h-1", item.state === "done" && "[&>[data-slot=progress-indicator]]:bg-muted-foreground/60")}
          />
        )}
        {item.state === "failed" && item.error && (
          <p className="mt-1 line-clamp-2 text-[11px] text-destructive select-text">{item.error}</p>
        )}
        {item.state === "done" && item.outputs.length > 0 && (
          <p className="mt-1 truncate text-[11px] text-muted-foreground select-text" title={item.outputs.join("\n")}>
            →{" "}
            {item.outputs.length === 1
              ? item.outputs[0]
              : `${dirName(item.outputs[0] ?? "")}（${item.outputs.map(fileName).join("、")}）`}
          </p>
        )}
      </div>
      <span className={cn("shrink-0 text-[11px] tabular-nums", TOOL_STATE_TONE[item.state])}>{toolItemStatus(item)}</span>
      {active && running && (
        <Hint label="取消此项">
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={`取消 ${item.label}`}
            onClick={() => void cancelItem(item.id)}
            className="-my-0.5 size-6 text-muted-foreground"
          >
            <X />
          </Button>
        </Hint>
      )}
    </li>
  );
}

/** 拿到工具定义后的对话框内容。 */
function LoadedToolBody({ def }: { def: ToolDef }) {
  const id = def.id;
  const run = useTools((s) => s.run);
  const params = useTools((s) => s.params[id]);
  const { setParams, resetParams, setFolder, start, stop } = useTools.getState();
  const bitrateNote = useBitrateNote();
  const { hover: dropHover, dropBox } = useFolderDrop((key, folder) => {
    if (key === OUTPUT_DROP) useSettings.getState().update({ outputDir: folder });
    else setFolder(id, key.slice(folderDrop("").length), folder);
  });

  const ready = isToolReady(id);
  const thisRun = run && run.toolId === id ? run : null;
  const otherRunning = !!run?.running && run.toolId !== id;
  const otherName = otherRunning ? TOOLS.find((t) => t.id === run?.toolId)?.name : null;
  const logFrom = thisRun?.logFrom;
  const logFilter = useCallback((entry: LogEntry) => logFrom !== undefined && entry.id > logFrom, [logFrom]);
  const value = useMemo(() => params ?? toolDefaults(id), [params, id]);
  const Form = def.Form;

  return (
    <>
      <div className="flex min-h-0 flex-1">
        <section className="flex w-[300px] shrink-0 flex-col border-r">
          <header className="flex items-center gap-2 px-4 pt-3 pb-1">
            <SectionTitle>参数</SectionTitle>
            {Form && (
              <Button
                variant="ghost"
                size="xs"
                onClick={() => resetParams(id)}
                disabled={thisRun?.running}
                className="ml-auto h-6 px-1.5 text-[11px] font-normal text-muted-foreground"
              >
                <RotateCcw /> 恢复默认
              </Button>
            )}
          </header>
          <ScrollArea className="min-h-0 flex-1">
            <div className="px-4 pb-4">
              {Form ? (
                <Form value={value} onChange={(patch) => setParams(id, patch)} />
              ) : (
                <p className="py-6 text-center text-xs text-muted-foreground">该工具尚未实现，敬请期待。</p>
              )}
            </div>
          </ScrollArea>
        </section>

        <section className="flex min-w-0 flex-1 flex-col">
          <div className="space-y-3 border-b px-4 py-3">
            <Inputs def={def} dropBox={dropBox} dropHover={dropHover} />
            <div>
              <SectionTitle>输出位置</SectionTitle>
              <OutputPicker boxRef={dropBox(OUTPUT_DROP)} dropTarget={dropHover === OUTPUT_DROP} />
              <p className="mt-1 text-[11px] text-muted-foreground">
                与批量处理共用输出位置；视频码率沿用批量处理的“码率”设置（当前：{bitrateNote}）。
              </p>
            </div>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            {thisRun && thisRun.items.length > 0 ? (
              <ul className="divide-y">
                {thisRun.items.map((item) => (
                  <ItemRow key={item.id} item={item} active={thisRun.running} />
                ))}
              </ul>
            ) : (
              <p className="py-10 text-center text-xs text-muted-foreground">
                {thisRun?.running ? "正在准备…" : "点击“开始”后在这里显示每一项的进度与结果"}
              </p>
            )}
          </ScrollArea>
          <LogPanel filter={logFilter} className="mx-4 mt-2 mb-3" bodyClassName="h-28" />
        </section>
      </div>

      <DialogFooter className="items-center border-t px-5 py-3 sm:justify-between">
        <span className="text-xs text-muted-foreground tabular-nums">
          {otherRunning
            ? `“${otherName ?? ""}”正在运行，结束后才能开始新的工具`
            : thisRun?.summary
              ? `上次：完成 ${thisRun.summary.done} · 失败 ${thisRun.summary.failed} · 取消 ${thisRun.summary.cancelled}`
              : ""}
        </span>
        <div className="flex items-center gap-2">
          {def.preview && def.inputs.videos && (
            <Hint label="用当前参数处理当前视频的几秒低清片段，在预览窗口播放">
              <Button
                variant="outline"
                onClick={() => usePreview.getState().previewTool(id, value)}
                disabled={!isTauri || !ready}
                className="px-4"
              >
                <Eye /> 预览
              </Button>
            </Hint>
          )}
          <RunButton
            running={!!thisRun?.running}
            onStart={() => void start()}
            onStop={() => void stop()}
            disabled={!isTauri || !ready || otherRunning}
            disabledReason={!isTauri ? "需要在桌面应用中使用" : !ready ? "该工具尚未实现" : undefined}
          />
        </div>
      </DialogFooter>
    </>
  );
}

function RunButton({
  running,
  onStart,
  onStop,
  disabled,
  disabledReason,
}: {
  running: boolean;
  onStart: () => void;
  onStop: () => void;
  disabled: boolean;
  disabledReason: string | undefined;
}) {
  return running ? (
    <Button variant="outline" onClick={onStop} className="px-4">
      <Square /> 停止
    </Button>
  ) : (
    <Button onClick={onStart} disabled={disabled} title={disabledReason} className="px-4">
      <Play /> 开始
    </Button>
  );
}

/** 按需加载工具定义；加载中显示转圈，失败时可重试。 */
function ToolDialogBody({ id }: { id: string }) {
  const { def, error, retry } = useToolDef(id);
  if (def) return <LoadedToolBody def={def} />;
  return (
    <div className="grid flex-1 place-items-center text-xs text-muted-foreground">
      {error ? (
        <div className="space-y-2 text-center">
          <p className="text-destructive">无法加载该工具：{error}</p>
          <Button variant="outline" size="sm" onClick={retry}>
            重试
          </Button>
        </div>
      ) : (
        <span className="flex items-center gap-2">
          <Spinner className="size-4" /> 正在加载…
        </span>
      )}
    </div>
  );
}

function ToolHeader({ feature }: { feature: Feature }) {
  return (
    <DialogHeader className="border-b px-5 py-3 pr-12">
      <DialogTitle className="flex items-center gap-2 text-sm">
        {feature.name}
        <Badge variant="secondary" className="font-normal text-muted-foreground">
          {feature.group}
        </Badge>
        {!isToolReady(feature.id) && (
          <Badge variant="outline" className="font-normal text-muted-foreground">
            规划中
          </Badge>
        )}
      </DialogTitle>
      <DialogDescription className="text-xs">{feature.summary}</DialogDescription>
    </DialogHeader>
  );
}

export default function ToolDialog() {
  const openId = useTools((s) => s.openId);
  const close = useTools((s) => s.close);
  const feature = TOOLS.find((t) => t.id === openId);
  if (!openId || !feature) return null;

  return (
    <Dialog open onOpenChange={(open) => !open && close()}>
      <DialogContent className="flex h-[min(88vh,760px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-4xl">
        <ToolHeader feature={feature} />
        <ToolDialogBody key={openId} id={openId} />
      </DialogContent>
    </Dialog>
  );
}
