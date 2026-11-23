// 键盘快捷键的唯一来源：每个快捷键的按键定义，按平台生成显示文字（提示的 Kbd、菜单里的快捷键、设置 → 关于里的快捷键表）
// 并判断按键事件是否匹配。实现见 App.tsx 的 useAppShortcuts 与 views/batch/QueueList.tsx（队列列表获得焦点时）。
//
// 主修饰键（mod）：macOS 是 ⌘（metaKey），Windows / Linux 是 Ctrl（ctrlKey）。显示：macOS 用符号（⌥⇧⌘，按系统菜单的顺序），
// 其余平台用 “Ctrl+Shift+O” 式的文字。平台来自 lib/platform.ts（开发环境下可被调试面板模拟）。

import { getPlatform, isModKey, isOtherModKey, usePlatform, type Platform } from "./platform";

/** 一个按键组合。`label` 是主键的显示文字；`code` / `keys` 用于匹配（有 code 时按物理键，不受键盘布局影响）。 */
export type Combo = {
  mod?: boolean;
  shift?: boolean;
  alt?: boolean;
  label: string;
  /** macOS 上主键的显示文字（如 Enter → ↩） */
  macLabel?: string;
  code?: string;
  /** KeyboardEvent.key 的取值（不区分大小写），任一相同即匹配 */
  keys?: string[];
};

const SHORTCUTS = {
  addVideos: { mod: true, label: "O", code: "KeyO" },
  addFolder: { mod: true, shift: true, label: "O", code: "KeyO" },
  start: { mod: true, label: "Enter", macLabel: "↩", keys: ["Enter"] },
  stop: { label: "Esc", macLabel: "⎋", keys: ["Escape"] },
  settings: { mod: true, label: ",", keys: [","] },
  batch: { mod: true, label: "1", code: "Digit1" },
  toolbox: { mod: true, label: "2", code: "Digit2" },
  settingsView: { mod: true, label: "3", code: "Digit3" },
  docs: { label: "F1", keys: ["F1"] },
  selectAll: { mod: true, label: "A", code: "KeyA" },
  play: { label: "Enter", macLabel: "↩", keys: ["Enter"] },
  remove: { label: "Delete", keys: ["Delete"] },
  /** 只在开发环境有处理（src/debug/，生产构建里没有）；不列入快捷键表 */
  debugPanel: { mod: true, shift: true, label: "D", code: "KeyD" },
} satisfies Record<string, Combo>;

/** macOS 上与其它平台按键不同的快捷键（Mac 键盘没有独立的 Delete 键，删除用 ⌫）。 */
const MAC_SHORTCUTS: Partial<Record<keyof typeof SHORTCUTS, Combo>> = {
  remove: { label: "⌫", keys: ["Backspace", "Delete"] },
};

export type ShortcutId = keyof typeof SHORTCUTS;

/** 某个快捷键在某个平台上的按键组合。 */
export function shortcutCombo(id: ShortcutId, platform: Platform = getPlatform()): Combo {
  return (platform === "macos" && MAC_SHORTCUTS[id]) || SHORTCUTS[id];
}

/** 按键组合的显示文字：macOS “⇧⌘O”，其余 “Ctrl+Shift+O”。 */
export function formatCombo(combo: Combo, platform: Platform = getPlatform()): string {
  if (platform === "macos") {
    return `${combo.alt ? "⌥" : ""}${combo.shift ? "⇧" : ""}${combo.mod ? "⌘" : ""}${combo.macLabel ?? combo.label}`;
  }
  return [combo.mod && "Ctrl", combo.alt && "Alt", combo.shift && "Shift", combo.label].filter(Boolean).join("+");
}

/** 快捷键的显示文字。 */
export function shortcutLabel(id: ShortcutId, platform: Platform = getPlatform()): string {
  return formatCombo(shortcutCombo(id, platform), platform);
}

/** 按键事件是否就是这个快捷键（修饰键必须完全一致）。 */
export function matchShortcut(e: KeyboardEvent, id: ShortcutId, platform: Platform = getPlatform()): boolean {
  const combo = shortcutCombo(id, platform);
  if (!!combo.mod !== isModKey(e, platform) || isOtherModKey(e, platform)) return false;
  if (!!combo.shift !== e.shiftKey || !!combo.alt !== e.altKey) return false;
  if (combo.code) return e.code === combo.code;
  return (combo.keys ?? []).some((k) => k.toLowerCase() === e.key.toLowerCase());
}

/** 当前平台下全部快捷键的显示文字（平台变化时重新渲染）。 */
export function useShortcutLabels(): Record<ShortcutId, string> {
  const platform = usePlatform();
  return Object.fromEntries(
    (Object.keys(SHORTCUTS) as ShortcutId[]).map((id) => [id, shortcutLabel(id, platform)]),
  ) as Record<ShortcutId, string>;
}

/** 修饰键 + 点击 / 方向键之类的说明文字（如 “Ctrl+点击”、macOS “⌘ 点击”）。 */
export function modifierText(modifier: "mod" | "shift", what: string, platform: Platform = getPlatform()): string {
  if (platform === "macos") return `${modifier === "mod" ? "⌘" : "⇧"}${what.length === 1 ? "" : " "}${what}`;
  return `${modifier === "mod" ? "Ctrl" : "Shift"}+${what}`;
}

export type ShortcutGroup = { title: string; items: { keys: string[]; label: string }[] };

/** 设置 → 关于里的快捷键表。 */
export function shortcutGroups(platform: Platform): ShortcutGroup[] {
  const k = (id: ShortcutId) => shortcutLabel(id, platform);
  const groups: ShortcutGroup[] = [
    {
      title: "全局",
      items: [
        { keys: [k("addVideos")], label: "添加视频" },
        { keys: [k("addFolder")], label: "添加文件夹" },
        { keys: [k("start")], label: "开始处理" },
        { keys: [k("stop")], label: "停止处理（处理中、没有打开的对话框时）；关闭对话框 / 菜单" },
        { keys: [k("batch"), k("toolbox"), k("settingsView")], label: "切换到批量处理 / 工具箱 / 设置" },
        { keys: [k("settings")], label: "打开设置" },
        { keys: [k("docs")], label: "使用文档" },
      ],
    },
    {
      title: "视频队列（点击列表后）",
      items: [
        { keys: ["↑", "↓"], label: "移动当前行" },
        { keys: [modifierText("shift", "↑", platform), modifierText("shift", "↓", platform)], label: "扩展选择" },
        { keys: ["Home", "End"], label: "第一行 / 最后一行" },
        { keys: [platform === "macos" ? "␣" : "Space"], label: "选中 / 取消选中当前行" },
        { keys: [k("play")], label: "播放当前行" },
        { keys: [k("selectAll")], label: "全选" },
        { keys: [k("remove")], label: "移除所选" },
        { keys: [modifierText("mod", "点击", platform), modifierText("shift", "点击", platform)], label: "多选 / 连续选择" },
      ],
    },
  ];
  return groups;
}

/** 队列工具栏的多选说明：“Ctrl / Shift + 点击多选”，macOS “⌘ / ⇧ 点击多选”。 */
export function multiSelectHint(platform: Platform): string {
  return platform === "macos" ? "⌘ / ⇧ 点击多选" : "Ctrl / Shift + 点击多选";
}
