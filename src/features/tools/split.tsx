// tool.split 视频分割。后端：src-tauri/src/tools/split.rs
// 参考实现（一进多出 + 先分析后处理）：按时长 / 段数 / 场景，重编码或无损。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type SplitMode = "duration" | "count" | "scene";

export type SplitParams = {
  mode: SplitMode;
  /** 按时长：每段秒数 */
  segmentSeconds: number;
  /** 按段数：段数 */
  count: number;
  /** 无损（不重编码，切点落在关键帧上） */
  lossless: boolean;
  /** 按场景：场景切换阈值 0～1 */
  sceneThreshold: number;
  /** 最短片段（秒） */
  minSegmentSeconds: number;
};

function SplitForm({ value, onChange }: ToolFormProps<SplitParams>) {
  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<SplitMode>
          label="分割方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "duration", label: "按时长" },
            { value: "count", label: "按段数" },
            { value: "scene", label: "按场景" },
          ]}
        />
      </ParamField>
      {value.mode === "duration" && (
        <ParamField label="每段时长" hint="末段短于“最短片段”时并入上一段。">
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="每段时长"
              value={value.segmentSeconds}
              onChange={(segmentSeconds) => onChange({ segmentSeconds })}
              min={0.1}
              max={86400}
              step="any"
            />
            <span className="text-xs text-muted-foreground">秒</span>
          </div>
        </ParamField>
      )}
      {value.mode === "count" && (
        <ParamField label="段数" hint="平均分成这么多段。">
          <NumberInput
            label="段数"
            value={value.count}
            onChange={(count) => onChange({ count })}
            min={2}
            max={1000}
            integer
          />
        </ParamField>
      )}
      {value.mode === "scene" && (
        <ParamField label="灵敏度" hint="场景切换阈值（0～1）：越小越敏感，切出的段越多；常用 0.3～0.4。">
          <NumberInput
            label="场景切换阈值"
            value={value.sceneThreshold}
            onChange={(sceneThreshold) => onChange({ sceneThreshold })}
            min={0.01}
            max={1}
            step={0.05}
          />
        </ParamField>
      )}
      {value.mode !== "count" && (
        <ParamField
          label="最短片段"
          hint={value.mode === "scene" ? "相邻切换点至少间隔这么久，更近的切换点被忽略。" : undefined}
        >
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="最短片段"
              value={value.minSegmentSeconds}
              onChange={(minSegmentSeconds) => onChange({ minSegmentSeconds })}
              min={0}
              max={3600}
              step="any"
            />
            <span className="text-xs text-muted-foreground">秒</span>
          </div>
        </ParamField>
      )}
      <ParamField
        label="无损分割"
        hint="不重编码、速度快，但只能在关键帧处切开，时长会有误差。默认重编码，切点精确。"
      >
        <Switch aria-label="无损分割" checked={value.lossless} onCheckedChange={(lossless) => onChange({ lossless })} />
      </ParamField>
    </>
  );
}

export default defineTool<SplitParams>({
  id: "tool.split",
  defaults: {
    mode: "duration",
    segmentSeconds: 60,
    count: 2,
    lossless: false,
    sceneThreshold: 0.3,
    minSegmentSeconds: 1,
  },
  inputs: VIDEO_INPUTS,
  Form: SplitForm,
});
