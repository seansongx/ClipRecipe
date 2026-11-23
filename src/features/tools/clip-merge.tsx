// tool.clip-merge 截取合并。后端：src-tauri/src/tools/clip_merge.rs（合并引擎在 merge.rs）。

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { VIDEO_EXTENSIONS, type AssetSource, type Range } from "@/lib/backend";
import { WithUnit } from "./shared/with-unit";
import { defineTool, type ToolFormProps } from "./types";

export type ClipPosition = "head" | "middle" | "tail" | "random";
export type ClipOutput = "single" | "target";

export type ClipMergeParams = {
  /** 每个视频截取的长度（秒，范围值） */
  segmentSeconds: Range;
  position: ClipPosition;
  shuffle: boolean;
  outputMode: ClipOutput;
  /** 目标总时长（秒） */
  targetSeconds: number;
  /** 截去超出长度（启用后中间视频、片头片尾不可用） */
  trimExcess: boolean;
  middleEnabled: boolean;
  middle: AssetSource;
  introEnabled: boolean;
  intro: AssetSource;
  outroEnabled: boolean;
  outro: AssetSource;
};

type ExtraKey = "middle" | "intro" | "outro";

const EXTRAS: { key: ExtraKey; enabled: "middleEnabled" | "introEnabled" | "outroEnabled"; label: string; hint: string }[] = [
  { key: "intro", enabled: "introEnabled", label: "片头", hint: "加在每个输出的开头。" },
  { key: "middle", enabled: "middleEnabled", label: "中间视频", hint: "插入在相邻片段之间；目录时每处各取一个。" },
  { key: "outro", enabled: "outroEnabled", label: "片尾", hint: "加在每个输出的结尾。" },
];

function ClipMergeForm({ value, onChange }: ToolFormProps<ClipMergeParams>) {
  const needsTarget = value.outputMode === "target" || value.trimExcess;
  return (
    <>
      <ParamField label="片段长度" hint="每个视频（含子文件夹）截取一段；比最小值还短的视频跳过。">
        <RangeInput
          value={value.segmentSeconds}
          onChange={(segmentSeconds) => onChange({ segmentSeconds })}
          min={0.5}
          max={3600}
          step={0.1}
          unit="秒"
        />
      </ParamField>
      <ParamField label="截取位置">
        <ChoiceGroup<ClipPosition>
          label="截取位置"
          value={value.position}
          onChange={(position) => onChange({ position })}
          options={[
            { value: "head", label: "开头" },
            { value: "middle", label: "中间" },
            { value: "tail", label: "结尾" },
            { value: "random", label: "随机" },
          ]}
        />
      </ParamField>
      <ParamField label="随机顺序" hint="按随机种子打乱视频顺序；关闭时按文件名顺序。">
        <Switch aria-label="随机顺序" checked={value.shuffle} onCheckedChange={(shuffle) => onChange({ shuffle })} />
      </ParamField>
      <Separator className="my-1" />
      <ParamField
        label="输出"
        hint={
          value.outputMode === "target"
            ? "依次累加片段，达到目标时长就开始下一个视频（文件夹名_01、_02…）。"
            : "全部片段拼成一个视频，以文件夹命名。"
        }
      >
        <ChoiceGroup<ClipOutput>
          label="输出方式"
          value={value.outputMode}
          onChange={(outputMode) => onChange({ outputMode })}
          options={[
            { value: "single", label: "合成一个" },
            { value: "target", label: "按目标时长" },
          ]}
        />
      </ParamField>
      <ParamField label="截去超出" hint="把每个输出裁到正好目标时长。开启后中间视频、片头、片尾不可用。">
        <Switch
          aria-label="截去超出长度"
          checked={value.trimExcess}
          onCheckedChange={(trimExcess) => onChange({ trimExcess })}
        />
      </ParamField>
      {needsTarget && (
        <ParamField label="目标时长">
          <WithUnit unit="秒">
            <NumberInput
              label="目标时长"
              value={value.targetSeconds}
              onChange={(targetSeconds) => onChange({ targetSeconds })}
              min={1}
              max={86400}
              step="any"
            />
          </WithUnit>
        </ParamField>
      )}
      {!value.trimExcess && (
        <>
          <Separator className="my-1" />
          {EXTRAS.map((extra) => (
            <div key={extra.key}>
              <ParamField label={extra.label} hint={extra.hint}>
                <Switch
                  aria-label={`添加${extra.label}`}
                  checked={value[extra.enabled]}
                  onCheckedChange={(on) => {
                    const patch: Partial<ClipMergeParams> = {};
                    patch[extra.enabled] = on;
                    onChange(patch);
                  }}
                />
              </ParamField>
              {value[extra.enabled] && (
                <AssetSourceInput
                  value={value[extra.key]}
                  onChange={(source) => {
                    const patch: Partial<ClipMergeParams> = {};
                    patch[extra.key] = source;
                    onChange(patch);
                  }}
                  extensions={VIDEO_EXTENSIONS}
                  kindLabel={extra.label}
                />
              )}
            </div>
          ))}
        </>
      )}
    </>
  );
}

const EMPTY: AssetSource = { mode: "file", path: "" };

export default defineTool<ClipMergeParams>({
  id: "tool.clip-merge",
  defaults: {
    segmentSeconds: { min: 3, max: 5 },
    position: "random",
    shuffle: false,
    outputMode: "single",
    targetSeconds: 60,
    trimExcess: false,
    middleEnabled: false,
    middle: EMPTY,
    introEnabled: false,
    intro: EMPTY,
    outroEnabled: false,
    outro: EMPTY,
  },
  inputs: { videos: false, folders: [{ key: "source", label: "视频文件夹" }] },
  Form: ClipMergeForm,
});
