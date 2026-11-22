import { useEffect, useState } from "react";
import { Film, FolderOpen, Play, Plus, Trash2, X } from "lucide-react";
import { PIPELINE_MODULES, groupBy } from "../features/registry";
import { isTauri, onFilesDropped, pickFolder, pickVideos } from "../lib/backend";

const MODULE_GROUPS = groupBy(PIPELINE_MODULES);

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

export default function BatchView() {
  const [queue, setQueue] = useState<string[]>([]);
  const [hovering, setHovering] = useState(false);
  const [enabled, setEnabled] = useState<Set<string>>(() => new Set(["pipe.fps"]));
  const [fission, setFission] = useState(1);
  const [outputDir, setOutputDir] = useState("");

  const addPaths = (paths: string[]) =>
    setQueue((prev) => [...prev, ...paths.filter((p) => !prev.includes(p))]);

  useEffect(() => onFilesDropped(addPaths, setHovering), []);

  const toggle = (id: string) =>
    setEnabled((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div className="flex h-full">
      {/* 队列 */}
      <section className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center gap-2 border-b border-line px-5 py-3">
          <h1 className="text-base font-semibold">视频队列</h1>
          <span className="text-muted">{queue.length} 个</span>
          <div className="ml-auto flex gap-2">
            <button
              onClick={() => void pickVideos().then(addPaths)}
              disabled={!isTauri}
              className="flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 hover:bg-surface-2 disabled:opacity-50"
            >
              <Plus size={16} /> 添加视频
            </button>
            <button
              onClick={() => setQueue([])}
              disabled={queue.length === 0}
              className="flex items-center gap-1.5 rounded-lg border border-line bg-surface px-3 py-1.5 hover:bg-surface-2 disabled:opacity-50"
            >
              <Trash2 size={16} /> 清空
            </button>
          </div>
        </header>

        <div
          className={`m-4 flex-1 overflow-y-auto rounded-xl border-2 border-dashed transition-colors ${
            hovering ? "border-accent bg-surface-2" : "border-line"
          }`}
        >
          {queue.length === 0 ? (
            <div className="grid h-full place-items-center text-center text-muted">
              <div>
                <Film size={40} className="mx-auto mb-3 opacity-60" />
                <p className="text-ink">把视频拖到这里</p>
                <p className="mt-1 text-xs">
                  {isTauri ? "或点击右上角“添加视频”" : "当前在浏览器中预览，导入功能需要在桌面应用中使用"}
                </p>
              </div>
            </div>
          ) : (
            <ul className="divide-y divide-line">
              {queue.map((path, index) => (
                <li key={path} className="group flex items-center gap-3 px-4 py-2.5">
                  <span className="w-6 text-right text-xs text-muted tabular-nums">{index + 1}</span>
                  <div className="min-w-0 flex-1">
                    <p className="truncate">{fileName(path)}</p>
                    <p className="truncate text-xs text-muted">{path}</p>
                  </div>
                  <span className="text-xs text-muted">等待中</span>
                  <button
                    onClick={() => setQueue((prev) => prev.filter((p) => p !== path))}
                    title="移除"
                    className="rounded p-1 text-muted opacity-0 hover:bg-surface-2 hover:text-ink group-hover:opacity-100"
                  >
                    <X size={16} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>

        <footer className="flex items-center gap-3 border-t border-line bg-surface px-5 py-3">
          <button
            onClick={() => void pickFolder().then((dir) => dir && setOutputDir(dir))}
            disabled={!isTauri}
            className="flex min-w-0 flex-1 items-center gap-2 rounded-lg border border-line px-3 py-1.5 text-left hover:bg-surface-2 disabled:opacity-50"
          >
            <FolderOpen size={16} className="shrink-0 text-muted" />
            <span className={`truncate ${outputDir ? "" : "text-muted"}`}>
              {outputDir || "选择输出位置"}
            </span>
          </button>
          <label className="flex items-center gap-2 text-muted">
            裂变
            <input
              type="number"
              min={1}
              max={99}
              value={fission}
              onChange={(e) => setFission(Math.max(1, Number(e.target.value) || 1))}
              className="w-14 rounded-lg border border-line bg-bg px-2 py-1.5 text-ink"
            />
          </label>
          <button
            disabled
            title="处理引擎尚未实现"
            className="flex items-center gap-1.5 rounded-lg bg-accent px-4 py-1.5 font-medium text-accent-ink disabled:opacity-50"
          >
            <Play size={16} /> 开始处理
          </button>
        </footer>
      </section>

      {/* 配方 */}
      <aside className="flex w-[340px] shrink-0 flex-col border-l border-line bg-surface">
        <header className="border-b border-line px-5 py-3">
          <h2 className="text-base font-semibold">配方</h2>
          <p className="text-xs text-muted">已启用 {enabled.size} / {PIPELINE_MODULES.length} 个步骤</p>
        </header>
        <div className="flex-1 overflow-y-auto px-3 py-2">
          {MODULE_GROUPS.map(([group, modules]) => (
            <div key={group} className="mb-3">
              <h3 className="px-2 py-1 text-xs font-medium text-muted">{group}</h3>
              {modules.map((m) => {
                const on = enabled.has(m.id);
                return (
                  <button
                    key={m.id}
                    role="switch"
                    aria-checked={on}
                    onClick={() => toggle(m.id)}
                    className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left hover:bg-surface-2"
                  >
                    <span
                      className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
                        on ? "bg-accent" : "bg-line"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 size-4 rounded-full bg-white shadow transition-[left] ${
                          on ? "left-[18px]" : "left-0.5"
                        }`}
                      />
                    </span>
                    <span className="min-w-0">
                      <span className="block">{m.name}</span>
                      <span className="block truncate text-xs text-muted">{m.summary}</span>
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </aside>
    </div>
  );
}
