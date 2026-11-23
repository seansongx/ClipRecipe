// pipe.dedupe-frames 重复帧移除。后端：src-tauri/src/pipeline/modules/dedupe_frames.rs

import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import { defineModule, type ModuleFormProps } from "./types";

export type DedupeSensitivity = "low" | "normal" | "high";

export interface DedupeFramesParams {
  enabled: boolean;
  /** 判断“重复”的灵敏度 */
  sensitivity: DedupeSensitivity;
  /** 最多连续去掉的帧数，0 表示不限制 */
  maxDrop: number;
}

function DedupeFramesForm({ value, onChange }: ModuleFormProps<DedupeFramesParams>) {
  return (
    <>
      <ParamField label="灵敏度" hint="越高，细微变化的相邻帧也会被当作重复帧去掉。">
        <ChoiceGroup<DedupeSensitivity>
          value={value.sensitivity}
          onChange={(sensitivity) => onChange({ sensitivity })}
          options={[
            { value: "low", label: "低" },
            { value: "normal", label: "标准" },
            { value: "high", label: "高" },
          ]}
        />
      </ParamField>
      <ParamField label="连续上限" hint="最多连续去掉多少帧，0 表示不限制。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            value={value.maxDrop}
            onChange={(maxDrop) => onChange({ maxDrop })}
            min={0}
            max={1000}
            integer
            label="连续上限"
          />
          <span className="text-xs text-muted-foreground">帧</span>
        </div>
      </ParamField>
      <p className="pt-1 text-xs text-muted-foreground">
        去掉的帧由前一帧延长显示，时长和音频不变；启用“帧率”或“变速”时会重新输出恒定帧率。
      </p>
    </>
  );
}

export default defineModule<DedupeFramesParams>({
  id: "pipe.dedupe-frames",
  defaults: {
    enabled: false,
    sensitivity: "normal",
    maxDrop: 0,
  },
  Form: DedupeFramesForm,
});
