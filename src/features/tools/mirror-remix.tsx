// tool.mirror-remix 镜像重组。后端：src-tauri/src/tools/mirror_remix.rs
// 按场景切换点分段，部分片段水平翻转后按原顺序拼回；音频原样保留。
// 场景分段设置（SceneFields）在 shared/scene-fields.tsx，与转场重组共用。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { SceneFields, type SceneParams } from "./shared/scene-fields";
import { WithUnit } from "./shared/with-unit";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

// 兼容旧的导入位置：这些公共部件已移到 ./shared/scene-fields
export { SceneFields, type SceneParams } from "./shared/scene-fields";

export type FlipMode = "ratio" | "nth" | "random";

export type MirrorRemixParams = SceneParams & {
  flipMode: FlipMode;
  /** 按比例：翻转的片段占比 0～1 */
  ratio: number;
  /** 每隔 N 段翻转一段 */
  nth: number;
};

function MirrorRemixForm({ value, onChange }: ToolFormProps<MirrorRemixParams>) {
  return (
    <>
      <SceneFields value={value} onChange={onChange} />
      <ParamField
        label="翻转"
        hint={
          value.flipMode === "ratio"
            ? "随机挑出这个比例的片段水平翻转。"
            : value.flipMode === "nth"
              ? "第 N、2N、3N… 段翻转。"
              : "每段各有一半的概率被翻转。"
        }
      >
        <ChoiceGroup<FlipMode>
          label="翻转方式"
          value={value.flipMode}
          onChange={(flipMode) => onChange({ flipMode })}
          options={[
            { value: "ratio", label: "按比例" },
            { value: "nth", label: "每隔 N 段" },
            { value: "random", label: "随机" },
          ]}
        />
      </ParamField>
      {value.flipMode === "ratio" && (
        <ParamField label="比例">
          <WithUnit unit="%">
            <NumberInput
              label="翻转比例（百分比）"
              value={Math.round(value.ratio * 100)}
              onChange={(percent) => onChange({ ratio: percent / 100 })}
              min={0}
              max={100}
              integer
            />
          </WithUnit>
        </ParamField>
      )}
      {value.flipMode === "nth" && (
        <ParamField label="N">
          <NumberInput
            label="每隔 N 段"
            value={value.nth}
            onChange={(nth) => onChange({ nth })}
            min={1}
            max={1000}
            integer
          />
        </ParamField>
      )}
    </>
  );
}

export default defineTool<MirrorRemixParams>({
  id: "tool.mirror-remix",
  defaults: {
    sceneThreshold: 0.3,
    minSegmentSeconds: 1,
    fallbackSeconds: 5,
    flipMode: "ratio",
    ratio: 0.5,
    nth: 2,
  },
  inputs: VIDEO_INPUTS,
  Form: MirrorRemixForm,
  preview: true,
});
