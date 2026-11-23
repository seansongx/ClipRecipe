// 设置：启动时从后端读取，修改后自动保存（防抖）。浏览器预览时只保存在内存里。

import { create } from "zustand";
import { DEFAULT_SETTINGS, isTauri, loadSettings, saveSettings, type Settings } from "../lib/backend";
import { useLog } from "./log";

interface SettingsState {
  settings: Settings;
  loaded: boolean;
  load: () => Promise<void>;
  update: (patch: Partial<Settings>) => void;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

export const useSettings = create<SettingsState>((set, get) => ({
  settings: DEFAULT_SETTINGS,
  loaded: false,

  load: async () => {
    if (!isTauri) {
      set({ loaded: true });
      return;
    }
    try {
      const settings = await loadSettings();
      set({ settings: { ...DEFAULT_SETTINGS, ...settings }, loaded: true });
    } catch (e) {
      useLog.getState().add("warn", `读取设置失败，已使用默认设置：${String(e)}`);
      set({ loaded: true });
    }
  },

  update: (patch) => {
    set({ settings: { ...get().settings, ...patch } });
    if (!isTauri) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveSettings(get().settings).catch((e) =>
        useLog.getState().add("error", `保存设置失败：${String(e)}`),
      );
    }, 300);
  },
}));
