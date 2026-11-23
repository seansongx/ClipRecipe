// pipe.grid 宫格分屏。后端：src-tauri/src/pipeline/modules/grid.rs

import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import { Switch } from "@/components/ui/switch";
import type { Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type GridDirection = "auto" | "horizontal" | "vertical";

export interface GridParams {
  enabled: boolean;
  /** 宫格数：2 / 3 / 4 / 6 / 9 */
  count: number;
  direction: GridDirection;
  /** 两端虚化（3、6、9 宫格） */
  blurEnds: boolean;
  /** 虚化强度（gblur sigma） */
  blur: Range;
}

const COUNTS = ["2", "3", "4", "6", "9"] as const;
type CountText = (typeof COUNTS)[number];

function GridForm({ value, onChange }: ModuleFormProps<GridParams>) {
  const hasDirection = value.count === 2 || value.count === 3 || value.count === 6 || value.count === 9;
  const canBlurEnds = value.count === 3 || value.count === 6 || value.count === 9;
  return (
    <>
      <ParamField label="宫格数" hint="每格保持原画面比例；输出长边不超过原视频长边。">
        <ChoiceGroup<CountText>
          value={String(value.count) as CountText}
          onChange={(count) => onChange({ count: Number(count) })}
          options={COUNTS.map((c) => ({ value: c, label: `${c} 宫格` }))}
        />
      </ParamField>
      {hasDirection && (
        <ParamField
          label="方向"
          hint={value.count === 9 ? "9 宫格的方向只决定“两端”是上下两行还是左右两列。" : "自动：横屏竖排，竖屏横排。"}
        >
          <ChoiceGroup<GridDirection>
            value={value.direction}
            onChange={(direction) => onChange({ direction })}
            options={[
              { value: "auto", label: "自动" },
              { value: "vertical", label: "竖排" },
              { value: "horizontal", label: "横排" },
            ]}
          />
        </ParamField>
      )}
      <ParamField label="两端虚化" hint={canBlurEnds ? "首尾两排虚化，中间保持清晰。" : "仅 3、6、9 宫格可用。"}>
        <Switch
          aria-label="两端虚化"
          checked={value.blurEnds}
          disabled={!canBlurEnds}
          onCheckedChange={(blurEnds) => onChange({ blurEnds })}
        />
      </ParamField>
      {canBlurEnds && value.blurEnds && (
        <ParamField label="虚化强度">
          <RangeInput value={value.blur} onChange={(blur) => onChange({ blur })} min={0} max={100} step={1} />
        </ParamField>
      )}
    </>
  );
}

export default defineModule<GridParams>({
  id: "pipe.grid",
  defaults: {
    enabled: false,
    count: 3,
    direction: "auto",
    blurEnds: false,
    blur: { min: 20, max: 20 },
  },
  Form: GridForm,
});
