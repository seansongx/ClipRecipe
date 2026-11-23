// 实际会用的视频编码器（状态栏显示）。只读取后端已缓存的探测结果，不主动探测：
// 探测在开始处理、运行工具、设置页“运行环境”时发生，之后在这里刷新。

import { create } from "zustand";
import { getCachedEncoderInfo, getEncoderInfo, isTauri, type EncoderInfo } from "../lib/backend";

interface EncodersState {
  /** null：还没有探测过 */
  info: EncoderInfo | null;
  /** 重新读取缓存的结果 */
  refresh: () => Promise<void>;
  /** 探测（refresh 为 true 时丢弃缓存重测），并更新状态 */
  detect: (refresh: boolean) => Promise<EncoderInfo>;
}

export const useEncoders = create<EncodersState>((set) => ({
  info: null,
  refresh: async () => {
    if (!isTauri) return;
    try {
      set({ info: await getCachedEncoderInfo() });
    } catch {
      // 读取失败时保持原样
    }
  },
  detect: async (refresh) => {
    const info = await getEncoderInfo(refresh);
    set({ info });
    return info;
  },
}));

/** 编码器名称的说明，如 “软件编码 libx264”“硬件编码 h264_nvenc（NVIDIA）”。 */
export function describeEncoder(name: string): string {
  if (name.startsWith("lib")) return `软件编码 ${name}`;
  const vendor = name.includes("nvenc") ? "NVIDIA" : name.includes("qsv") ? "Intel" : name.includes("amf") ? "AMD" : "";
  return `硬件编码 ${name}${vendor ? `（${vendor}）` : ""}`;
}
