// tool.reverse 视频倒放。后端：src-tauri/src/tools/reverse.rs
// 画面倒放，音频一起倒放或去掉；长视频分段倒放再逆序拼接，避免内存耗尽。

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type ReverseParams = {
  /** 去掉音频（否则音频一起倒放） */
  removeAudio: boolean;
  /** 每段时长（秒） */
  chunkSeconds: number;
};

function ReverseForm({ value, onChange }: ToolFormProps<ReverseParams>) {
  return (
    <>
      <ParamField label="去掉音频" hint="关闭时音频与画面一起倒放。">
        <Switch
          aria-label="去掉音频"
          checked={value.removeAudio}
          onCheckedChange={(removeAudio) => onChange({ removeAudio })}
        />
      </ParamField>
      <ParamField
        label="分段时长"
        hint="倒放需要把整段画面放进内存，长视频按这个时长分段倒放后再拼接。越短内存占用越少；1080p 每秒约占 100 MB。"
      >
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="分段时长"
            value={value.chunkSeconds}
            onChange={(chunkSeconds) => onChange({ chunkSeconds })}
            min={0.5}
            max={120}
            step={0.5}
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
    </>
  );
}

export default defineTool<ReverseParams>({
  id: "tool.reverse",
  defaults: { removeAudio: false, chunkSeconds: 3 },
  inputs: VIDEO_INPUTS,
  Form: ReverseForm,
});
