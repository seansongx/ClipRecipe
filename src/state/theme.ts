// 主题（set.theme）：浅色 / 深色 / 跟随系统。首帧前由 public/boot-theme.js 按设置写好 <html data-theme>，
// 这里只在设置读取后与主题变化时更新它，并把主题颜色同步给窗口（标题栏、底色，见 src-tauri/src/desktop.rs）。

import { useEffect, useState } from "react";
import { isTauri, setWindowChrome, windowReady, type ThemeSetting } from "../lib/backend";
import { useSettings } from "./settings";

export type Theme = "light" | "dark";

const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

/** 首帧时的主题（boot-theme.js 已写在 <html> 上）。 */
function bootTheme(): Theme {
  const current = document.documentElement.dataset.theme;
  if (current === "light" || current === "dark") return current;
  return darkQuery().matches ? "dark" : "light";
}

/** 系统当前是否为深色（随系统切换实时更新）。 */
function useSystemDark(): boolean {
  const [dark, setDark] = useState(() => darkQuery().matches);
  useEffect(() => {
    const query = darkQuery();
    const onChange = () => setDark(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return dark;
}

/** 当前主题与“切换浅色 / 深色”（导航栏按钮直接改为浅色或深色）。 */
export function useTheme(): [Theme, () => void] {
  const loaded = useSettings((s) => s.loaded);
  const setting = useSettings((s) => s.settings.theme);
  const systemDark = useSystemDark();
  const [initial] = useState(bootTheme);
  const theme: Theme = !loaded ? initial : setting === "system" ? (systemDark ? "dark" : "light") : setting;
  const toggle = () => useSettings.getState().update({ theme: theme === "dark" ? "light" : "dark" });
  return [theme, toggle];
}

/** 主题变量的颜色，统一成 #rrggbb（构建时 CSS 压缩会把 #ffffff 写成 #fff，也可能是别的写法）。 */
function cssVar(name: string): string {
  const probe = document.createElement("span");
  probe.style.color = `var(${name})`;
  probe.style.display = "none";
  document.body.append(probe);
  const rgb = getComputedStyle(probe).color.match(/\d+(\.\d+)?/g)?.map(Number) ?? [];
  probe.remove();
  return `#${rgb.slice(0, 3).map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
}

/** 把主题应用到页面与窗口。 */
function applyTheme(theme: Theme, preference: ThemeSetting): Promise<void> {
  // index.css 的深色变量与 shadcn 的 dark: 变体都以 data-theme 为准
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem("theme", theme);
  } catch {
    // 忽略：只影响浏览器预览时的初始主题
  }
  if (!isTauri) return Promise.resolve();
  return setWindowChrome({
    preference,
    dark: theme === "dark",
    titlebar: cssVar("--titlebar"),
    text: cssVar("--foreground"),
    border: cssVar("--border"),
    background: cssVar("--background"),
  }).catch(() => {});
}

/** 主题变化时更新页面与窗口；第一次应用后（首帧已画好）通知后端显示窗口。 */
export function useApplyTheme(theme: Theme): void {
  const loaded = useSettings((s) => s.loaded);
  const setting = useSettings((s) => s.settings.theme);
  // 设置读取前用首帧时的偏好（boot-theme.js 读取的就是同一个设置）
  const preference: ThemeSetting = loaded ? setting : bootPreference();
  useEffect(() => {
    void applyTheme(theme, preference);
  }, [theme, preference]);
  useEffect(() => {
    if (!isTauri) return;
    // 两帧之后：React 的首屏已提交并绘制
    requestAnimationFrame(() => requestAnimationFrame(() => void windowReady().catch(() => {})));
  }, []);
}

function bootPreference(): ThemeSetting {
  const pref = (window as { __CLIPRECIPE_BOOT__?: { theme?: string } }).__CLIPRECIPE_BOOT__?.theme;
  return pref === "light" || pref === "dark" ? pref : "system";
}
