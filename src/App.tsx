import { lazy, Suspense, useEffect, useState } from "react";
import { Layers, Moon, Settings, Sun, Wrench } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TooltipProvider } from "@/components/ui/tooltip";
import Hint from "./components/Hint";
import PreviewModal from "./components/PreviewModal";
import ShutdownDialog from "./components/ShutdownDialog";
import UpdateDialog from "./components/UpdateDialog";
import { isTauri } from "./lib/backend";
import { initBackend } from "./state/events";
import { useSettings } from "./state/settings";
import { useUpdate } from "./state/update";
import BatchView from "./views/BatchView";

// 不在首屏的页面与工具对话框按需加载（减小首屏脚本）
const ToolboxView = lazy(() => import("./views/ToolboxView"));
const SettingsView = lazy(() => import("./views/SettingsView"));
const ToolDialog = lazy(() => import("./views/tools/ToolDialog"));

type ViewId = "batch" | "toolbox" | "settings";
type Theme = "light" | "dark";

const NAV = [
  { id: "batch", label: "批量处理", icon: Layers },
  { id: "toolbox", label: "工具箱", icon: Wrench },
  { id: "settings", label: "设置", icon: Settings },
] as const;

const triggerId = (id: ViewId) => `nav-${id}`;
const panelId = (id: ViewId) => `view-${id}`;

const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

/** 设置读取完成前使用上次的主题（localStorage 缓存），避免启动时闪一下。 */
function cachedTheme(): Theme {
  try {
    const saved = localStorage.getItem("theme");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // 读不到就跟随系统
  }
  return darkQuery().matches ? "dark" : "light";
}

/** 系统当前是否为深色（随系统切换更新）。 */
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

/** set.theme：浅色 / 深色 / 跟随系统；导航栏按钮直接切换为浅色或深色。 */
function useTheme(): [Theme, () => void] {
  const loaded = useSettings((s) => s.loaded);
  const setting = useSettings((s) => s.settings.theme);
  const systemDark = useSystemDark();
  const [initial] = useState(cachedTheme);
  const theme: Theme = !loaded ? initial : setting === "system" ? (systemDark ? "dark" : "light") : setting;
  const toggle = () => useSettings.getState().update({ theme: theme === "dark" ? "light" : "dark" });
  return [theme, toggle];
}

function ViewFallback() {
  return (
    <div className="grid h-full place-items-center text-muted-foreground">
      <Spinner aria-label="加载中" />
    </div>
  );
}

export default function App() {
  const [view, setView] = useState<ViewId>("batch");
  const [theme, toggleTheme] = useTheme();
  const settingsLoaded = useSettings((s) => s.loaded);

  useEffect(() => initBackend(), []);

  useEffect(() => {
    // set.auto-update：启动时检查一次（离线时静默）
    if (settingsLoaded && isTauri && useSettings.getState().settings.autoUpdate) void useUpdate.getState().check(false);
  }, [settingsLoaded]);

  useEffect(() => {
    // index.css 的深色变量与 shadcn 的 dark: 变体都以 data-theme 为准
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("theme", theme);
    } catch {
      // 忽略：主题仅本次生效
    }
  }, [theme]);

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full">
        {/*
          主导航是竖向 Tabs，但页面内容不放在它的 Tabs 根节点里：shadcn Tabs 的样式用
          group-data-vertical/tabs 这类“任意祖先”变体，页面若在竖向根节点之内，嵌套在页面里的横向 Tabs
          （如文本模块的“文本 1/2/3”）也会被套上竖向样式。所以 Tabs 只包住导航条，页面作为 tabpanel 放在旁边，
          用 id / aria-controls 与导航按钮关联。
        */}
        <Tabs
          value={view}
          onValueChange={(v) => setView(v as ViewId)}
          orientation="vertical"
          className="shrink-0 gap-0"
        >
          <nav className="flex h-full w-[76px] flex-col items-center gap-1 border-r bg-card py-3">
            <div className="mb-3 grid size-10 place-items-center rounded-xl bg-primary text-base font-bold text-primary-foreground">
              CR
            </div>
            <TabsList variant="line" aria-label="主导航" className="w-full gap-1 bg-transparent p-0">
              {NAV.map(({ id, label, icon: Icon }) => (
                <TabsTrigger
                  key={id}
                  value={id}
                  id={triggerId(id)}
                  aria-controls={panelId(id)}
                  className="h-auto w-16 flex-none flex-col justify-center gap-1 rounded-lg px-0 py-2 text-[11px] font-normal text-muted-foreground after:hidden hover:bg-muted hover:text-foreground data-active:bg-muted data-active:text-primary dark:data-active:border-transparent dark:data-active:bg-muted dark:data-active:text-primary group-data-vertical/tabs:w-16 group-data-vertical/tabs:justify-center [&_svg:not([class*='size-'])]:size-5"
                >
                  <Icon />
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
            <Hint label={theme === "dark" ? "切换到浅色" : "切换到深色"} side="right">
              <Button
                variant="ghost"
                size="icon"
                aria-label={theme === "dark" ? "切换到浅色" : "切换到深色"}
                onClick={toggleTheme}
                className="mt-auto size-10 text-muted-foreground"
              >
                {theme === "dark" ? <Sun /> : <Moon />}
              </Button>
            </Hint>
          </nav>
        </Tabs>

        <main
          key={view}
          role="tabpanel"
          id={panelId(view)}
          aria-labelledby={triggerId(view)}
          className="h-full min-w-0 flex-1 text-sm"
        >
          <Suspense fallback={<ViewFallback />}>
            {view === "batch" && <BatchView />}
            {view === "toolbox" && <ToolboxView />}
            {view === "settings" && <SettingsView />}
          </Suspense>
        </main>
      </div>
      <PreviewModal />
      <ShutdownDialog />
      <UpdateDialog />
      <Suspense fallback={null}>
        <ToolDialog />
      </Suspense>
    </TooltipProvider>
  );
}
