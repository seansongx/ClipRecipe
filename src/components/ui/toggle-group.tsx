"use client"

import * as React from "react"
import { Toggle as TogglePrimitive } from "@base-ui/react/toggle"
import { ToggleGroup as ToggleGroupPrimitive } from "@base-ui/react/toggle-group"
import { type VariantProps } from "class-variance-authority"
import { cn } from "cn"

import { toggleVariants } from "@/components/ui/toggle"

type ToggleSize = NonNullable<VariantProps<typeof toggleVariants>["size"]>

/**
 * segmented 变体、spacing=0：整组是一条轨道（细边框、background 底、2px 内边距），各项是轨道里没有描边的段。
 * 轨道总高与同尺寸的按钮 / 输入框相同（h-6 / h-7 / h-8），段高 = 总高 − 2 × 内边距 − 2 × 边框。选项一行放不下时轨道换成多行。
 */
const TRACK =
  "min-h-7 gap-0.5 rounded-lg border bg-background p-0.5 data-[size=lg]:min-h-8 data-[size=sm]:min-h-6 data-[size=sm]:rounded-lg"
const SEGMENT: Record<ToggleSize, string> = {
  default: "h-[calc(--spacing(6)-2px)] rounded-md px-2.5",
  sm: "h-[calc(--spacing(5)-2px)] rounded-md px-2.5 text-xs",
  lg: "h-[calc(--spacing(7)-2px)] rounded-md px-3",
}
/** segmented 变体、spacing ≥ 1：各自独立的小块，边框用 border、底色与轨道相同，选中态与段一致。 */
const BLOCK = "border border-border bg-background"

/** 其它变体 spacing=0 时连在一起的按钮组（相邻项去掉中间的圆角与重复边框）。 */
const ATTACHED =
  "group-data-[spacing=0]/toggle-group:rounded-none group-data-[spacing=0]/toggle-group:px-2 group-data-[spacing=0]/toggle-group:has-data-[icon=inline-end]:pr-1.5 group-data-[spacing=0]/toggle-group:has-data-[icon=inline-start]:pl-1.5 group-data-horizontal/toggle-group:data-[spacing=0]:first:rounded-l-md group-data-vertical/toggle-group:data-[spacing=0]:first:rounded-t-md group-data-horizontal/toggle-group:data-[spacing=0]:last:rounded-r-md group-data-vertical/toggle-group:data-[spacing=0]:last:rounded-b-md"

const ToggleGroupContext = React.createContext<
  VariantProps<typeof toggleVariants> & {
    spacing?: number
    orientation?: "horizontal" | "vertical"
  }
>({
  size: "default",
  variant: "default",
  spacing: 2,
  orientation: "horizontal",
})

function ToggleGroup({
  className,
  variant,
  size,
  spacing = 2,
  orientation = "horizontal",
  children,
  ...props
}: ToggleGroupPrimitive.Props &
  VariantProps<typeof toggleVariants> & {
    spacing?: number
    orientation?: "horizontal" | "vertical"
  }) {
  const track = variant === "segmented" && spacing === 0
  return (
    <ToggleGroupPrimitive
      data-slot="toggle-group"
      data-variant={variant}
      data-size={size}
      data-spacing={spacing}
      data-orientation={orientation}
      style={{ "--gap": spacing } as React.CSSProperties}
      className={cn(
        "group/toggle-group flex w-fit flex-row items-center gap-[--spacing(var(--gap))] rounded-md data-[size=sm]:rounded-[min(var(--radius-md),8px)] data-vertical:flex-col data-vertical:items-stretch",
        track && TRACK,
        className
      )}
      {...props}
    >
      <ToggleGroupContext.Provider
        value={{ variant, size, spacing, orientation }}
      >
        {children}
      </ToggleGroupContext.Provider>
    </ToggleGroupPrimitive>
  )
}

function ToggleGroupItem({
  className,
  children,
  variant = "default",
  size = "default",
  ...props
}: TogglePrimitive.Props & VariantProps<typeof toggleVariants>) {
  const context = React.useContext(ToggleGroupContext)
  const v = context.variant || variant
  const s = context.size || size || "default"
  const segmented = v === "segmented"

  return (
    <TogglePrimitive
      data-slot="toggle-group-item"
      data-variant={v}
      data-size={s}
      data-spacing={context.spacing}
      className={cn(
        "shrink-0 focus:z-10 focus-visible:z-10",
        !segmented && ATTACHED,
        toggleVariants({ variant: v, size: s }),
        segmented && (context.spacing === 0 ? SEGMENT[s] : BLOCK),
        className
      )}
      {...props}
    >
      {children}
    </TogglePrimitive>
  )
}

export { ToggleGroup, ToggleGroupItem }
