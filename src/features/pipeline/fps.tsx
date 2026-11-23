// pipe.fps 帧率。后端：src-tauri/src/pipeline/modules/fps.rs
// 这是一个完整的参考实现：参数类型 + 默认值 + 表单。

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export interface FpsParams {
  enabled: boolean;
  /** 目标帧率，取值保留两位小数 */
  fps: Range;
}

function FpsForm({ value, onChange }: ModuleFormProps<FpsParams>) {
  return (
    <ParamField label="帧率" hint="每个视频在范围内随机取值；两端相同即固定帧率。">
      <RangeInput value={value.fps} onChange={(fps) => onChange({ fps })} min={1} max={240} step="any" unit="fps" />
    </ParamField>
  );
}

export default defineModule<FpsParams>({
  id: "pipe.fps",
  defaults: { enabled: true, fps: { min: 30, max: 30 } },
  Form: FpsForm,
});
