import { useCallback, useEffect, useRef, useState, type Ref } from "react";
import {
  ChevronDown,
  ChevronRight,
  Ellipsis,
  Eye,
  FileDown,
  FileUp,
  Play,
  Plus,
  RotateCcw,
  Square,
  Terminal,
} from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import Hint from "@/components/Hint";
import NumberInput from "@/components/NumberInput";
import OutputPicker from "@/components/OutputPicker";
import { useShortcutLabels } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useQueue } from "@/state/queue";
import { moduleSummary, PIPELINE_DEFS, PREVIEW_EXCLUDED } from "@/features/pipeline";
import { PIPELINE_MODULES, groupBy, type Feature } from "@/features/registry";
import { isTauri, MAX_FISSION, previewCommand, type ModuleParams } from "@/lib/backend";
import { useLog } from "@/state/log";
import { usePreview } from "@/state/preview";
import { useRecipe } from "@/state/recipe";
import { getSelectedVideo, useSelectedVideo } from "@/state/selection";
import { useSettings } from "@/state/settings";

/** 该模块能否单独预览：有表单（已实现）且不在预览排除列表中。 */
function canPreview(id: string): boolean {
  return !!PIPELINE_DEFS[id]?.Form && !PREVIEW_EXCLUDED.includes(id);
}

/**
 * 配方面板里列出的步骤：当前启用的 ∪ 本次会话里用户添加或开关过的（关掉开关后不会立刻消失，而是显示“已停用”）。
 * 整份替换配方（重置、导入：store 的 `revision` 变化）后重新按“已启用”计算，并调用 `onReplaced(新的可见步骤)`。
 */
function useVisibleSteps(
  modules: Record<string, ModuleParams>,
  revision: number,
  onReplaced: (visible: Feature[]) => void,
) {
  const [touched, setTouched] = useState<ReadonlySet<string>>(() => new Set());
  const [seenRevision, setSeenRevision] = useState(revision);
  let current = touched;
  let replaced = false;
  if (revision !== seenRevision) {
    // 渲染中调整状态（React 的“根据属性变化调整状态”写法），不用等 effect 再渲染一次
    setSeenRevision(revision);
    current = new Set();
    setTouched(current);
    replaced = true;
  }
  const visible = PIPELINE_MODULES.filter((m) => modules[m.id]?.enabled || current.has(m.id));
  if (replaced) onReplaced(visible);
  const touch = (id: string) => setTouched((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
  return { visible, touch };
}

function ModulePreviewButton({ id }: { id: string }) {
  const video = useSelectedVideo();
  return (
    <>
      <Separator className="mt-2" />
      <div className="flex items-center gap-2 pt-2">
        <Hint label={video ? `只启用此步骤，预览「${video.item.name}」` : "先在队列中添加视频"} disabledTrigger>
          <Button
            variant="outline"
            size="sm"
            onClick={() => usePreview.getState().previewPipeline(id)}
            disabled={!isTauri || !video}
          >
            <Eye /> 预览此步骤
          </Button>
        </Hint>
        {video && <span className="min-w-0 truncate text-muted-foreground">{video.item.name}</span>}
      </div>
    </>
  );
}

function ModuleRow({
  feature,
  open,
  onToggleOpen,
  onToggleEnabled,
  scrollIntoView,
  onScrolled,
}: {
  feature: Feature;
  open: boolean;
  onToggleOpen: () => void;
  onToggleEnabled: (on: boolean) => void;
  /** 刚从“添加步骤”加入：渲染后滚动到可见，然后调用 onScrolled */
  scrollIntoView: boolean;
  onScrolled: () => void;
}) {
  const params = useRecipe((s) => s.recipe.modules[feature.id]);
  const updateModule = useRecipe((s) => s.updateModule);
  const def = PIPELINE_DEFS[feature.id];
  const planned = feature.status === "planned";
  const enabled = params?.enabled ?? false;
  const Form = def?.Form;
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!scrollIntoView) return;
    // 等展开的表单排好版再滚动；nearest：整行放得下时整行可见，放不下时标题行对齐顶端
    const frame = requestAnimationFrame(() => {
      box.current?.scrollIntoView({ block: "nearest" });
      onScrolled();
    });
    return () => cancelAnimationFrame(frame);
  }, [scrollIntoView, onScrolled]);

  return (
    // 展开的步骤用 primary 描边标出（不只靠文字颜色）
    <div ref={box} className={cn("rounded-lg border", open ? "border-primary" : "border-transparent")}>
      <Collapsible open={open} onOpenChange={onToggleOpen}>
        <div className="flex items-center gap-1 rounded-md pl-2 hover:bg-muted">
          <Switch
            checked={enabled}
            disabled={planned && !enabled}
            aria-label={feature.name}
            onCheckedChange={onToggleEnabled}
          />
          <CollapsibleTrigger
            render={
              <Button
                variant="ghost"
                // 两行内容（名称 + 摘要）：高度随内容（Mira 的按钮是固定高度 h-7）
                className="h-auto min-w-0 flex-1 justify-start text-left whitespace-normal"
              />
            }
          >
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                {feature.name}
                {planned && <Badge variant="secondary">规划中</Badge>}
              </span>
              {/* 摘要是次要文字；展开 / 悬停时按钮是 muted 底，muted 文字在上面不够 4.5:1，改为继承按钮的前景色 */}
              <span className="block truncate text-xs text-muted-foreground group-hover/button:text-inherit group-aria-expanded/button:text-inherit">
                {enabled ? moduleSummary(feature.id, params) : "已停用"}
              </span>
            </span>
            {open ? <ChevronDown /> : <ChevronRight />}
          </CollapsibleTrigger>
        </div>
        <CollapsibleContent className="px-3 pb-3">
          {Form && params ? (
            <Form value={params} onChange={(patch) => updateModule(feature.id, patch)} />
          ) : (
            <p className="py-1 text-xs text-muted-foreground">参数表单尚未实现。</p>
          )}
          {canPreview(feature.id) && <ModulePreviewButton id={feature.id} />}
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}

