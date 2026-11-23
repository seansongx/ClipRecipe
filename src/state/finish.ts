// 批量处理全部结束后的动作：提示音（set.sound）、打开输出文件夹（set.open-output）、
// 关机 / 退出（set.after-all）。由 state/events.ts 在收到 runFinished 后调用。
//
// 关机：后端开始 30 秒倒计时（shutdown-countdown 事件），界面显示可取消的对话框（components/ShutdownDialog）。
// “全部完成后”只对一次运行有效：触发后立即改回“无操作”（启动时后端也会重置）。

import { create } from "zustand";
import {
  allowSoundFile,
  cancelShutdown,
  isTauri,
  mediaUrl,
  onShutdownCountdown,
  onShutdownFailed,
  openFolder,
  quitApp,
  scheduleShutdown,
  type AfterAll,
  type Settings,
} from "../lib/backend";
import { dirName } from "../lib/format";
import { useLog } from "./log";
import type { RunStats } from "./queue";
import { useSettings } from "./settings";

interface ShutdownState {
  /** 关机倒计时剩余秒数；null 表示没有倒计时 */
  remaining: number | null;
  cancel: () => void;
}

export const useShutdown = create<ShutdownState>((set) => ({
  remaining: null,
  cancel: () => {
    set({ remaining: null });
    cancelShutdown().catch((e) => useLog.getState().add("error", `取消关机失败：${String(e)}`));
    useLog.getState().add("info", "已取消关机");
  },
}));

/** 结束后要打开的文件夹：设置了输出目录时为它，否则为第一个输出所在的文件夹。 */
export function folderToOpen(settings: Settings, outputs: string[]): string | null {
  const dir = settings.outputDir.trim();
  if (dir) return dir;
  const first = outputs[0];
  return first ? dirName(first) || null : null;
}

/** 本次运行结束后要做的事（纯函数，便于检查）。只有至少完成一个任务时才打开文件夹 / 关机 / 退出。 */
export function finishActions(
  settings: Settings,
  stats: RunStats,
  outputs: string[],
): { sound: boolean; openFolder: string | null; afterAll: AfterAll } {
  const anyDone = stats.done > 0;
  return {
    sound: settings.sound && stats.done + stats.failed > 0,
    openFolder: settings.openOutput && anyDone ? folderToOpen(settings, outputs) : null,
    afterAll: anyDone || stats.failed > 0 ? settings.afterAll : "none",
  };
}

/** 内置提示音：用 WebAudio 合成两声短促的“叮咚”，不需要音频文件。 */
function playBuiltInChime() {
  const AudioContextClass = window.AudioContext;
  if (!AudioContextClass) return;
  const ctx = new AudioContextClass();
  const notes = [880, 1318.5];
  notes.forEach((freq, i) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    const start = ctx.currentTime + i * 0.18;
    osc.type = "sine";
    osc.frequency.value = freq;
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.25, start + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.45);
    osc.connect(gain).connect(ctx.destination);
    osc.start(start);
    osc.stop(start + 0.5);
  });
  setTimeout(() => void ctx.close(), 1200);
}

/** 播放完成提示音：自定义 wav（不可用时退回内置提示音）。 */
export async function playCompletionSound(soundPath: string): Promise<void> {
  const custom = soundPath.trim();
  if (custom && isTauri) {
    try {
      await allowSoundFile(custom);
      await new Audio(mediaUrl(custom)).play();
      return;
    } catch (e) {
      useLog.getState().add("warn", `无法播放自定义提示音，改用内置提示音：${String(e)}`);
    }
  }
  playBuiltInChime();
}

let countdownListening = false;

function listenCountdown() {
  if (countdownListening) return;
  countdownListening = true;
  onShutdownCountdown(({ remaining }) => {
    if (useShutdown.getState().remaining !== null) useShutdown.setState({ remaining });
  });
  onShutdownFailed((error) => {
    useShutdown.setState({ remaining: null });
    useLog.getState().add("error", `关机失败：${error}`);
  });
}

/** 收到 runFinished 后调用。 */
export function handleRunFinished(stats: RunStats, outputs: string[]) {
  const settings = useSettings.getState().settings;
  const actions = finishActions(settings, stats, outputs);
  const log = useLog.getState();
  if (actions.sound) void playCompletionSound(settings.soundPath);
  if (actions.openFolder) {
    openFolder(actions.openFolder).catch((e) => log.add("warn", `无法打开输出文件夹：${String(e)}`));
  }
  if (actions.afterAll === "none") return;
  // 只对一次运行有效
  useSettings.getState().update({ afterAll: "none" });
  if (actions.afterAll === "quit") {
    log.add("info", "全部完成，正在退出程序");
    setTimeout(() => void quitApp().catch(() => {}), 1500);
    return;
  }
  listenCountdown();
  useShutdown.setState({ remaining: 30 });
  scheduleShutdown().catch((e) => {
    useShutdown.setState({ remaining: null });
    log.add("error", `无法开始关机倒计时：${String(e)}`);
  });
  log.add("warn", "全部完成，30 秒后关机（可取消）");
}
