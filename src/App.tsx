import { lazy, Suspense, useEffect } from "react";
import { Layers, Moon, Settings, Sun, Wrench } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TooltipProvider } from "@/components/ui/tooltip";
import Hint from "./components/Hint";
import PreviewModal from "./components/PreviewModal";
import QuitConfirmDialog from "./components/QuitConfirmDialog";
import ShutdownDialog from "./components/ShutdownDialog";
import UpdateDialog from "./components/UpdateDialog";
import { isTauri, onOpenPaths, openProjectPage, takeLaunchPaths } from "./lib/backend";
import { browserKeyEvents, isEditable } from "./lib/desktop";
import { matchShortcut, useShortcutLabels, type ShortcutId } from "./lib/shortcuts";
import { initBackend } from "./state/events";
import { useLog } from "./state/log";
import { addFolder, addVideos, openPaths } from "./state/open";
import { useQueue } from "./state/queue";
import { useRecipe } from "./state/recipe";
import { useSettings } from "./state/settings";
import { syncTaskbarProgress } from "./state/taskbar";
import { useApplyTheme, useTheme } from "./state/theme";
import { useUpdate } from "./state/update";
import { useView, type ViewId } from "./state/view";
import BatchView from "./views/BatchView";
import StatusBar from "./views/StatusBar";

// 不在首屏的页面与工具对话框按需加载（减小首屏脚本）
const ToolboxView = lazy(() => import("./views/ToolboxView"));
const SettingsView = lazy(() => import("./views/SettingsView"));
// 开发环境的调试面板（src/debug/，docs/architecture/frontend.md“开发环境的调试面板”）：生产构建里 import.meta.env.DEV 是常量 false，
// 这个动态导入连同 src/debug/ 一起被剔除，产物里没有面板与入口
const DebugPanel = import.meta.env.DEV ? lazy(() => import("./debug/DebugPanel")) : null;

const NAV = [
  { id: "batch", label: "批量处理", icon: Layers, shortcut: "batch" },
  { id: "toolbox", label: "工具箱", icon: Wrench, shortcut: "toolbox" },
  { id: "settings", label: "设置", icon: Settings, shortcut: "settingsView" },
] as const satisfies readonly { id: ViewId; label: string; icon: unknown; shortcut: ShortcutId }[];

const triggerId = (id: ViewId) => `nav-${id}`;
const panelId = (id: ViewId) => `view-${id}`;

/** 是否有打开的对话框 / 菜单（这时全局快捷键让给它们，Esc 由它们自己关闭）。 */
const overlayOpen = () =>
  !!document.querySelector("[role=dialog], [role=alertdialog], [role=menu], [role=listbox][data-open], [data-slot=popover-content]");

const openDocs = () =>
  openProjectPage().catch((e) => useLog.getState().add("error", `无法打开使用文档：${String(e)}`));

/**
 * 全局快捷键（按键定义与显示文字都在 lib/shortcuts.ts，按当前平台匹配：macOS 用 ⌘，其余用 Ctrl）。
 * 监听 document 的冒泡阶段：输入框、菜单、对话框已处理（取消了默认行为）的按键不再处理；
 * 只有 lib/desktop.ts 当作浏览器快捷键拦下的（如“添加视频”的 O）仍交给应用。
 */
function useAppShortcuts() {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented && !browserKeyEvents.has(e)) return;
      if (e.repeat && e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      const { setView } = useView.getState();
      const handled = (() => {
        if (matchShortcut(e, "docs")) {
          if (isTauri) void openDocs();
          return true;
        }
        if (overlayOpen()) return false;
        if (matchShortcut(e, "addVideos") || matchShortcut(e, "addFolder")) {
          setView("batch");
          void (e.shiftKey ? addFolder() : addVideos());
          return true;
        }
        if (matchShortcut(e, "settings")) {
          setView("settings");
          return true;
        }
        const view = NAV.find((item) => matchShortcut(e, item.shortcut))?.id;
        if (view) {
          setView(view);
          return true;
        }
        if (matchShortcut(e, "start") && !isEditable(e.target)) {
          const queue = useQueue.getState();
          if (useView.getState().view !== "batch" || queue.running || queue.importing || queue.items.length === 0) return false;
          void queue.start(useRecipe.getState().recipe);
          return true;
        }
        if (matchShortcut(e, "stop") && !isEditable(e.target)) {
          const queue = useQueue.getState();
          if (!queue.running || useView.getState().view !== "batch") return false;
          void queue.stop();
          return true;
        }
        return false;
      })();
      if (handled) e.preventDefault();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);
}

