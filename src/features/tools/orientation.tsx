// tool.orientation 横竖互转。后端：src-tauri/src/tools/orientation.rs
// 文件夹内视频批量横竖屏转换：视频按比例完整放进目标画面，空白处填充模糊画面、纯色或图片。

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { AssetSource } from "@/lib/backend";
import { defineTool, type ToolFormProps } from "./types";

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp"];

export type OrientationTarget = "swap" | "portrait" | "landscape" | "custom";
export type OrientationFill = "blur" | "color" | "image";

export type OrientationParams = {
  target: OrientationTarget;
  /** 自定义宽高（像素） */
  width: number;
  height: number;
  fill: OrientationFill;
  /** 纯色填充 #rrggbb */
  color: string;
  image: AssetSource;
  /** 模糊半径（像素） */
  blurStrength: number;
};

const TARGETS: { value: OrientationTarget; label: string }[] = [
  { value: "swap", label: "宽高互换（横 ↔ 竖）" },
  { value: "portrait", label: "竖屏 1080×1920" },
  { value: "landscape", label: "横屏 1920×1080" },
  { value: "custom", label: "自定义" },
];

function OrientationForm({ value, onChange }: ToolFormProps<OrientationParams>) {
  return (
    <>
      <ParamField label="目标尺寸" hint="宽高互换：1920×1080 变成 1080×1920，竖屏同理。">
        <Select items={TARGETS} value={value.target} onValueChange={(target) => target && onChange({ target })}>
          <SelectTrigger aria-label="目标尺寸" className="w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {TARGETS.map((o) => (
              <SelectItem key={o.value} value={o.value}>
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      {value.target === "custom" && (
        <ParamField label="宽 × 高" hint="奇数会减 1 取偶。">
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="宽度"
              value={value.width}
              onChange={(width) => onChange({ width })}
              min={16}
              max={7680}
              integer
            />
            <span className="text-xs text-muted-foreground">×</span>
            <NumberInput
              label="高度"
              value={value.height}
              onChange={(height) => onChange({ height })}
              min={16}
              max={7680}
              integer
            />
          </div>
        </ParamField>
      )}
      <ParamField label="空白填充">
        <ChoiceGroup<OrientationFill>
          label="空白填充方式"
          value={value.fill}
          onChange={(fill) => onChange({ fill })}
          options={[
            { value: "blur", label: "模糊画面" },
            { value: "color", label: "纯色" },
            { value: "image", label: "图片" },
          ]}
        />
      </ParamField>
      {value.fill === "blur" && (
        <ParamField label="模糊强度" hint="高斯模糊半径（像素），越大越模糊。">
          <NumberInput
            label="模糊强度"
            value={value.blurStrength}
            onChange={(blurStrength) => onChange({ blurStrength })}
            min={1}
            max={200}
            step={1}
          />
        </ParamField>
      )}
      {value.fill === "color" && (
        <ParamField label="颜色">
          <ColorField label="填充颜色" value={value.color} onChange={(color) => onChange({ color })} />
        </ParamField>
      )}
      {value.fill === "image" && (
        <ParamField label="图片" hint="放大铺满画面并居中裁切；选目录时每个视频随机或依次取一张。">
          <AssetSourceInput
            value={value.image}
            onChange={(image) => onChange({ image })}
            extensions={IMAGE_EXTENSIONS}
            kindLabel="图片"
          />
        </ParamField>
      )}
    </>
  );
}

export default defineTool<OrientationParams>({
  id: "tool.orientation",
  defaults: {
    target: "swap",
    width: 1080,
    height: 1920,
    fill: "blur",
    color: "#000000", // token-ignore：视频参数默认值，不是界面颜色
    image: { mode: "file", path: "" },
    blurStrength: 20,
  },
  inputs: { videos: false, folders: [{ key: "source", label: "视频文件夹" }] },
  Form: OrientationForm,
});
