// pipe.zoom 动态缩放。后端：src-tauri/src/pipeline/modules/zoom.rs

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import type { Range } from "@/lib/backend";
import { range } from "./summary";
import { defineModule, type ModuleFormProps } from "./types";

export interface ZoomParams {
  enabled: boolean;
  /** 缩放倍数的往复区间（不是随机取值） */
  scale: Range;
  /** 往复周期（秒），每个视频随机取值 */
  period: Range;
}

function ZoomForm({ value, onChange }: ModuleFormProps<ZoomParams>) {
  return (
    <>
      <ParamField label="缩放倍数" hint="画面在两个倍数之间平滑往复；小于 1 为缩小（四周补黑）。输出尺寸不变。">
        <RangeInput value={value.scale} onChange={(scale) => onChange({ scale })} min={0.5} max={3} step={0.05} unit="×" />
      </ParamField>
      <ParamField label="周期" hint="一次放大再缩小所用的时间，每个视频在范围内随机取值。">
        <RangeInput
          value={value.period}
          onChange={(period) => onChange({ period })}
          min={0.5}
          max={60}
          step={0.5}
          unit="秒"
        />
      </ParamField>
    </>
  );
}

export default defineModule<ZoomParams>({
  id: "pipe.zoom",
  defaults: {
    enabled: false,
    scale: { min: 1, max: 1.2 },
    period: { min: 4, max: 4 },
  },
  Form: ZoomForm,
  summary: (p) => `${range(p.scale, " 倍")}往复 · 周期 ${range(p.period, " 秒")}`,
});
