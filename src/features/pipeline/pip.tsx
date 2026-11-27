// pipe.pip 画中画。后端：src-tauri/src/pipeline/modules/pip.rs
//
// 自定义区域（rect）存的是**输出画布**的像素坐标。RegionPicker 显示的是当前视频的源画面，
// 所以框选时按“源尺寸 ↔ 画布尺寸”的比例换算：未改画布尺寸时两者相同；改了画布尺寸（如横屏 → 1080×1920）时，
// 源画面就代表整块画布，框选位置按比例对应到画布上。下方的数字输入框直接是画布像素。

import AssetSourceInput from "@/components/AssetSourceInput";
import BlurCenterChoice from "@/components/BlurCenterChoice";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import RegionPicker, { type Rect } from "@/components/RegionPicker";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { AssetSource, Range } from "@/lib/backend";
import { useSelectedVideo } from "@/state/selection";
import { asset, join, range } from "./summary";
import { defineModule, type ModuleFormProps } from "./types";

export type PipBackground = "video" | "asset";
export type PipLayout = "centered" | "rect";

export interface PipRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PipParams {
  enabled: boolean;
  background: PipBackground;
  /** 外部背景素材 */
  asset: AssetSource;
  /** 背景虚化强度（gblur sigma，0 为不虚化） */
  blur: Range;
  /** 背景取中间（否则拉伸整幅） */
  /** null：跟随设置 set.blur-center */
  blurCenter: boolean | null;
  /** 自定义输出画布尺寸 */
  canvasEnabled: boolean;
  canvasWidth: number;
  canvasHeight: number;
  layout: PipLayout;
  /** 前景所占画布的百分比 */
  size: Range;
  /** 相对画布中心的偏移（%） */
  offsetX: Range;
  offsetY: Range;
  /** 自定义前景区域（画布像素） */
  rect: PipRect;
  /** 前景抠除纯色（绿幕） */
  keyEnabled: boolean;
  keyColor: string;
  keySimilarity: number;
  keyBlend: number;
}

/** 与后端 BACKGROUND_EXTENSIONS 一致 */
const BACKGROUND_EXTENSIONS = [
  "png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "mov", "mkv", "webm", "avi", "flv", "m4v",
];

const CANVAS_PRESETS: { label: string; width: number; height: number }[] = [
  { label: "竖屏 1080×1920", width: 1080, height: 1920 },
  { label: "竖屏 720×1280", width: 720, height: 1280 },
  { label: "横屏 1920×1080", width: 1920, height: 1080 },
  { label: "方形 1080×1080", width: 1080, height: 1080 },
];

const RECT_FIELDS: [keyof PipRect, string][] = [
  ["x", "X"],
  ["y", "Y"],
  ["width", "宽"],
  ["height", "高"],
];

function scaleRect(r: PipRect, sx: number, sy: number): PipRect {
  return {
    x: Math.round(r.x * sx),
    y: Math.round(r.y * sy),
    width: Math.round(r.width * sx),
    height: Math.round(r.height * sy),
  };
}

/** 框选按钮（源画面 ↔ 画布按比例换算）。 */
function CanvasRegionPicker({ value, onChange }: ModuleFormProps<PipParams>) {
  const video = useSelectedVideo()?.item.info?.video;
  // 当前视频转正后的尺寸
  const rotated = video?.rotation === 90 || video?.rotation === 270;
  const source =
    video && video.width > 0 && video.height > 0
      ? rotated
        ? { width: video.height, height: video.width }
        : { width: video.width, height: video.height }
      : null;
  const canvas = value.canvasEnabled ? { width: value.canvasWidth, height: value.canvasHeight } : source;
  const sx = source && canvas ? source.width / canvas.width : 1;
  const sy = source && canvas ? source.height / canvas.height : 1;
  const picked: Rect[] = value.rect.width > 0 && value.rect.height > 0 ? [scaleRect(value.rect, sx, sy)] : [];
  return (
    <RegionPicker
      label="框选前景位置"
      value={picked}
      onChange={(rects) => {
        const r = rects[0];
        onChange({ rect: r ? scaleRect(r, 1 / sx, 1 / sy) : { x: 0, y: 0, width: 0, height: 0 } });
      }}
    />
  );
}

