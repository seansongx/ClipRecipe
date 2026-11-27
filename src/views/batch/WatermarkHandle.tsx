import { useState, type KeyboardEvent, type PointerEvent } from "react";
import { placementFromRect, snapToCenter, type AddWatermarkParams } from "@/features/pipeline/add-watermark";
import type { ModuleParams, PreviewRegion } from "@/lib/backend";
import { cn } from "@/lib/utils";
import { useRecipe } from "@/state/recipe";

const MODULE_ID = "pipe.add-watermark";

/** 方向键每次移动的距离（占画面的比例）；按住 Shift 时为大步。 */
const NUDGE = 0.005;
const NUDGE_LARGE = 0.02;

const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), Math.max(max, 0));

/** 预览结果里主水印所在的区域；没有（没开水印、滚动、铺满、分段轮换等）时为 null。 */
export function watermarkRegion(regions: PreviewRegion[] | undefined): PreviewRegion | null {
  return regions?.find((r) => r.module === MODULE_ID) ?? null;
}

/**
 * 压在预览画面上的水印把手：拖动（或聚焦后按方向键）把水印移到新位置，松手时换算成“位置 + 边距”写回配方，
 * 预览随后自动刷新。父元素必须正好盖住画面（见 PreviewStage 的 Frame），这里的位置都用百分比。
 *
 * `region` 是上一次渲染出来的位置。松手后到新画面出来之前，把手留在松手的位置（`moved`，按 region 对象记：
 * 新的预览结果带来新的 region 对象，旧的 `moved` 自然作废）。
 */
export default function WatermarkHandle({ region }: { region: PreviewRegion }) {
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null);
  const [moved, setMoved] = useState<{ region: PreviewRegion; x: number; y: number } | null>(null);
  const settled = moved?.region === region ? moved : region;
  const at = drag ?? settled;
  const { width, height } = region;

  /** 限制在画面内，并在靠近中线时吸附到居中。 */
  const fit = (x: number, y: number) => ({
    x: snapToCenter(clamp(x, 1 - width), width),
    y: snapToCenter(clamp(y, 1 - height), height),
  });

  const commit = (next: { x: number; y: number }) => {
    setMoved({ region, ...next });
    const params = useRecipe.getState().recipe.modules[MODULE_ID] as unknown as AddWatermarkParams | undefined;
    if (!params) return;
    // 配方里的模块参数是按 ID 存的宽松类型，这里是加水印模块自己的字段
    const patch: Partial<AddWatermarkParams> = placementFromRect({ ...next, width, height }, params);
    useRecipe.getState().updateModule(MODULE_ID, patch as Partial<ModuleParams>);
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const frame = e.currentTarget.parentElement?.getBoundingClientRect();
    if (!frame || frame.width <= 0 || frame.height <= 0) return;
    e.preventDefault();
    e.currentTarget.focus({ preventScroll: true });
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    const origin = { x: settled.x, y: settled.y, clientX: e.clientX, clientY: e.clientY };
    let latest = { x: origin.x, y: origin.y };
    let dragged = false;
    const move = (ev: globalThis.PointerEvent) => {
      dragged = true;
      latest = fit(
        origin.x + (ev.clientX - origin.clientX) / frame.width,
        origin.y + (ev.clientY - origin.clientY) / frame.height,
      );
      setDrag(latest);
    };
    const end = (ev: globalThis.PointerEvent) => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", end);
      target.removeEventListener("pointercancel", end);
      setDrag(null);
      // 只是点了一下（没有移动）或被系统取消时不改参数
      if (dragged && ev.type === "pointerup") commit(latest);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end);
    target.addEventListener("pointercancel", end);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? NUDGE_LARGE : NUDGE;
    const delta: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const d = delta[e.key];
    if (!d) return;
    e.preventDefault();
    // 方向键不吸附：否则一步的距离比吸附范围小，会卡在中线上走不出去
    commit({ x: clamp(settled.x + d[0], 1 - width), y: clamp(settled.y + d[1], 1 - height) });
  };

  const centeredX = Math.abs(at.x + width / 2 - 0.5) < 1e-6;
  const centeredY = Math.abs(at.y + height / 2 - 0.5) < 1e-6;
  const percent = (value: number) => `${value * 100}%`;

  return (
    <>
      {/* 拖动中吸附到居中时的参考线 */}
      {drag && centeredX && <span className="pointer-events-none absolute inset-y-0 left-1/2 w-px bg-primary" />}
      {drag && centeredY && <span className="pointer-events-none absolute inset-x-0 top-1/2 h-px bg-primary" />}
      <div
        role="button"
        tabIndex={0}
        aria-label="水印位置：拖动或按方向键调整"
        data-dragging={drag ? "" : undefined}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        style={{ left: percent(at.x), top: percent(at.y), width: percent(width), height: percent(height) }}
        className={cn(
          "absolute cursor-move touch-none rounded-sm border border-dashed border-primary/70 outline-none select-none",
          "hover:border-solid hover:border-primary hover:bg-primary/10",
          "focus-visible:border-solid focus-visible:border-primary focus-visible:ring-2 focus-visible:ring-ring/50",
          "data-dragging:border-solid data-dragging:border-primary data-dragging:bg-primary/15",
        )}
      />
    </>
  );
}
