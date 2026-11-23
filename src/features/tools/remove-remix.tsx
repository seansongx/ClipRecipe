// tool.remove-remix 移除重组。后端：src-tauri/src/tools/remove_remix.rs
// 在设定位置起移除一段（长度为范围值，每处随机取），拼接剩余部分。位置设置与拆分重组共用（shared/position-fields.tsx）。

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import type { Range } from "@/lib/backend";
import { PositionFields, type PositionParams } from "./shared/position-fields";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type RemoveRemixParams = PositionParams & {
  /** 每处移除的长度（秒） */
  length: Range;
};

function RemoveRemixForm({ value, onChange }: ToolFormProps<RemoveRemixParams>) {
  return (
    <>
      <PositionFields value={value} onChange={onChange} verb="开始移除" />
      <ParamField label="移除长度" hint="从每个位置起移除这么长；最小与最大不同时每处随机取。">
        <RangeInput
          value={value.length}
          onChange={(length) => onChange({ length })}
          min={0.04}
          max={3600}
          step={0.1}
          unit="秒"
        />
      </ParamField>
    </>
  );
}

export default defineTool<RemoveRemixParams>({
  id: "tool.remove-remix",
  defaults: {
    positionMode: "percents",
    times: [10],
    percents: [50],
    parts: 2,
    length: { min: 1, max: 1 },
  },
  inputs: VIDEO_INPUTS,
  Form: RemoveRemixForm,
});