/** 自定义区域：框选 + 画布像素输入。 */
function RectField({ value, onChange }: ModuleFormProps<PipParams>) {
  return (
    <ParamField
      label="前景区域"
      hint={
        value.canvasEnabled
          ? `坐标为输出画布（${value.canvasWidth}×${value.canvasHeight}）像素；在源画面上框选时按比例换算到画布。前景等比放进区域并居中。`
          : "坐标为输出画布像素（与当前画面相同）。前景等比放进区域并居中。"
      }
    >
      <CanvasRegionPicker value={value} onChange={onChange} />
      <div className="grid grid-cols-2 gap-x-3 gap-y-1.5">
        {RECT_FIELDS.map(([key, label]) => (
          <Label key={key}>
            <span className="w-4">{label}</span>
            <NumberInput
              className="w-[72px]"
              label={`前景区域 ${label}`}
              value={value.rect[key]}
              min={0}
              max={7680}
              step={1}
              integer
              onChange={(n) => onChange({ rect: { ...value.rect, [key]: n } })}
            />
          </Label>
        ))}
      </div>
    </ParamField>
  );
}

function PipForm(props: ModuleFormProps<PipParams>) {
  const { value, onChange } = props;
  return (
    <>
      <ParamField label="背景">
        <ChoiceGroup<PipBackground>
          label="背景来源"
          value={value.background}
          onChange={(background) => onChange({ background })}
          options={[
            { value: "video", label: "画面自身" },
            { value: "asset", label: "图片 / 视频" },
          ]}
        />
      </ParamField>
      {value.background === "asset" && (
        <ParamField label="背景素材" hint="图片一直显示；视频比正片短时循环、长时截断。">
          <AssetSourceInput
            value={value.asset}
            onChange={(asset) => onChange({ asset })}
            extensions={BACKGROUND_EXTENSIONS}
            kindLabel="背景"
          />
        </ParamField>
      )}
      <ParamField label="虚化强度" hint="0 表示背景不虚化。">
        <RangeInput value={value.blur} onChange={(blur) => onChange({ blur })} min={0} max={100} step={1} />
      </ParamField>
      <ParamField label="背景取法" hint="取中：背景等比放大后取中间；拉伸：把整幅拉伸铺满画布；跟随设置：按设置页“虚化背景取画面中间”。">
        <BlurCenterChoice value={value.blurCenter} onChange={(blurCenter) => onChange({ blurCenter })} />
      </ParamField>
      <ParamField label="画布尺寸" hint="关：沿用当前画面尺寸；开：改为指定尺寸（可做横竖屏转换）。">
        <Switch
          aria-label="自定义画布尺寸"
          checked={value.canvasEnabled}
          onCheckedChange={(canvasEnabled) => onChange({ canvasEnabled })}
        />
        {value.canvasEnabled && (
          <>
            <div className="flex flex-wrap items-center gap-1.5">
              <NumberInput
                className="w-[72px]"
                label="画布宽度"
                value={value.canvasWidth}
                min={16}
                max={7680}
                step={2}
                integer
                onChange={(canvasWidth) => onChange({ canvasWidth })}
              />
              <span className="text-muted-foreground">×</span>
              <NumberInput
                className="w-[72px]"
                label="画布高度"
                value={value.canvasHeight}
                min={16}
                max={7680}
                step={2}
                integer
                onChange={(canvasHeight) => onChange({ canvasHeight })}
              />
            </div>
            <ToggleGroup
              variant="segmented"
              spacing={1}
              aria-label="常用画布尺寸"
              value={CANVAS_PRESETS.filter((p) => p.width === value.canvasWidth && p.height === value.canvasHeight).map(
                (p) => p.label,
              )}
              onValueChange={(labels) => {
                const preset = CANVAS_PRESETS.find((p) => p.label === labels[0]);
                if (preset) onChange({ canvasWidth: preset.width, canvasHeight: preset.height });
              }}
              className="flex-wrap"
            >
              {CANVAS_PRESETS.map((p) => (
                <ToggleGroupItem
                  key={p.label}
                  value={p.label}
                >
                  {p.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </>
        )}
      </ParamField>
      <ParamField label="前景位置">
        <ChoiceGroup<PipLayout>
          label="前景位置方式"
          value={value.layout}
          onChange={(layout) => onChange({ layout })}
          options={[
            { value: "centered", label: "居中缩放" },
            { value: "rect", label: "自定义区域" },
          ]}
        />
      </ParamField>
      {value.layout === "centered" ? (
        <>
          <ParamField label="大小" hint="前景占画布的百分比（等比缩放），四周边距为 (100 − 大小) / 2。">
            <RangeInput value={value.size} onChange={(size) => onChange({ size })} min={5} max={100} step={1} unit="%" />
          </ParamField>
          <ParamField label="左右偏移" hint="相对画布中心，占画布宽度的百分比，正数向右。">
            <RangeInput
              value={value.offsetX}
              onChange={(offsetX) => onChange({ offsetX })}
              min={-50}
              max={50}
              step={0.5}
              unit="%"
            />
          </ParamField>
          <ParamField label="上下偏移" hint="相对画布中心，占画布高度的百分比，正数向下。">
            <RangeInput
              value={value.offsetY}
              onChange={(offsetY) => onChange({ offsetY })}
              min={-50}
              max={50}
              step={0.5}
              unit="%"
            />
          </ParamField>
        </>
      ) : (
        <RectField {...props} />
      )}
      <ParamField label="前景抠像" hint="抠除前景中的纯色（如绿幕），让主体直接叠在背景上。">
        <Switch
          aria-label="前景抠像"
          checked={value.keyEnabled}
          onCheckedChange={(keyEnabled) => onChange({ keyEnabled })}
        />
      </ParamField>
      {value.keyEnabled && (
        <>
          <ParamField label="抠除颜色">
            <ColorField label="抠除颜色" value={value.keyColor} onChange={(keyColor) => onChange({ keyColor })} />
          </ParamField>
          <ParamField label="相似度" hint="0.01～1，越大抠掉的颜色范围越宽。">
            <NumberInput
              className="w-20"
              label="相似度"
              value={value.keySimilarity}
              min={0.01}
              max={1}
              step={0.01}
              onChange={(keySimilarity) => onChange({ keySimilarity })}
            />
          </ParamField>
          <ParamField label="边缘混合" hint="0～1，越大边缘过渡越柔和。">
            <NumberInput
              className="w-20"
              label="边缘混合"
              value={value.keyBlend}
              min={0}
              max={1}
              step={0.01}
              onChange={(keyBlend) => onChange({ keyBlend })}
            />
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineModule<PipParams>({
  id: "pipe.pip",
  defaults: {
    enabled: false,
    background: "video",
    asset: { mode: "file", path: "" },
    blur: { min: 20, max: 20 },
    blurCenter: null,
    canvasEnabled: false,
    canvasWidth: 1080,
    canvasHeight: 1920,
    layout: "centered",
    size: { min: 80, max: 80 },
    offsetX: { min: 0, max: 0 },
    offsetY: { min: 0, max: 0 },
    rect: { x: 0, y: 0, width: 0, height: 0 },
    keyEnabled: false,
    keyColor: "#00ff00",
    keySimilarity: 0.1,
    keyBlend: 0.05,
  },
  Form: PipForm,
  summary: (p) =>
    join(
      p.canvasEnabled && `${p.canvasWidth}×${p.canvasHeight}`,
      p.background === "video" ? "自身作背景" : `背景 ${asset(p.asset)}`,
      p.layout === "centered" ? `前景 ${range(p.size, "%")}` : "自定义区域",
      p.keyEnabled && "抠像",
    ),
});
