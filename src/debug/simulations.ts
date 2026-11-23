// 调试面板的状态模拟（只在开发环境存在，见 DebugPanel.tsx）：把平时难复现的界面状态直接注入前端状态层
// （state/ffmpeg、state/queue、state/log），视图照常从状态层读取，不需要任何 if (debug) 分支。
// 每个模拟都能单独打开、恢复：打开时记下被替换的真实值，恢复时放回去；注入的队列行、日志按 ID 精确移除，用户自己的数据不受影响。
// 浏览器里（npm run dev，没有 Tauri）同样可用。桌面应用里后端的异步结果（启动时检测 FFmpeg、同步“是否正在处理”）可能晚于模拟到达，
// 所以会被真实结果覆盖的字段由订阅守住：真实结果记为“恢复时的值”，界面仍显示模拟值。

import type { FfmpegDownloadProgress, FfmpegStatus, LogLevel, MediaInfo } from "@/lib/backend";
import { EXAMPLE_PATHS, examplePath, getPlatform } from "@/lib/platform";
import { markSimulated, unmarkSimulated } from "@/lib/simulated";
import { useFfmpeg } from "@/state/ffmpeg";
import { useLog } from "@/state/log";
import { useQueue, type QueueItem } from "@/state/queue";

export type SimulationId = "ffmpegMissing" | "ffmpegDownloading" | "ffmpegFailed" | "queueStates" | "logLevels";

export type Simulation = {
  id: SimulationId;
  label: string;
  description: string;
  /** 同组的模拟互斥（打开一个会先恢复另一个） */
  group?: string;
  /** 打开；`end` 在模拟自己结束时调用（例如在模拟的下载中点了“取消”） */
  apply: (end: () => void) => void;
  restore: () => void;
};

const MB = 1024 * 1024;

/** 打开 / 关闭另一个模拟（由 store.ts 接上；模拟里的按钮要切换到别的模拟时用）。 */
let requestSimulation: (id: SimulationId, on: boolean) => void = () => {};

export function connectSimulationSwitch(fn: (id: SimulationId, on: boolean) => void): void {
  requestSimulation = fn;
}

// ---------------------------------------------------------------- FFmpeg

type FfmpegActions = Pick<
  ReturnType<typeof useFfmpeg.getState>,
  "startDownload" | "cancelDownload" | "remove" | "redetect"
>;

/**
 * 任一 FFmpeg 模拟打开期间，把状态层里会触发后端动作的操作（下载、取消下载、删除、重新检测）换成前端的模拟：
 * 点“下载 / 重试下载”进入“下载进行中”模拟，“取消”结束它；删除与重新检测只在处理记录里说明，不碰真实的 FFmpeg。
 * 全部 FFmpeg 模拟关闭后换回原来的操作。
 */
const FAKE_FFMPEG_ACTIONS: FfmpegActions = {
  startDownload: async () => requestSimulation("ffmpegDownloading", true),
  cancelDownload: async () => {
    const download = useFfmpeg.getState().download;
    if (download.kind === "running") useFfmpeg.setState({ download: { ...download, cancelling: true } });
  },
  remove: async () => useLog.getState().add("info", "调试面板正在模拟 FFmpeg 状态：没有删除真实的 FFmpeg"),
  redetect: async () =>
    useLog.getState().add("info", "调试面板正在模拟 FFmpeg 状态：没有重新检测；关掉模拟即恢复真实状态"),
};

const ffmpegHolders = new Set<SimulationId>();
let realFfmpegActions: FfmpegActions | null = null;

function holdFfmpegActions(id: SimulationId) {
  if (ffmpegHolders.size === 0) {
    const { startDownload, cancelDownload, remove, redetect } = useFfmpeg.getState();
    realFfmpegActions = { startDownload, cancelDownload, remove, redetect };
    useFfmpeg.setState(FAKE_FFMPEG_ACTIONS);
  }
  ffmpegHolders.add(id);
}

function releaseFfmpegActions(id: SimulationId) {
  ffmpegHolders.delete(id);
  if (ffmpegHolders.size > 0 || !realFfmpegActions) return;
  useFfmpeg.setState(realFfmpegActions);
  realFfmpegActions = null;
}

