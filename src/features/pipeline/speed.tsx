// pipe.speed 变速。后端：src-tauri/src/pipeline/modules/speed.rs

import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import { Switch } from "@/components/ui/switch";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type SpeedMode = "fixed" | "segmented";

export interface SpeedParams {
  enabled: boolean;
  mode: SpeedMode;
  /** 固定方式的倍率 */
  rate: Range;
  /** 分段数 */
  segmentCount: Range;
  /** 每段倍率的范围 */
  segmentRate: Range;
  /** 各段随机取倍率；关闭时由慢到快均匀递增 */
  shuffle: boolean;
  /** 变速后的最短时长（秒），0 表示不限制 */
  minDuration: number;
  /** 音高是否随速度变化 */
  changePitch: boolean;
}

function SpeedForm({ value, onChange }: ModuleFormProps<SpeedParams>) {
  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<SpeedMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "fixed", label: "固定倍率" },
            { value: "segmented", label: "分段变速" },
          ]}
        />
      </ParamField>
      {value.mode === "fixed" ? (
        <ParamField label="倍率" hint="大于 1 加速，小于 1 减速。">
          <RangeInput
            value={value.rate}
            onChange={(rate) => onChange({ rate })}
            min={0.25}
            max={4}
            step={0.05}
            unit="×"
          />
        </ParamField>
      ) : (
        <>
          <ParamField label="分段数" hint="视频按时长等分成若干段，每段单独变速。">
            <RangeInput
              value={value.segmentCount}
              onChange={(segmentCount) => onChange({ segmentCount })}
              min={2}
              max={20}
              integer
              unit="段"
            />
          </ParamField>
          <ParamField label="分段倍率">
            <RangeInput
              value={value.segmentRate}
              onChange={(segmentRate) => onChange({ segmentRate })}
              min={0.25}
              max={4}
              step={0.05}
              unit="×"
            />
          </ParamField>
          <ParamField
            label="随机组合"
            hint={
              value.shuffle
                ? "每段在范围内随机取倍率。"
                : "各段倍率从最小值到最大值均匀递增（渐快）。"
            }
          >
            <Switch
              checked={value.shuffle}
              onCheckedChange={(shuffle) => onChange({ shuffle })}
              aria-label="随机组合"
            />
          </ParamField>
        </>
      )}
      <ParamField label="变调" hint="开启后音高随速度变化；关闭则保持原音高。">
        <Switch
          checked={value.changePitch}
          onCheckedChange={(changePitch) => onChange({ changePitch })}
          aria-label="变调"
        />
      </ParamField>
      <ParamField
        label="最短时长"
        hint="0 表示不限制。加速后短于此时长时自动降低倍率，但不会因此比原视频更长。"
      >
        <div className="flex items-center gap-1.5">
          <NumberInput
            value={value.minDuration}
            onChange={(minDuration) => onChange({ minDuration })}
            min={0}
            max={86400}
            step={0.5}
            label="最短时长"
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
      <p className="pt-1 text-xs text-muted-foreground">变速不支持即时预览。</p>
    </>
  );
}

export default defineModule<SpeedParams>({
  id: "pipe.speed",
  defaults: {
    enabled: false,
    mode: "fixed",
    rate: { min: 1.2, max: 1.2 },
    segmentCount: { min: 3, max: 3 },
    segmentRate: { min: 0.8, max: 1.5 },
    shuffle: true,
    minDuration: 0,
    changePitch: false,
  },
  Form: SpeedForm,
});
