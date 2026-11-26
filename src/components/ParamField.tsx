import type { ReactNode } from "react";
import { Field, FieldContent, FieldDescription, FieldTitle } from "@/components/ui/field";

/**
 * 参数表单中的一行：左侧标签，右侧控件，可带说明。由 shadcn Field 组合而成（外观是 Mira 原样，这里只排版）。
 * 控件太宽时整体换到标签下方。
 */
export default function ParamField({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <Field orientation="horizontal" className="flex-wrap items-start gap-x-3 gap-y-1 py-1 *:data-[slot=field-label]:flex-none">
      {/* 标签与右侧第一行控件（Mira 的控件高 h-7）居中对齐 */}
      <FieldTitle className="w-16 shrink-0 pt-1">{label}</FieldTitle>
      {/* 单独的开关比输入框矮，上下补齐使其与标签对齐 */}
      <FieldContent className="min-w-0 flex-1 *:data-[slot=switch]:my-1.5">
        {children}
        {hint && <FieldDescription>{hint}</FieldDescription>}
      </FieldContent>
    </Field>
  );
}