/** 一个“没有找到 FFmpeg、可以一键下载”的状态（下载信息沿用真实状态里的，没有时用示例值）。 */
function missingStatus(real: FfmpegStatus | null): FfmpegStatus {
  const managed = real?.managed;
  return {
    available: false,
    path: "ffmpeg",
    ffprobePath: "ffprobe",
    version: null,
    ffprobeVersion: null,
    error: "ffmpeg：无法启动（program not found）",
    source: "missing",
    whisper: false,
    managed: {
      supported: true,
      unsupportedReason: null,
      pinnedVersion: managed?.pinnedVersion || "9.0.2",
      packageVersion: managed?.packageVersion || "9.0.1",
      downloadSize: managed?.downloadSize || 78 * MB,
      installedVersion: null,
      installedDir: null,
      updateAvailable: false,
      root: managed?.root ?? null,
    },
  };
}

/** 模拟“缺少 FFmpeg”时被替换掉的真实状态（调试信息显示真实值时用）。 */
let realFfmpeg: { status: FfmpegStatus | null } | null = null;
let stopFfmpegGuard: (() => void) | null = null;

/** 真实的 FFmpeg 状态（不受“缺少 FFmpeg”模拟影响）。 */
export function realFfmpegStatus(): FfmpegStatus | null {
  return realFfmpeg ? realFfmpeg.status : useFfmpeg.getState().status;
}

const ffmpegMissing: Simulation = {
  id: "ffmpegMissing",
  label: "缺少 FFmpeg",
  description:
    "批量处理页出现“未找到 FFmpeg”提示，状态栏与设置 → 关于 → 运行环境显示未找到；导入与开始处理被拦下。点“下载”进入“下载进行中”模拟，不会真的下载。",
  apply: () => {
    holdFfmpegActions("ffmpegMissing");
    const state = useFfmpeg.getState();
    realFfmpeg = { status: state.status };
    const fake = missingStatus(state.status);
    useFfmpeg.setState({ status: fake, bannerDismissed: false });
    stopFfmpegGuard = useFfmpeg.subscribe((next) => {
      // 真实检测结果（启动检测、重新检测、下载后）到达：记下，界面仍显示模拟值
      if (next.status === fake || !realFfmpeg) return;
      realFfmpeg.status = next.status;
      useFfmpeg.setState({ status: fake });
    });
  },
  restore: () => {
    stopFfmpegGuard?.();
    stopFfmpegGuard = null;
    if (realFfmpeg) useFfmpeg.setState({ status: realFfmpeg.status });
    realFfmpeg = null;
    releaseFfmpegActions("ffmpegMissing");
  },
};

/** 下载类模拟需要一个状态（浏览器里没有）才会显示提示；这里补上的，恢复时只在它还在时移除。 */
let addedStatus: FfmpegStatus | null = null;

function ensureStatus() {
  if (useFfmpeg.getState().status) return;
  addedStatus = missingStatus(null);
  useFfmpeg.setState({ status: addedStatus, bannerDismissed: false });
}

function dropAddedStatus() {
  if (addedStatus && useFfmpeg.getState().status === addedStatus) useFfmpeg.setState({ status: null });
  addedStatus = null;
}

let downloadTimer: ReturnType<typeof setInterval> | undefined;
let stopDownloadGuard: (() => void) | null = null;

const ffmpegDownloading: Simulation = {
  id: "ffmpegDownloading",
  label: "FFmpeg 下载进行中",
  description: "下载进度条、速度、来源与“取消”按钮（进度循环推进）；点“取消”结束模拟。",
  group: "ffmpegDownload",
  apply: (end) => {
    holdFfmpegActions("ffmpegDownloading");
    ensureStatus();
    const total = 41 * MB;
    let downloaded = 6 * MB;
    const progress = (): FfmpegDownloadProgress => ({
      phase: "download",
      tool: "ffmpeg",
      source: "registry.npmmirror.com",
      downloaded,
      total,
      bytesPerSec: 5.2 * MB,
    });
    useFfmpeg.setState({ download: { kind: "running", progress: progress(), cancelling: false }, bannerDismissed: false });
    downloadTimer = setInterval(() => {
      downloaded = downloaded + 1.3 * MB > total ? 0 : downloaded + 1.3 * MB;
      useFfmpeg.getState().setProgress(progress());
    }, 400);
    stopDownloadGuard = useFfmpeg.subscribe((next) => {
      if (next.download.kind === "running" && next.download.cancelling) end();
    });
  },
  restore: () => {
    clearInterval(downloadTimer);
    stopDownloadGuard?.();
    stopDownloadGuard = null;
    if (useFfmpeg.getState().download.kind === "running") useFfmpeg.setState({ download: { kind: "idle" } });
    dropAddedStatus();
    releaseFfmpegActions("ffmpegDownloading");
  },
};

