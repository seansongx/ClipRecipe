// 与 Rust 后端通信的封装。命令定义见 src-tauri/src/commands.rs。

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

/** 是否运行在 Tauri 窗口中（`npm run dev` 直接用浏览器打开时为 false）。 */
export const isTauri = "__TAURI_INTERNALS__" in window;

export interface AppInfo {
  name: string;
  version: string;
  cpuCores: number;
}

export interface FfmpegStatus {
  available: boolean;
  path: string;
  version: string | null;
  error: string | null;
}

export const getAppInfo = () => invoke<AppInfo>("app_info");
export const getFfmpegStatus = () => invoke<FfmpegStatus>("ffmpeg_status");

export const VIDEO_EXTENSIONS = [
  "mp4", "mkv", "mov", "avi", "flv", "wmv", "webm", "ts", "m4v", "mpg", "mpeg", "3gp",
];

export async function pickVideos(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    filters: [{ name: "视频", extensions: VIDEO_EXTENSIONS }],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

export async function pickFolder(): Promise<string | null> {
  const picked = await open({ directory: true });
  return typeof picked === "string" ? picked : null;
}

/** 监听拖入窗口的文件；返回取消监听的函数。 */
export function onFilesDropped(
  onDrop: (paths: string[]) => void,
  onHover: (hovering: boolean) => void,
): () => void {
  if (!isTauri) return () => {};
  const unlisten = getCurrentWebview().onDragDropEvent((event) => {
    const { type } = event.payload;
    if (type === "enter" || type === "over") onHover(true);
    else onHover(false);
    if (event.payload.type === "drop") onDrop(event.payload.paths);
  });
  return () => {
    void unlisten.then((stop) => stop());
  };
}
