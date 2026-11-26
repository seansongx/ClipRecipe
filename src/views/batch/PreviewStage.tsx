import { useEffect, type ReactNode } from "react";
import { Eye, RefreshCw, Shuffle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Slider } from "@/components/ui/slider";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import Hint from "@/components/Hint";
import { useVideoFrame } from "@/components/useVideoFrame";
import { isTauri } from "@/lib/backend";
import { formatFps } from "@/lib/format";
import { cn } from "@/lib/utils";
import { stepName, usePreview } from "@/state/preview";
import { useQueue, type QueueItem } from "@/state/queue";
import { useRecipe } from "@/state/recipe";
import { useSelectedVideo } from "@/state/selection";
import { useSettings } from "@/state/settings";
import { stageKey, useStage, variantCount, type StageView } from "@/state/stage";

/** 自动刷新的防抖（毫秒）。 */
const DEBOUNCE_MS = 400;

const VIEWS: { value: StageView; label: string }[] = [
  { value: "source", label: "原片" },
  { value: "effect", label: "效果" },
];

/** 秒 → m:ss（超过一小时为 h:mm:ss）。 */
function clock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** 源规格：分辨率 · 帧率 · 时长。 */
function sourceSpec(item: QueueItem): string {
  const info = item.info;
  if (!info) return item.probeError ?? "未读取媒体信息";
  const v = info.video;
  const parts: string[] = [];
  if (v) {
    const [w, h] = v.rotation % 180 === 90 ? [v.height, v.width] : [v.width, v.height];
    parts.push(`${w}×${h}`, formatFps(v.fps));
  }
  if (info.duration > 0) parts.push(clock(info.duration));
  return parts.filter(Boolean).join(" · ");
}

/** 换视频后的默认取帧时间：开头往后一点（避开片头黑场），最多 10 秒。 */
const defaultTime = (duration: number) => (duration > 0 ? Math.min(duration / 10, 10) : 0);

/**
 * 批量处理页中栏：当前视频套用当前配方后的一帧（参数变化后自动刷新），可切换看原片同一时刻的画面；
 * 裂变时旁边列出几个不同随机值的变体。渲染与文件管理见 state/stage.ts。
 */