/** 列表末尾的“添加步骤”：按 registry 的分组列出还没显示的模块。 */
function AddStepMenu({ hidden, onAdd }: { hidden: Feature[]; onAdd: (id: string) => void }) {
  const groups = groupBy(hidden);
  return (
    <div className="flex items-center gap-2 px-1 pt-1.5">
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button variant="outline" size="sm" />}>
          <Plus /> 添加步骤
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-auto min-w-56">
          {groups.map(([group, features], i) => (
            <DropdownMenuGroup key={group}>
              {i > 0 && <DropdownMenuSeparator />}
              <DropdownMenuLabel>{group}</DropdownMenuLabel>
              {features.map((feature) => (
                <DropdownMenuItem
                  key={feature.id}
                  disabled={feature.status === "planned"}
                  onClick={() => onAdd(feature.id)}
                >
                  <span className="flex min-w-0 flex-col">
                    <span>{feature.name}</span>
                    <span className="text-muted-foreground">{feature.summary}</span>
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <span className="text-xs text-muted-foreground">还有 {hidden.length} 个</span>
    </div>
  );
}

interface OutputDropProps {
  /** 输出位置输入框的容器：窗口级拖放据此判断是否落在输出位置上（views/batch/useWindowDrop.ts） */
  outputBox: Ref<HTMLDivElement>;
  outputDropTarget: boolean;
}

/** 裂变次数、输出位置与开始 / 停止。 */
function RecipeFooter({ outputBox, outputDropTarget }: OutputDropProps) {
  const fission = useRecipe((s) => s.recipe.fission);
  const setFission = useRecipe((s) => s.setFission);
  const fissionFolder = useSettings((s) => s.settings.fissionFolder);
  const items = useQueue((s) => s.items);
  const running = useQueue((s) => s.running);
  const importing = useQueue((s) => s.importing);
  const { start, stop } = useQueue.getState();
  const keys = useShortcutLabels();

  return (
    // Hint 的 disabledTrigger 把按钮包在一层 inline-flex 的 span 里（只有文字那么宽）：在底部改为块级 flex，
    // 让“开始处理”和“停止”一样占满整行
    <footer className="space-y-2 border-t px-4 py-2.5 *:data-[slot=hint-wrapper]:flex">
      <div className="flex items-center gap-2">
        <span className="text-xs text-muted-foreground">裂变次数</span>
        <NumberInput value={fission} onChange={setFission} min={1} max={MAX_FISSION} integer className="w-16" label="裂变次数" />
        <span className="min-w-0 flex-1 text-muted-foreground">
          {fission > 1
            ? `每个视频输出 ${fission} 个，放在视频同名文件夹${fissionFolder ? "的“第 x 次裂变”子文件夹" : ""}中`
            : "每个视频输出 1 个"}
        </span>
      </div>
      <OutputPicker boxRef={outputBox} dropTarget={outputDropTarget} />
      {items.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {items.length} 个视频 → {items.length * fission} 个输出
        </p>
      )}
      {running ? (
        <Hint label="停止处理" keys={keys.stop}>
          <Button variant="outline" onClick={() => void stop()} className="w-full">
            <Square /> 停止
          </Button>
        </Hint>
      ) : (
        <Hint label="按当前配方处理队列" keys={keys.start} disabledTrigger>
          <Button
            onClick={() => void start(useRecipe.getState().recipe)}
            disabled={!isTauri || items.length === 0 || !!importing}
            className="w-full"
          >
            <Play /> 开始处理
          </Button>
        </Hint>
      )}
    </footer>
  );
}

/** 在处理记录里显示选中（或第一个）视频将执行的 FFmpeg 命令。 */
async function showCommand() {
  const log = useLog.getState();
  const video = getSelectedVideo();
  if (!video) {
    log.add("warn", "队列为空，无法生成命令");
    return;
  }
  const { settings } = useSettings.getState();
  try {
    const preview = await previewCommand({
      path: video.item.path,
      info: video.item.info,
      recipe: useRecipe.getState().recipe,
      settings,
      outputDir: settings.outputDir,
      number: video.number,
    });
    log.add("info", `「${video.item.name}」的处理命令（编码器 ${preview.encoder}）：\n${preview.commandLine}`);
    for (const warning of preview.warnings) log.add("warn", warning);
  } catch (e) {
    log.add("error", `生成命令失败：${String(e)}`);
  }
}

/** 标题区：导入、导出，“更多”里是查看命令与重置（pipe.reset，二次确认）。 */
function RecipeHeader({ enabledCount }: { enabledCount: number }) {
  const { importFile, exportFile, reset } = useRecipe.getState();
  // 受控：shadcn（Base UI）的 AlertDialogAction 只是按钮，不会自己关闭对话框
  const [resetOpen, setResetOpen] = useState(false);

  return (
    <header className="flex items-center gap-1 border-b px-4 py-2">
      <div className="mr-auto">
        <h2 className="text-sm font-semibold">配方</h2>
        <p className="text-xs text-muted-foreground">已启用 {enabledCount} 个步骤</p>
      </div>
      <Hint label="从配方文件导入全部参数（也可以把配方文件拖进窗口）" disabledTrigger>
        <Button variant="ghost" size="icon-sm" aria-label="导入配方" onClick={() => void importFile()} disabled={!isTauri}>
          <FileUp />
        </Button>
      </Hint>
      <Hint label="把当前全部参数保存为配方文件" disabledTrigger>
        <Button variant="ghost" size="icon-sm" aria-label="导出配方" onClick={() => void exportFile()} disabled={!isTauri}>
          <FileDown />
        </Button>
      </Hint>
      <DropdownMenu>
        <Hint label="更多">
          <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="更多" />}>
            <Ellipsis />
          </DropdownMenuTrigger>
        </Hint>
        <DropdownMenuContent align="end" className="w-auto min-w-40">
          <DropdownMenuItem onClick={() => void showCommand()} disabled={!isTauri}>
            <Terminal /> 查看命令
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setResetOpen(true)}>
            <RotateCcw /> 重置…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <AlertDialog open={resetOpen} onOpenChange={setResetOpen}>
        <AlertDialogContent size="sm">
          <AlertDialogHeader>
            <AlertDialogTitle>参数重置</AlertDialogTitle>
            <AlertDialogDescription>确定把全部参数恢复为默认值吗？当前参数会丢失（可先导出配方）。</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                reset();
                setResetOpen(false);
              }}
            >
              确定
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </header>
  );
}

