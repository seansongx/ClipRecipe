// FFmpeg 状态与应用内下载（设置页“运行环境”、批量处理页的“未找到 FFmpeg”提示共用）。
// 只有用户点击“下载”时才会下载；下载在 Rust 中进行（commands/ffmpeg.rs），网页本身不联网。
// 与 state/queue、state/log 一样是底层状态：只依赖 lib/backend 与 state/log。

import { create } from "zustand";
import {
  cancelFfmpegDownload,
  downloadFfmpeg,
  getFfmpegStatus,
  isTauri,
  redetectFfmpeg,
  removeManagedFfmpeg,
  type FfmpegDownloadProgress,
  type FfmpegSource,
  type FfmpegStatus,
} from "../lib/backend";
import { formatSize } from "../lib/format";
import { useLog } from "./log";

export type FfmpegDownload =
  | { kind: "idle" }
  | { kind: "running"; progress: FfmpegDownloadProgress | null; cancelling: boolean }
  | { kind: "failed"; error: string };

type FfmpegState = {
  status: FfmpegStatus | null;
  /** 正在检测（启动时、重新检测） */
  checking: boolean;
  download: FfmpegDownload;
  removing: boolean;
  /** 用户关掉了批量处理页的“未找到 FFmpeg”提示（本次运行有效） */
  bannerDismissed: boolean;
  refresh: () => Promise<FfmpegStatus | null>;
  redetect: () => Promise<void>;
  startDownload: () => Promise<void>;
  cancelDownload: () => Promise<void>;
  remove: () => Promise<void>;
  setProgress: (progress: FfmpegDownloadProgress) => void;
  dismissBanner: () => void;
};

const message = (e: unknown) => String(e).replace(/^Error: /, "");

export const SOURCE_LABELS: Record<FfmpegSource, string> = {
  env: "环境变量 CLIPRECIPE_FFMPEG",
  managed: "应用内下载",
  bundled: "与程序放在一起",
  path: "系统 PATH",
  dev: "开发依赖（node_modules/@seankit）",
  missing: "未找到",
};

export const useFfmpeg = create<FfmpegState>((set, get) => ({
  status: null,
  checking: false,
  download: { kind: "idle" },
  removing: false,
  bannerDismissed: false,

  refresh: async () => {
    if (!isTauri) return null;
    set({ checking: true });
    try {
      const status = await getFfmpegStatus();
      set({ status });
      return status;
    } catch (e) {
      useLog.getState().add("error", `检测 FFmpeg 失败：${message(e)}`);
      return null;
    } finally {
      set({ checking: false });
    }
  },

  redetect: async () => {
    if (!isTauri) return;
    set({ checking: true });
    try {
      set({ status: await redetectFfmpeg() });
    } catch (e) {
      useLog.getState().add("error", `检测 FFmpeg 失败：${message(e)}`);
    } finally {
      set({ checking: false });
    }
  },

  startDownload: async () => {
    if (!isTauri || get().download.kind === "running") return;
    set({ download: { kind: "running", progress: null, cancelling: false } });
    const log = useLog.getState();
    try {
      const outcome = await downloadFfmpeg();
      set({ status: outcome.status, download: { kind: "idle" } });
      if (outcome.cancelled) {
        log.add("info", "已取消下载 FFmpeg");
      } else {
        const m = outcome.status.managed;
        log.add("info", `FFmpeg ${m.installedVersion ?? m.pinnedVersion} 下载完成，已开始使用（${m.installedDir ?? ""}）`);
      }
    } catch (e) {
      set({ download: { kind: "failed", error: message(e) } });
      log.add("error", `下载 FFmpeg 失败：${message(e)}`);
      void get().refresh();
    }
  },

  cancelDownload: async () => {
    const download = get().download;
    if (download.kind !== "running") return;
    set({ download: { ...download, cancelling: true } });
    await cancelFfmpegDownload().catch(() => false);
  },

  remove: async () => {
    if (!isTauri || get().removing) return;
    set({ removing: true });
    try {
      set({ status: await removeManagedFfmpeg(), download: { kind: "idle" } });
      useLog.getState().add("info", "已删除应用内下载的 FFmpeg");
    } catch (e) {
      useLog.getState().add("error", `删除 FFmpeg 失败：${message(e)}`);
    } finally {
      set({ removing: false });
    }
  },

  setProgress: (progress) => {
    const download = get().download;
    if (download.kind === "running") set({ download: { ...download, progress } });
  },

  dismissBanner: () => set({ bannerDismissed: true }),
}));

/** 只读：当前 FFmpeg 的状态（未检测时为 null）。工具表单可用它提前说明缺少的功能（如 whisper 滤镜）。 */
export const useFfmpegStatus = () => useFfmpeg((s) => s.status);

/**
 * 导入或处理前确认 FFmpeg 可用。上次检测为不可用时重新检测一次（用户可能刚装好）；仍不可用时在处理记录里说明、
 * 重新显示批量处理页的下载提示，返回 false。状态未知（还在检测）时放行，由后端报告错误。
 */
export async function requireFfmpeg(action: string): Promise<boolean> {
  let status = useFfmpeg.getState().status;
  if (!status || status.available) return true;
  status = (await useFfmpeg.getState().refresh()) ?? status;
  if (status.available) return true;
  useFfmpeg.setState({ bannerDismissed: false });
  const hint = status.managed.supported
    ? "可以在批量处理页顶部的提示或“设置 → 关于 → 运行环境”中一键下载"
    : "请安装 FFmpeg 并加入 PATH，或设置环境变量 CLIPRECIPE_FFMPEG";
  useLog.getState().add("error", `未找到可用的 FFmpeg，无法${action}。${hint}`);
  return false;
}

/** 下载进度的文字：“12.3 MB / 75.6 MB · 5.2 MB/s”。 */
export function progressText(progress: FfmpegDownloadProgress | null, total: number): string {
  if (!progress) return `准备下载（共 ${formatSize(total)}）…`;
  if (progress.phase === "extract") return `正在校验并解出 ${progress.tool}…`;
  if (progress.phase === "install") return "正在安装…";
  const speed = progress.bytesPerSec > 0 ? ` · ${formatSize(progress.bytesPerSec)}/s` : "";
  return `正在下载 ${progress.tool}：${formatSize(progress.downloaded) || "0 B"} / ${formatSize(progress.total)}${speed}`;
}
