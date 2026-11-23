// 把后端事件接到各个状态仓库。在 App 挂载时调用一次。

import { isTauri, jobsRunning, onImportProgress, onJobEvent } from "../lib/backend";
import { onToolEvent } from "../lib/tools";
import { handleRunFinished } from "./finish";
import { persistQueue, useQueue } from "./queue";
import { useSettings } from "./settings";
import { useTools } from "./tools";

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
    if (event.type === "runFinished") {
      handleRunFinished(
        { done: event.done, failed: event.failed, cancelled: event.cancelled },
        useQueue.getState().runOutputs,
      );
    }
  });
  const stopTool = onToolEvent((event) => useTools.getState().applyEvent(event));
  const stopImport = onImportProgress((progress) => useQueue.getState().setImportProgress(progress));
  if (isTauri) {
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
  };
}
