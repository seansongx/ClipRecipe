// pipe.bitrate 码率。后端：src-tauri/src/pipeline/modules/bitrate.rs

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type BitrateMode = "multiplier" | "fixed";

export interface BitrateParams {
  enabled: boolean;
  mode: BitrateMode;
  /** 倍率模式：源码率 × 倍率 */
  multiplier: Range;
  /** 固定模式：kbit/s */
  kbps: Range;
}

function BitrateForm({ value, onChange }: ModuleFormProps<BitrateParams>) {
  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<BitrateMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "multiplier", label: "按源码率倍率" },
            { value: "fixed", label: "固定码率" },
          ]}
        />
      </ParamField>
      {value.mode === "multiplier" ? (
        <ParamField label="倍率" hint="小于 1 压缩体积，大于 1 提升画质；源码率未知时改用恒定质量。">
          <RangeInput
            value={value.multiplier}
            onChange={(multiplier) => onChange({ multiplier })}
            min={0.05}
            max={10}
            step={0.05}
            unit="×"
          />
        </ParamField>
      ) : (
        <ParamField label="码率" hint="视频流目标码率，可手动输入。">
          <RangeInput
            value={value.kbps}
            onChange={(kbps) => onChange({ kbps })}
            min={50}
            max={200000}
            step={100}
            integer
            unit="kbps"
          />
        </ParamField>
      )}
      <p className="pt-1 text-xs text-muted-foreground">不启用此项时沿用源视频码率。</p>
    </>
  );
}

export default defineModule<BitrateParams>({
  id: "pipe.bitrate",
  defaults: {
    enabled: false,
    mode: "multiplier",
    multiplier: { min: 1, max: 1 },
    kbps: { min: 3000, max: 3000 },
  },
  Form: BitrateForm,
});
