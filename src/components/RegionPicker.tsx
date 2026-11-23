// 区域框选 / 位置选取：在“当前视频”（useSelectedVideo）的某一帧上拖拽矩形或点选位置。
// 坐标一律是源视频（按旋转信息转正后）的像素坐标，与 FFmpeg 滤镜看到的画面一致。
// 用法见 docs/PLAN.md“模块/工具编写指南”。

import { useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { Crosshair, SquareDashedMousePointer, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import Hint from "@/components/Hint";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Slider } from "@/components/ui/slider";
import { Spinner } from "@/components/ui/spinner";
import { formatDuration } from "@/lib/format";
import { useSelectedVideo } from "@/state/selection";
import { useVideoFrame, type VideoFrame } from "./useVideoFrame";

/** 矩形区域（源像素）。 */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** 画面上的一点（源像素）。 */
export interface Point {
  x: number;
  y: number;
}

const MIN_SIZE = 2;

// ---------------------------------------------------------------------------
// 几何

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** 以 anchor 为固定角、pointer 为对角生成矩形；可锁定宽高比；限制在画面内。 */
export function rectFromCorners(
  anchor: Point,
  pointer: Point,
  bounds: { width: number; height: number },
  aspect?: number,
): Rect {
  const dirX = pointer.x >= anchor.x ? 1 : -1;
  const dirY = pointer.y >= anchor.y ? 1 : -1;
  let w = Math.abs(pointer.x - anchor.x);
  let h = Math.abs(pointer.y - anchor.y);
  const maxW = dirX > 0 ? bounds.width - anchor.x : anchor.x;
  const maxH = dirY > 0 ? bounds.height - anchor.y : anchor.y;
  if (aspect && aspect > 0) {
    w = Math.max(w, h * aspect);
    h = w / aspect;
    const scale = Math.min(1, maxW / (w || 1), maxH / (h || 1));
    w *= scale;
    h *= scale;
  } else {
    w = Math.min(w, maxW);
    h = Math.min(h, maxH);
  }
  const x = dirX > 0 ? anchor.x : anchor.x - w;
  const y = dirY > 0 ? anchor.y : anchor.y - h;
  return roundRect({ x, y, width: w, height: h }, bounds);
}

function roundRect(r: Rect, bounds: { width: number; height: number }): Rect {
  const x = clamp(Math.round(r.x), 0, bounds.width);
  const y = clamp(Math.round(r.y), 0, bounds.height);
  return {
    x,
    y,
    width: clamp(Math.round(r.width), 0, bounds.width - x),
    height: clamp(Math.round(r.height), 0, bounds.height - y),
  };
}

type Corner = "nw" | "ne" | "sw" | "se";
const CORNERS: Corner[] = ["nw", "ne", "sw", "se"];

function oppositeCorner(r: Rect, corner: Corner): Point {
  return {
    x: corner.includes("w") ? r.x + r.width : r.x,
    y: corner.includes("n") ? r.y + r.height : r.y,
  };
}

type Drag =
  | { kind: "draw"; index: number; anchor: Point }
  | { kind: "resize"; index: number; anchor: Point }
  | { kind: "move"; index: number; offset: Point; start: Rect };

// ---------------------------------------------------------------------------
// 编辑器

function FrameEditor({
  frame,
  rects,
  onRects,
  max,
  aspect,
  point,
  onPoint,
}: {
  frame: VideoFrame;
  rects: Rect[];
  onRects: (rects: Rect[]) => void;
  max: number;
  aspect?: number;
  /** 位置模式：传入 onPoint 时点击设置位置，不画矩形 */
  point?: Point | null;
  onPoint?: (point: Point) => void;
}) {
  const overlay = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [active, setActive] = useState<number | null>(rects.length > 0 ? 0 : null);
  const bounds = { width: frame.width, height: frame.height };

  const toSource = (e: ReactPointerEvent): Point => {
    const box = overlay.current?.getBoundingClientRect();
    if (!box || box.width === 0 || box.height === 0) return { x: 0, y: 0 };
    return {
      x: clamp(((e.clientX - box.left) / box.width) * frame.width, 0, frame.width),
      y: clamp(((e.clientY - box.top) / box.height) * frame.height, 0, frame.height),
    };
  };

  const replace = (index: number, rect: Rect) => onRects(rects.map((r, i) => (i === index ? rect : r)));

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const p = toSource(e);
    if (onPoint) {
      onPoint({ x: Math.round(p.x), y: Math.round(p.y) });
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = { kind: "draw", index: -1, anchor: p };
      return;
    }
    const target = e.target as HTMLElement;
    const handle = target.closest<HTMLElement>("[data-corner]");
    const body = target.closest<HTMLElement>("[data-index]");
    if (handle && body) {
      const index = Number(body.dataset.index);
      const rect = rects[index];
      if (!rect) return;
      drag.current = { kind: "resize", index, anchor: oppositeCorner(rect, handle.dataset.corner as Corner) };
      setActive(index);
    } else if (body) {
      const index = Number(body.dataset.index);
      const rect = rects[index];
      if (!rect) return;
      drag.current = { kind: "move", index, offset: { x: p.x - rect.x, y: p.y - rect.y }, start: rect };
      setActive(index);
    } else {
      // 画新矩形：已满时替换当前选中的（只能有一个时即替换它）
      const index = rects.length < max ? rects.length : (active ?? rects.length - 1);
      const fresh = { x: Math.round(p.x), y: Math.round(p.y), width: 0, height: 0 };
      onRects(index < rects.length ? rects.map((r, i) => (i === index ? fresh : r)) : [...rects, fresh]);
      drag.current = { kind: "draw", index, anchor: p };
      setActive(index);
    }
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const p = toSource(e);
    if (onPoint) {
      onPoint({ x: Math.round(p.x), y: Math.round(p.y) });
      return;
    }
    if (d.kind === "move") {
      const x = clamp(p.x - d.offset.x, 0, frame.width - d.start.width);
      const y = clamp(p.y - d.offset.y, 0, frame.height - d.start.height);
      replace(d.index, roundRect({ ...d.start, x, y }, bounds));
    } else {
      replace(d.index, rectFromCorners(d.anchor, p, bounds, aspect));
    }
  };

  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (!d || onPoint) return;
    // 太小的矩形（误点）去掉
    const rect = rects[d.index];
    if (rect && (rect.width < MIN_SIZE || rect.height < MIN_SIZE)) {
      onRects(rects.filter((_, i) => i !== d.index));
      setActive(null);
    }
  };

  const pct = (v: number, total: number) => `${(v / total) * 100}%`;

  return (
    <div className="relative inline-block max-w-full select-none">
      <img
        src={frame.url}
        alt="视频画面"
        draggable={false}
        className="block max-h-[60vh] max-w-full"
        style={{ aspectRatio: `${frame.width} / ${frame.height}` }}
      />
      <div
        ref={overlay}
        className="absolute inset-0 cursor-crosshair touch-none"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        {!onPoint &&
          rects.map((r, i) => (
            <div
              key={i}
              data-index={i}
              className={`absolute cursor-move border-2 ${
                i === active ? "border-primary bg-primary/20" : "border-white/80 bg-white/10"
              }`}
              style={{
                left: pct(r.x, frame.width),
                top: pct(r.y, frame.height),
                width: pct(r.width, frame.width),
                height: pct(r.height, frame.height),
                boxShadow: "0 0 0 1px rgba(0,0,0,.6)",
              }}
            >
              <span className="pointer-events-none absolute left-0 top-0 bg-black/70 px-1 text-xs leading-4 text-white">
                {i + 1}
              </span>
              {i === active &&
                CORNERS.map((corner) => (
                  <span
                    key={corner}
                    data-corner={corner}
                    className="absolute size-2.5 border border-black/60 bg-white"
                    style={{
                      left: corner.includes("w") ? -6 : undefined,
                      right: corner.includes("e") ? -6 : undefined,
                      top: corner.includes("n") ? -6 : undefined,
                      bottom: corner.includes("s") ? -6 : undefined,
                      cursor: corner === "nw" || corner === "se" ? "nwse-resize" : "nesw-resize",
                    }}
                  />
                ))}
              {i === active && (
                <Button
                  type="button"
                  size="icon-xs"
                  aria-label="删除此区域"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => {
                    onRects(rects.filter((_, j) => j !== i));
                    setActive(null);
                  }}
                  className="absolute -top-2.5 right-3"
                >
                  <X />
                </Button>
              )}
            </div>
          ))}
        {onPoint && point && (
          <div
            className="pointer-events-none absolute"
            style={{ left: pct(point.x, frame.width), top: pct(point.y, frame.height) }}
          >
            <span className="absolute -left-3 top-0 h-px w-6 bg-primary shadow" />
            <span className="absolute -top-3 left-0 h-6 w-px bg-primary shadow" />
            <span className="absolute -left-1.5 -top-1.5 size-3 rounded-full border-2 border-primary" />
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// 对话框（选帧 + 编辑）

function FrameDialog({
  title,
  hint,
  onClose,
  onConfirm,
  onClear,
  children,
}: {
  title: string;
  hint: string;
  onClose: () => void;
  onConfirm: () => void;
  onClear?: () => void;
  children: (frame: VideoFrame) => ReactNode;
}) {
  const selected = useSelectedVideo();
  const item = selected?.item ?? null;
  const duration = item?.info?.duration ?? 0;
  const [time, setTime] = useState(() => Math.min(1, duration / 2));
  const { frame, loading, error } = useVideoFrame(item, time);

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        aria-describedby={undefined}
        className="flex max-h-[calc(100vh-3rem)] flex-col overflow-hidden sm:max-w-5xl"
      >
        <DialogHeader>
          <DialogTitle className="truncate">{`${title}${item ? ` · ${item.name}` : ""}`}</DialogTitle>
        </DialogHeader>
        <div className="grid min-h-[300px] place-items-center rounded-lg bg-black/90 p-3">
          {frame ? (
            children(frame)
          ) : (
            <div className="text-center text-sm text-white/80">
              {loading ? <Spinner className="mx-auto size-6" aria-label="加载中" /> : error}
            </div>
          )}
        </div>
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          <span className="shrink-0">画面位置</span>
          <Slider
            min={0}
            max={Math.max(duration, 0.1)}
            step={0.1}
            value={[time]}
            disabled={duration <= 0}
            onValueChange={([t]) => setTime(t ?? 0)}
            className="min-w-0 flex-1"
            aria-label="取帧时间"
          />
          <span className="w-24 shrink-0 text-right tabular-nums">
            {formatDuration(time)} / {formatDuration(duration)}
          </span>
          {loading && frame && <Spinner className="size-3" aria-label="加载中" />}
          {frame && (
            <span className="shrink-0 tabular-nums">
              源尺寸 {frame.width}×{frame.height}
            </span>
          )}
        </div>
        <DialogFooter className="items-center sm:justify-start">
          <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{hint}</span>
          {onClear && (
            <Button variant="outline" size="sm" onClick={onClear}>
              清空
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={onClose}>
            取消
          </Button>
          <Button size="sm" onClick={onConfirm} disabled={!frame}>
            确定
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const describeRect = (r: Rect) => `(${r.x}, ${r.y}) ${r.width}×${r.height}`;

/** 打开选帧对话框的按钮；队列为空时禁用。 */
function PickerButton({
  icon,
  label,
  disabled,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  disabled: boolean;
  onClick: () => void;
}) {
  const selected = useSelectedVideo();
  return (
    <Hint label={selected ? null : "先在队列中添加视频"} disabledTrigger>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={onClick}
        disabled={disabled || !selected}
      >
        {icon} {label}
      </Button>
    </Hint>
  );
}

/**
 * 区域框选：按钮 + 对话框。在当前视频的画面上画 1～max 个矩形，确定后以源像素坐标回传。
 */
export default function RegionPicker({
  value,
  onChange,
  max = 1,
  aspect,
  label = "在画面上框选",
  disabled = false,
}: {
  /** 当前区域（源像素） */
  value: Rect[];
  /** 点“确定”后回传全部区域 */
  onChange: (rects: Rect[]) => void;
  /** 最多几个矩形，默认 1 */
  max?: number;
  /** 锁定宽高比（宽 / 高），如 4/3；不传则自由 */
  aspect?: number;
  /** 按钮文字 */
  label?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Rect[]>(value);

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <PickerButton
        icon={<SquareDashedMousePointer />}
        label={label}
        disabled={disabled}
        onClick={() => {
          setDraft(value.slice(0, max));
          setOpen(true);
        }}
      />
      <span className="min-w-0 truncate text-xs text-muted-foreground tabular-nums">
        {value.length === 0 ? "未框选" : value.map(describeRect).join("；")}
      </span>
      {open && (
        <FrameDialog
          title="框选区域"
          hint={`拖拽画出区域（最多 ${max} 个），拖动可移动，拖四角可调整大小。坐标为源视频像素。`}
          onClose={() => setOpen(false)}
          onClear={() => setDraft([])}
          onConfirm={() => {
            onChange(draft.filter((r) => r.width >= MIN_SIZE && r.height >= MIN_SIZE));
            setOpen(false);
          }}
        >
          {(frame) => <FrameEditor frame={frame} rects={draft} onRects={setDraft} max={max} aspect={aspect} />}
        </FrameDialog>
      )}
    </div>
  );
}

/** 位置选取：按钮 + 对话框。在当前视频的画面上点选一个位置（源像素）。 */
export function PointPicker({
  value,
  onChange,
  label = "在画面上选位置",
  disabled = false,
}: {
  value: Point | null;
  onChange: (point: Point) => void;
  label?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Point | null>(value);

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2">
      <PickerButton
        icon={<Crosshair />}
        label={label}
        disabled={disabled}
        onClick={() => {
          setDraft(value);
          setOpen(true);
        }}
      />
      <span className="text-xs text-muted-foreground tabular-nums">
        {value ? `(${value.x}, ${value.y})` : "未选择"}
      </span>
      {open && (
        <FrameDialog
          title="选择位置"
          hint="点击或拖动选择位置。坐标为源视频像素。"
          onClose={() => setOpen(false)}
          onConfirm={() => {
            if (draft) onChange(draft);
            setOpen(false);
          }}
        >
          {(frame) => (
            <FrameEditor frame={frame} rects={[]} onRects={() => {}} max={0} point={draft} onPoint={setDraft} />
          )}
        </FrameDialog>
      )}
    </div>
  );
}
