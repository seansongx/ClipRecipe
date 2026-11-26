// 调试面板的状态：面板是否打开、模拟的平台、打开的状态模拟。选择保存在 localStorage，刷新后恢复。
// “全部恢复默认”只恢复面板自己的东西，不改设置。
// 只在开发环境存在（App.tsx 在 import.meta.env.DEV 时才动态导入 DebugPanel，生产构建里整个 src/debug/ 被剔除）。

import { create } from "zustand";
import { PLATFORMS, simulatePlatform, type Platform } from "@/lib/platform";
import { connectSimulationSwitch, isSimulationId, SIMULATIONS, type SimulationId } from "./simulations";

const STORAGE_KEY = "cliprecipe.debug-panel";

type Saved = {
  platform: Platform | null;
  simulations: SimulationId[];
};

function load(): Saved {
  const empty: Saved = { platform: null, simulations: [] };
  try {
    const raw: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
    if (!raw || typeof raw !== "object") return empty;
    const saved = raw as Partial<Record<keyof Saved, unknown>>;
    return {
      platform: PLATFORMS.includes(saved.platform as Platform) ? (saved.platform as Platform) : null,
      simulations: Array.isArray(saved.simulations) ? saved.simulations.filter(isSimulationId) : [],
    };
  } catch {
    return empty;
  }
}

function save(state: Saved) {
  try {
    const { platform, simulations } = state;
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ platform, simulations }));
  } catch {
    // 存不了只是刷新后不保留
  }
}

type DebugState = Saved & {
  open: boolean;
  setOpen: (open: boolean) => void;
  setPlatform: (platform: Platform | null) => void;
  setSimulation: (id: SimulationId, on: boolean) => void;
  resetAll: () => void;
};

export const useDebug = create<DebugState>((set, get) => {
  const persist = () => save(get());

  const stop = (id: SimulationId) => {
    if (!get().simulations.includes(id)) return;
    SIMULATIONS.find((s) => s.id === id)?.restore();
    set({ simulations: get().simulations.filter((s) => s !== id) });
  };

  const start = (id: SimulationId) => {
    const sim = SIMULATIONS.find((s) => s.id === id);
    if (!sim || get().simulations.includes(id)) return;
    // 同组互斥：先恢复同组里已打开的
    if (sim.group) {
      for (const other of SIMULATIONS) if (other.group === sim.group && other.id !== id) stop(other.id);
    }
    set({ simulations: [...get().simulations, id] });
    sim.apply(() => {
      stop(id);
      persist();
    });
  };

  return {
    ...load(),
    open: false,
    setOpen: (open) => set({ open }),
    setPlatform: (platform) => {
      simulatePlatform(platform);
      set({ platform });
      persist();
    },
    setSimulation: (id, on) => {
      if (on) start(id);
      else stop(id);
      persist();
    },
    // 只恢复面板自己的东西：平台模拟、状态模拟、面板的 localStorage。
    resetAll: () => {
      for (const id of [...get().simulations]) stop(id);
      simulatePlatform(null);
      set({ platform: null, simulations: [] });
      try {
        localStorage.removeItem(STORAGE_KEY);
      } catch {
        // 忽略
      }
    },
  };
});

// 模拟里的按钮（如“缺少 FFmpeg”时点“下载”）切换到别的模拟时，经面板的开关走，面板上的状态保持一致
connectSimulationSwitch((id, on) => useDebug.getState().setSimulation(id, on));

let started = false;

/** 面板加载时把上次保存的选择重新应用一遍（刷新后保留）。 */
export function restoreSavedChoices(): void {
  if (started) return;
  started = true;
  const { platform, simulations } = useDebug.getState();
  if (platform) simulatePlatform(platform);
  useDebug.setState({ simulations: [] });
  for (const id of simulations) useDebug.getState().setSimulation(id, true);
}
