// tool.remove-black-bars 黑边去除。后端：src-tauri/src/tools/remove_black_bars.rs
// 先检测黑边（cropdetect），再裁掉；没有黑边时跳过或原样复制；可恢复到原尺寸。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type WhenNoBars = "skip" | "copy";
export type RestoreSize = "none" | "fill" | "stretch";

export type RemoveBlackBarsParams = {
  /** 黑色阈值 0～255 */
  limit: number;
  /** 每秒抽样帧数 */
  sampleFps: number;
  /** 最小黑边（四边合计像素） */
  minBar: number;
  whenNone: WhenNoBars;
  restore: RestoreSize;
};

const RESTORE_HINTS: Record<RestoreSize, string> = {
  none: "输出裁剪后的尺寸（偶数）。",
  fill: "等比放大铺满原尺寸，居中裁去多余部分，画面不变形。",
  stretch: "直接拉伸到原尺寸，画面会变形。",
};

function RemoveBlackBarsForm({ value, onChange }: ToolFormProps<RemoveBlackBarsParams>) {
  return (
    <>
      <ParamField label="黑色阈值" hint="亮度不超过它的像素算作黑色（0～255）；画面偏暗时调小，黑边发灰时调大。">
        <NumberInput
          label="黑色阈值"
          value={value.limit}
          onChange={(limit) => onChange({ limit })}
          min={0}
          max={255}
          integer
        />
      </ParamField>
      <ParamField label="抽样帧率" hint="每秒检查几帧，越多越准、越慢。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="抽样帧率"
            value={value.sampleFps}
            onChange={(sampleFps) => onChange({ sampleFps })}
            min={0.1}
            max={30}
            step="any"
          />
          <span className="text-xs text-muted-foreground">帧/秒</span>
        </div>
      </ParamField>
      <ParamField label="最小黑边" hint="四边合计少于这么多像素时视为没有黑边。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="最小黑边"
            value={value.minBar}
            onChange={(minBar) => onChange({ minBar })}
            min={0}
            max={10000}
            integer
          />
          <span className="text-xs text-muted-foreground">像素</span>
        </div>
      </ParamField>
      <ParamField
        label="没有黑边时"
        hint={
          value.whenNone === "skip"
            ? "不输出，在处理记录里提示。"
            : "不重编码，原样复制一份到输出位置。"
        }
      >
        <ChoiceGroup<WhenNoBars>
          label="没有黑边时"
          value={value.whenNone}
          onChange={(whenNone) => onChange({ whenNone })}
          options={[
            { value: "skip", label: "跳过" },
            { value: "copy", label: "原样复制" },
          ]}
        />
      </ParamField>
      <ParamField label="输出尺寸" hint={RESTORE_HINTS[value.restore]}>
        <ChoiceGroup<RestoreSize>
          label="输出尺寸"
          value={value.restore}
          onChange={(restore) => onChange({ restore })}
          options={[
            { value: "none", label: "裁剪后" },
            { value: "fill", label: "铺满原尺寸" },
            { value: "stretch", label: "拉伸" },
          ]}
        />
      </ParamField>
    </>
  );
}

export default defineTool<RemoveBlackBarsParams>({
  id: "tool.remove-black-bars",
  defaults: {
    limit: 24,
    sampleFps: 2,
    minBar: 4,
    whenNone: "skip",
    restore: "none",
  },
  inputs: VIDEO_INPUTS,
  Form: RemoveBlackBarsForm,
  preview: true,
});
