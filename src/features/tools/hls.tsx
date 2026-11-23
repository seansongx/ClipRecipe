// tool.hls M3U8 切片。后端：src-tauri/src/tools/hls.rs

import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { WithUnit } from "./shared/with-unit";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type HlsParams = {
  /** 目标分片时长（秒） */
  segmentSeconds: number;
};

function HlsForm({ value, onChange }: ToolFormProps<HlsParams>) {
  return (
    <>
      <ParamField
        label="分片时长"
        hint="无损切片只能在关键帧处切开，实际分片时长会有误差（播放列表里写的是实际时长）。"
      >
        <WithUnit unit="秒">
          <NumberInput
            label="分片时长"
            value={value.segmentSeconds}
            onChange={(segmentSeconds) => onChange({ segmentSeconds })}
            min={1}
            max={600}
            step="any"
          />
        </WithUnit>
      </ParamField>
      <p className="px-0.5 py-1 text-[11px] leading-snug text-muted-foreground">
        每个视频输出到同名文件夹：index.m3u8 与 index_0000.ts 等分片，视频直接复制不重编码。
      </p>
    </>
  );
}

export default defineTool<HlsParams>({
  id: "tool.hls",
  defaults: { segmentSeconds: 6 },
  inputs: VIDEO_INPUTS,
  Form: HlsForm,
});