/** 命令行带的文件：首次启动时的，以及再次启动程序时转交过来的（单实例）。 */
function useLaunchPaths(settingsLoaded: boolean) {
  useEffect(() => onOpenPaths((paths) => {
    useView.getState().setView("batch");
    openPaths(paths);
  }), []);
  useEffect(() => {
    if (!settingsLoaded || !isTauri) return;
    takeLaunchPaths()
      .then((paths) => paths.length > 0 && openPaths(paths))
      .catch(() => {});
  }, [settingsLoaded]);
}

function ViewFallback() {
  return (
    <div className="grid h-full place-items-center text-muted-foreground">
      <Spinner aria-label="加载中" />
    </div>
  );
}

export default function App() {
  const view = useView((s) => s.view);
  const setView = useView((s) => s.setView);
  const [theme, toggleTheme] = useTheme();
  const settingsLoaded = useSettings((s) => s.loaded);
  const keys = useShortcutLabels();

  useApplyTheme(theme);
  useAppShortcuts();
  useLaunchPaths(settingsLoaded);
  useEffect(() => initBackend(), []);
  useEffect(() => syncTaskbarProgress(), []);

  useEffect(() => {
    // set.auto-update：启动时检查一次（离线时静默）
    if (settingsLoaded && isTauri && useSettings.getState().settings.autoUpdate) void useUpdate.getState().check(false);
  }, [settingsLoaded]);

  return (
    <TooltipProvider delay={500} timeout={300}>
      <div className="flex h-full flex-col">
        {/*
          顶栏：主导航是横向 Tabs。页面内容不放在它的 Tabs 根节点里（页面里还有自己的 Tabs，如设置分组的竖向 Tabs、
          文本模块的“文本 1/2/3”，shadcn Tabs 的样式按“任意祖先”的方向套用）：Tabs 只包住导航条，页面作为 tabpanel 放在下面，
          用 id / aria-controls 与导航按钮关联。
        */}
        <header className="flex h-11 shrink-0 items-center gap-3 border-b bg-card px-3">
          <div aria-hidden className="grid size-6 place-items-center rounded-md bg-primary font-bold text-primary-foreground">
            CR
          </div>
          <span aria-hidden className="text-sm font-semibold">
            ClipRecipe
          </span>
          <Tabs value={view} onValueChange={(v) => setView(v as ViewId)}>
            <nav>
              <TabsList aria-label="主导航">
                {NAV.map(({ id, label, icon: Icon, shortcut }) => (
                  <Hint key={id} label={label} keys={keys[shortcut]} side="bottom">
                    <TabsTrigger value={id} id={triggerId(id)} aria-controls={panelId(id)}>
                      <Icon />
                      {label}
                    </TabsTrigger>
                  </Hint>
                ))}
              </TabsList>
            </nav>
          </Tabs>
          <Hint label={theme === "dark" ? "切换到浅色" : "切换到深色"} side="bottom">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={theme === "dark" ? "切换到浅色" : "切换到深色"}
              onClick={toggleTheme}
              className="ml-auto"
            >
              {theme === "dark" ? <Sun /> : <Moon />}
            </Button>
          </Hint>
        </header>

        <main
          key={view}
          role="tabpanel"
          id={panelId(view)}
          aria-labelledby={triggerId(view)}
          className="min-h-0 min-w-0 flex-1"
        >
          <Suspense fallback={<ViewFallback />}>
            {view === "batch" && <BatchView />}
            {view === "toolbox" && <ToolboxView />}
            {view === "settings" && <SettingsView />}
          </Suspense>
        </main>
        <StatusBar
          extra={
            DebugPanel && (
              <Suspense fallback={null}>
                <DebugPanel />
              </Suspense>
            )
          }
        />
      </div>
      <PreviewModal />
      <ShutdownDialog />
      <QuitConfirmDialog />
      <UpdateDialog />
    </TooltipProvider>
  );
}
