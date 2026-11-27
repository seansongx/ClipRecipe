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
    <section aria-label="效果预览" className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
      <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-2.5 gap-y-1 px-4 py-2">
        {/*
          文件名一栏至少约 8 个汉字宽（min-w-[8em]）：放不下时右侧控件组整组换到下一行，而不是把文件名挤没；
          规格先收缩（shrink-[999]），文件名放不下才截断。控件组在最窄的中栏里一行也放不下时，组内再换行（靠右）。
        */}
        <div className="flex min-w-[8em] flex-1 items-baseline gap-2.5">
          <span className="truncate font-semibold">{item?.name ?? "效果预览"}</span>
          {item && <span className="min-w-0 shrink-[999] truncate text-muted-foreground">{sourceSpec(item)}</span>}
        </div>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
          <ToggleGroup
            variant="segmented"
            size="sm"
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
        </div>
      </header>

      {/* 画面区：四周留边的黑底大圆角容器，画面居中；裂变时变体栏在画面右侧 */}
      <div className="mx-4 flex min-h-0 flex-1 gap-7 overflow-hidden rounded-xl border border-media-foreground/10 bg-media p-4 text-media-foreground">
        <div className="relative min-h-0 min-w-0 flex-1">
          {body}
          {usable && refreshing && (
            <span className="absolute top-0 right-0 grid place-items-center rounded-full bg-media/70 p-1">
              <Spinner aria-label="正在刷新" className="text-media-foreground/80" />
            </span>
          )}
        </div>
        {usable && count > 0 && <Variants count={count} />}
      </div>

      <div className="flex shrink-0 items-center gap-3 px-4 pt-3.5 pb-1.5">
        <span className="shrink-0 text-muted-foreground">
          {clock(item ? time : 0)} / {duration > 0 ? clock(duration) : "--:--"}
        </span>
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
          className="min-w-0 flex-1"
        />
      </div>

      <div className="shrink-0 space-y-1 px-4 pt-1.5 pb-3.5 text-muted-foreground">
        <p className="flex flex-wrap gap-x-4 gap-y-1">
          {view === "effect" && result ? (
            <span>
              参与预览：{result.modules.length > 0 ? result.modules.map(stepName).join("、") : "无"}
            </span>
          ) : null}
          {/* 这句整体换行；一行确实放不下时只在“加头尾、”之后折断，不会把最后一两个字单独挤到下一行 */}
          <span>
            <span className="whitespace-nowrap">变速、背景音、加头尾、</span>
            <span className="whitespace-nowrap">帧率、码率不参与预览</span>
          </span>
          {running ? <span>批量处理中，自动刷新已暂停</span> : null}
        </p>
        {view === "effect" &&
          result?.warnings.map((w, i) => (
            <p key={i} className="text-warning select-text">
              {w}
            </p>
          ))}
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

  return (
    <aside aria-label="裂变变体" className="flex min-h-0 w-40 shrink-0 flex-col justify-center gap-2.5">
      <p className="shrink-0 text-media-foreground/70">
        裂变变体<span className="ml-1.5 text-media-foreground/50">随机取值的 {count} 个结果</span>
      </p>
      <ScrollArea className="min-h-0 shrink">
        <ToggleGroup
          orientation="vertical"
          spacing={2.5}
          value={[String(selected)]}
          aria-label="选择变体"
          onValueChange={(next) => {
            const i = Number(next[0]);
            if (Number.isInteger(i)) useStage.getState().pickVariant(i);
          }}
          className="w-full"
        >
          {Array.from({ length: count }, (_, i) => {
            const thumb = thumbs[i];
            const pending = thumbsLoading && !thumb?.image && !thumb?.error;
            return (
              <Hint key={i} label={thumb?.error ? <span className="select-text">{thumb.error}</span> : null}>
                <ToggleGroupItem
                  value={String(i)}
                  aria-label={`变体 ${i + 1}`}
                  className="h-auto w-full justify-start gap-2.5 rounded-lg border border-media-foreground/15 bg-transparent p-1.5 text-left font-normal text-media-foreground/80 hover:bg-media-foreground/10 hover:text-media-foreground aria-pressed:border-primary aria-pressed:bg-primary/15 aria-pressed:text-media-foreground"
                >
                  <span
                    className={cn(
                      "relative shrink-0 overflow-hidden rounded bg-media",
                      aspect < 1 ? "h-20" : "w-16",
                    )}
                    style={{ aspectRatio: aspect }}
                  >
                    {thumb?.image ? (
                      <img
                        src={thumb.image.url}
                        alt={`变体 ${i + 1} 的画面`}
                        draggable={false}
                        className="absolute inset-0 size-full object-contain"
                      />
                    ) : thumb?.error ? (
                      <span className="absolute inset-0 grid place-items-center text-media-foreground/70">失败</span>
                    ) : null}
                    {pending && (
                      <span className="absolute inset-0 grid place-items-center">
                        <Spinner aria-label="正在渲染" className="text-media-foreground/70" />
                      </span>
                    )}
                  </span>
                  <span className="min-w-0 truncate">变体 {i + 1}</span>
                </ToggleGroupItem>
              </Hint>
            );
          })}
        </ToggleGroup>
      </ScrollArea>
      <Button
        variant="outline"
        size="sm"
        className="w-full shrink-0 border-media-foreground/20 bg-transparent text-media-foreground hover:bg-media-foreground/10 hover:text-media-foreground dark:border-media-foreground/20 dark:bg-transparent dark:hover:bg-media-foreground/10"
        onClick={() => useStage.getState().shuffle()}
      >
        <Shuffle /> 换一组随机值
      </Button>
    </aside>
  );
}

/** 画面区里的图片：居中、等比缩放、不超出区域。 */
function Frame({ src, alt }: { src: string; alt: string }) {
  return (
    <div className="absolute inset-0 flex items-center justify-center">
      <img src={src} alt={alt} draggable={false} className="max-h-full max-w-full rounded-md object-contain" />
    </div>
  );
}

/** 画面区里的一行说明 / 错误。 */
function Message({ children, error }: { children: ReactNode; error?: boolean }) {
  return (
    <p
      role={error ? "alert" : undefined}
      className={cn(
        "absolute inset-0 grid place-items-center p-4 text-center",
        error ? "text-media-foreground select-text" : "text-media-foreground/70",
      )}
    >
      {children}
    </p>
  );
}