/** 配方面板：只列出用到的步骤（开关 + 名称 + 当前取值），点击展开参数表单；末尾“添加步骤”。 */
export default function RecipePanel({ outputBox, outputDropTarget }: OutputDropProps) {
  const modules = useRecipe((s) => s.recipe.modules);
  const setEnabled = useRecipe((s) => s.setEnabled);
  const [openId, setOpenId] = useState<string | null>("pipe.fps");
  const [scrollId, setScrollId] = useState<string | null>(null);
  const revision = useRecipe((s) => s.revision);
  const { visible, touch } = useVisibleSteps(modules, revision, (next) => {
    // 原先展开的步骤不再列出时收起，免得它以后再出现时自动展开
    if (openId && !next.some((m) => m.id === openId)) setOpenId(null);
  });
  const enabledCount = PIPELINE_MODULES.filter((m) => modules[m.id]?.enabled).length;
  const hidden = PIPELINE_MODULES.filter((m) => !visible.includes(m));

  const clearScroll = useCallback(() => setScrollId(null), []);

  const add = (id: string) => {
    setEnabled(id, true);
    touch(id);
    setOpenId(id);
    setScrollId(id);
  };

  return (
    <aside className="flex w-[380px] shrink-0 flex-col border-l bg-card">
      <RecipeHeader enabledCount={enabledCount} />
      <ScrollArea className="min-h-0 flex-1">
        <div className="space-y-0.5 px-2 py-1.5">
          {visible.length === 0 && (
            <p className="px-2 py-1 text-xs text-muted-foreground">还没有启用任何步骤。</p>
          )}
          {visible.map((feature) => (
            <ModuleRow
              key={feature.id}
              feature={feature}
              open={openId === feature.id}
              onToggleOpen={() => setOpenId(openId === feature.id ? null : feature.id)}
              onToggleEnabled={(on) => {
                touch(feature.id);
                setEnabled(feature.id, on);
              }}
              scrollIntoView={scrollId === feature.id}
              onScrolled={clearScroll}
            />
          ))}
          {hidden.length > 0 && <AddStepMenu hidden={hidden} onAdd={add} />}
        </div>
      </ScrollArea>
      <RecipeFooter outputBox={outputBox} outputDropTarget={outputDropTarget} />
    </aside>
  );
}
