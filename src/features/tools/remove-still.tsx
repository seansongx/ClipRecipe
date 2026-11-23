// tool.remove-still 静止帧移除。后端：src-tauri/src/tools/remove_still.rs
// 检测画面静止的片段（freezedetect），连同对应的音频一起移除，拼接剩余部分。

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type RemoveStillParams = {
  /** 灵敏度：帧间差异低于此值（dB）视为没有变化 */
  noiseDb: number;
  /** 静止至少这么久（秒）才移除 */
  minDuration: number;
};

function RemoveStillForm({ value, onChange }: ToolFormProps<RemoveStillParams>) {
  return (
    <>
      <ParamField
        label="灵敏度"
        hint="帧间差异低于此值（dB）视为静止。越接近 0 越容易判为静止（有噪点的画面可调到 -50）；默认 -60。"
      >
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="灵敏度（dB）"
            value={value.noiseDb}
            onChange={(noiseDb) => onChange({ noiseDb })}
            min={-100}
            max={-1}
            step={1}
          />
          <span className="text-xs text-muted-foreground">dB</span>
        </div>
      </ParamField>
      <ParamField label="最短静止" hint="画面静止至少这么久才移除；静止前的最后一帧会保留。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="最短静止时长"
            value={value.minDuration}
            onChange={(minDuration) => onChange({ minDuration })}
            min={0.1}
            max={3600}
            step={0.5}
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
    </>
  );
}

export default defineTool<RemoveStillParams>({
  id: "tool.remove-still",
  defaults: { noiseDb: -60, minDuration: 1 },
  inputs: VIDEO_INPUTS,
  Form: RemoveStillForm,
});
