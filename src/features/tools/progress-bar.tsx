// tool.progress-bar 进度条。后端：src-tauri/src/tools/progress_bar.rs
// 在画面顶部或底部绘制播放进度条；可按配置分成带中文标题的章节；可用图片 / 动图作进度指示器。

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { AssetSource } from "@/lib/backend";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

const IMAGE_EXTENSIONS = ["png", "gif", "webp", "jpg", "jpeg", "bmp"];

export type BarPosition = "top" | "bottom";

export type ProgressBarParams = {
  position: BarPosition;
  /** 与画面边缘的距离（画面高度 %） */
  margin: number;
  /** 粗细（画面高度 %） */
  height: number;
  barColor: string;
  /** 已播放部分不透明度 0～100 */
  barOpacity: number;
  backgroundColor: string;
  /** 未播放部分不透明度 0～100 */
  backgroundOpacity: number;
  rounded: boolean;
  /** 章节配置：每行“时间,标题” */
  chapters: string;
  /** 标题字号（画面高度 %） */
  labelSize: number;
  labelColor: string;
  showIndicator: boolean;
  indicator: AssetSource;
  /** 指示器高度（画面高度 %） */
  indicatorSize: number;
};

function Percent({
  label,
  value,
  onChange,
  min,
  max,
  step = 0.1,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
  step?: number;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <NumberInput label={label} value={value} onChange={onChange} min={min} max={max} step={step} />
      <span className="text-xs text-muted-foreground">%</span>
    </div>
  );
}

function ProgressBarForm({ value, onChange }: ToolFormProps<ProgressBarParams>) {
  return (
    <>
      <ParamField label="位置">
        <ChoiceGroup<BarPosition>
          label="进度条位置"
          value={value.position}
          onChange={(position) => onChange({ position })}
          options={[
            { value: "bottom", label: "底部" },
            { value: "top", label: "顶部" },
          ]}
        />
      </ParamField>
      <ParamField label="边距" hint="与画面边缘的距离，画面高度的百分比。">
        <Percent label="边距" value={value.margin} onChange={(margin) => onChange({ margin })} min={0} max={45} />
      </ParamField>
      <ParamField label="粗细" hint="画面高度的百分比，至少 2 像素。">
        <Percent label="粗细" value={value.height} onChange={(height) => onChange({ height })} min={0.2} max={20} />
      </ParamField>
      <ParamField label="已播放">
        <div className="flex items-center gap-1.5">
          <ColorField label="已播放颜色" value={value.barColor} onChange={(barColor) => onChange({ barColor })} />
          <Percent
            label="已播放不透明度"
            value={value.barOpacity}
            onChange={(barOpacity) => onChange({ barOpacity })}
            min={0}
            max={100}
            step={5}
          />
        </div>
      </ParamField>
      <ParamField label="未播放" hint="颜色与不透明度；不透明度为 0 时不画背景。">
        <div className="flex items-center gap-1.5">
          <ColorField
            label="未播放颜色"
            value={value.backgroundColor}
            onChange={(backgroundColor) => onChange({ backgroundColor })}
          />
          <Percent
            label="未播放不透明度"
            value={value.backgroundOpacity}
            onChange={(backgroundOpacity) => onChange({ backgroundOpacity })}
            min={0}
            max={100}
            step={5}
          />
        </div>
      </ParamField>
      <ParamField label="圆角" hint="进度条两端画成半圆。">
        <Switch aria-label="圆角" checked={value.rounded} onCheckedChange={(rounded) => onChange({ rounded })} />
      </ParamField>
      <ParamField
        label="章节"
        hint="可选。每行一个章节“时间,标题”，时间可写秒数、分:秒或百分比；# 开头的行忽略。标题写在对应段的上方（顶部进度条时在下方），分界处画分隔线。"
      >
        <Textarea
          value={value.chapters}
          onChange={(e) => onChange({ chapters: e.target.value })}
          aria-label="章节配置"
          placeholder={"0,开场\n1:30,第一部分\n75%,总结"}
          className="min-h-20"
        />
      </ParamField>
      {value.chapters.trim() !== "" && (
        <ParamField label="标题">
          <div className="flex items-center gap-1.5">
            <ColorField label="标题颜色" value={value.labelColor} onChange={(labelColor) => onChange({ labelColor })} />
            <Percent
              label="标题字号"
              value={value.labelSize}
              onChange={(labelSize) => onChange({ labelSize })}
              min={1}
              max={20}
              step={0.5}
            />
          </div>
        </ParamField>
      )}
      <ParamField label="进度指示器" hint="跟随进度条前端移动的图片，GIF 会循环播放。">
        <Switch
          aria-label="进度指示器"
          checked={value.showIndicator}
          onCheckedChange={(showIndicator) => onChange({ showIndicator })}
        />
      </ParamField>
      {value.showIndicator && (
        <>
          <ParamField label="指示器图片">
            <AssetSourceInput
              value={value.indicator}
              onChange={(indicator) => onChange({ indicator })}
              extensions={IMAGE_EXTENSIONS}
              kindLabel="指示器"
            />
          </ParamField>
          <ParamField label="指示器大小" hint="高度为画面高度的百分比，等比缩放。">
            <Percent
              label="指示器大小"
              value={value.indicatorSize}
              onChange={(indicatorSize) => onChange({ indicatorSize })}
              min={1}
              max={50}
              step={0.5}
            />
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineTool<ProgressBarParams>({
  id: "tool.progress-bar",
  defaults: {
    position: "bottom",
    margin: 0,
    height: 1.5,
    barColor: "#f4b740",
    barOpacity: 100,
    backgroundColor: "#000000",
    backgroundOpacity: 40,
    rounded: false,
    chapters: "",
    labelSize: 3.5,
    labelColor: "#ffffff",
    showIndicator: false,
    indicator: { mode: "file", path: "" },
    indicatorSize: 6,
  },
  inputs: VIDEO_INPUTS,
  Form: ProgressBarForm,
  preview: true,
});
