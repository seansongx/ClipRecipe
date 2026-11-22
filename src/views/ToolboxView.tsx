import { useMemo, useState } from "react";
import { Search } from "lucide-react";
import { TOOLS, groupBy, type FeatureStatus } from "../features/registry";

const STATUS_LABEL: Record<FeatureStatus, string> = {
  planned: "规划中",
  wip: "开发中",
  done: "可用",
};

export default function ToolboxView() {
  const [query, setQuery] = useState("");

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matched = q
      ? TOOLS.filter((t) => `${t.name}${t.summary}${t.id}`.toLowerCase().includes(q))
      : TOOLS;
    return groupBy(matched);
  }, [query]);

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-4 border-b border-line px-5 py-3">
        <h1 className="text-base font-semibold">工具箱</h1>
        <span className="text-muted">{TOOLS.length} 个独立工具</span>
        <label className="ml-auto flex w-64 items-center gap-2 rounded-lg border border-line bg-surface px-3 py-1.5">
          <Search size={16} className="text-muted" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索工具"
            className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted"
          />
        </label>
      </header>

      <div className="flex-1 overflow-y-auto px-5 py-4">
        {groups.length === 0 && <p className="py-16 text-center text-muted">没有匹配的工具</p>}
        {groups.map(([group, tools]) => (
          <section key={group} className="mb-6">
            <h2 className="mb-2 text-xs font-medium text-muted">{group}</h2>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
              {tools.map((tool) => (
                <button
                  key={tool.id}
                  disabled={tool.status !== "done"}
                  className="rounded-xl border border-line bg-surface p-3 text-left enabled:hover:border-accent disabled:opacity-70"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium">{tool.name}</span>
                    <span className="shrink-0 rounded-full bg-surface-2 px-2 py-0.5 text-[11px] text-muted">
                      {STATUS_LABEL[tool.status]}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-muted">{tool.summary}</p>
                </button>
              ))}
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
