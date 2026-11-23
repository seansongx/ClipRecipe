// 按场景切换点分段的公共参数与表单（镜像重组、转场重组等共用）。

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { WithUnit } from "./with-unit";

/** 场景分段的公共参数（平铺在各工具参数里，与后端字段一致）。 */
export type SceneParams = {
  /** 场景切换阈值 0～1 */
  sceneThreshold: number;
  /** 最短片段（秒） */
  minSegmentSeconds: number;
  /** 没有场景切换时每段的时长（秒） */
  fallbackSeconds: number;
};

/** 场景分段设置：灵敏度、最短片段、无切换点时的分段时长。 */
export function SceneFields({
  value,
  onChange,
  minSegmentHint,
}: {
  value: SceneParams;
  onChange: (patch: Partial<SceneParams>) => void;
  minSegmentHint?: string;
}) {
  return (
    <>
      <ParamField label="灵敏度" hint="场景切换阈值（0～1）：越小越敏感，分出的片段越多；常用 0.3～0.4。">
        <NumberInput
          label="场景切换阈值"
          value={value.sceneThreshold}
          onChange={(sceneThreshold) => onChange({ sceneThreshold })}
          min={0.01}
          max={1}
          step={0.05}
        />
      </ParamField>
      <ParamField label="最短片段" hint={minSegmentHint ?? "相邻切换点至少间隔这么久，更近的切换点被忽略。"}>
        <WithUnit unit="秒">
          <NumberInput
            label="最短片段"
            value={value.minSegmentSeconds}
            onChange={(minSegmentSeconds) => onChange({ minSegmentSeconds })}
            min={0}
            max={3600}
            step="any"
          />
        </WithUnit>
      </ParamField>
      <ParamField label="无切换时" hint="没有检测到场景切换时，改为每隔这么久分一段（会记在处理记录里）。">
        <WithUnit unit="秒">
          <NumberInput
            label="无场景切换时的分段时长"
            value={value.fallbackSeconds}
            onChange={(fallbackSeconds) => onChange({ fallbackSeconds })}
            min={0.5}
            max={3600}
            step="any"
          />
        </WithUnit>
      </ParamField>
    </>
  );
}
