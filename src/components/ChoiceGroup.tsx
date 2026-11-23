import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";

/**
 * 少量互斥选项（单选），基于 shadcn ToggleGroup（Mira 的 outline 变体，选中项是 muted 底）。
 * 与原生 ToggleGroup 不同：再次点击已选项不会清空选择。
 */
export default function ChoiceGroup<T extends string>({
  value,
  options,
  onChange,
  disabled,
  label,
  spacing = 0,
  className,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  /** 无障碍名称 */
  label?: string;
  /** 选项之间的间距（ToggleGroup 的 spacing）：0 为连在一起的分段按钮；排成多行栅格时用 1，每项保留完整边框 */
  spacing?: number;
  /** 排版（宽度、栅格），不改外观 */
  className?: string;
}) {
  return (
    <ToggleGroup
      type="single"
      variant="outline"
      spacing={spacing}
      value={value}
      disabled={disabled}
      aria-label={label}
      onValueChange={(next) => {
        if (next) onChange(next as T);
      }}
      className={cn("max-w-full flex-wrap", className)}
    >
      {options.map((option) => (
        <ToggleGroupItem key={option.value} value={option.value}>
          {option.label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}
