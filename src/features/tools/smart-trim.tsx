// tool.smart-trim 智能去头尾。后端：src-tauri/src/tools/smart_trim.rs
// 依据场景切换点判断片头片尾：开头范围内的第一个切换点之前、结尾范围内的最后一个切换点之后的部分被去掉；
// 范围内没有切换点的一端保持不变（处理记录里会说明）。

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type SmartTrimParams = {
  /** 场景切换阈值 0～1 */
  sceneThreshold: number;
  /** 片头最长（秒），0 表示不去片头 */
  maxHeadSeconds: number;
  /** 片尾最长（秒），0 表示不去片尾 */
  maxTailSeconds: number;
};

function SmartTrimForm({ value, onChange }: ToolFormProps<SmartTrimParams>) {
  return (
    <>
      <ParamField label="灵敏度" hint="场景切换阈值（0～1）：越小越敏感；常用 0.3～0.4。">
        <NumberInput
          label="场景切换阈值"
          value={value.sceneThreshold}
          onChange={(sceneThreshold) => onChange({ sceneThreshold })}
          min={0.01}
          max={1}
          step={0.05}
        />
      </ParamField>
      <ParamField label="片头范围" hint="在开头这么长的范围内找第一个场景切换点，去掉它之前的部分；0 表示不去片头。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="片头范围"
            value={value.maxHeadSeconds}
            onChange={(maxHeadSeconds) => onChange({ maxHeadSeconds })}
            min={0}
            max={3600}
            step={1}
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
      <ParamField label="片尾范围" hint="在结尾这么长的范围内找最后一个场景切换点，去掉它之后的部分；0 表示不去片尾。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="片尾范围"
            value={value.maxTailSeconds}
            onChange={(maxTailSeconds) => onChange({ maxTailSeconds })}
            min={0}
            max={3600}
            step={1}
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
    </>
  );
}

export default defineTool<SmartTrimParams>({
  id: "tool.smart-trim",
  defaults: { sceneThreshold: 0.3, maxHeadSeconds: 10, maxTailSeconds: 10 },
  inputs: VIDEO_INPUTS,
  Form: SmartTrimForm,
});
