// 独立工具（tool.*）的后端命令与事件。命令见 src-tauri/src/commands/tools.rs，类型与后端 serde 结构一致。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  isTauri,
  type JobEvent,
  type MediaInfo,
  type ModuleParams,
  type PreviewResult,
  type Settings,
  type TaskSpec,
} from "./backend";

/** 工具参数：键值对象，具体字段见 features/tools/<工具>.tsx。 */
export type ToolParams = Record<string, unknown>;

/** 后端规划出的一个工作项（事件里的 taskId 就是它的 id）。 */
export interface PlannedItem {
  id: string;
  label: string;
  inputs: string[];
}

export interface StartToolRequest {
  tool: string;
  params: ToolParams;
  /** 队列中要处理的视频（工具声明了 videos 输入时） */
  videos: TaskSpec[];
  /** 文件夹输入：键为工具声明的文件夹键 */
  folders: Record<string, string>;
  settings: Settings;
  outputDir: string;
  /** 流水线的码率模块参数（pipe.bitrate）：工具沿用它 */
  bitrate: ModuleParams | null;
  seed?: number;
}

export interface StartToolResponse {
  seed: number;
  items: PlannedItem[];
}

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) return Promise.reject(new Error("此功能需要在桌面应用中使用"));
  return invoke<T>(command, args);
}

export const startTool = (request: StartToolRequest) => call<StartToolResponse>("start_tool", { request });
export const stopTool = () => call<void>("stop_tool");
export const cancelToolItem = (id: string) => call<boolean>("cancel_tool_item", { id });
export const toolRunning = () => call<boolean>("tool_running");

/** 工具预览请求（后端 tools::preview::ToolPreviewSpec）：第一个选中视频的一段低清片段经工具处理。 */
export interface ToolPreviewSpec {
  tool: string;
  params: ToolParams;
  path: string;
  info: MediaInfo | null;
  settings: Settings;
  bitrate: ModuleParams | null;
  /** 片段在源视频中的起点与时长（秒；时长 0 为默认 5 秒） */
  start: number;
  duration: number;
  number: number;
  seed?: number;
  maxHeight?: number;
}

/**
 * 生成工具预览（声明了 preview 的工具）。与流水线预览共用进度事件 preview-progress、
 * 取消（cancelPreview）与文件释放（releasePreviewFile）。
 */
export const renderToolPreview = (spec: ToolPreviewSpec) => call<PreviewResult>("render_tool_preview", { spec });

/** 工具运行事件（内容同 job-event；进度的 round/rounds 表示第几步/共几步）。 */
export function onToolEvent(handler: (event: JobEvent) => void): () => void {
  if (!isTauri) return () => {};
  const unlisten = listen<JobEvent>("tool-event", (e) => handler(e.payload));
  return () => {
    void unlisten.then((stop) => stop());
  };
}
