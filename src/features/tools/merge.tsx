// tool.merge 视频合并。后端：src-tauri/src/tools/merge.rs（合并引擎，folder-merge / clip-merge 共用）。
// 合并参数的公共表单 MergeOptionsFields 在 shared/merge-options.tsx（folder-merge.tsx 共用）。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { MERGE_OPTION_DEFAULTS, MergeOptionsFields, type MergeOptions } from "./shared/merge-options";
import { WithUnit } from "./shared/with-unit";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

// 兼容旧的导入位置：这些公共部件已移到 ./shared/
export { MERGE_OPTION_DEFAULTS, MergeOptionsFields, type MergeMode, type MergeOptions } from "./shared/merge-options";
export { WithUnit } from "./shared/with-unit";

export type MergeGrouping = "all" | "everyN";

export type MergeParams = MergeOptions & {
  grouping: MergeGrouping;
  groupSize: number;
  shuffle: boolean;
};

function MergeForm({ value, onChange }: ToolFormProps<MergeParams>) {
  return (
    <>
      <ParamField label="分组" hint="处理范围（全部 / 选中）在下方选择；按队列顺序合并。">
        <ChoiceGroup<MergeGrouping>
          label="分组方式"
          value={value.grouping}
          onChange={(grouping) => onChange({ grouping })}
          options={[
            { value: "all", label: "全部合成一个" },
            { value: "everyN", label: "每 N 个一组" },
          ]}
        />
      </ParamField>
      {value.grouping === "everyN" && (
        <ParamField label="每组个数" hint="最后一组可能不足。">
          <WithUnit unit="个">
            <NumberInput
              label="每组个数"
              value={value.groupSize}
              onChange={(groupSize) => onChange({ groupSize })}
              min={1}
              max={1000}
              integer
            />
          </WithUnit>
        </ParamField>
      )}
      <ParamField label="随机顺序" hint="按随机种子打乱顺序后再分组。">
        <Switch aria-label="随机顺序" checked={value.shuffle} onCheckedChange={(shuffle) => onChange({ shuffle })} />
      </ParamField>
      <Separator className="my-1" />
      <MergeOptionsFields value={value} onChange={onChange} />
    </>
  );
}

export default defineTool<MergeParams>({
  id: "tool.merge",
  defaults: { grouping: "all", groupSize: 2, shuffle: false, ...MERGE_OPTION_DEFAULTS },
  inputs: VIDEO_INPUTS,
  Form: MergeForm,
});
