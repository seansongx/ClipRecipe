// 配方面板里各模块“当前取值”的一句话摘要共用的格式化函数（各模块的 summary 使用）。

import type { AssetSource, Range } from "../../lib/backend";
import { fileName } from "../../lib/format";

/** 数字最多保留两位小数，去掉末尾的 0（`30.00` → `30`，`1.50` → `1.5`）。 */
export function num(value: number): string {
  if (!Number.isFinite(value)) return "?";
  return String(Number(value.toFixed(2)));
}

/**
 * 范围值：两端相等时只写一个数（`30 fps`），否则 `30 – 60 fps`。
 * `unit` 原样接在数字后面：需要空格时自己带上（`" fps"`、`" 秒"`），百分号、度数等紧贴（`"%"`、`"°"`）。
 */
export function range(value: Range, unit = ""): string {
  const { min, max } = value;
  return min === max ? `${num(min)}${unit}` : `${num(min)} – ${num(max)}${unit}`;
}

/** 素材来源：单文件为文件名，目录为“目录 名称”；未选择时为 `empty`。 */
export function asset(source: AssetSource, empty = "未选素材"): string {
  if (!source.path.trim()) return empty;
  const name = fileName(source.path.replace(/[\\/]+$/, ""));
  return source.mode === "directory" ? `目录 ${name}` : name;
}

/** 用“ · ”连接各部分，跳过空值。 */
export function join(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" · ");
}
