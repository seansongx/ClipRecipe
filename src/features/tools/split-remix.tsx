// tool.split-remix 拆分重组。后端：src-tauri/src/tools/split_remix.rs
// 在设定位置拆开视频，每处插入一段视频（单个文件或目录随机 / 依次取），统一成主视频规格后拼接。
// 位置设置（PositionFields / NumberListInput）在 shared/position-fields.tsx，与移除重组共用。

import AssetSourceInput from "@/components/AssetSourceInput";
import ParamField from "@/components/ParamField";
import { VIDEO_EXTENSIONS, type AssetSource } from "@/lib/backend";
import { PositionFields, type PositionParams } from "./shared/position-fields";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

// 兼容旧的导入位置：这些公共部件已移到 ./shared/position-fields
export { NumberListInput, PositionFields, type PositionMode, type PositionParams } from "./shared/position-fields";

export type SplitRemixParams = PositionParams & {
  /** 插入的视频 */
  insert: AssetSource;
};

function SplitRemixForm({ value, onChange }: ToolFormProps<SplitRemixParams>) {
  return (
    <>
      <PositionFields value={value} onChange={onChange} verb="拆开并插入视频" />
      <ParamField
        label="插入视频"
        hint="插入的视频会缩放到主视频的尺寸（等比、补黑边），帧率与音频格式也统一成主视频的；没有声音的补静音。"
      >
        <AssetSourceInput
          value={value.insert}
          onChange={(insert) => onChange({ insert })}
          extensions={VIDEO_EXTENSIONS}
          kindLabel="视频"
        />
      </ParamField>
    </>
  );
}

export default defineTool<SplitRemixParams>({
  id: "tool.split-remix",
  defaults: {
    positionMode: "parts",
    times: [10],
    percents: [50],
    parts: 2,
    insert: { mode: "file", path: "" },
  },
  inputs: VIDEO_INPUTS,
  Form: SplitRemixForm,
});
