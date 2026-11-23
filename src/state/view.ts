// 当前页面（主导航）。放在状态仓库里，快捷键（Ctrl+1/2/3、Ctrl+,）与“打开文件”都能切换页面。

import { create } from "zustand";

export type ViewId = "batch" | "toolbox" | "settings";

export const useView = create<{ view: ViewId; setView: (view: ViewId) => void }>((set) => ({
  view: "batch",
  setView: (view) => set({ view }),
}));
