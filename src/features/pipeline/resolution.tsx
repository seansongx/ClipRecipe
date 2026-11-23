// pipe.resolution 分辨率。后端：src-tauri/src/pipeline/modules/resolution.rs

import BlurCenterChoice from "@/components/BlurCenterChoice";
import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type ResolutionMode = "standard" | "custom";
export type ScaleMode = "stretch" | "fit";
export type FillMode = "blur" | "black";

export interface ResolutionParams {
  enabled: boolean;
  mode: ResolutionMode;
  /** 标准档位：统一高度，宽度等比（偶数） */
  standardHeight: number;
  /** 自定义宽高 */
  width: number;
  height: number;
  scaleMode: ScaleMode;
  /** 原比例时空白处的填充 */
  fill: FillMode;
  /** 背景虚化强度（gblur sigma，0 为不虚化） */
  blurStrength: Range;
  /** 背景取画面中心（否则拉伸整个画面） */
  /** null：跟随设置 set.blur-center */
  blurCenter: boolean | null;
  /** 黑色填充时的倒影效果 */
  reflection: boolean;
}

const STANDARD_HEIGHTS = [360, 480, 540, 720, 1080, 1440, 2160];
/** 下拉框里表示“当前高度不是标准档位”的值 */
const CUSTOM_HEIGHT = "custom";

const SIZE_PRESETS: { label: string; width: number; height: number }[] = [
  { label: "竖屏 1080×1920", width: 1080, height: 1920 },
  { label: "竖屏 720×1280", width: 720, height: 1280 },
  { label: "横屏 1920×1080", width: 1920, height: 1080 },
  { label: "方形 1080×1080", width: 1080, height: 1080 },
];

function ResolutionForm({ value, onChange }: ModuleFormProps<ResolutionParams>) {
  const fit = value.mode === "custom" && value.scaleMode === "fit";
  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<ResolutionMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "standard", label: "标准档位" },
            { value: "custom", label: "自定义宽高" },
          ]}
        />
      </ParamField>
      {value.mode === "standard" ? (
        <ParamField label="高度" hint="所有视频统一为此高度，宽度按原比例换算（取偶数）。">
          <div className="flex flex-wrap items-center gap-1.5">
            <Select
              value={STANDARD_HEIGHTS.includes(value.standardHeight) ? String(value.standardHeight) : CUSTOM_HEIGHT}
              onValueChange={(v) => v !== CUSTOM_HEIGHT && onChange({ standardHeight: Number(v) })}
            >
              <SelectTrigger size="sm" aria-label="标准高度" className="h-7 w-24 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {!STANDARD_HEIGHTS.includes(value.standardHeight) && (
                  <SelectItem value={CUSTOM_HEIGHT} disabled className="text-xs">
                    自定义
                  </SelectItem>
                )}
                {STANDARD_HEIGHTS.map((h) => (
                  <SelectItem key={h} value={String(h)} className="text-xs">
                    {h}p
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <NumberInput
              className="w-20"
              label="高度"
              value={value.standardHeight}
              min={16}
              max={4320}
              step={2}
              integer
              onChange={(standardHeight) => onChange({ standardHeight })}
            />
            <span className="text-xs text-muted-foreground">px</span>
          </div>
        </ParamField>
      ) : (
        <>
          <ParamField label="宽 × 高" hint="奇数会自动调整为偶数。">
            <div className="flex flex-wrap items-center gap-1.5">
              <NumberInput
                className="w-[72px]"
                label="宽度"
                value={value.width}
                min={16}
                max={7680}
                step={2}
                integer
                onChange={(width) => onChange({ width })}
              />
              <span className="text-muted-foreground">×</span>
              <NumberInput
                className="w-[72px]"
                label="高度"
                value={value.height}
                min={16}
                max={7680}
                step={2}
                integer
                onChange={(height) => onChange({ height })}
              />
              <span className="text-xs text-muted-foreground">px</span>
            </div>
          </ParamField>
          <ParamField label="常用">
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              spacing={1}
              aria-label="常用尺寸"
              value={SIZE_PRESETS.find((p) => p.width === value.width && p.height === value.height)?.label ?? ""}
              onValueChange={(label) => {
                const preset = SIZE_PRESETS.find((p) => p.label === label);
                if (preset) onChange({ width: preset.width, height: preset.height });
              }}
              className="flex-wrap"
            >
              {SIZE_PRESETS.map((p) => (
                <ToggleGroupItem
                  key={p.label}
                  value={p.label}
                  className="h-6 px-1.5 text-[11px] font-normal text-muted-foreground data-[state=on]:border-primary data-[state=on]:bg-primary/12 data-[state=on]:text-foreground"
                >
                  {p.label}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </ParamField>
          <ParamField label="缩放">
            <ChoiceGroup<ScaleMode>
              value={value.scaleMode}
              onChange={(scaleMode) => onChange({ scaleMode })}
              options={[
                { value: "fit", label: "保持原比例" },
                { value: "stretch", label: "拉伸" },
              ]}
            />
          </ParamField>
          {fit && (
            <ParamField label="空白填充">
              <ChoiceGroup<FillMode>
                value={value.fill}
                onChange={(fill) => onChange({ fill })}
                options={[
                  { value: "blur", label: "画面背景" },
                  { value: "black", label: "黑色" },
                ]}
              />
            </ParamField>
          )}
          {fit && value.fill === "blur" && (
            <>
              <ParamField label="虚化强度" hint="0 表示背景不虚化。">
                <RangeInput
                  value={value.blurStrength}
                  onChange={(blurStrength) => onChange({ blurStrength })}
                  min={0}
                  max={100}
                  step={1}
                />
              </ParamField>
              <ParamField label="背景取法" hint="取中：背景等比放大后取画面中心；拉伸：把整个画面拉伸铺满；跟随设置：按设置页“虚化背景取画面中间”。">
                <BlurCenterChoice value={value.blurCenter} onChange={(blurCenter) => onChange({ blurCenter })} />
              </ParamField>
            </>
          )}
          {fit && value.fill === "black" && (
            <ParamField label="倒影" hint="在画面下方的空白处显示渐隐的倒影；仅上下有空白时生效。">
              <Switch
                aria-label="倒影效果"
                checked={value.reflection}
                onCheckedChange={(reflection) => onChange({ reflection })}
              />
            </ParamField>
          )}
        </>
      )}
    </>
  );
}

export default defineModule<ResolutionParams>({
  id: "pipe.resolution",
  defaults: {
    enabled: false,
    mode: "standard",
    standardHeight: 720,
    width: 1080,
    height: 1920,
    scaleMode: "fit",
    fill: "blur",
    blurStrength: { min: 20, max: 20 },
    blurCenter: null,
    reflection: false,
  },
  Form: ResolutionForm,
});
