// pipe.crop 裁剪。后端：src-tauri/src/pipeline/modules/crop.rs
// 四种方式：指定四边（源像素）、按百分比、四边随机（百分比范围）、居中 + 比例。
// 画面框选得到的矩形 (x, y, w, h) 对应 margins = { left: x, top: y, right: 源宽-x-w, bottom: 源高-y-h }。

import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import RegionPicker, { type Rect } from "@/components/RegionPicker";
import { Label } from "@/components/ui/label";
import type { Range, VideoStream } from "@/lib/backend";
import { useSelectedVideo } from "@/state/selection";
import { defineModule, type ModuleFormProps } from "./types";

interface Size {
  width: number;
  height: number;
}

export type CropMode = "margins" | "percent" | "random" | "aspect";

export interface Sides<T> {
  left: T;
  top: T;
  right: T;
  bottom: T;
}

export interface CropParams {
  enabled: boolean;
  mode: CropMode;
  /** 指定四边：各边裁掉的源像素数 */
  margins: Sides<number>;
  /** 按百分比：各边裁掉的百分比（0～45） */
  percent: Sides<number>;
  /** 四边随机：各边裁掉的百分比范围（0～45） */
  randomPercent: Sides<Range>;
  /** 居中 + 比例：宽高比 */
  aspectWidth: number;
  aspectHeight: number;
}

/** 与后端 MAX_SIDE_PERCENT 一致 */
const MAX_SIDE_PERCENT = 45;

const SIDE_LABELS: [keyof Sides<unknown>, string][] = [
  ["left", "左"],
  ["top", "上"],
  ["right", "右"],
  ["bottom", "下"],
];

const ASPECT_PRESETS: [number, number][] = [
  [4, 3],
  [3, 4],
  [1, 1],
  [16, 9],
  [9, 16],
];

function SidesNumberInput({
  value,
  onChange,
  max,
  integer,
  step,
  unit,
}: {
  value: Sides<number>;
  onChange: (value: Sides<number>) => void;
  max?: number;
  integer?: boolean;
  step?: number;
  unit: string;
}) {
  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-1.5">
      {SIDE_LABELS.map(([side, label]) => (
        <Label key={side}>
          <span className="w-3">{label}</span>
          <NumberInput
            className="w-[72px]"
            label={`${label}边`}
            value={value[side]}
            min={0}
            max={max}
            step={step}
            integer={integer}
            onChange={(v) => onChange({ ...value, [side]: v })}
          />
          <span>{unit}</span>
        </Label>
      ))}
    </div>
  );
}

/** 视频（按旋转转正后）的源尺寸；未探测时为 null。 */
function displaySize(video: VideoStream | null | undefined): Size | null {
  if (!video || video.width <= 0 || video.height <= 0) return null;
  const rotated = Math.abs(video.rotation % 180) === 90;
  return rotated ? { width: video.height, height: video.width } : { width: video.width, height: video.height };
}

/** 四边 → 保留区域；四边都为 0 或超出画面时视为未框选。 */
function marginsToRects(m: Sides<number>, size: Size): Rect[] {
  const width = size.width - m.left - m.right;
  const height = size.height - m.top - m.bottom;
  if ((m.left === 0 && m.top === 0 && m.right === 0 && m.bottom === 0) || width <= 0 || height <= 0) return [];
  return [{ x: m.left, y: m.top, width, height }];
}

/** 保留区域 → 四边（源像素）。 */
function rectToMargins(r: Rect, size: Size): Sides<number> {
  return {
    left: r.x,
    top: r.y,
    right: Math.max(0, size.width - r.x - r.width),
    bottom: Math.max(0, size.height - r.y - r.height),
  };
}

/** 框选保留区域 → 四边。 */
function MarginsPicker({
  margins,
  onChange,
}: {
  margins: Sides<number>;
  onChange: (margins: Sides<number>) => void;
}) {
  const source = displaySize(useSelectedVideo()?.item.info?.video);
  return (
    <RegionPicker
      value={source ? marginsToRects(margins, source) : []}
      onChange={([rect]) => {
        if (source) onChange(rect ? rectToMargins(rect, source) : sides(0));
      }}
      disabled={!source}
      label="框选保留区域"
    />
  );
}

