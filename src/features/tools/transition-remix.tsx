// tool.transition-remix 转场重组。后端：src-tauri/src/tools/transition_remix.rs
// 按场景切换点分段，相邻片段之间加转场（指定一种或每处随机），再拼成一个视频；转场处前后重叠，输出会变短。

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { SceneFields, type SceneParams } from "./shared/scene-fields";
import { WithUnit } from "./shared/with-unit";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type TransitionRemixParams = SceneParams & {
  /** "random" 或 TRANSITIONS 中的一种（xfade 的转场名） */
  transition: string;
  /** 转场时长（秒） */
  transitionSeconds: number;
};

/** 与后端 TRANSITIONS 一致（随机时从中挑选）。 */
const TRANSITIONS: { value: string; label: string }[] = [
  { value: "fade", label: "淡化" },
  { value: "dissolve", label: "溶解" },
  { value: "fadeblack", label: "黑场过渡" },
  { value: "fadewhite", label: "白场过渡" },
  { value: "wipeleft", label: "向左擦除" },
  { value: "wiperight", label: "向右擦除" },
  { value: "wipeup", label: "向上擦除" },
  { value: "wipedown", label: "向下擦除" },
  { value: "slideleft", label: "向左滑动" },
  { value: "slideright", label: "向右滑动" },
  { value: "slideup", label: "向上滑动" },
  { value: "slidedown", label: "向下滑动" },
  { value: "smoothleft", label: "平滑左移" },
  { value: "smoothright", label: "平滑右移" },
  { value: "circleopen", label: "圆形展开" },
  { value: "circleclose", label: "圆形收拢" },
  { value: "radial", label: "径向扫过" },
  { value: "pixelize", label: "像素化" },
  { value: "hblur", label: "水平模糊" },
  { value: "zoomin", label: "放大" },
];

function TransitionRemixForm({ value, onChange }: ToolFormProps<TransitionRemixParams>) {
  return (
    <>
      <SceneFields
        value={value}
        onChange={onChange}
        minSegmentHint="相邻切换点至少间隔这么久（且不少于两倍转场时长），更近的切换点被忽略。"
      />
      <ParamField label="转场" hint="“每处随机”时每个衔接处各随机选一种（随种子可复现）。">
        <Select value={value.transition} onValueChange={(transition) => onChange({ transition })}>
          <SelectTrigger size="sm" aria-label="转场效果" className="h-7 w-36 px-2 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="random" className="text-xs">
              每处随机
            </SelectItem>
            {TRANSITIONS.map((t) => (
              <SelectItem key={t.value} value={t.value} className="text-xs">
                {t.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      <ParamField label="转场时长" hint="前后两段重叠这么久，每个转场让输出缩短同样的时长，声音同步交叉淡化。">
        <WithUnit unit="秒">
          <NumberInput
            label="转场时长"
            value={value.transitionSeconds}
            onChange={(transitionSeconds) => onChange({ transitionSeconds })}
            min={0.1}
            max={5}
            step={0.1}
          />
        </WithUnit>
      </ParamField>
    </>
  );
}

export default defineTool<TransitionRemixParams>({
  id: "tool.transition-remix",
  defaults: {
    sceneThreshold: 0.3,
    minSegmentSeconds: 2,
    fallbackSeconds: 5,
    transition: "random",
    transitionSeconds: 1,
  },
  inputs: VIDEO_INPUTS,
  Form: TransitionRemixForm,
  preview: true,
});
