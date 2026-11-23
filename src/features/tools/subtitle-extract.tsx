// tool.subtitle-extract 字幕提取。后端：src-tauri/src/tools/subtitle_extract.rs

import ChoiceGroup from "@/components/ChoiceGroup";
import ParamField from "@/components/ParamField";
import { Input } from "@/components/ui/input";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type TrackChoice = "all" | "selected";
export type TextFormat = "srt" | "ass" | "original";

export type SubtitleExtractParams = {
  tracks: TrackChoice;
  /** 字幕轨序号（从 1 起）和 / 或语言代码，逗号分隔 */
  selection: string;
  textFormat: TextFormat;
};

function SubtitleExtractForm({ value, onChange }: ToolFormProps<SubtitleExtractParams>) {
  return (
    <>
      <ParamField
        label="字幕轨"
        hint="文件名为“视频名_track序号_语言”，如 movie_track1_chi.srt。没有内嵌字幕的视频会跳过。"
      >
        <ChoiceGroup<TrackChoice>
          label="导出哪些字幕轨"
          value={value.tracks}
          onChange={(tracks) => onChange({ tracks })}
          options={[
            { value: "all", label: "全部" },
            { value: "selected", label: "指定" },
          ]}
        />
      </ParamField>
      {value.tracks === "selected" && (
        <ParamField label="选择" hint="填第几条字幕轨（从 1 起）或语言代码，可混写，用逗号分隔，如：1, eng。zh 也能匹配 chi / zho。">
          <Input
            value={value.selection}
            onChange={(e) => onChange({ selection: e.target.value })}
            placeholder="如 1, chi, eng"
            aria-label="字幕轨选择"
            className="h-7 px-2 text-xs md:text-xs"
          />
        </ParamField>
      )}
      <ParamField
        label="格式"
        hint="只影响文字字幕。PGS / DVD 等图像字幕无法转成文字，会按原格式保存为 .sup / .mks。"
      >
        <ChoiceGroup<TextFormat>
          label="文字字幕格式"
          value={value.textFormat}
          onChange={(textFormat) => onChange({ textFormat })}
          options={[
            { value: "srt", label: "SRT" },
            { value: "ass", label: "ASS" },
            { value: "original", label: "原格式" },
          ]}
        />
      </ParamField>
    </>
  );
}

export default defineTool<SubtitleExtractParams>({
  id: "tool.subtitle-extract",
  defaults: { tracks: "all", selection: "", textFormat: "srt" },
  inputs: VIDEO_INPUTS,
  Form: SubtitleExtractForm,
});
