// tool.remove-silence 静音移除。后端：src-tauri/src/tools/remove_silence.rs
// 检测静音片段（silencedetect），移除或按倍率加速，音画保持同步。没有音轨的视频会失败。

import type { ReactNode } from "react";
import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type SilenceMode = "remove" | "speed";

export type RemoveSilenceParams = {
  /** 低于此音量（dB）视为静音 */
  noiseDb: number;
  /** 静音至少这么久（秒）才处理 */
  minDuration: number;
  /** 每段静音两端保留的时长（秒） */
  paddingSeconds: number;
  mode: SilenceMode;
  /** 加速倍率 */
  speed: number;
};

function Unit({ unit, children }: { unit: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-1.5">
      {children}
      <span className="text-xs text-muted-foreground">{unit}</span>
    </div>
  );
}

function RemoveSilenceForm({ value, onChange }: ToolFormProps<RemoveSilenceParams>) {
  return (
    <>
      <ParamField label="处理方式">
        <ChoiceGroup<SilenceMode>
          label="静音的处理方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "remove", label: "移除" },
            { value: "speed", label: "加速" },
          ]}
        />
      </ParamField>
      {value.mode === "speed" && (
        <ParamField label="倍率" hint="静音片段的画面与声音按此倍率加速播放。">
          <Unit unit="倍">
            <NumberInput
              label="加速倍率"
              value={value.speed}
              onChange={(speed) => onChange({ speed })}
              min={1.1}
              max={100}
              step={0.5}
            />
          </Unit>
        </ParamField>
      )}
      <ParamField label="静音阈值" hint="音量低于此值（dB）视为静音；有底噪时可调高到 -25。">
        <Unit unit="dB">
          <NumberInput
            label="静音阈值（dB）"
            value={value.noiseDb}
            onChange={(noiseDb) => onChange({ noiseDb })}
            min={-100}
            max={-1}
            step={1}
          />
        </Unit>
      </ParamField>
      <ParamField label="最短静音" hint="静音持续至少这么久才处理。">
        <Unit unit="秒">
          <NumberInput
            label="最短静音时长"
            value={value.minDuration}
            onChange={(minDuration) => onChange({ minDuration })}
            min={0.1}
            max={3600}
            step={0.1}
          />
        </Unit>
      </ParamField>
      <ParamField label="保留边距" hint="每段静音两端各保留这么久，避免说话的开头结尾被切得太紧。">
        <Unit unit="秒">
          <NumberInput
            label="保留边距"
            value={value.paddingSeconds}
            onChange={(paddingSeconds) => onChange({ paddingSeconds })}
            min={0}
            max={10}
            step={0.05}
          />
        </Unit>
      </ParamField>
    </>
  );
}

export default defineTool<RemoveSilenceParams>({
  id: "tool.remove-silence",
  defaults: {
    noiseDb: -30,
    minDuration: 0.5,
    paddingSeconds: 0.1,
    mode: "remove",
    speed: 4,
  },
  inputs: VIDEO_INPUTS,
  Form: RemoveSilenceForm,
});
