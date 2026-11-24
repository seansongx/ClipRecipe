// 正在处理时关闭主窗口 / 托盘“退出”：后端阻止退出并发出 close-requested 事件（state/events.ts），
// 这里打开确认框（components/QuitConfirmDialog）并告诉后端界面已应答（close_prompt_shown；
// 不应答时短时间内再次关闭会直接退出），确认后调用 quit_app（后端会先停止处理再退出）。

import { create } from "zustand";
import { closePromptDismissed, closePromptShown, quitApp } from "../lib/backend";
import { useLog } from "./log";

interface QuitConfirmState {
  open: boolean;
  request: () => void;
  dismiss: () => void;
  confirm: () => void;
}

export const useQuitConfirm = create<QuitConfirmState>((set) => ({
  open: false,
  request: () => {
    set({ open: true });
    closePromptShown().catch(() => {});
  },
  dismiss: () => {
    set({ open: false });
    closePromptDismissed().catch(() => {});
  },
  confirm: () => {
    set({ open: false });
    quitApp().catch((e) => useLog.getState().add("error", `退出失败：${String(e)}`));
  },
}));
