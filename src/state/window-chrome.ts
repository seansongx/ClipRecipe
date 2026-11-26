// 窗口配色：应用只有深色主题（index.html 写死 <html data-theme="dark">），窗口主题在后端创建时已固定为深色。
// 这里把颜色变量的实际值同步给窗口（底色、Windows 11 的标题栏，见 src-tauri/src/desktop.rs），并在首帧画好后通知后端显示窗口。

import { useEffect } from "react";
import { isTauri, setWindowChrome, windowReady } from "../lib/backend";
import { usePlatform, type Platform } from "../lib/platform";

/** 颜色变量的计算值（如 `oklch(0.145 0 0)`、`oklch(1 0 0 / 10%)`）。 */
function computedColor(name: string): string {
  const probe = document.createElement("span");
  probe.style.color = `var(${name})`;
  probe.style.display = "none";
  document.body.append(probe);
  const color = getComputedStyle(probe).color;
  probe.remove();
  return color;
}

/**
 * 颜色变量换算成 #rrggbb（Windows 的 DWM 只接受 RGB）。变量是 oklch，可能带透明度，
 * 所以画在 1×1 的 canvas 上读回 sRGB：先铺不透明的 `over`（默认 --background），再叠上这个颜色。
 */
function cssVar(name: string, over = "--background"): string {
  const ctx = document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  if (!ctx) return "";
  ctx.canvas.width = ctx.canvas.height = 1;
  ctx.fillStyle = computedColor(over);
  ctx.fillRect(0, 0, 1, 1);
  ctx.fillStyle = computedColor(name);
  ctx.fillRect(0, 0, 1, 1);
  const [r = 0, g = 0, b = 0] = ctx.getImageData(0, 0, 1, 1).data;
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

/**
 * 把配色传给窗口。底色各平台都设置；标题栏 / 标题文字 / 边框着色只有 Windows 11 的 DWM 支持，只在平台为 Windows 时
 * 换算并传给后端；其它平台传空串，后端把这三个颜色恢复为系统默认（开发环境模拟平台来回切换时结果确定）。
 */
function applyChrome(platform: Platform): Promise<void> {
  const caption = platform === "windows";
  return setWindowChrome({
    titlebar: caption ? cssVar("--titlebar") : "",
    text: caption ? cssVar("--foreground") : "",
    border: caption ? cssVar("--border") : "",
    background: cssVar("--background"),
  }).catch(() => {});
}

/** 启动时与平台变化时同步窗口配色；第一次渲染后（首帧已画好）通知后端显示窗口。 */
export function useWindowChrome(): void {
  const platform = usePlatform();
  useEffect(() => {
    if (isTauri) void applyChrome(platform);
  }, [platform]);
  useEffect(() => {
    if (!isTauri) return;
    // 两帧之后：React 的首屏已提交并绘制
    requestAnimationFrame(() => requestAnimationFrame(() => void windowReady().catch(() => {})));
  }, []);
}
