// tool.folder-merge 文件夹合并。后端：src-tauri/src/tools/folder_merge.rs（合并引擎在 merge.rs）。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { MERGE_OPTION_DEFAULTS, MergeOptionsFields, type MergeOptions } from "./shared/merge-options";
import { WithUnit } from "./shared/with-unit";
import { defineTool, type ToolFormProps } from "./types";

export type FolderGrouping = "subfolder" | "all" | "everyN";

export type FolderMergeParams = MergeOptions & {
  grouping: FolderGrouping;
  groupSize: number;
  shuffle: boolean;
};

const GROUPING_HINTS: Record<FolderGrouping, string> = {
  subfolder: "每个子文件夹合成一个视频，以子文件夹命名；文件夹顶层的视频另成一个，以文件夹命名。",
  all: "文件夹里的全部视频合成一个，以文件夹命名。",
  everyN: "按文件名顺序每 N 个合成一个，最后一组可能不足。",
};

function FolderMergeForm({ value, onChange }: ToolFormProps<FolderMergeParams>) {
  return (
    <>
      <ParamField label="分组" hint={`${GROUPING_HINTS[value.grouping]}是否包含更深的子目录按设置“目录模式包含子目录”。`}>
        <ChoiceGroup<FolderGrouping>
          label="分组方式"
          value={value.grouping}
          onChange={(grouping) => onChange({ grouping })}
          options={[
            { value: "subfolder", label: "每个子文件夹" },
            { value: "all", label: "全部" },
            { value: "everyN", label: "每 N 个" },
          ]}
        />
      </ParamField>
      {value.grouping === "everyN" && (
        <ParamField label="每组个数">
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
      <ParamField label="随机顺序" hint="按随机种子打乱顺序（每 N 个时先打乱再分组）。">
        <Switch aria-label="随机顺序" checked={value.shuffle} onCheckedChange={(shuffle) => onChange({ shuffle })} />
      </ParamField>
      <Separator className="my-1" />
      <MergeOptionsFields
        value={value}
        onChange={onChange}
        defaultName={value.grouping === "everyN" ? "第一个视频名 + “_merged”" : "（子）文件夹名"}
      />
    </>
  );
}

export default defineTool<FolderMergeParams>({
  id: "tool.folder-merge",
  defaults: { grouping: "subfolder", groupSize: 2, shuffle: false, ...MERGE_OPTION_DEFAULTS },
  inputs: { videos: false, folders: [{ key: "source", label: "视频文件夹" }] },
  Form: FolderMergeForm,
});
