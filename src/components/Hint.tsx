import type { ComponentProps, ReactElement, ReactNode } from "react";
import { Slot } from "radix-ui";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/**
 * 把 Tooltip 触发器的属性合并到被包的控件上，但丢掉它的 `data-state`（提示的 closed / delayed-open）与 `data-slot`。
 *
 * 原因：Radix 的 `asChild` 会把外层属性作为 props 传给被包的组件，而 shadcn / Radix 控件把收到的 props 放在自己的属性
 * 之后展开——于是 Tooltip 的 `data-state="closed"` 覆盖了 TabsTrigger 的 `active`、ToggleGroupItem 的 `on`、
 * Switch / Checkbox 的 `checked`，依赖 `data-state` 的选中样式全部失效（侧栏导航曾因此没有选中态）。
 * 在这一层把它们去掉，被包的控件就保留自己的状态属性；其余属性（事件、ref、aria-describedby）照常合并。
 */
function StateSafeSlot({
  "data-state": _tooltipState,
  "data-slot": _tooltipSlot,
  ...props
}: ComponentProps<typeof Slot.Root> & { "data-state"?: string; "data-slot"?: string }) {
  return <Slot.Root {...props} />;
}

/**
 * 给一个控件加 shadcn Tooltip 提示（代替原生 title 属性，桌面风格、统一延迟）。图标按钮仍需自带 aria-label。
 * 可以直接包带状态的 Radix 控件（TabsTrigger、ToggleGroupItem、Switch…）：它们的 `data-state` 不会被提示改写
 * （见上面的 StateSafeSlot；e2e 的界面状态审计会检查 aria 状态与 data-state 一致）。
 * `keys`：快捷键，显示在提示右侧（如 "Ctrl+O"）。
 * `disabledTrigger`：控件可能被禁用时设为 true——禁用的按钮收不到鼠标事件，提示改挂在外面包的一层上
 * （例如说明“为什么不能点”）。
 */
export default function Hint({
  label,
  keys,
  side,
  align,
  disabledTrigger,
  children,
}: {
  label: ReactNode;
  keys?: string;
  side?: "top" | "right" | "bottom" | "left";
  align?: "start" | "center" | "end";
  disabledTrigger?: boolean;
  children: ReactElement;
}) {
  if (!label && !keys) return children;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <StateSafeSlot>
          {disabledTrigger ? (
            <span data-slot="hint-wrapper" className="inline-flex" tabIndex={-1}>
              {children}
            </span>
          ) : (
            children
          )}
        </StateSafeSlot>
      </TooltipTrigger>
      <TooltipContent side={side} align={align}>
        {label}
        {keys && <Kbd>{keys}</Kbd>}
      </TooltipContent>
    </Tooltip>
  );
}
