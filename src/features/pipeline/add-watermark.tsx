// pipe.add-watermark 加水印。后端：src-tauri/src/pipeline/modules/add_watermark.rs

import { useId } from "react";
import {
  ArrowDown,
  ArrowDownLeft,
  ArrowDownRight,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ArrowUpLeft,
  ArrowUpRight,
  Dot,
  type LucideIcon,
} from "lucide-react";
import AssetSourceInput from "@/components/AssetSourceInput";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { AssetSource, Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type Anchor =
  | "topLeft"
  | "top"
  | "topRight"
  | "left"
  | "center"
  | "right"
  | "bottomLeft"
  | "bottom"
  | "bottomRight";

export type ScrollMode = "none" | "left" | "right" | "up" | "down" | "bounce";

export interface SubWatermarkParams {
  enabled: boolean;
  source: AssetSource;
  anchor: Anchor;
  /** 宽度占画面宽度的百分比 */
  size: Range;
  /** 不透明度（%） */
  opacity: Range;
}

export interface AddWatermarkParams {
  enabled: boolean;
  /** 水印素材：单文件或目录 */
  source: AssetSource;
  /** 目录模式：分段依次显示多个素材 */
  turns: boolean;
  /** 每段秒数 */
  segment: number;
  anchor: Anchor;
  /** 左右边距（占画面宽度的百分比） */
  marginX: Range;
  /** 上下边距（占画面高度的百分比） */
  marginY: Range;
  /** 宽度占画面宽度的百分比 */
  size: Range;
  /** 不透明度（%） */
  opacity: Range;
  /** 居中时拉伸铺满画面 */
  stretch: boolean;
  scroll: ScrollMode;
  /** 每秒移动画面宽度的百分比 */
  scrollSpeed: Range;
  /** 动态素材倒放 */
  reverse: boolean;
  /** 去除素材背景色 */
  keyEnabled: boolean;
  keyColor: string;
  keySimilarity: number;
  keyBlend: number;
  sub: SubWatermarkParams;
}

/** 与后端 WATERMARK_EXTENSIONS 一致 */
const WATERMARK_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "gif", "mp4", "mov", "webm", "mkv"];

const ANCHORS: { value: Anchor; label: string; icon: LucideIcon }[] = [
  { value: "topLeft", label: "左上", icon: ArrowUpLeft },
  { value: "top", label: "上中", icon: ArrowUp },
  { value: "topRight", label: "右上", icon: ArrowUpRight },
  { value: "left", label: "左中", icon: ArrowLeft },
  { value: "center", label: "居中", icon: Dot },
  { value: "right", label: "右中", icon: ArrowRight },
  { value: "bottomLeft", label: "左下", icon: ArrowDownLeft },
  { value: "bottom", label: "下中", icon: ArrowDown },
  { value: "bottomRight", label: "右下", icon: ArrowDownRight },
];

const SCROLL_OPTIONS: { value: ScrollMode; label: string }[] = [
  { value: "none", label: "不滚动" },
  { value: "left", label: "从右向左" },
  { value: "right", label: "从左向右" },
  { value: "up", label: "从下向上" },
  { value: "down", label: "从上向下" },
  { value: "bounce", label: "画面内反弹" },
];

