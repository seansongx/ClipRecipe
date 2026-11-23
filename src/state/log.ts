// 处理记录（日志面板）。

import { create } from "zustand";
import type { LogLevel } from "../lib/backend";

export interface LogEntry {
  id: number;
  time: Date;
  level: LogLevel;
  taskId: string | null;
  message: string;
}

const MAX_ENTRIES = 2000;
let nextId = 1;

interface LogState {
  entries: LogEntry[];
  add: (level: LogLevel, message: string, taskId?: string | null) => void;
  clear: () => void;
}

export const useLog = create<LogState>((set) => ({
  entries: [],
  add: (level, message, taskId = null) =>
    set((state) => {
      const entry: LogEntry = { id: nextId++, time: new Date(), level, taskId, message };
      const entries = [...state.entries, entry];
      return { entries: entries.length > MAX_ENTRIES ? entries.slice(-MAX_ENTRIES) : entries };
    }),
  clear: () => set({ entries: [] }),
}));
