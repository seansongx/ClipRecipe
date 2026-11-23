// 开发环境调试面板注入的模拟数据（队列行的 ID 与路径）的登记处（src/debug/simulations.ts 登记）。
// 作用：模拟数据只在前端存在，永远不写到磁盘、不发给后端——
// - 状态层在保存队列、开始处理、插队、重载字幕封面时用 isSimulated() 排除模拟行，取消 / 停止在前端自洽地模拟；
// - lib/backend.ts 的命令调用在参数里发现登记过的值时直接拒绝（兜底：工具、预览、播放、媒体信息等其它入口）。
// 生产构建里 import.meta.env.DEV 是常量 false：这里永远为空、isSimulated() 永远是 false，检查代码被剔除。

const values = new Set<string>();

export function markSimulated(list: string[]): void {
  if (!import.meta.env.DEV) return;
  for (const v of list) values.add(v);
}

export function unmarkSimulated(list: string[]): void {
  if (!import.meta.env.DEV) return;
  for (const v of list) values.delete(v);
}

/** 这个 ID / 路径是不是调试面板注入的模拟数据。 */
export function isSimulated(value: string | null | undefined): boolean {
  return import.meta.env.DEV && value != null && values.has(value);
}

/** 参数（任意嵌套的对象 / 数组）里第一个模拟值；没有时为 null。 */
export function findSimulated(args: unknown): string | null {
  if (!import.meta.env.DEV || values.size === 0) return null;
  const seen = new Set<unknown>();
  const visit = (v: unknown): string | null => {
    if (typeof v === "string") return values.has(v) ? v : null;
    if (!v || typeof v !== "object" || seen.has(v)) return null;
    seen.add(v);
    for (const child of Array.isArray(v) ? v : Object.values(v)) {
      const hit = visit(child);
      if (hit) return hit;
    }
    return null;
  };
  return visit(args);
}
