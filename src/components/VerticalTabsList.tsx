import type { ComponentProps } from "react";
import { TabsList } from "@/components/ui/tabs";

type TabsListProps = ComponentProps<typeof TabsList>;

/**
 * 竖向标签栏里按键 → 交给 Base UI（它按横向处理）时当作的按键；null 表示竖向时不处理。
 * Base UI 的横向“前进 / 后退”键在从右到左的布局里互换；本应用只有从左到右的布局。
 */
const VERTICAL_KEYS: Record<string, string | null> = {
  ArrowUp: "ArrowLeft",
  ArrowDown: "ArrowRight",
  ArrowLeft: null,
  ArrowRight: null,
};

/**
 * 竖向 Tabs 的标签栏：shadcn 的 TabsList，外加竖向的键盘行为。与 `<Tabs orientation="vertical">` 一起用。
 *
 * 为什么需要：shadcn（base-mira）的 `Tabs` 把 `orientation` 取走、只写成 `data-orientation`（决定竖向样式），
 * 没有传给 Base UI 的 `Tabs.Root`，所以 Base UI 按横向处理：方向键是 ← / →，`aria-orientation` 缺失。
 * components/ui/ 是 CLI 原版不能改，这里在 TabsList 上补齐：
 * - `aria-orientation="vertical"`（Base UI 把使用处的属性合并在它的默认属性之后，所以能覆盖）；
 * - 按键映射：使用处的 onKeyDown 先于 Base UI 的处理执行、拿到的是同一个事件对象。↑ / ↓ 改成 ← / → 再交给 Base UI，
 *   ← / → 用 preventBaseUIHandler 拦下。移动焦点仍由 Base UI 自己完成，所以它记录的高亮项、roving tabindex 与焦点
 *   始终一致（Home / End、Tab 离开再回来都以它为准），首尾循环、跳过禁用项、activateOnFocus 也照旧。
 *   不要改成自己调用 focus()：Base UI 只在收到 focus 事件时同步高亮项，窗口不在前台时 focus() 不发事件，会不同步。
 * shadcn 上游把 orientation 传给 Base UI 之后，删掉这个组件、改回 TabsList 即可。
 */
export default function VerticalTabsList({ onKeyDown, ...props }: TabsListProps) {
  return (
    <TabsList
      aria-orientation="vertical"
      {...props}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (event.defaultPrevented || event.baseUIHandlerPrevented) return;
        const key = VERTICAL_KEYS[event.key];
        if (key === undefined) return;
        if (key === null) event.preventBaseUIHandler();
        else Object.defineProperty(event, "key", { value: key });
      }}
    />
  );
}
