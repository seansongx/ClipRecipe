// 输出文件名格式（set.name-template）。实际命名在后端 jobs/output.rs 的 render_name；
// 这里按同样的规则算设置页的示例，并生成开始处理时传给后端的本地时间。

import { DEFAULT_SETTINGS, type BatchTime } from "@/lib/backend";

export const DEFAULT_NAME_TEMPLATE = DEFAULT_SETTINGS.nameTemplate;

/** 可用的变量（设置页的插入按钮）。 */
export const NAME_VARIABLES = [
  { token: "{name}", label: "文件名" },
  { token: "{index}", label: "序号" },
  { token: "{date}", label: "日期" },
  { token: "{time}", label: "时间" },
  { token: "{folder}", label: "文件夹" },
] as const;

/** 某个时刻的本地时间。 */
export function batchTime(date: Date = new Date()): BatchTime {
  return {
    year: date.getFullYear(),
    month: date.getMonth() + 1,
    day: date.getDate(),
    hour: date.getHours(),
    minute: date.getMinutes(),
    second: date.getSeconds(),
  };
}

const pad = (n: number, width = 2) => String(n).padStart(width, "0");

export interface NameVars {
  name: string;
  index: string;
  date: string;
  time: string;
  folder: string;
}

export function timeVars(t: BatchTime): Pick<NameVars, "date" | "time"> {
  return {
    date: `${pad(t.year, 4)}${pad(t.month)}${pad(t.day)}`,
    time: `${pad(t.hour)}${pad(t.minute)}${pad(t.second)}`,
  };
}

// eslint-disable-next-line no-control-regex
const ILLEGAL = /[\\/:*?"<>|\u0000-\u001f\u007f-\u009f]/g;

/** 与后端 render_name 相同：替换已知变量，其它原样保留；非法字符换成 _，去掉首尾空格与结尾的点；结果为空时退回 {name}。 */
export function renderName(template: string, vars: NameVars): string {
  const known = new Map<string, string>(Object.entries(vars));
  const replaced = template.replace(/\{([^{}]*)\}/g, (whole, key: string) => known.get(key) ?? whole);
  const cleaned = replaced
    .replace(ILLEGAL, "_")
    .trim()
    .replace(/[.\s]+$/, "");
  return cleaned || vars.name;
}
