import { useState } from "react";
import { ChevronDown, ChevronRight, Eye, FileDown, FileUp, RotateCcw, Terminal } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import Hint from "@/components/Hint";
import NumberInput from "@/components/NumberInput";
import { PIPELINE_DEFS, PREVIEW_EXCLUDED } from "@/features/pipeline";
import { PIPELINE_MODULES, groupBy, type Feature } from "@/features/registry";
import { isTauri, MAX_FISSION, previewCommand } from "@/lib/backend";
import { useLog } from "@/state/log";
import { usePreview } from "@/state/preview";
import { useRecipe } from "@/state/recipe";
import { getSelectedVideo, useSelectedVideo } from "@/state/selection";
import { useSettings } from "@/state/settings";

const MODULE_GROUPS = groupBy(PIPELINE_MODULES);

/** 该模块能否单独预览：有表单（已实现）且不在预览排除列表中。 */
function canPreview(id: string): boolean {
  return !!PIPELINE_DEFS[id]?.Form && !PREVIEW_EXCLUDED.includes(id);
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

function ModuleRow({ feature, open, onToggleOpen }: { feature: Feature; open: boolean; onToggleOpen: () => void }) {
  const params = useRecipe((s) => s.recipe.modules[feature.id]);
  const { setEnabled, updateModule } = useRecipe.getState();
  const def = PIPELINE_DEFS[feature.id];
  const planned = feature.status === "planned";
  const enabled = params?.enabled ?? false;
  const Form = def?.Form;

  return (
    <Collapsible open={open} onOpenChange={onToggleOpen}>
      <div className="flex items-center gap-1 rounded-md pl-2 hover:bg-muted">
        <Switch
          checked={enabled}
          disabled={planned && !enabled}
          aria-label={feature.name}
          onCheckedChange={(on) => setEnabled(feature.id, on)}
        />
        <CollapsibleTrigger asChild>
          <Button
            variant="ghost"
            // 两行内容（名称 + 摘要）：高度随内容（Mira 的按钮是固定高度 h-7）
            className="h-auto min-w-0 flex-1 justify-start text-left whitespace-normal"
          >
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                {feature.name}
                {planned && (
                  <Badge variant="secondary">
                    规划中
                  </Badge>
                )}
              </span>
              <span className="block truncate text-xs text-muted-foreground">{feature.summary}</span>
            </span>
            {open ? (
              <ChevronDown />
            ) : (
              <ChevronRight />
            )}
          </Button>
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
  );
}

/** 裂变次数与整条流水线预览。 */
function RecipeFooter() {
  const fission = useRecipe((s) => s.recipe.fission);
  const setFission = useRecipe((s) => s.setFission);
  const fissionFolder = useSettings((s) => s.settings.fissionFolder);
  const video = useSelectedVideo();

  return (
    <footer className="space-y-2 border-t px-4 py-2.5">
      <div className="flex items-center gap-2">
        <span className="text-xs text-muted-foreground">裂变次数</span>
        <NumberInput value={fission} onChange={setFission} min={1} max={MAX_FISSION} integer className="w-16" label="裂变次数" />
        <span className="min-w-0 flex-1 text-muted-foreground">
          {fission > 1
            ? `每个视频输出 ${fission} 个，放在视频同名文件夹${fissionFolder ? "的“第 x 次裂变”子文件夹" : ""}中`
            : "每个视频输出 1 个"}
        </span>
      </div>
      <Hint
        label={
          video
            ? `用当前配方预览「${video.item.name}」（变速、背景音、加头尾、帧率、码率除外）`
            : "先在队列中添加视频"
        }
        disabledTrigger
      >
        <Button
          variant="outline"
          size="sm"
          onClick={() => usePreview.getState().previewPipeline(null)}
          disabled={!isTauri || !video}
          className="w-full"
        >
          <Eye /> 生成预览
        </Button>
      </Hint>
    </footer>
  );
}

/** pipe.reset：二次确认后恢复默认参数。 */
function ResetButton() {
  const reset = useRecipe((s) => s.reset);
  return (
    <AlertDialog>
      <Hint label="恢复默认参数（仅启用帧率）">
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" className="ml-auto">
            <RotateCcw /> 重置
          </Button>
        </AlertDialogTrigger>
      </Hint>
      <AlertDialogContent size="sm">
        <AlertDialogHeader>
          <AlertDialogTitle>参数重置</AlertDialogTitle>
          <AlertDialogDescription>确定把全部参数恢复为默认值吗？当前参数会丢失（可先导出配方）。</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>取消</AlertDialogCancel>
          <AlertDialogAction onClick={reset}>确定</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** 配方面板：每个模块一个开关，点击展开参数表单。 */
export default function RecipePanel() {
  const modules = useRecipe((s) => s.recipe.modules);
  const [openId, setOpenId] = useState<string | null>("pipe.fps");
  const enabledCount = PIPELINE_MODULES.filter((m) => modules[m.id]?.enabled).length;

  const showCommand = async () => {
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
  };

  const { importFile, exportFile } = useRecipe.getState();

  return (
    <aside className="flex w-[360px] shrink-0 flex-col border-l bg-card">
      <header className="border-b px-4 py-2">
        <div className="flex items-center gap-2">
          <div>
            <h2 className="text-sm font-semibold">配方</h2>
            <p className="text-xs text-muted-foreground">
              已启用 {enabledCount} / {PIPELINE_MODULES.length} 个步骤
            </p>
          </div>
          <Hint label="在处理记录中显示选中（或第一个）视频将执行的 FFmpeg 命令">
            <Button
              variant="outline"
              size="sm"
              onClick={() => void showCommand()}
              disabled={!isTauri}
              className="ml-auto"
            >
              <Terminal /> 查看命令
            </Button>
          </Hint>
        </div>
        <div className="mt-1.5 flex gap-1.5">
          <Hint label="从配方文件导入全部参数（也可以把配方文件拖进窗口）">
            <Button variant="outline" size="sm" onClick={() => void importFile()} disabled={!isTauri}>
              <FileUp /> 导入
            </Button>
          </Hint>
          <Hint label="把当前全部参数保存为配方文件">
            <Button variant="outline" size="sm" onClick={() => void exportFile()} disabled={!isTauri}>
              <FileDown /> 导出
            </Button>
          </Hint>
          <ResetButton />
        </div>
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="px-2 py-1.5">
          {MODULE_GROUPS.map(([group, features]) => (
            <div key={group} className="mb-2">
              <h3 className="px-2 py-1 text-xs font-medium text-muted-foreground">{group}</h3>
              {features.map((feature) => (
                <ModuleRow
                  key={feature.id}
                  feature={feature}
                  open={openId === feature.id}
                  onToggleOpen={() => setOpenId(openId === feature.id ? null : feature.id)}
                />
              ))}
            </div>
          ))}
        </div>
      </ScrollArea>
      <RecipeFooter />
    </aside>
  );
}
