import { useEffect, useState } from "react";
import { Layers, Moon, Settings, Sun, Wrench } from "lucide-react";
import BatchView from "./views/BatchView";
import ToolboxView from "./views/ToolboxView";
import SettingsView from "./views/SettingsView";

type ViewId = "batch" | "toolbox" | "settings";
type Theme = "light" | "dark";

const NAV = [
  { id: "batch", label: "批量处理", icon: Layers },
  { id: "toolbox", label: "工具箱", icon: Wrench },
  { id: "settings", label: "设置", icon: Settings },
] as const;

function initialTheme(): Theme {
  try {
    const saved = localStorage.getItem("theme");
    if (saved === "light" || saved === "dark") return saved;
  } catch {
    // 读不到就跟随系统
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export default function App() {
  const [view, setView] = useState<ViewId>("batch");
  const [theme, setTheme] = useState<Theme>(initialTheme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("theme", theme);
    } catch {
      // 忽略：主题仅本次生效
    }
  }, [theme]);

  return (
    <div className="flex h-full">
      <nav className="flex w-[76px] shrink-0 flex-col items-center gap-1 border-r border-line bg-surface py-3">
        <div className="mb-3 grid size-10 place-items-center rounded-xl bg-accent text-base font-bold text-accent-ink">
          CR
        </div>
        {NAV.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setView(id)}
            aria-current={view === id ? "page" : undefined}
            className={`flex w-16 flex-col items-center gap-1 rounded-lg py-2 text-[11px] transition-colors ${
              view === id ? "bg-surface-2 text-accent" : "text-muted hover:text-ink"
            }`}
          >
            <Icon size={20} />
            {label}
          </button>
        ))}
        <button
          onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          title={theme === "dark" ? "切换到浅色" : "切换到深色"}
          className="mt-auto grid size-10 place-items-center rounded-lg text-muted hover:bg-surface-2 hover:text-ink"
        >
          {theme === "dark" ? <Sun size={18} /> : <Moon size={18} />}
        </button>
      </nav>

      <main className="min-w-0 flex-1">
        {view === "batch" && <BatchView />}
        {view === "toolbox" && <ToolboxView />}
        {view === "settings" && <SettingsView />}
      </main>
    </div>
  );
}
