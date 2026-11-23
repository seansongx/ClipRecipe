// tool.extract-av 音视频提取。后端：src-tauri/src/tools/extract_av.rs
// 参考实现（一进多出）：模式切换、条件显示的参数、下拉选择。

import ChoiceGroup from "@/components/ChoiceGroup";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type ExtractMode = "audio" | "video" | "both";
export type AudioTracks = "first" | "all";
export type AudioFormat = "original" | "mp3" | "m4a" | "wav";

export type ExtractAvParams = {
  mode: ExtractMode;
  tracks: AudioTracks;
  audioFormat: AudioFormat;
  /** 转为 mp3 时的码率（kbit/s） */
  mp3Kbps: number;
};

/** 与后端 MP3_KBPS 一致。 */
const MP3_KBPS = [128, 160, 192, 256, 320];
const MP3_ITEMS = MP3_KBPS.map((kbps) => ({ value: kbps, label: `${kbps} kbps` }));

const FORMATS: { value: AudioFormat; label: string }[] = [
  { value: "original", label: "原格式（不重编码）" },
  { value: "mp3", label: "MP3" },
  { value: "m4a", label: "M4A（AAC）" },
  { value: "wav", label: "WAV（PCM）" },
];

function ExtractAvForm({ value, onChange }: ToolFormProps<ExtractAvParams>) {
  const audio = value.mode !== "video";
  return (
    <>
      <ParamField label="提取" hint="纯视频：去掉音频与字幕，视频直接复制。">
        <ChoiceGroup<ExtractMode>
          label="提取内容"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "audio", label: "音频" },
            { value: "video", label: "纯视频" },
            { value: "both", label: "两者" },
          ]}
        />
      </ParamField>
      {audio && (
        <>
          <ParamField label="音轨" hint="多音轨视频可以全部导出，文件名带序号。">
            <ChoiceGroup<AudioTracks>
              label="音轨"
              value={value.tracks}
              onChange={(tracks) => onChange({ tracks })}
              options={[
                { value: "all", label: "全部音轨" },
                { value: "first", label: "第一条" },
              ]}
            />
          </ParamField>
          <ParamField label="音频格式" hint="源编码与所选格式相同时直接复制，不重编码。">
            <Select items={FORMATS} value={value.audioFormat} onValueChange={(audioFormat) => audioFormat && onChange({ audioFormat })}>
              <SelectTrigger aria-label="音频格式" className="w-40">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {FORMATS.map((f) => (
                  <SelectItem key={f.value} value={f.value}>
                    {f.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </ParamField>
          {value.audioFormat === "mp3" && (
            <ParamField label="MP3 码率">
              <Select items={MP3_ITEMS} value={value.mp3Kbps} onValueChange={(mp3Kbps) => mp3Kbps && onChange({ mp3Kbps })}>
                <SelectTrigger aria-label="MP3 码率" className="w-28">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {MP3_KBPS.map((kbps) => (
                    <SelectItem key={kbps} value={kbps}>
                      {kbps} kbps
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </ParamField>
          )}
        </>
      )}
    </>
  );
}

export default defineTool<ExtractAvParams>({
  id: "tool.extract-av",
  defaults: { mode: "audio", tracks: "all", audioFormat: "original", mp3Kbps: 192 },
  inputs: VIDEO_INPUTS,
  Form: ExtractAvForm,
});
