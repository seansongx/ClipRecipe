// tool.insert-frame 插帧。后端：src-tauri/src/tools/insert_frame.rs
// 在指定位置把图片全屏显示若干帧：覆盖原画面（时长不变）或插入新帧（时长变长，插入处静音）。

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Input } from "@/components/ui/input";
import type { AssetSource } from "@/lib/backend";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

export type InsertMode = "overlay" | "insert";
export type ImageFit = "fit" | "fill" | "stretch";

export type InsertFrameParams = {
  image: AssetSource;
  /** 位置：秒、分:秒或百分比，逗号 / 空格分隔 */
  positions: string;
  /** 显示帧数 1～1000 */
  frames: number;
  fit: ImageFit;
  /** 完整显示时的背景色 */
  background: string;
  mode: InsertMode;
};

const FIT_HINTS: Record<ImageFit, string> = {
  fit: "等比缩放完整显示，空白处填背景色。",
  fill: "等比放大铺满画面，裁去多余部分。",
  stretch: "拉伸到画面尺寸，图片可能变形。",
};

function InsertFrameForm({ value, onChange }: ToolFormProps<InsertFrameParams>) {
  return (
    <>
      <ParamField label="图片" hint="目录模式下每个视频随机或依次取一张。">
        <AssetSourceInput
          value={value.image}
          onChange={(image) => onChange({ image })}
          extensions={IMAGE_EXTENSIONS}
          kindLabel="图片"
        />
      </ParamField>
      <ParamField
        label="位置"
        hint="多个位置用逗号或空格分隔；可写秒数（12.5）、分:秒（1:05）或百分比（50%）。"
      >
        <Input
          value={value.positions}
          onChange={(e) => onChange({ positions: e.target.value })}
          aria-label="插入位置"
          placeholder="如 3, 50%, 1:20"
          className="h-7 w-40 text-xs md:text-xs"
        />
      </ParamField>
      <ParamField label="显示帧数">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="显示帧数"
            value={value.frames}
            onChange={(frames) => onChange({ frames })}
            min={1}
            max={1000}
            integer
          />
          <span className="text-xs text-muted-foreground">帧</span>
        </div>
      </ParamField>
      <ParamField
        label="方式"
        hint={
          value.mode === "overlay"
            ? "图片盖住该位置起的若干帧，时长不变。"
            : "在该位置之前插入图片帧，原画面顺延，视频变长，插入处静音。"
        }
      >
        <ChoiceGroup<InsertMode>
          label="插帧方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "overlay", label: "覆盖" },
            { value: "insert", label: "插入" },
          ]}
        />
      </ParamField>
      <ParamField label="适配" hint={FIT_HINTS[value.fit]}>
        <ChoiceGroup<ImageFit>
          label="图片适配"
          value={value.fit}
          onChange={(fit) => onChange({ fit })}
          options={[
            { value: "fit", label: "完整" },
            { value: "fill", label: "铺满" },
            { value: "stretch", label: "拉伸" },
          ]}
        />
      </ParamField>
      {value.fit === "fit" && (
        <ParamField label="背景色">
          <ColorField
            label="背景色"
            value={value.background}
            onChange={(background) => onChange({ background })}
          />
        </ParamField>
      )}
    </>
  );
}

export default defineTool<InsertFrameParams>({
  id: "tool.insert-frame",
  defaults: {
    image: { mode: "file", path: "" },
    positions: "50%",
    frames: 1,
    fit: "fit",
    background: "#000000",
    mode: "overlay",
  },
  inputs: VIDEO_INPUTS,
  Form: InsertFrameForm,
  preview: true,
});
