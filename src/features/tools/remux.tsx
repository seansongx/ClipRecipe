// tool.remux 无损转换。后端：src-tauri/src/tools/remux.rs
// 参考实现（一进一出、不重编码）：参数类型 + 默认值 + 输入声明 + 表单。

import ChoiceGroup from "@/components/ChoiceGroup";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type RemuxContainer = "mp4" | "mkv" | "mov" | "ts" | "flv";

export type RemuxParams = {
  container: RemuxContainer;
  /** 音频编码与目标容器不兼容时转为 AAC（视频仍不重编码） */
  convertIncompatibleAudio: boolean;
  keepSubtitles: boolean;
};

const CONTAINERS: { value: RemuxContainer; label: string }[] = [
  { value: "mp4", label: "MP4" },
  { value: "mkv", label: "MKV" },
  { value: "mov", label: "MOV" },
  { value: "ts", label: "TS" },
  { value: "flv", label: "FLV" },
];

function RemuxForm({ value, onChange }: ToolFormProps<RemuxParams>) {
  return (
    <>
      <ParamField label="封装格式" hint="只换封装，画面不重编码，速度很快、画质无损。装不下源视频编码时会报错，可改用 MKV。">
        <ChoiceGroup<RemuxContainer>
          label="封装格式"
          value={value.container}
          options={CONTAINERS}
          onChange={(container) => onChange({ container })}
        />
      </ParamField>
      <ParamField label="音频转码" hint="音频编码放不进目标格式时（如 PCM → MP4）转为 AAC；关闭则直接报错。">
        <Switch
          aria-label="音频不兼容时转为 AAC"
          checked={value.convertIncompatibleAudio}
          onCheckedChange={(convertIncompatibleAudio) => onChange({ convertIncompatibleAudio })}
        />
      </ParamField>
      <ParamField label="保留字幕" hint="MP4 / MOV 中文本字幕转为 mov_text，图形字幕丢弃；TS、FLV 不保留字幕。">
        <Switch
          aria-label="保留字幕"
          checked={value.keepSubtitles}
          onCheckedChange={(keepSubtitles) => onChange({ keepSubtitles })}
        />
      </ParamField>
    </>
  );
}

export default defineTool<RemuxParams>({
  id: "tool.remux",
  defaults: { container: "mp4", convertIncompatibleAudio: true, keepSubtitles: true },
  inputs: VIDEO_INPUTS,
  Form: RemuxForm,
});
