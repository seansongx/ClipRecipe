import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { cn } from "@/lib/utils";

/**
 * 少量互斥选项（单选），基于 shadcn ToggleGroup。
 * 与原生 ToggleGroup 不同：再次点击已选项不会清空选择。
 */
export default function ChoiceGroup<T extends string>({
  value,
  options,
  onChange,
  disabled,
  label,
  className,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  /** 无障碍名称 */
  label?: string;
  className?: string;
}) {
  return (
    <ToggleGroup
      type="single"
      variant="outline"
      size="sm"
      spacing={0}
      value={value}
      disabled={disabled}
      aria-label={label}
      onValueChange={(next) => {
        if (next) onChange(next as T);
      }}
      className={cn("max-w-full bg-background dark:bg-input/30", className)}
    >
      {options.map((option) => (
        <ToggleGroupItem
          key={option.value}
          value={option.value}
          className="h-7 px-2 text-xs font-normal text-muted-foreground data-[state=on]:bg-primary/12 data-[state=on]:text-foreground data-[state=on]:font-medium dark:data-[state=on]:bg-primary/18"
        >
          {option.label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}