function CropForm({ value, onChange }: ModuleFormProps<CropParams>) {
  const preset = ASPECT_PRESETS.find(([w, h]) => w === value.aspectWidth && h === value.aspectHeight);
  const presetValue = preset ? `${preset[0]}:${preset[1]}` : "custom";

  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<CropMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          // 四个选项一行放不下：排成整齐的 2×2，而不是让最后一个单独折到下一行
          spacing={1}
          className="grid w-full grid-cols-2"
          options={[
            { value: "margins", label: "指定四边" },
            { value: "percent", label: "百分比" },
            { value: "random", label: "四边随机" },
            { value: "aspect", label: "居中比例" },
          ]}
        />
      </ParamField>

      {value.mode === "margins" && (
        <>
          <ParamField label="裁掉" hint="各边裁掉的像素数，以源视频画面为准；结果宽高自动取偶数。">
            <SidesNumberInput
              value={value.margins}
              onChange={(margins) => onChange({ margins })}
              integer
              unit="px"
            />
          </ParamField>
          <ParamField label="框选" hint="在当前视频画面上框出要保留的区域，自动换算成四边。">
            <MarginsPicker margins={value.margins} onChange={(margins) => onChange({ margins })} />
          </ParamField>
        </>
      )}

      {value.mode === "percent" && (
        <ParamField label="裁掉" hint={`各边裁掉画面宽 / 高的百分比，每边 0～${MAX_SIDE_PERCENT}%。`}>
          <SidesNumberInput
            value={value.percent}
            onChange={(percent) => onChange({ percent })}
            max={MAX_SIDE_PERCENT}
            step={0.5}
            unit="%"
          />
        </ParamField>
      )}

      {value.mode === "random" &&
        SIDE_LABELS.map(([side, label], i) => (
          <ParamField
            key={side}
            label={`${label}边`}
            hint={i === SIDE_LABELS.length - 1 ? "每边独立在范围内随机取百分比。" : undefined}
          >
            <RangeInput
              value={value.randomPercent[side]}
              onChange={(range) => onChange({ randomPercent: { ...value.randomPercent, [side]: range } })}
              min={0}
              max={MAX_SIDE_PERCENT}
              step={0.5}
              unit="%"
            />
          </ParamField>
        ))}

      {value.mode === "aspect" && (
        <>
          <ParamField label="比例">
            <ChoiceGroup<string>
              value={presetValue}
              onChange={(v) => {
                if (v === "custom") return;
                const [w, h] = v.split(":").map(Number);
                onChange({ aspectWidth: w, aspectHeight: h });
              }}
              options={[
                ...ASPECT_PRESETS.map(([w, h]) => ({ value: `${w}:${h}`, label: `${w}:${h}` })),
                { value: "custom", label: "自定义" },
              ]}
            />
          </ParamField>
          <ParamField label="宽 : 高" hint="从画面中央取该比例下最大的区域。">
            <div className="flex items-center gap-1.5">
              <NumberInput
                className="w-16"
                label="比例宽"
                value={value.aspectWidth}
                min={1}
                max={1000}
                integer
                onChange={(aspectWidth) => onChange({ aspectWidth })}
              />
              <span className="text-muted-foreground">:</span>
              <NumberInput
                className="w-16"
                label="比例高"
                value={value.aspectHeight}
                min={1}
                max={1000}
                integer
                onChange={(aspectHeight) => onChange({ aspectHeight })}
              />
            </div>
          </ParamField>
        </>
      )}
      <p className="pt-1 text-xs text-muted-foreground">裁剪后剩余画面过小（不足 16 像素或原画面的 10%）时该任务会报错。</p>
    </>
  );
}

const sides = <T,>(v: T): Sides<T> => ({ left: v, top: v, right: v, bottom: v });

export default defineModule<CropParams>({
  id: "pipe.crop",
  defaults: {
    enabled: false,
    mode: "margins",
    margins: sides(0),
    percent: sides(5),
    randomPercent: sides({ min: 0, max: 5 }),
    aspectWidth: 4,
    aspectHeight: 3,
  },
  Form: CropForm,
});
