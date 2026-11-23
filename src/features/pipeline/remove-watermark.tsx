// pipe.remove-watermark 去水印（区域模糊）。后端：src-tauri/src/pipeline/modules/remove_watermark.rs
// 预设位置按画面尺寸换算；自定义区域为源视频像素（RegionPicker 框选）。
// 生效时段是“去头尾 / 变速之后”的视频时间。预览（单模块预览、生成预览）里区域会用黄色框线标出，正式输出不画。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import RegionPicker, { type Rect } from "@/components/RegionPicker";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { defineModule, type ModuleFormProps } from "./types";

export type RegionMode = "preset" | "custom";

export type WatermarkPreset =
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight"
  | "bottomCaption"
  | "topBanner";

export interface RemoveWatermarkParams {
  enabled: boolean;
  mode: RegionMode;
  /** 预设位置（可多选） */
  presets: WatermarkPreset[];
  /** 自定义区域（源像素，最多 2 个） */
  regions: Rect[];
  /** 模糊强度 1～10 */
  strength: number;
  /** 只在 start～end 秒内模糊 */
  limitTime: boolean;
  start: number;
  end: number;
}

const PRESETS: { value: WatermarkPreset; label: string }[] = [
  { value: "topLeft", label: "左上角" },
  { value: "topRight", label: "右上角" },
  { value: "bottomLeft", label: "左下角" },
  { value: "bottomRight", label: "右下角" },
  { value: "bottomCaption", label: "底部字幕条" },
  { value: "topBanner", label: "顶部横条" },
];

const round1 = (v: number) => Math.round(v * 10) / 10;

function RemoveWatermarkForm({ value, onChange }: ModuleFormProps<RemoveWatermarkParams>) {
  return (
    <>
      <ParamField label="区域">
        <ChoiceGroup<RegionMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "preset", label: "预设位置" },
            { value: "custom", label: "自定义框选" },
          ]}
        />
      </ParamField>

      {value.mode === "preset" ? (
        <ParamField label="位置" hint="可多选。角标按画面短边的 30%×12% 计算，横竖屏一致；字幕条为画面底部 76%～90% 高度处。">
          <ToggleGroup
            type="multiple"
            variant="outline"
            spacing={1}
            value={value.presets}
            onValueChange={(presets) => onChange({ presets: presets as WatermarkPreset[] })}
            className="flex-wrap"
            aria-label="预设位置"
          >
            {PRESETS.map((p) => (
              <ToggleGroupItem
                key={p.value}
                value={p.value}
              >
                {p.label}
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </ParamField>
      ) : (
        <ParamField
          label="框选"
          hint="最多 2 处，坐标为源视频像素；超出画面的部分自动裁掉，过小（不足 8 像素）的区域会报错。预览时区域用黄框标出。"
        >
          <RegionPicker
            value={value.regions}
            onChange={(regions) => onChange({ regions })}
            max={2}
            label="框选水印区域"
          />
        </ParamField>
      )}

      <ParamField label="模糊强度" hint="1～10，越大越模糊；10 几乎抹成一片纯色。">
        <NumberInput
          className="w-16"
          label="模糊强度"
          value={value.strength}
          min={1}
          max={10}
          integer
          onChange={(strength) => onChange({ strength })}
        />
      </ParamField>

      <ParamField
        label="生效时段"
        hint={
          value.limitTime
            ? "只在这段时间内模糊（例如遮住几句歌词）。时间指去头尾、抽帧、变速之后的视频时间。"
            : undefined
        }
      >
        <div className="flex flex-wrap items-center gap-2">
          <Switch
            aria-label="限定生效时段"
            checked={value.limitTime}
            onCheckedChange={(limitTime) => onChange({ limitTime })}
          />
          {value.limitTime && (
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <NumberInput
                className="w-16"
                label="开始（秒）"
                value={value.start}
                min={0}
                step={0.1}
                onChange={(start) => onChange({ start, end: Math.max(value.end, round1(start + 0.1)) })}
              />
              ~
              <NumberInput
                className="w-16"
                label="结束（秒）"
                value={value.end}
                min={0.1}
                step={0.1}
                onChange={(end) => onChange({ end, start: Math.min(value.start, Math.max(0, round1(end - 0.1))) })}
              />
              秒
            </span>
          )}
        </div>
      </ParamField>
    </>
  );
}

export default defineModule<RemoveWatermarkParams>({
  id: "pipe.remove-watermark",
  defaults: {
    enabled: false,
    mode: "preset",
    presets: ["topRight"],
    regions: [],
    strength: 6,
    limitTime: false,
    start: 0,
    end: 5,
  },
  Form: RemoveWatermarkForm,
});