const ffmpegFailed: Simulation = {
  id: "ffmpegFailed",
  label: "FFmpeg 下载失败",
  description: "失败原因与“重试下载”按钮；点重试进入“下载进行中”模拟（不会真的下载）。",
  group: "ffmpegDownload",
  apply: () => {
    holdFfmpegActions("ffmpegFailed");
    ensureStatus();
    useFfmpeg.setState({
      download: {
        kind: "failed",
        error: "registry.npmmirror.com：连接超时（15 秒）\nregistry.npmjs.org：HTTP 503 Service Unavailable",
      },
      bannerDismissed: false,
    });
  },
  restore: () => {
    if (useFfmpeg.getState().download.kind === "failed") useFfmpeg.setState({ download: { kind: "idle" } });
    dropAddedStatus();
    releaseFfmpegActions("ffmpegFailed");
  },
};

// ---------------------------------------------------------------- 队列

const QUEUE_PREFIX = "debug-sim-";

function sampleInfo(path: string, i: number): MediaInfo {
  const portrait = i % 3 === 2;
  return {
    path,
    formatName: "mov,mp4,m4a,3gp,3g2,mj2",
    duration: 37.5 + i * 23.1,
    size: (18 + i * 7) * MB,
    bitRate: 4_200_000,
    video: {
      index: 0,
      codec: "h264",
      width: portrait ? 1080 : 1920,
      height: portrait ? 1920 : 1080,
      fps: 30,
      fpsText: "30/1",
      bitRate: 4_000_000,
      pixFmt: "yuv420p",
      rotation: 0,
      frames: null,
      colorTransfer: "",
      colorPrimaries: "",
      colorSpace: "",
    },
    audio: [{ index: 1, codec: "aac", channels: 2, sampleRate: 48000, bitRate: 192_000, language: null, title: null }],
    subtitles: [],
    hasCover: false,
  };
}

type FakeRow = Pick<QueueItem, "state" | "percent" | "round" | "rounds" | "error" | "speed">;

const FAKE_ROWS: FakeRow[] = [
  { state: "running", percent: 42, round: 0, rounds: 1, error: null, speed: 2.4 },
  { state: "running", percent: 77, round: 2, rounds: 3, error: null, speed: 1.1 },
  { state: "waiting", percent: 0, round: 0, rounds: 1, error: null, speed: 0 },
  { state: "waiting", percent: 0, round: 0, rounds: 1, error: null, speed: 0 },
  { state: "done", percent: 100, round: 0, rounds: 1, error: null, speed: 0 },
  { state: "done", percent: 100, round: 3, rounds: 3, error: null, speed: 0 },
  { state: "failed", percent: 31, round: 0, rounds: 1, error: "ffmpeg 退出码 1：Error while filtering: Invalid argument", speed: 0 },
  { state: "failed", percent: 0, round: 0, rounds: 1, error: "180 秒内没有任何进展，判定卡死并结束 ffmpeg", speed: 0 },
  { state: "cancelled", percent: 56, round: 0, rounds: 1, error: null, speed: 0 },
  { state: "cancelled", percent: 0, round: 0, rounds: 1, error: null, speed: 0 },
];

function fakeQueueItems(): QueueItem[] {
  const platform = getPlatform();
  const dir = EXAMPLE_PATHS[platform].videos;
  const out = `${dir}${platform === "windows" ? "\\" : "/"}输出`;
  return FAKE_ROWS.map((row, i) => {
    const name = `示例视频 ${String(i + 1).padStart(2, "0")}.mp4`;
    const path = examplePath(dir, name, platform);
    const outputs =
      row.state === "done"
        ? Array.from({ length: row.rounds }, (_, r) =>
            examplePath(out, row.rounds > 1 ? `示例视频 ${i + 1}_${r + 1}.mp4` : name, platform),
          )
        : [];
    return {
      id: `${QUEUE_PREFIX}${i}`,
      path,
      name,
      sourceRoot: null,
      info: sampleInfo(path, i),
      probeError: null,
      output: outputs[0] ?? null,
      outputs,
      subtitle: i === 1 ? path.replace(/\.mp4$/, ".srt") : null,
      cover: i === 4 ? path.replace(/\.mp4$/, ".jpg") : null,
      ...row,
    };
  });
}

let queueBefore: { running: boolean; runIds: string[]; lastRun: ReturnType<typeof useQueue.getState>["lastRun"] } | null = null;
let stopQueueGuard: (() => void) | null = null;
/** 登记到 lib/simulated.ts 的 ID 与路径：保存队列、开始处理等会排除它们，命令调用遇到它们直接拒绝 */
let simulatedValues: string[] = [];

