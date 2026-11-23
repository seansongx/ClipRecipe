// pipe.rotate-flip 旋转与翻转。后端：src-tauri/src/pipeline/modules/rotate_flip.rs

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

/** 顺时针固定角度 */
export type Rotation = "none" | "cw90" | "cw180" | "cw270";
export type AngleFit = "showAll" | "removeBlack";

export interface RotateFlipParams {
  enabled: boolean;
  rotation: Rotation;
  hflip: boolean;
  vflip: boolean;
  /** 随机任意角度旋转 */
  randomAngle: boolean;
  /** 角度（度，正值顺时针），保留 1 位小数 */
  angle: Range;
  fit: AngleFit;
}

function RotateFlipForm({ value, onChange }: ModuleFormProps<RotateFlipParams>) {
  return (
    <>
      <ParamField label="旋转" hint="顺时针方向。">
        <ChoiceGroup<Rotation>
          value={value.rotation}
          onChange={(rotation) => onChange({ rotation })}
          options={[
            { value: "none", label: "不旋转" },
            { value: "cw90", label: "90°" },
            { value: "cw180", label: "180°" },
            { value: "cw270", label: "270°" },
          ]}
        />
      </ParamField>
      <ParamField label="翻转">
        <div className="flex h-7 flex-wrap items-center gap-4">
          <Label>
            <Switch aria-label="水平翻转" checked={value.hflip} onCheckedChange={(hflip) => onChange({ hflip })} />
            水平
          </Label>
          <Label>
            <Switch aria-label="垂直翻转" checked={value.vflip} onCheckedChange={(vflip) => onChange({ vflip })} />
            垂直
          </Label>
        </div>
      </ParamField>
      <ParamField label="任意角度" hint="每个视频在范围内随机取一个角度（正值顺时针），在上面的旋转翻转之后应用。">
        <Switch
          aria-label="随机任意角度旋转"
          checked={value.randomAngle}
          onCheckedChange={(randomAngle) => onChange({ randomAngle })}
        />
      </ParamField>
      {value.randomAngle && (
        <>
          <ParamField label="角度">
            <RangeInput
              value={value.angle}
              onChange={(angle) => onChange({ angle })}
              min={-180}
              max={180}
              step={0.5}
              unit="°"
            />
          </ParamField>
          <ParamField
            label="四角"
            hint={
              value.fit === "showAll"
                ? "画布扩大以完整显示旋转后的画面，四角填黑。"
                : "保持原尺寸，裁掉黑边后放大铺满。"
            }
          >
            <ChoiceGroup<AngleFit>
              value={value.fit}
              onChange={(fit) => onChange({ fit })}
              options={[
                { value: "removeBlack", label: "去除黑边" },
                { value: "showAll", label: "完全显示" },
              ]}
            />
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineModule<RotateFlipParams>({
  id: "pipe.rotate-flip",
  defaults: {
    enabled: false,
    rotation: "none",
    hflip: false,
    vflip: false,
    randomAngle: false,
    angle: { min: -5, max: 5 },
    fit: "removeBlack",
  },
  Form: RotateFlipForm,
});
