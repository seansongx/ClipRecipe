import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { Range } from "@/lib/backend";
import NumberInput from "./NumberInput";

/**
 * 范围值输入：最小 ~ 最大。两端相等即固定值（右侧显示“固定”）。
 * 改最小值使其大于最大值时，最大值跟着调整，反之亦然。
 */
export default function RangeInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  integer = false,
  unit,
  disabled,
}: {
  value: Range;
  onChange: (value: Range) => void;
  min?: number;
  max?: number;
  step?: number | "any";
  integer?: boolean;
  unit?: string;
  disabled?: boolean;
}) {
  const fixed = value.min === value.max;
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <NumberInput
        className="w-[72px]"
        label="最小值"
        value={value.min}
        min={min}
        max={max}
        step={step}
        integer={integer}
        disabled={disabled}
        onChange={(v) => onChange({ min: v, max: Math.max(v, value.max) })}
      />
      <span className="text-muted-foreground">~</span>
      <NumberInput
        className="w-[72px]"
        label="最大值"
        value={value.max}
        min={min}
        max={max}
        step={step}
        integer={integer}
        disabled={disabled}
        onChange={(v) => onChange({ min: Math.min(v, value.min), max: v })}
      />
      {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
      {fixed ? (
        <Badge variant="secondary" className="h-5 rounded-sm px-1.5 text-[11px] font-normal text-muted-foreground">
          固定
        </Badge>
      ) : (
        <Button
          type="button"
          variant="ghost"
          size="xs"
          disabled={disabled}
          onClick={() => onChange({ min: value.min, max: value.min })}
          className="h-5 px-1.5 text-[11px] font-normal text-muted-foreground"
          title="把最大值设为与最小值相同"
        >
          设为固定
        </Button>
      )}
    </div>
  );
}
