// 去掉网页式的行为，让窗口像桌面程序（docs/PLAN.md“桌面应用体验约定”）：
// - 右键：空白处什么也不弹；应用自己的右键菜单（shadcn ContextMenu）照常；输入框保留系统的编辑菜单（剪切 / 复制 / 粘贴 / 全选）。
// - 浏览器快捷键：刷新、查找、打印、保存 / 打开网页、查看源码、光标浏览、缩放、前进后退都不起作用；
//   Ctrl+A 只在输入框里全选（队列列表自己处理 Ctrl+A）。正式版另由 WebView2 设置关闭浏览器快捷键（src-tauri/src/desktop.rs）。
// - Ctrl + 滚轮缩放、鼠标侧键前进后退、中键自动滚动、拖动文字 / 图片、把东西拖进来导航，全部禁止。
// - 链接不在窗口里打开：本项目的网址交给系统浏览器（open_project_page 只接受本项目地址）。

import { isTauri, openProjectPage } from "./backend";

const TEXT_INPUT_TYPES = new Set(["text", "search", "url", "tel", "email", "password", "number", ""]);

/** 可编辑的文本控件（输入框、文本框、contenteditable）。 */
export function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  const field = target.closest("input, textarea, [contenteditable='true'], [contenteditable='']");
  if (!field) return false;
  if (field instanceof HTMLInputElement) return TEXT_INPUT_TYPES.has(field.type) && !field.readOnly;
  if (field instanceof HTMLTextAreaElement) return !field.readOnly;
  return true;
}

/** 被本模块当作“浏览器快捷键”拦下的按键事件（应用自己的快捷键处理时据此区分，见 App.tsx）。 */
export const browserKeyEvents = new WeakSet<Event>();

/** 只对浏览器有意义的按键。 */
export function isBrowserShortcut(e: KeyboardEvent): boolean {
  const key = e.key;
  // 有意不按平台区分（不用 lib/platform.ts 的 isModKey）：这里拦的是 WebView 自己的快捷键，Ctrl 与 ⌘ 两种都拦最稳妥，
  // 也不受调试面板的平台模拟影响。应用自己的快捷键按平台匹配，见 lib/shortcuts.ts。
  const ctrl = e.ctrlKey || e.metaKey;
  if (["F3", "F5", "F7", "BrowserBack", "BrowserForward", "BrowserRefresh", "BrowserSearch", "BrowserHome"].includes(key)) {
    return true;
  }
  if (e.altKey && !ctrl && (key === "ArrowLeft" || key === "ArrowRight" || key === "Home")) return true;
  if (!ctrl || e.altKey) return false;
  const k = key.length === 1 ? key.toLowerCase() : key;
  // 刷新、查找（下一个）、打印、保存 / 打开网页、查看源码、缩放
  if (["r", "f", "g", "p", "s", "o", "u", "+", "=", "-", "_", "0"].includes(k)) return true;
  // 小键盘的缩放键
  if (e.code === "NumpadAdd" || e.code === "NumpadSubtract" || e.code === "Numpad0") return true;
  // Ctrl+A：输入框里照常全选，其它地方不全选整页
  if (k === "a" && !e.shiftKey) return !isEditable(e.target);
  return false;
}

/** 安装全局拦截，返回卸载函数。在 main.tsx 中、渲染之前调用一次。 */
export function installDesktopGuards(): () => void {
  const onKeyDown = (e: KeyboardEvent) => {
    if (isBrowserShortcut(e)) {
      browserKeyEvents.add(e);
      e.preventDefault();
    }
  };
  // 冒泡到 document 时应用自己的右键菜单已经处理（shadcn ContextMenu 会取消默认行为）；剩下的只给输入框保留系统编辑菜单
  const onContextMenu = (e: MouseEvent) => {
    if (e.defaultPrevented || isEditable(e.target)) return;
    e.preventDefault();
  };
  const onWheel = (e: WheelEvent) => {
    if (e.ctrlKey) e.preventDefault();
  };
  // 鼠标侧键（3 后退、4 前进）与中键自动滚动
  const onMouse = (e: MouseEvent) => {
    if (e.button === 1 || e.button === 3 || e.button === 4) e.preventDefault();
  };
  // 只允许在输入框里拖动选中的文字；图片、链接、界面文字都不产生拖动残影
  const onDragStart = (e: DragEvent) => {
    if (!isEditable(e.target)) e.preventDefault();
  };
  // 拖进窗口的东西不让 WebView 打开（文件拖放由 Tauri 在系统层接管，见 lib/backend.ts 的 onDragDrop）
  const onDragOver = (e: DragEvent) => {
    if (!isEditable(e.target)) e.preventDefault();
  };
  const onDrop = (e: DragEvent) => {
    if (!isEditable(e.target)) e.preventDefault();
  };
  // 输入框（路径、参数）不做拼写检查与自动填充：spellcheck 由 <html> 继承给所有输入框；autocomplete 不继承，
  // 在获得焦点时补上（调用方显式写了的保留）。放在这里而不是改 shadcn 的 Input / Textarea（ui/ 保持 CLI 原样）。
  document.documentElement.spellcheck = false;
  const onFocusIn = (e: FocusEvent) => {
    const field = e.target;
    if ((field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) && !field.hasAttribute("autocomplete")) {
      field.setAttribute("autocomplete", "off");
    }
  };
  const onClick = (e: MouseEvent) => {
    const link = e.target instanceof Element ? e.target.closest("a[href]") : null;
    if (!(link instanceof HTMLAnchorElement)) return;
    e.preventDefault();
    if (isTauri && /^https?:\/\//.test(link.href)) void openProjectPage(link.href).catch(() => {});
  };

  window.addEventListener("keydown", onKeyDown, true);
  document.addEventListener("contextmenu", onContextMenu);
  window.addEventListener("wheel", onWheel, { capture: true, passive: false });
  window.addEventListener("mousedown", onMouse, true);
  window.addEventListener("mouseup", onMouse, true);
  window.addEventListener("auxclick", onMouse, true);
  document.addEventListener("dragstart", onDragStart);
  document.addEventListener("dragover", onDragOver);
  document.addEventListener("drop", onDrop);
  document.addEventListener("click", onClick, true);
  document.addEventListener("focusin", onFocusIn);
  return () => {
    document.removeEventListener("focusin", onFocusIn);
    window.removeEventListener("keydown", onKeyDown, true);
    document.removeEventListener("contextmenu", onContextMenu);
    window.removeEventListener("wheel", onWheel, true);
    window.removeEventListener("mousedown", onMouse, true);
    window.removeEventListener("mouseup", onMouse, true);
    window.removeEventListener("auxclick", onMouse, true);
    document.removeEventListener("dragstart", onDragStart);
    document.removeEventListener("dragover", onDragOver);
    document.removeEventListener("drop", onDrop);
    document.removeEventListener("click", onClick, true);
  };
}