export default function PreviewStage() {
  const selected = useSelectedVideo();
  const item = selected?.item ?? null;
  const recipe = useRecipe((s) => s.recipe);
  const settings = useSettings((s) => s.settings);
  const modalOpen = usePreview((s) => s.open);
  const running = useQueue((s) => s.running);

  const view = useStage((s) => s.view);
  const time = useStage((s) => s.time);
  const timeFor = useStage((s) => s.timeFor);
  const seeds = useStage((s) => s.seeds);
  const variant = useStage((s) => s.variant);
  const renderedKey = useStage((s) => s.key);
  const interrupted = useStage((s) => s.interrupted);
  const loading = useStage((s) => s.loading);
  const image = useStage((s) => s.image);
  const result = useStage((s) => s.result);
  const error = useStage((s) => s.error);

  const duration = item?.info?.duration ?? 0;
  const count = variantCount(recipe.fission);
  const usable = isTauri && item !== null;

  // 换视频（或刚读到时长）时把取帧时间放回默认位置
  const timeOwner = item ? `${item.id}:${duration > 0}` : null;
  useEffect(() => {
    if (timeOwner && timeOwner !== timeFor) useStage.getState().setTime(defaultTime(duration), timeOwner);
  }, [timeOwner, timeFor, duration]);

  const wanted = stageKey(
    selected && { path: selected.item.path, subtitle: selected.item.subtitle, number: selected.number },
    recipe,
    settings,
    time,
    seeds,
    variant,
  );
  const ready = timeOwner !== null && timeOwner === timeFor;
  // 预览窗口打开时不渲染（后端只有一个预览槽，会取消窗口里的片段）；批量处理中暂停自动刷新（只留手动刷新）
  const autoPaused = running;
  const stale = wanted !== renderedKey || interrupted;

  useEffect(() => {
    if (!isTauri || !wanted || !ready || modalOpen || autoPaused || !stale) return;
    const timer = setTimeout(() => void useStage.getState().render(), DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [wanted, ready, modalOpen, autoPaused, stale]);

  // 队列清空时删除画面；卸载时删除全部文件
  useEffect(() => {
    if (!item) useStage.getState().dispose();
  }, [item]);
  useEffect(() => () => useStage.getState().dispose(), []);

  const source = useVideoFrame(isTauri && view === "source" ? item : null, time);

  const refreshing = view === "effect" ? loading || (stale && !autoPaused) : source.loading;

  let body: ReactNode = null;
  if (!isTauri) {
    body = <Message>预览需要在桌面应用中使用</Message>;
  } else if (!item) {
    body = <Message>队列为空，添加视频后在这里查看效果</Message>;
  } else if (view === "source") {
    if (source.error) body = <Message error>{source.error}</Message>;
    else if (source.frame) body = <Frame src={source.frame.url} alt={`「${item.name}」原片画面`} />;
  } else if (error) {
    body = <Message error>{error}</Message>;
  } else if (image) {
    body = <Frame src={image.url} alt={`「${item.name}」套用配方后的画面`} />;
  } else if (autoPaused && !loading) {
    body = <Message>批量处理中，自动刷新已暂停，可点刷新按钮手动生成画面</Message>;
  }

  return (
    <section aria-label="效果预览" className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <header className="flex h-11 shrink-0 items-center gap-2 border-b px-3">
        <div className="flex min-w-0 flex-1 items-baseline gap-2">
          <span className="truncate font-medium">{item?.name ?? "效果预览"}</span>
          {item && <span className="min-w-0 shrink truncate text-muted-foreground">{sourceSpec(item)}</span>}
        </div>
        <ToggleGroup
          variant="outline"
          spacing={0}
          value={[view]}
          disabled={!usable}
          aria-label="显示原片或效果"
          onValueChange={(next) => {
            const chosen = VIEWS.find((v) => v.value === next[0]);
            if (chosen) useStage.getState().setView(chosen.value);
          }}
        >
          {VIEWS.map((v) => (
            <ToggleGroupItem key={v.value} value={v.value}>
              {v.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <Hint
          label={
            !isTauri
              ? "预览需要在桌面应用中使用"
              : item
                ? `用当前配方预览「${item.name}」（变速、背景音、加头尾、帧率、码率除外）`
                : "先在队列中添加视频"
          }
          disabledTrigger
        >
          <Button
            variant="outline"
            size="sm"
            disabled={!usable}
            onClick={() => usePreview.getState().previewPipeline(null)}
          >
            <Eye /> 生成预览
          </Button>
        </Hint>
        <Hint label={running ? "批量处理中自动刷新已暂停，点此刷新效果画面" : "刷新效果画面"} disabledTrigger>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="刷新"
            disabled={!usable}
            onClick={() => void useStage.getState().render()}
          >
            <RefreshCw />
          </Button>
        </Hint>
      </header>

      <div className="flex min-h-0 flex-1 gap-3 p-3">
        <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
          <div className="relative min-h-0 flex-1 overflow-hidden rounded-md bg-media text-media-foreground">
            {body}
            {usable && refreshing && (
              <Spinner aria-label="正在刷新" className="absolute top-2 right-2" />
            )}
          </div>

          <div className="flex shrink-0 items-center gap-3">
            <Slider
              aria-label="取帧时间"
              min={0}
              max={Math.max(duration, 0.1)}
              step={0.1}
              value={[Math.min(time, duration)]}
              disabled={!usable || duration <= 0}
              onValueChange={(next) => {
                const t = (Array.isArray(next) ? next[0] : next) ?? 0;
                useStage.getState().setTime(t);
              }}
              className="flex-1"
            />
            <span className="shrink-0 text-muted-foreground">
              {clock(item ? time : 0)} / {duration > 0 ? clock(duration) : "--:--"}
            </span>
          </div>

          <div className="shrink-0 space-y-0.5 text-muted-foreground">
            <p>
              {view === "effect" && result
                ? `参与预览：${result.modules.length > 0 ? result.modules.map(stepName).join("、") : "无"}。`
                : null}
              变速、背景音、加头尾、帧率、码率不参与预览。
              {running ? "批量处理中，自动刷新已暂停。" : null}
            </p>
            {view === "effect" &&
              result?.warnings.map((w, i) => (
                <p key={i} className="text-warning select-text">
                  {w}
                </p>
              ))}
          </div>
        </div>

        {usable && count > 0 && <Variants count={count} />}
      </div>
    </section>
  );
}

/** 裂变变体：每个用不同的随机值渲染同一帧，点选后主画面改用它的随机值。 */
function Variants({ count }: { count: number }) {
  const variant = useStage((s) => s.variant);
  const thumbs = useStage((s) => s.thumbs);
  const thumbsLoading = useStage((s) => s.thumbsLoading);
  const result = useStage((s) => s.result);
  const selected = Math.min(variant, count - 1);
  const aspect = result && result.width > 0 && result.height > 0 ? result.width / result.height : 16 / 9;
  const pick = (i: number) => useStage.getState().pickVariant(i);

  return (
    <aside aria-label="裂变变体" className="flex min-h-0 w-36 shrink-0 flex-col gap-2">
      <span className="shrink-0 font-medium">裂变变体</span>
      <ScrollArea className="min-h-0 flex-1">
        <ToggleGroup
          variant="outline"
          orientation="vertical"
          spacing={2}
          value={[String(selected)]}
          aria-label="选择变体"
          onValueChange={(next) => {
            const i = Number(next[0]);
            if (Number.isInteger(i)) pick(i);
          }}
          className="w-full"
        >
          {Array.from({ length: count }, (_, i) => {
            const thumb = thumbs[i];
            return (
              <div key={i} className="flex flex-col gap-1">
                <div
                  className={cn(
                    "relative overflow-hidden rounded-md bg-media text-media-foreground",
                    i === selected && "ring-2 ring-ring",
                  )}
                  style={{ aspectRatio: aspect }}
                  onClick={() => pick(i)}
                >
                  {thumb?.image ? (
                    <Frame src={thumb.image.url} alt={`变体 ${i + 1} 的画面`} />
                  ) : thumb?.error ? (
                    <Hint label={<span className="select-text">{thumb.error}</span>}>
                      <span className="absolute inset-0 grid place-items-center p-1 text-center">渲染失败</span>
                    </Hint>
                  ) : null}
                  {thumbsLoading && !thumb?.image && !thumb?.error && (
                    <Spinner aria-label="正在渲染" className="absolute top-1 right-1" />
                  )}
                </div>
                <ToggleGroupItem value={String(i)} className="w-full">
                  变体 {i + 1}
                </ToggleGroupItem>
              </div>
            );
          })}
        </ToggleGroup>
      </ScrollArea>
      <Button variant="outline" size="sm" className="shrink-0" onClick={() => useStage.getState().shuffle()}>
        <Shuffle /> 换一组随机值
      </Button>
    </aside>
  );
}

/** 画面区里的图片：等比缩放、不超出区域。 */
function Frame({ src, alt }: { src: string; alt: string }) {
  return <img src={src} alt={alt} draggable={false} className="absolute inset-0 size-full object-contain" />;
}

/** 画面区里的一行说明 / 错误。 */
function Message({ children, error }: { children: ReactNode; error?: boolean }) {
  return (
    <p
      role={error ? "alert" : undefined}
      className={cn("absolute inset-0 grid place-items-center p-4 text-center", error && "select-text")}
    >
      {children}
    </p>
  );
}
