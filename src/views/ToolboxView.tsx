import { useMemo, useState } from "react";
import { Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { TOOLS, groupBy, type FeatureStatus } from "@/features/registry";
import { toolName, useTools } from "@/state/tools";

/** 只有还不能用的工具才标出状态；可用是常态，不加徽标。 */
const STATUS_LABEL: Record<Exclude<FeatureStatus, "done">, string> = {
  planned: "规划中",
  wip: "开发中",
};

/** 标题旁的数量：全部可用时只说总数，否则说明有几个还没做好。 */
function countText(): string {
  const pending = TOOLS.filter((t) => t.status !== "done").length;
  return pending === 0 ? `${TOOLS.length} 个独立工具` : `${TOOLS.length} 个独立工具，其中 ${pending} 个尚未完成`;
}

/** 正在运行的工具：关闭对话框后在这里重新打开。 */
function RunningBanner() {
  const run = useTools((s) => s.run);
  const open = useTools((s) => s.open);
  if (!run?.running) return null;
  const done = run.items.filter((i) => i.state === "done").length;
  return (
    <div className="flex items-center gap-2 border-b bg-card px-4 py-1.5 text-xs">
      <Spinner className="size-3.5" aria-label="运行中" />
      <span>
        “{toolName(run.toolId)}”正在运行 · 完成 {done}/{run.items.length}
      </span>
      <Button variant="outline" size="sm" className="ml-auto" onClick={() => open(run.toolId)}>
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
      <header className="flex h-11 shrink-0 items-center gap-3 border-b px-4">
        <h1 className="text-sm font-semibold">工具箱</h1>
        <span className="text-xs text-muted-foreground">
          {countText()}
        </span>
        <InputGroup className="ml-auto w-60">
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
        <div className="px-4 py-3">
          {groups.length === 0 && <p className="py-6 text-xs text-muted-foreground">没有匹配的工具</p>}
          {groups.map(([group, tools]) => (
            <section key={group} className="mb-4">
              <h2 className="mb-1.5 text-xs font-medium text-muted-foreground">{group}</h2>
              <div className="grid grid-cols-[repeat(auto-fill,minmax(200px,1fr))] gap-2">
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
                    // shadcn 的 Card 不是可交互组件：当按钮用时补上悬停底色与键盘焦点（与 Mira 按钮的焦点样式相同）
                    className="outline-none hover:bg-muted focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                  >
                    <CardHeader>
                      <CardTitle className="flex items-center justify-between gap-2">
                        {tool.name}
                        {tool.status !== "done" && (
                          <Badge variant="secondary">
                            {STATUS_LABEL[tool.status]}
                          </Badge>
                        )}
                      </CardTitle>
                      <CardDescription className="truncate">{tool.summary}</CardDescription>
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
