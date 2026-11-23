// 平台来源：全应用只从这里读“当前是哪个操作系统”（快捷键的显示与匹配、界面字体、标题栏着色、示例路径……）。
// 不要在别处读 navigator.userAgent / navigator.platform，也不要自己判断 metaKey / ctrlKey 谁是主修饰键。
//
// - 真实平台：桌面应用里以后端 app_info 的 platform（Rust 编译目标的 std::env::consts::OS）为准，启动时由
//   loadRealPlatform() 读取；读到之前、以及 `npm run dev` 在浏览器里跑（没有 Tauri）时，用 User-Agent 推断。
//   WebView2 / WKWebView / WebKitGTK 的 User-Agent 与编译目标一致，所以桌面应用里两者不会出现先后不同。
// - 模拟平台：只在开发环境（import.meta.env.DEV）存在，由调试面板设置（src/debug/）。生产构建里
//   `import.meta.env.DEV` 是常量 false，模拟相关的分支与函数被摇树剔除，getPlatform() 永远返回真实平台。
// - 用法：React 组件用 usePlatform()（平台变化时重新渲染）；事件处理等非渲染代码用 getPlatform()。
//   <html data-platform> 跟随当前平台，index.css 据此切换 --font-sans。

import { useSyncExternalStore } from "react";
import { getAppInfo, isTauri } from "./backend";

export type Platform = "windows" | "macos" | "linux";

export const PLATFORMS: readonly Platform[] = ["windows", "macos", "linux"];

export const PLATFORM_LABELS: Record<Platform, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

const isPlatform = (value: unknown): value is Platform => PLATFORMS.includes(value as Platform);

/** 没有 Tauri 时（或读到后端结果之前）从 User-Agent 推断。 */
function platformFromUserAgent(): Platform {
  const ua = typeof navigator === "undefined" ? "" : navigator.userAgent;
  if (/Macintosh|Mac OS X|iPhone|iPad/.test(ua)) return "macos";
  if (/Windows/.test(ua)) return "windows";
  if (/Linux|X11|CrOS/.test(ua)) return "linux";
  return "windows";
}

/** 真实平台从哪里得到的：后端（app_info）或 User-Agent 推断。 */
export type PlatformSource = "tauri" | "userAgent";

let real: Platform = platformFromUserAgent();
let realSource: PlatformSource = "userAgent";
let simulated: Platform | null = null;
const listeners = new Set<() => void>();

function changed() {
  applyPlatformAttribute();
  for (const listener of listeners) listener();
}

/** 当前平台（开发环境下可能是调试面板模拟的平台）。 */
export function getPlatform(): Platform {
  if (import.meta.env.DEV && simulated) return simulated;
  return real;
}

/** 真实平台（不受模拟影响）与它的来源。 */
export function getRealPlatform(): { platform: Platform; source: PlatformSource } {
  return { platform: real, source: realSource };
}

export function subscribePlatform(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** 当前平台；平台变化（读到后端结果、调试面板切换）时重新渲染。 */
export function usePlatform(): Platform {
  return useSyncExternalStore(subscribePlatform, getPlatform);
}

/** 把当前平台写到 <html data-platform>（index.css 按它选择界面字体）。main.tsx 在首次渲染前调用一次。 */
export function applyPlatformAttribute(): void {
  if (typeof document !== "undefined") document.documentElement.dataset.platform = getPlatform();
}

/** 桌面应用里向后端读取真实平台（启动时调用一次；浏览器里什么也不做）。 */
export async function loadRealPlatform(): Promise<void> {
  if (!isTauri) return;
  try {
    const { platform } = await getAppInfo();
    if (!isPlatform(platform)) return;
    const before = getPlatform();
    real = platform;
    realSource = "tauri";
    if (getPlatform() !== before) changed();
    else applyPlatformAttribute();
  } catch {
    // 读不到时保留 User-Agent 推断的结果
  }
}

/** 开发环境：模拟平台（null 取消模拟）。只给调试面板用；生产构建里是空操作并被剔除。 */
export function simulatePlatform(platform: Platform | null): void {
  if (!import.meta.env.DEV) return;
  simulated = platform;
  changed();
}

/** 开发环境：正在模拟的平台（没有模拟时为 null）。 */
export function getSimulatedPlatform(): Platform | null {
  return import.meta.env.DEV ? simulated : null;
}

// ---------------------------------------------------------------- 按平台不同的写法

/** 主修饰键是否按下：macOS 是 ⌘（metaKey），其余是 Ctrl（ctrlKey）。 */
export function isModKey(e: { ctrlKey: boolean; metaKey: boolean }, platform: Platform = getPlatform()): boolean {
  return platform === "macos" ? e.metaKey : e.ctrlKey;
}

/** 另一个修饰键（macOS 的 Control、其余平台的 Win / Super）是否按下：快捷键匹配时要求它没按。 */
export function isOtherModKey(e: { ctrlKey: boolean; metaKey: boolean }, platform: Platform = getPlatform()): boolean {
  return platform === "macos" ? e.ctrlKey : e.metaKey;
}

/** 系统文件管理器的名字。 */
export const FILE_MANAGER: Record<Platform, string> = { windows: "资源管理器", macos: "访达", linux: "文件管理器" };

/** 界面里的示例路径（只用于说明文字、占位符；真实路径的处理与平台无关，见 lib/format.ts）。 */
export const EXAMPLE_PATHS: Record<Platform, { ffmpeg: string; font: string; videos: string }> = {
  windows: { ffmpeg: "C:\\ffmpeg\\bin\\ffmpeg.exe", font: "C:\\Windows\\Fonts\\msyh.ttc", videos: "D:\\视频\\素材" },
  macos: { ffmpeg: "/opt/homebrew/bin/ffmpeg", font: "/System/Library/Fonts/PingFang.ttc", videos: "/Users/me/Movies/素材" },
  linux: {
    ffmpeg: "/usr/bin/ffmpeg",
    font: "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    videos: "/home/me/Videos/素材",
  },
};

/** 把示例目录与文件名拼成示例路径（按平台的分隔符）。 */
export function examplePath(dir: string, name: string, platform: Platform = getPlatform()): string {
  return `${dir}${platform === "windows" ? "\\" : "/"}${name}`;
}