/** 九宫格位置选择（shadcn ToggleGroup，3×3）。 */
function AnchorGrid({ value, onChange, label }: { value: Anchor; onChange: (anchor: Anchor) => void; label: string }) {
  return (
    <ToggleGroup
      type="single"
      variant="outline"
      size="sm"
      spacing={1}
      aria-label={label}
      value={value}
      onValueChange={(next) => {
        if (next) onChange(next as Anchor);
      }}
      className="grid w-fit grid-cols-3"
    >
      {ANCHORS.map(({ value: anchor, label: name, icon: Icon }) => (
        <ToggleGroupItem
          key={anchor}
          value={anchor}
          aria-label={name}
          title={name}
          className="size-7 min-w-0 p-0 text-muted-foreground data-[state=on]:border-primary data-[state=on]:bg-primary/12 data-[state=on]:text-foreground"
        >
          <Icon className="size-3.5" />
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}

/** 开关 + 文字（一行）。 */
function SwitchRow({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  children: string;
}) {
  const id = useId();
  return (
    <div className="flex items-center gap-1.5">
      <Switch id={id} checked={checked} onCheckedChange={onChange} />
      <Label htmlFor={id} className="text-xs font-normal text-muted-foreground">
        {children}
      </Label>
    </div>
  );
}

function SubWatermarkForm({
  value,
  onChange,
}: {
  value: SubWatermarkParams;
  onChange: (patch: Partial<SubWatermarkParams>) => void;
}) {
  return (
    <>
      <ParamField label="子水印素材" hint="图片、GIF 或视频；动态素材循环播放。">
        <AssetSourceInput
          value={value.source}
          onChange={(source) => onChange({ source })}
          extensions={WATERMARK_EXTENSIONS}
          kindLabel="子水印"
        />
      </ParamField>
      <ParamField label="位置" hint="边距与主水印相同。">
        <AnchorGrid label="子水印位置" value={value.anchor} onChange={(anchor) => onChange({ anchor })} />
      </ParamField>
      <ParamField label="大小" hint="宽度占画面宽度的百分比，高度按比例。">
        <RangeInput value={value.size} onChange={(size) => onChange({ size })} min={1} max={100} step={1} unit="%" />
      </ParamField>
      <ParamField label="不透明度">
        <RangeInput
          value={value.opacity}
          onChange={(opacity) => onChange({ opacity })}
          min={0}
          max={100}
          step={1}
          integer
          unit="%"
        />
      </ParamField>
    </>
  );
}

function AddWatermarkForm({ value, onChange }: ModuleFormProps<AddWatermarkParams>) {
  const directory = value.source.mode === "directory";
  const stretched = value.anchor === "center" && value.stretch;
  const setSub = (patch: Partial<SubWatermarkParams>) => onChange({ sub: { ...value.sub, ...patch } });

  return (
    <>
      <ParamField label="水印素材" hint="图片（PNG 透明）、GIF 或视频（mp4、mov，可带透明通道）；动态素材循环到视频结束。">
        <AssetSourceInput
          value={value.source}
          onChange={(source) => onChange({ source })}
          extensions={WATERMARK_EXTENSIONS}
          kindLabel="水印"
        />
      </ParamField>
      {directory && (
        <ParamField
          label="分段显示"
          hint="开：目录中的多个水印（最多 10 个）按时间分段轮流显示；关：每个视频只取一个。"
        >
          <SwitchRow checked={value.turns} onChange={(turns) => onChange({ turns })}>
            依次轮换显示
          </SwitchRow>
          {value.turns && (
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              每段
              <NumberInput
                className="w-16"
                label="每段秒数"
                value={value.segment}
                min={0.5}
                max={3600}
                step={0.5}
                onChange={(segment) => onChange({ segment })}
              />
              秒
            </div>
          )}
        </ParamField>
      )}
      <ParamField label="位置">
        <AnchorGrid label="水印位置" value={value.anchor} onChange={(anchor) => onChange({ anchor })} />
        {value.anchor === "center" && (
          <SwitchRow checked={value.stretch} onChange={(stretch) => onChange({ stretch })}>
            拉伸铺满画面
          </SwitchRow>
        )}
      </ParamField>
      {!stretched && (
        <>
          <ParamField label="左右边距" hint="距画面左 / 右边缘，占画面宽度的百分比；居中的方向不偏移。">
            <RangeInput
              value={value.marginX}
              onChange={(marginX) => onChange({ marginX })}
              min={0}
              max={50}
              step={0.5}
              unit="%"
            />
          </ParamField>
          <ParamField label="上下边距" hint="距画面上 / 下边缘，占画面高度的百分比。">
            <RangeInput
              value={value.marginY}
              onChange={(marginY) => onChange({ marginY })}
              min={0}
              max={50}
              step={0.5}
              unit="%"
            />
          </ParamField>
          <ParamField label="大小" hint="水印宽度占画面宽度的百分比，高度按素材比例。">
            <RangeInput value={value.size} onChange={(size) => onChange({ size })} min={1} max={100} step={1} unit="%" />
          </ParamField>
        </>
      )}
      <ParamField label="不透明度">
        <RangeInput
          value={value.opacity}
          onChange={(opacity) => onChange({ opacity })}
          min={0}
          max={100}
          step={1}
          integer
          unit="%"
        />
      </ParamField>
      <ParamField label="滚动">
        <Select value={value.scroll} onValueChange={(scroll) => onChange({ scroll: scroll as ScrollMode })}>
          <SelectTrigger size="sm" aria-label="滚动效果" className="h-7 w-32 px-2 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SCROLL_OPTIONS.map((o) => (
              <SelectItem key={o.value} value={o.value} className="text-xs">
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      {value.scroll !== "none" && (
        <ParamField label="滚动速度" hint="每秒移动画面宽度的百分比。">
          <RangeInput
            value={value.scrollSpeed}
            onChange={(scrollSpeed) => onChange({ scrollSpeed })}
            min={1}
            max={500}
            step={1}
            unit="%"
          />
        </ParamField>
      )}
      <ParamField label="倒放" hint="仅对 GIF / 视频素材生效（整段缓存在内存中，适合短动画）。">
        <Switch aria-label="倒放动态水印" checked={value.reverse} onCheckedChange={(reverse) => onChange({ reverse })} />
      </ParamField>
      <ParamField label="去背景色" hint="把素材中接近所选颜色的部分变透明（如白底 Logo、绿幕视频）。">
        <Switch
          aria-label="去除水印背景色"
          checked={value.keyEnabled}
          onCheckedChange={(keyEnabled) => onChange({ keyEnabled })}
        />
      </ParamField>
      {value.keyEnabled && (
        <>
          <ParamField label="背景色">
            <ColorField label="背景色" value={value.keyColor} onChange={(keyColor) => onChange({ keyColor })} />
          </ParamField>
          <ParamField label="相似度" hint="0.01～1，越大去掉的颜色范围越宽。">
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
      <Separator className="my-1.5" />
      <ParamField label="子水印" hint="同时叠加第二个水印（参数较少，不滚动）。">
        <Switch aria-label="启用子水印" checked={value.sub.enabled} onCheckedChange={(enabled) => setSub({ enabled })} />
      </ParamField>
      {value.sub.enabled && <SubWatermarkForm value={value.sub} onChange={setSub} />}
    </>
  );
}

export default defineModule<AddWatermarkParams>({
  id: "pipe.add-watermark",
  defaults: {
    enabled: false,
    source: { mode: "file", path: "" },
    turns: false,
    segment: 5,
    anchor: "topRight",
    marginX: { min: 3, max: 3 },
    marginY: { min: 3, max: 3 },
    size: { min: 20, max: 20 },
    opacity: { min: 80, max: 80 },
    stretch: false,
    scroll: "none",
    scrollSpeed: { min: 20, max: 20 },
    reverse: false,
    keyEnabled: false,
    keyColor: "#00ff00",
    keySimilarity: 0.1,
    keyBlend: 0.05,
    sub: {
      enabled: false,
      source: { mode: "file", path: "" },
      anchor: "bottomLeft",
      size: { min: 15, max: 15 },
      opacity: { min: 80, max: 80 },
    },
  },
  Form: AddWatermarkForm,
});
