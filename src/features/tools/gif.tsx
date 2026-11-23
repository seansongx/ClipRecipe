// tool.gif 生成 GIF。后端：src-tauri/src/tools/gif.rs
// 每个视频转成同名 GIF；可截取一段；高质量模式用两步调色板（慢、体积大，颜色更接近原片）。

import type { ReactNode } from "react";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type GifParams = {
  fps: number;
  /** 宽度（像素），0 = 保持源宽度 */
  width: number;
  /** 开始时间（秒） */
  start: number;
  /** 时长（秒），0 = 到结尾 */
  duration: number;
  highQuality: boolean;
};

function Unit({ children, unit }: { children: ReactNode; unit: string }) {
  return (
    <div className="flex items-center gap-1.5">
      {children}
      <span className="text-xs text-muted-foreground">{unit}</span>
    </div>
  );
}

function GifForm({ value, onChange }: ToolFormProps<GifParams>) {
  return (
    <>
      <ParamField label="帧率" hint="帧率越高越流畅，文件越大。">
        <Unit unit="fps">
          <NumberInput label="帧率" value={value.fps} onChange={(fps) => onChange({ fps })} min={1} max={50} step={1} />
        </Unit>
      </ParamField>
      <ParamField label="宽度" hint="高度按比例；0 表示保持源宽度。">
        <Unit unit="像素">
          <NumberInput
            label="宽度"
            value={value.width}
            onChange={(width) => onChange({ width })}
            min={0}
            max={4096}
            integer
          />
        </Unit>
      </ParamField>
      <ParamField label="开始">
        <Unit unit="秒">
          <NumberInput
            label="开始时间"
            value={value.start}
            onChange={(start) => onChange({ start })}
            min={0}
            step="any"
          />
        </Unit>
      </ParamField>
      <ParamField label="时长" hint="0 表示到视频结尾。">
        <Unit unit="秒">
          <NumberInput
            label="时长"
            value={value.duration}
            onChange={(duration) => onChange({ duration })}
            min={0}
            step="any"
          />
        </Unit>
      </ParamField>
      <ParamField
        label="高质量"
        hint="先为画面生成专用调色板再抖动量化，颜色接近原片；需要两遍处理，更慢、文件更大。关闭时用固定调色板，渐变处有色带。"
      >
        <Switch
          aria-label="高质量"
          checked={value.highQuality}
          onCheckedChange={(highQuality) => onChange({ highQuality })}
        />
      </ParamField>
    </>
  );
}

export default defineTool<GifParams>({
  id: "tool.gif",
  defaults: { fps: 10, width: 480, start: 0, duration: 0, highQuality: false },
  inputs: VIDEO_INPUTS,
  Form: GifForm,
});
