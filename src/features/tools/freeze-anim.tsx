// tool.freeze-anim 定帧动画。后端：src-tauri/src/tools/freeze_anim.rs
// 定格动画：每段只显示该段第一帧（时长不变）；插入停顿：在定帧点停住一段时间再继续（时长变长）。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type FreezeAnimMode = "stopMotion" | "insertHolds";
export type FreezeSpacing = "interval" | "count";
export type HoldAudio = "silence" | "continue";

export type FreezeAnimParams = {
  mode: FreezeAnimMode;
  /** 定帧点的分布：固定间隔 / 平均分成 N 段 */
  spacing: FreezeSpacing;
  /** 按数量：定帧点个数 1～1000 */
  count: number;
  /** 按间隔：秒 */
  intervalSeconds: number;
  /** 插入停顿：每次停顿秒数 */
  holdSeconds: number;
  /** 插入停顿：停顿期间的音频 */
  holdAudio: HoldAudio;
};

function Seconds({
  label,
  value,
  onChange,
  min,
  max,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
  min: number;
  max: number;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <NumberInput label={label} value={value} onChange={onChange} min={min} max={max} step="any" />
      <span className="text-xs text-muted-foreground">秒</span>
    </div>
  );
}

function FreezeAnimForm({ value, onChange }: ToolFormProps<FreezeAnimParams>) {
  const insert = value.mode === "insertHolds";
  return (
    <>
      <ParamField
        label="效果"
        hint={
          insert
            ? "播放到每个定帧点时停住一段时间再继续，视频会变长。"
            : "每个定帧点到下一个定帧点之间只显示第一帧，形成定格动画；时长与音频不变。"
        }
      >
        <ChoiceGroup<FreezeAnimMode>
          label="定帧效果"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "stopMotion", label: "定格动画" },
            { value: "insertHolds", label: "插入停顿" },
          ]}
        />
      </ParamField>
      <ParamField label="定帧点" hint="从开头起算；按数量时把视频平均分段，每段开头一个定帧点。">
        <ChoiceGroup<FreezeSpacing>
          label="定帧点分布"
          value={value.spacing}
          onChange={(spacing) => onChange({ spacing })}
          options={[
            { value: "interval", label: "按间隔" },
            { value: "count", label: "按数量" },
          ]}
        />
      </ParamField>
      {value.spacing === "interval" ? (
        <ParamField label="间隔">
          <Seconds
            label="定帧间隔"
            value={value.intervalSeconds}
            onChange={(intervalSeconds) => onChange({ intervalSeconds })}
            min={0.05}
            max={3600}
          />
        </ParamField>
      ) : (
        <ParamField label="数量">
          <NumberInput
            label="定帧点数量"
            value={value.count}
            onChange={(count) => onChange({ count })}
            min={1}
            max={1000}
            integer
          />
        </ParamField>
      )}
      {insert && (
        <>
          <ParamField label="停顿时长" hint="按帧取整，至少 1 帧。">
            <Seconds
              label="停顿时长"
              value={value.holdSeconds}
              onChange={(holdSeconds) => onChange({ holdSeconds })}
              min={0.01}
              max={60}
            />
          </ParamField>
          <ParamField
            label="停顿时的声音"
            hint={
              value.holdAudio === "silence"
                ? "停顿处插入静音，音画始终同步。"
                : "音频不停照常播放，结尾补静音；停顿之后音画不再对齐。"
            }
          >
            <ChoiceGroup<HoldAudio>
              label="停顿时的声音"
              value={value.holdAudio}
              onChange={(holdAudio) => onChange({ holdAudio })}
              options={[
                { value: "silence", label: "静音" },
                { value: "continue", label: "继续播放" },
              ]}
            />
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineTool<FreezeAnimParams>({
  id: "tool.freeze-anim",
  defaults: {
    mode: "stopMotion",
    spacing: "interval",
    count: 10,
    intervalSeconds: 0.5,
    holdSeconds: 1,
    holdAudio: "silence",
  },
  inputs: VIDEO_INPUTS,
  Form: FreezeAnimForm,
  preview: true,
});
