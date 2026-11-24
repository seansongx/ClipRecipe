// 把后端事件接到各个状态仓库。在 App 挂载时调用一次。

import {
  isTauri,
  jobsRunning,
  onCloseRequested,
  onFfmpegDownload,
  onImportProgress,
  onJobEvent,
  onUpdateFailed,
  onUpdateInstalling,
  onUpdateProgress,
} from "../lib/backend";
import { loadRealPlatform } from "../lib/platform";
import { onToolEvent } from "../lib/tools";
import { useEncoders } from "./encoders";
import { useFfmpeg } from "./ffmpeg";
import { handleRunFinished } from "./finish";
import { persistQueue, useQueue } from "./queue";
import { useQuitConfirm } from "./quit";
import { useSettings } from "./settings";
import { useTools } from "./tools";
import { useUpdate } from "./update";

export function initBackend(): () => void {
  let stopPersist: (() => void) | null = null;
  let disposed = false;
  void useSettings
    .getState()
    .load()
    .then(async () => {
      // set.keep-queue：先恢复上次未完成的任务，再开始保存（避免先把空队列写回去）
      if (isTauri && useSettings.getState().settings.keepQueue) await useQueue.getState().restore();
      if (!disposed && isTauri) stopPersist = persistQueue();
    });
  const stopJob = onJobEvent((event) => {
    useQueue.getState().applyEvent(event);
    // 开始处理时后端已探测编码器：第一个任务开始后读取结果
    if (event.type === "taskState" && event.state === "running" && !useEncoders.getState().info) {
      void useEncoders.getState().refresh();
    }
    if (event.type === "runFinished") {
      // 开始处理时后端探测了编码器
      void useEncoders.getState().refresh();
      handleRunFinished(
        { done: event.done, failed: event.failed, cancelled: event.cancelled },
        useQueue.getState().runOutputs,
      );
    }
  });
  const stopTool = onToolEvent((event) => {
    useTools.getState().applyEvent(event);
    if (event.type === "runFinished") void useEncoders.getState().refresh();
  });
  const stopImport = onImportProgress((progress) => useQueue.getState().setImportProgress(progress));
  const stopFfmpeg = onFfmpegDownload((progress) => useFfmpeg.getState().setProgress(progress));
  const stopUpdateProgress = onUpdateProgress((progress) => useUpdate.getState().setProgress(progress));
  const stopUpdateInstalling = onUpdateInstalling(() => useUpdate.getState().setInstalling());
  const stopUpdateFailed = onUpdateFailed((error) => useUpdate.getState().setFailed(error));
  const stopCloseRequested = onCloseRequested(() => useQuitConfirm.getState().request());
  if (isTauri) {
    // 真实平台以后端为准（lib/platform.ts；读到之前用 User-Agent 推断）
    void loadRealPlatform();
    // 启动时检测 FFmpeg：没有时批量处理页提示一键下载（不会自动下载）
    void useFfmpeg.getState().refresh().then(() => useEncoders.getState().refresh());
    // 页面重新加载后同步“是否正在处理”
    jobsRunning()
      .then((running) => useQueue.setState({ running }))
      .catch(() => {});
  }
  return () => {
    disposed = true;
    stopPersist?.();
    stopJob();
    stopTool();
    stopImport();
    stopFfmpeg();
    stopUpdateProgress();
    stopUpdateInstalling();
    stopUpdateFailed();
    stopCloseRequested();
  };
}
