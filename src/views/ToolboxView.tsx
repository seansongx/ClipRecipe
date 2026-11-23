import { useMemo, useState } from "react";
import { Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { TOOLS, groupBy, type FeatureStatus } from "@/features/registry";
import { cn } from "@/lib/utils";
import { toolName, useTools } from "@/state/tools";

const STATUS_LABEL: Record<FeatureStatus, string> = {
  planned: "规划中",
  wip: "开发中",
  done: "可用",
};

/** 正在运行的工具：关闭对话框后在这里重新打开。 */
function RunningBanner() {
  const run = useTools((s) => s.run);
  const open = useTools((s) => s.open);
  if (!run?.running) return null;
  const done = run.items.filter((i) => i.state === "done").length;
  return (
    <div className="flex items-center gap-2 border-b bg-card px-5 py-2 text-xs">
      <Spinner className="size-3.5 text-primary" aria-label="运行中" />
      <span>
        “{toolName(run.toolId)}”正在运行 · 完成 {done}/{run.items.length}
      </span>
      <Button variant="outline" size="xs" className="ml-auto" onClick={() => open(run.toolId)}>
        查看进度
      </Button>
    </div>
  );
}

export default function ToolboxView() {
  const [query, setQuery] = useState("");
  const open = useTools((s) => s.open);

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matched = q
      ? TOOLS.filter((t) => `${t.name}${t.summary}${t.id}`.toLowerCase().includes(q))
      : TOOLS;
    return groupBy(matched);
  }, [query]);

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-4 border-b px-5 py-3">
        <h1 className="text-base font-semibold">工具箱</h1>
        <span className="text-muted-foreground">
          {TOOLS.length} 个独立工具 · {TOOLS.filter((t) => t.status === "done").length} 个可用
        </span>
        <InputGroup className="ml-auto w-64 bg-card">
          <InputGroupAddon>
            <Search />
          </InputGroupAddon>
          <InputGroupInput
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索工具"
            aria-label="搜索工具"
          />
        </InputGroup>
      </header>
      <RunningBanner />

      <ScrollArea className="min-h-0 flex-1">
        <div className="px-5 py-4">
          {groups.length === 0 && <p className="py-16 text-center text-muted-foreground">没有匹配的工具</p>}
          {groups.map(([group, tools]) => (
            <section key={group} className="mb-6">
              <h2 className="mb-2 text-xs font-medium text-muted-foreground">{group}</h2>
              <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
                {tools.map((tool) => (
                  <Card
                    key={tool.id}
                    size="sm"
                    role="button"
                    tabIndex={0}
                    aria-label={`打开 ${tool.name}`}
                    onClick={() => open(tool.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        open(tool.id);
                      }
                    }}
                    className={cn(
                      "cursor-pointer py-3 shadow-none outline-none focus-visible:ring-2 focus-visible:ring-ring",
                      tool.status === "done" ? "hover:ring-primary" : "opacity-70 hover:opacity-100",
                    )}
                  >
                    <CardHeader className="px-3">
                      <CardTitle className="flex items-center justify-between gap-2 font-medium">
                        {tool.name}
                        <Badge
                          variant={tool.status === "done" ? "default" : "secondary"}
                          className={cn("font-normal", tool.status !== "done" && "text-muted-foreground")}
                        >
                          {STATUS_LABEL[tool.status]}
                        </Badge>
                      </CardTitle>
                      <CardDescription className="text-xs">{tool.summary}</CardDescription>
                    </CardHeader>
                  </Card>
                ))}
              </div>
            </section>
          ))}
        </div>
      </ScrollArea>
    </div>
  );
}
