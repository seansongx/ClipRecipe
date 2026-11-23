// tool.av-mux 音视频合成。后端：src-tauri/src/tools/av_mux.rs

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { WithUnit } from "./shared/with-unit";
import { defineTool, type ToolFormProps } from "./types";

export type Pairing = "name" | "order" | "random";

export type AvMuxParams = {
  pairing: Pairing;
  /** 保留原声并与新音频混合 */
  keepOriginal: boolean;
  /** 原声音量（%） */
  originalVolume: number;
  /** 新音频音量（%） */
  audioVolume: number;
  /** 音频比视频短时循环 */
  loopAudio: boolean;
};

const PAIRING_HINTS: Record<Pairing, string> = {
  name: "文件名（不含扩展名）相同的音频与视频配成一对，找不到同名音频的视频不处理。",
  order: "两边各按文件名顺序一一配对，多出的文件忽略。",
  random: "每个视频随机取一个音频（可重复）。",
};

function AvMuxForm({ value, onChange }: ToolFormProps<AvMuxParams>) {
  return (
    <>
      <ParamField label="配对" hint={PAIRING_HINTS[value.pairing]}>
        <ChoiceGroup<Pairing>
          label="配对方式"
          value={value.pairing}
          onChange={(pairing) => onChange({ pairing })}
          options={[
            { value: "name", label: "同名" },
            { value: "order", label: "按顺序" },
            { value: "random", label: "随机" },
          ]}
        />
      </ParamField>
      <ParamField label="保留原声" hint="开启：与新音频混合；关闭：用新音频替换原声。">
        <Switch
          aria-label="保留原声"
          checked={value.keepOriginal}
          onCheckedChange={(keepOriginal) => onChange({ keepOriginal })}
        />
      </ParamField>
      {value.keepOriginal && (
        <ParamField label="原声音量">
          <WithUnit unit="%">
            <NumberInput
              label="原声音量"
              value={value.originalVolume}
              onChange={(originalVolume) => onChange({ originalVolume })}
              min={0}
              max={400}
              integer
            />
          </WithUnit>
        </ParamField>
      )}
      <ParamField label="新音频音量">
        <WithUnit unit="%">
          <NumberInput
            label="新音频音量"
            value={value.audioVolume}
            onChange={(audioVolume) => onChange({ audioVolume })}
            min={0}
            max={400}
            integer
          />
        </WithUnit>
      </ParamField>
      <ParamField label="循环音频" hint="时长跟随视频：音频较短时循环（关闭则补静音），较长时截断。视频直接复制，不重编码。">
        <Switch aria-label="循环音频" checked={value.loopAudio} onCheckedChange={(loopAudio) => onChange({ loopAudio })} />
      </ParamField>
    </>
  );
}

export default defineTool<AvMuxParams>({
  id: "tool.av-mux",
  defaults: { pairing: "name", keepOriginal: false, originalVolume: 100, audioVolume: 100, loopAudio: true },
  inputs: { videos: false, folders: [{ key: "audio", label: "音频文件夹" }, { key: "video", label: "视频文件夹" }] },
  Form: AvMuxForm,
});
