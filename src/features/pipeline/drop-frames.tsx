// pipe.drop-frames 抽帧。后端：src-tauri/src/pipeline/modules/drop_frames.rs

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import { Switch } from "@/components/ui/switch";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export interface DropFramesParams {
  enabled: boolean;
  /** 每多少帧丢 1 帧 */
  interval: Range;
  /** 同步加速音频（缩短时长）；关闭时保持原时间轴 */
  syncAudio: boolean;
}

function DropFramesForm({ value, onChange }: ModuleFormProps<DropFramesParams>) {
  return (
    <>
      <ParamField label="间隔" hint="每隔这么多帧丢掉 1 帧。">
        <RangeInput
          value={value.interval}
          onChange={(interval) => onChange({ interval })}
          min={2}
          max={1000}
          integer
          unit="帧"
        />
      </ParamField>
      <ParamField
        label="同步音频"
        hint={
          value.syncAudio
            ? "剩下的帧紧密排列，视频变短，音频同步加速。"
            : "保持原时间轴：丢掉的帧由前一帧补位，时长与音频不变。"
        }
      >
        <Switch
          checked={value.syncAudio}
          onCheckedChange={(syncAudio) => onChange({ syncAudio })}
          aria-label="同步加速音频"
        />
      </ParamField>
    </>
  );
}

export default defineModule<DropFramesParams>({
  id: "pipe.drop-frames",
  defaults: {
    enabled: false,
    interval: { min: 20, max: 30 },
    syncAudio: true,
  },
  Form: DropFramesForm,
});
