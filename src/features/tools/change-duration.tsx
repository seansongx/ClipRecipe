// tool.change-duration 改变视频时长（只改容器里报告的时长）。后端：src-tauri/src/tools/change_duration.rs

import ChoiceGroup from "@/components/ChoiceGroup";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import type { Range } from "@/lib/backend";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type DurationMode = "multiplier" | "absolute";

export type ChangeDurationParams = {
  mode: DurationMode;
  /** 实际时长的倍数 */
  multiplier: Range;
  /** 指定时长（秒） */
  seconds: Range;
};

/** 秒 → 时:分:秒。 */
function clock(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`;
}

function ChangeDurationForm({ value, onChange }: ToolFormProps<ChangeDurationParams>) {
  const { min, max } = value.seconds;
  return (
    <>
      <p className="pb-1 text-[11px] leading-snug text-muted-foreground">
        只改写 MP4 / MOV 文件头里记录的总时长（mvhd），画面和声音逐字节保持不变、不重编码。
        资源管理器、MediaInfo 等读取文件头的程序会显示新时长；FFmpeg / ffprobe（以及用它解析的网站）按实际数据计算，
        仍显示真实时长。其它格式（mkv 等）不支持。
      </p>
      <ParamField label="方式">
        <ChoiceGroup<DurationMode>
          label="时长设置方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "multiplier", label: "实际时长的倍数" },
            { value: "absolute", label: "指定时长" },
          ]}
        />
      </ParamField>
      {value.mode === "multiplier" ? (
        <ParamField label="倍数" hint="如 2 表示显示为实际时长的两倍；两端不同时每个视频随机取值。">
          <RangeInput
            value={value.multiplier}
            onChange={(multiplier) => onChange({ multiplier })}
            min={0.01}
            max={100}
            step="any"
            unit="倍"
          />
        </ParamField>
      ) : (
        <ParamField
          label="时长"
          hint={`即 ${min === max ? clock(min) : `${clock(Math.min(min, max))} ～ ${clock(Math.max(min, max))}`}；两端不同时每个视频随机取值。`}
        >
          <RangeInput
            value={value.seconds}
            onChange={(seconds) => onChange({ seconds })}
            min={0.1}
            max={360000}
            step="any"
            unit="秒"
          />
        </ParamField>
      )}
    </>
  );
}

export default defineTool<ChangeDurationParams>({
  id: "tool.change-duration",
  defaults: {
    mode: "multiplier",
    multiplier: { min: 2, max: 2 },
    seconds: { min: 600, max: 600 },
  },
  inputs: VIDEO_INPUTS,
  Form: ChangeDurationForm,
});
