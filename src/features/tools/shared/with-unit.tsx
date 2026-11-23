import type { ReactNode } from "react";

/** 数字 + 单位（如 NumberInput 后跟“秒”“%”）。 */
export function WithUnit({ unit, children }: { unit: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-1.5">
      {children}
      <span className="text-xs text-muted-foreground">{unit}</span>
    </div>
  );
}
