import { cloneElement, type HTMLAttributes, type ReactElement, type ReactNode, type Ref } from "react";
import { mergeProps } from "@base-ui/react/merge-props";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

type TriggerProps = HTMLAttributes<HTMLElement> & { ref?: Ref<HTMLElement>; "data-slot"?: string };

/**
 * 给一个控件加 shadcn Tooltip 提示（代替原生 title 属性，桌面风格、统一延迟）。图标按钮仍需自带 aria-label。
 * `keys`：快捷键，显示在提示右侧（如 "Ctrl+O"）。
 * `disabledTrigger`：控件可能被禁用时设为 true——禁用的按钮收不到鼠标事件，提示改挂在外面包的一层上
 * （例如说明“为什么不能点”）。
 *
 * 触发器用 Base UI 的 `render` 渲染成被包的控件。Base UI 的状态属性各不相同（提示是 data-popup-open，
 * Tabs 是 data-active，Toggle 是 data-pressed），不会互相覆盖；但 shadcn 的 TooltipTrigger 传进来的
 * `data-slot="tooltip-trigger"` 会被 shadcn 控件放在自己的 data-slot 之后展开、把它改掉，
 * 而 Mira 的样式与组合（ButtonGroup、InputGroup 等）按 data-slot 选择元素，所以合并时丢掉这一个属性。
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
  const target: ReactElement<TriggerProps> = disabledTrigger ? (
    <span data-slot="hint-wrapper" className="inline-flex" tabIndex={-1}>
      {children}
    </span>
  ) : (
    (children as ReactElement<TriggerProps>)
  );
  return (
    <Tooltip>
      <TooltipTrigger
        render={({ "data-slot": _tooltipSlot, ...props }: TriggerProps) => {
          const merged = mergeProps<"span">(props, target.props);
          return cloneElement(target, { ...merged, ref: props.ref });
        }}
      />
      <TooltipContent side={side} align={align}>
        {label}
        {keys && <Kbd>{keys}</Kbd>}
      </TooltipContent>
    </Tooltip>
  );
}