const queueStates: Simulation = {
  id: "queueStates",
  label: "队列里各种状态的任务",
  description:
    "处理中（含裂变第 2/3 次）、等待、已完成、失败、已取消各 2 个；状态栏显示处理中与总进度。停止 / 取消在前端模拟；这些行不保存，也不会发给后端（播放、工具等会提示是模拟数据）。",
  apply: () => {
    const state = useQueue.getState();
    queueBefore = { running: state.running, runIds: state.runIds, lastRun: state.lastRun };
    const fake = fakeQueueItems();
    simulatedValues = fake.flatMap((i) => [i.id, i.path, ...i.outputs, i.subtitle ?? "", i.cover ?? ""].filter(Boolean));
    markSimulated(simulatedValues);
    useQueue.setState({ items: [...state.items, ...fake], running: true, runIds: fake.map((i) => i.id), lastRun: null });
    // 页面重新加载后后端回报“没有在处理”（jobs_running）：仍显示模拟的处理中
    stopQueueGuard = useQueue.subscribe((next) => {
      if (!next.running && next.items.some((i) => i.id.startsWith(QUEUE_PREFIX) && i.state === "running")) {
        useQueue.setState({ running: true });
      }
    });
  },
  restore: () => {
    stopQueueGuard?.();
    stopQueueGuard = null;
    const state = useQueue.getState();
    const isFake = (id: string) => id.startsWith(QUEUE_PREFIX);
    useQueue.setState({
      items: state.items.filter((i) => !isFake(i.id)),
      selected: new Set([...state.selected].filter((id) => !isFake(id))),
      anchor: state.anchor && isFake(state.anchor) ? null : state.anchor,
      running: queueBefore?.running ?? false,
      runIds: queueBefore?.runIds ?? [],
      lastRun: queueBefore?.lastRun ?? null,
    });
    queueBefore = null;
    unmarkSimulated(simulatedValues);
    simulatedValues = [];
  },
};

// ---------------------------------------------------------------- 处理记录

const FAKE_LOGS: { level: LogLevel; message: string; task?: boolean }[] = [
  { level: "debug", message: "随机种子：3141592653" },
  {
    level: "debug",
    task: true,
    message: "ffmpeg -hide_banner -y -i 示例视频.mp4 -filter_complex \"[0:v:0]fps=30,scale=1080:1920[v0]\" -map [v0] -map 0:a:0? -c:v libx264 -b:v 4000k 输出.mp4",
  },
  { level: "info", message: "导入 10 个视频，2 个已在队列中，跳过 1 个非视频文件" },
  { level: "info", task: true, message: "完成：示例视频 05.mp4 → 示例视频 05.mp4（用时 12.4 秒）" },
  { level: "warn", task: true, message: "重复帧移除之后还有变速：建议把变速放在前面，否则时长可能与预计不同" },
  { level: "warn", message: "无法导入：损坏的文件.mp4：moov atom not found" },
  { level: "error", task: true, message: "处理失败：ffmpeg 退出码 1：Error while filtering: Invalid argument" },
  { level: "error", message: "未找到可用的 FFmpeg，无法开始处理。可以在批量处理页顶部的提示或“设置 → 关于 → 运行环境”中一键下载" },
];

let logIds: Set<number> | null = null;

const logLevels: Simulation = {
  id: "logLevels",
  label: "处理记录里各级别日志",
  description: "调试（勾选“显示调试信息”后可见）、信息、警告、错误各 2 条，含长命令行与带任务的记录。",
  apply: () => {
    const before = new Set(useLog.getState().entries.map((e) => e.id));
    const log = useLog.getState();
    for (const entry of FAKE_LOGS) log.add(entry.level, entry.message, entry.task ? `${QUEUE_PREFIX}log` : null);
    logIds = new Set(useLog.getState().entries.filter((e) => !before.has(e.id)).map((e) => e.id));
  },
  restore: () => {
    const ids = logIds;
    if (ids) useLog.setState({ entries: useLog.getState().entries.filter((e) => !ids.has(e.id)) });
    logIds = null;
  },
};

export const SIMULATIONS: Simulation[] = [ffmpegMissing, ffmpegDownloading, ffmpegFailed, queueStates, logLevels];

export const isSimulationId = (value: unknown): value is SimulationId => SIMULATIONS.some((s) => s.id === value);
