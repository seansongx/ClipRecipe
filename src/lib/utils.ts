export { cn } from "cn"

/** shadcn ScrollArea 真正滚动的元素（根节点下的 viewport），用于自动滚动。 */
export function scrollViewport(root: HTMLElement | null): HTMLElement | null {
  return root?.querySelector<HTMLElement>('[data-slot="scroll-area-viewport"]') ?? null
}
