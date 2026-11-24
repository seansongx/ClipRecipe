// 检查更新与自动更新（set.auto-update 与设置页的手动检查）。
//
// 后端（src-tauri/src/commands/system.rs）决定能否自动更新：用 NSIS 安装包或 AppImage 安装、且构建时配置了更新签名公钥时，
// 读 GitHub Release 的 latest.json；发现新版本 → 显示版本号与更新说明 → 用户确认后下载（显示进度，可取消）→ 校验签名 →
// 安装并重启。其它情况（deb / rpm、开发版、未配置公钥的构建、读取 latest.json 失败）以及下载、校验、安装失败时，
// 回退为在浏览器中打开发布页。何时检查（启动时 / 手动）不变。

import { create } from "zustand";
import {
  cancelUpdate,
  checkUpdate,
  installUpdate,
  openProjectPage,
  type UpdateInfo,
  type UpdateProgress,
} from "../lib/backend";
import { useLog } from "./log";

/** 自动更新的进行情况。 */
export type UpdateRun =
  | { kind: "idle" }
  | { kind: "downloading"; progress: UpdateProgress | null; cancelling: boolean }
  | { kind: "installing" }
  /** 下载、校验或安装失败：可以重试，或打开发布页手动下载 */
  | { kind: "failed"; error: string };

interface UpdateState {
  checking: boolean;
  info: UpdateInfo | null;
  /** 手动检查失败的原因（启动时的自动检查失败不打扰用户） */
  error: string | null;
  /** 是否显示“发现新版本”对话框 */
  prompt: boolean;
  run: UpdateRun;
  /** 检查一次；`manual` 为 false（启动时自动检查）时离线等错误不打扰用户 */
  check: (manual: boolean) => Promise<void>;
  /** 下载并安装（只在 info.auto 时可用） */
  install: () => Promise<void>;
  /** 取消下载 */
  cancel: () => Promise<void>;
  /** 在浏览器中打开发布页，并关闭对话框 */
  openReleasePage: () => void;
  /** 关闭对话框（下载中会先取消下载；安装中不能关闭） */
  dismiss: () => void;
  setProgress: (progress: UpdateProgress) => void;
  setInstalling: () => void;
  setFailed: (error: string) => void;
}

const message = (e: unknown) => String(e).replace(/^Error: /, "");

/** 发现的新版本能否在程序内安装。 */
export const canAutoInstall = (info: UpdateInfo | null) => !!info && info.newer && info.auto;

/** 下载进度的百分比（总大小未知时为 null）。 */
export function progressPercent(progress: UpdateProgress | null): number | null {
  if (!progress?.total) return null;
  return Math.min(100, (progress.downloaded / progress.total) * 100);
}

export const useUpdate = create<UpdateState>((set, get) => ({
  checking: false,
  info: null,
  error: null,
  prompt: false,
  run: { kind: "idle" },

  check: async (manual) => {
    const { checking, run } = get();
    if (checking || run.kind === "downloading" || run.kind === "installing") return;
    set({ checking: true, error: null });
    try {
      const info = await checkUpdate();
      set({ info, prompt: info.newer, run: { kind: "idle" } });
    } catch (e) {
      set({ error: manual ? message(e) : null });
    } finally {
      set({ checking: false });
    }
  },

  install: async () => {
    const { info, run } = get();
    if (!canAutoInstall(info) || run.kind === "downloading" || run.kind === "installing") return;
    set({ run: { kind: "downloading", progress: null, cancelling: false } });
    try {
      await installUpdate();
      useLog.getState().add("info", `开始下载新版本 ${info?.latest ?? ""}`);
    } catch (e) {
      set({ run: { kind: "failed", error: message(e) } });
    }
  },

  cancel: async () => {
    const run = get().run;
    if (run.kind !== "downloading" || run.cancelling) return;
    set({ run: { ...run, cancelling: true } });
    const cancelled = await cancelUpdate().catch(() => false);
    // 没取消成功说明下载刚好结束（随后会收到 installing 或 failed 事件）
    if (cancelled) {
      set({ run: { kind: "idle" } });
      useLog.getState().add("info", "已取消下载更新");
    }
  },

  openReleasePage: () => {
    const info = get().info;
    if (info)
      openProjectPage(info.url).catch((e) => useLog.getState().add("error", `无法打开发布页：${String(e)}`));
    get().dismiss();
  },

  dismiss: () => {
    const run = get().run;
    if (run.kind === "installing") return;
    if (run.kind === "downloading") void get().cancel();
    set({ prompt: false, run: run.kind === "failed" ? { kind: "idle" } : get().run });
  },

  setProgress: (progress) => {
    const run = get().run;
    if (run.kind === "downloading") set({ run: { ...run, progress } });
  },

  setInstalling: () => set({ run: { kind: "installing" }, prompt: true }),

  setFailed: (error) => {
    set({ run: { kind: "failed", error }, prompt: true });
    useLog.getState().add("error", error);
  },
}));
