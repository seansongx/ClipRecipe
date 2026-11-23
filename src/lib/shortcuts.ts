// 键盘快捷键：显示用的按键文字（提示、设置 → 关于里的快捷键表）。实现见 App.tsx 的 useAppShortcuts
// 与 views/batch/QueueList.tsx（队列列表获得焦点时）。

export const KEYS = {
  addVideos: "Ctrl+O",
  addFolder: "Ctrl+Shift+O",
  start: "Ctrl+Enter",
  stop: "Esc",
  settings: "Ctrl+,",
  batch: "Ctrl+1",
  toolbox: "Ctrl+2",
  settingsView: "Ctrl+3",
  docs: "F1",
  selectAll: "Ctrl+A",
  remove: "Delete",
} as const;

export type ShortcutGroup = { title: string; items: { keys: string[]; label: string }[] };

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  {
    title: "全局",
    items: [
      { keys: [KEYS.addVideos], label: "添加视频" },
      { keys: [KEYS.addFolder], label: "添加文件夹" },
      { keys: [KEYS.start], label: "开始处理" },
      { keys: [KEYS.stop], label: "停止处理（处理中、没有打开的对话框时）；关闭对话框 / 菜单" },
      { keys: [KEYS.batch, KEYS.toolbox, KEYS.settingsView], label: "切换到批量处理 / 工具箱 / 设置" },
      { keys: [KEYS.settings], label: "打开设置" },
      { keys: [KEYS.docs], label: "使用文档" },
    ],
  },
  {
    title: "视频队列（点击列表后）",
    items: [
      { keys: ["↑", "↓"], label: "移动当前行" },
      { keys: ["Shift+↑", "Shift+↓"], label: "扩展选择" },
      { keys: ["Home", "End"], label: "第一行 / 最后一行" },
      { keys: ["Space"], label: "选中 / 取消选中当前行" },
      { keys: ["Enter"], label: "播放当前行" },
      { keys: [KEYS.selectAll], label: "全选" },
      { keys: [KEYS.remove], label: "移除所选" },
      { keys: ["Ctrl+点击", "Shift+点击"], label: "多选 / 连续选择" },
    ],
  },
];
