// 检查更新（set.auto-update 与设置页的手动检查）。后端只向 GitHub 公开 API 发一个不带身份信息的 GET，
// 发现新版本时由界面询问是否在浏览器中打开发布页（不会自动下载或替换程序）。

import { create } from "zustand";
import { checkUpdate, type UpdateInfo } from "../lib/backend";

interface UpdateState {
  checking: boolean;
  info: UpdateInfo | null;
  error: string | null;
  /** 是否显示“发现新版本”对话框 */
  prompt: boolean;
  /** 检查一次；`manual` 为 false（启动时自动检查）时离线等错误不打扰用户 */
  check: (manual: boolean) => Promise<void>;
  dismiss: () => void;
}

export const useUpdate = create<UpdateState>((set, get) => ({
  checking: false,
  info: null,
  error: null,
  prompt: false,

  check: async (manual) => {
    if (get().checking) return;
    set({ checking: true, error: null });
    try {
      const info = await checkUpdate();
      set({ info, prompt: info.newer });
    } catch (e) {
      set({ error: manual ? String(e).replace(/^Error: /, "") : null });
    } finally {
      set({ checking: false });
    }
  },

  dismiss: () => set({ prompt: false }),
}));
