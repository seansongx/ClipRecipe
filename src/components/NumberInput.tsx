import { useEffect, useState } from "react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * 数字输入：shadcn Input 的薄封装。输入过程中允许暂时无效的文本，
 * 失焦或回车时取整（integer）、校正到 [min, max] 并提交；无效文本恢复原值。
 */
export default function NumberInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  integer = false,
  className = "w-20",
  label,
  disabled,
  id,
}: {
  value: number;
  onChange: (value: number) => void;
  min?: number;
  max?: number;
  step?: number | "any";
  integer?: boolean;
  /** 宽度等样式，默认 w-20 */
  className?: string;
  /** 无障碍名称 */
  label?: string;
  disabled?: boolean;
  id?: string;
}) {
  const [text, setText] = useState(String(value));

  useEffect(() => setText(String(value)), [value]);

  const commit = () => {
    let parsed = Number(text);
    if (text.trim() === "" || !Number.isFinite(parsed)) {
      setText(String(value));
      return;
    }
    if (integer) parsed = Math.round(parsed);
    if (min !== undefined) parsed = Math.max(min, parsed);
    if (max !== undefined) parsed = Math.min(max, parsed);
    setText(String(parsed));
    if (parsed !== value) onChange(parsed);
  };

  return (
    <Input
      id={id}
      type="number"
      inputMode="decimal"
      aria-label={label}
      value={text}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
      className={cn("h-7 px-2 text-xs tabular-nums md:text-xs", className)}
    />
  );
}
