// 工具箱：分类 → 工具列表 → 工作区三栏。
// 左栏分类（竖向 Tabs）、中栏搜索 + 当前分类的工具（竖向 Tabs，选中项用组件自带的选中态）、右栏是选中工具的工作区。
// 选中的工具在 state/tools.ts 的 openId：批量处理页的工具菜单经 useTools.open 跳到这里并选中；没有选中时选第一个。

import { useEffect, useMemo, useState } from "react";
import { Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Tabs, TabsTrigger } from "@/components/ui/tabs";
import VerticalTabsList from "@/components/VerticalTabsList";
import { TOOLS, groupBy } from "@/features/registry";
import { useTools } from "@/state/tools";
import ToolWorkspace, { STATUS_LABEL, TOOL_PANEL_ID, toolTabId } from "./tools/ToolWorkspace";

const ALL = "all";
const CATEGORIES = groupBy(TOOLS);

function Categories({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return (
    <Tabs value={value} onValueChange={(v) => onChange(String(v))} orientation="vertical" className="gap-0">
      <VerticalTabsList aria-label="工具分类" className="w-full">
        <TabsTrigger value={ALL} className="flex-none">
          全部
          <span className="ml-auto text-muted-foreground in-data-active:text-inherit">{TOOLS.length}</span>
        </TabsTrigger>
        {CATEGORIES.map(([group, tools]) => (
          <TabsTrigger key={group} value={group} className="flex-none">
            {group}
            <span className="ml-auto text-muted-foreground in-data-active:text-inherit">{tools.length}</span>
          </TabsTrigger>
        ))}
      </VerticalTabsList>
    </Tabs>
  );
}

function ToolList({ query, category }: { query: string; category: string }) {
  const openId = useTools((s) => s.openId);
  const runningId = useTools((s) => (s.run?.running ? s.run.toolId : null));
  const select = useTools((s) => s.select);

  const tools = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (q) return TOOLS.filter((t) => `${t.name}${t.summary}${t.id}`.toLowerCase().includes(q));
    return category === ALL ? TOOLS : TOOLS.filter((t) => t.group === category);
  }, [query, category]);

  if (tools.length === 0) return <p className="px-3 py-6 text-xs text-muted-foreground">没有匹配的工具</p>;
  return (
    <Tabs value={openId ?? ""} onValueChange={(v) => select(String(v))} orientation="vertical" className="gap-0">
      <VerticalTabsList aria-label="工具" className="w-full gap-0.5">
        {tools.map((tool) => (
          <TabsTrigger
            key={tool.id}
            value={tool.id}
            id={toolTabId(tool.id)}
            aria-controls={tool.id === openId ? TOOL_PANEL_ID : undefined}
            aria-label={`打开 ${tool.name}`}
            className="flex-none"
          >
            <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
              <span className="max-w-full truncate">{tool.name}</span>
              <span className="max-w-full truncate font-normal text-muted-foreground in-data-active:text-inherit">{tool.summary}</span>
            </span>
            {tool.status !== "done" && <Badge variant="outline">{STATUS_LABEL[tool.status]}</Badge>}
            {runningId === tool.id && <Spinner aria-label="运行中" />}
          </TabsTrigger>
        ))}
      </VerticalTabsList>
    </Tabs>
  );
}

export default function ToolboxView() {
  const openId = useTools((s) => s.openId);
  const [query, setQuery] = useState("");
  const groupOf = (id: string | null) => TOOLS.find((t) => t.id === id)?.group ?? ALL;
  const [category, setCategory] = useState(() => groupOf(openId));
  const [seen, setSeen] = useState(openId);

  // 从别处（批量处理页的工具菜单）选中了不在当前分类里的工具时，切到它的分类
  if (seen !== openId) {
    setSeen(openId);
    if (!query.trim() && category !== ALL && groupOf(openId) !== category) setCategory(groupOf(openId));
  }

  useEffect(() => {
    if (!useTools.getState().openId && TOOLS[0]) useTools.getState().select(TOOLS[0].id);
  }, []);

  return (
    <div className="flex h-full">
      <nav className="w-[180px] shrink-0 border-r px-2 py-2">
        <Categories value={category} onChange={setCategory} />
      </nav>

      <section aria-label="工具列表" className="flex w-[300px] shrink-0 flex-col border-r">
        <div className="shrink-0 px-2 pt-2 pb-1">
          <InputGroup>
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
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="px-2 pb-2">
            <ToolList query={query} category={category} />
          </div>
        </ScrollArea>
      </section>

      <div className="min-w-0 flex-1">{openId && <ToolWorkspace id={openId} />}</div>
    </div>
  );
}
