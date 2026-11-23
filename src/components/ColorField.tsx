import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

const SWATCHES = [
  "#ffffff", "#d9d9d9", "#8c8c8c", "#3f3f3f", "#000000", "#f5e6c8",
  "#ef4444", "#f97316", "#f4b740", "#facc15", "#22c55e", "#14b8a6",
  "#0ea5e9", "#3b82f6", "#6366f1", "#a855f7", "#ec4899", "#7c2d12",
];

const HEX = /^#?([0-9a-f]{6}|[0-9a-f]{3})$/i;

/** 把 #rgb / rgb / #rrggbb 规范为小写 #rrggbb；无效时返回 null。 */
export function normalizeHex(text: string): string | null {
  const match = HEX.exec(text.trim());
  if (!match) return null;
  const hex = match[1].toLowerCase();
  return `#${hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex}`;
}

/**
 * 颜色选择（#rrggbb）：按钮显示当前颜色，弹出层里点选色板或输入十六进制值。
 * 由 shadcn Popover + Button + Input 组合（shadcn 没有取色器）。
 */
export default function ColorField({
  value,
  onChange,
  disabled,
  label = "颜色",
}: {
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  /** 无障碍名称 */
  label?: string;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);

  const commit = () => {
    const hex = normalizeHex(text);
    if (hex) {
      setText(hex);
      if (hex !== value) onChange(hex);
    } else {
      setText(value);
    }
  };

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="xs"
          disabled={disabled}
          aria-label={`${label}：${value}`}
          className="h-7 gap-1.5 px-1.5 font-mono text-[11px] font-normal"
        >
          <span className="size-4 rounded-sm ring-1 ring-foreground/20" style={{ backgroundColor: value }} />
          {value}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto gap-2 p-2">
        <div className="grid grid-cols-6 gap-1" role="listbox" aria-label={label}>
          {SWATCHES.map((swatch) => (
            <Button
              key={swatch}
              type="button"
              variant="ghost"
              size="icon-xs"
              role="option"
              aria-selected={swatch === value}
              aria-label={swatch}
              title={swatch}
              onClick={() => onChange(swatch)}
              className={cn(
                "size-6 rounded-sm p-0 ring-1 ring-foreground/15 hover:opacity-85",
                swatch === value && "ring-2 ring-ring ring-offset-1 ring-offset-popover",
              )}
              style={{ backgroundColor: swatch }}
            />
          ))}
        </div>
        <Input
          value={text}
          aria-label={`${label}（十六进制）`}
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
          aria-invalid={normalizeHex(text) === null}
          className="h-7 px-2 font-mono text-xs md:text-xs"
        />
      </PopoverContent>
    </Popover>
  );
}
