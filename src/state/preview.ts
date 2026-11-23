// 预览窗口（pipe.preview、单模块预览、工具预览、双击播放源视频）的状态。
// 预览文件由后端写在临时目录，换片段或关闭窗口时通知后端删除。

import { create } from "zustand";
import {
  allowMediaFile,
  cancelPreview,
  isTauri,
  mediaUrl,
  onPreviewProgress,
  releasePreviewFile,
  renderPreview,
  type PreviewResult,
} from "../lib/backend";
import { PIPELINE_MODULES, TOOLS } from "../features/registry";
import { renderToolPreview, type ToolParams } from "../lib/tools";
import type { QueueItem } from "./queue";
import { useRecipe } from "./recipe";
import { getSelectedVideo } from "./selection";
import { useSettings } from "./settings";

export type PreviewMode =
  /** 按配方生成的预览；moduleId 为 null 表示整条流水线 */
  | { kind: "pipeline"; moduleId: string | null }
  /** 工具预览：当前视频的一段低清片段经工具处理（工具声明了 preview） */
  | { kind: "tool"; toolId: string; params: ToolParams }
  /** 直接播放源视频 */
  | { kind: "source" };

export type PreviewStatus = "idle" | "rendering" | "ready" | "error";

interface PreviewState {
  open: boolean;
  mode: PreviewMode;
  item: QueueItem | null;
  number: number;
  status: PreviewStatus;
  percent: number;
  /** 可直接给 <video src> 的地址 */
  url: string | null;
  result: PreviewResult | null;
  error: string | null;
  /** 片段起点与时长（秒，输出时间轴） */
  start: number;
  duration: number;

  /** 用当前配方预览当前视频；moduleId 为 null 时预览整条流水线。 */
  previewPipeline: (moduleId: string | null) => void;
  /** 用工具自己的运行逻辑预览当前视频（第一个选中的视频）。 */
  previewTool: (toolId: string, params: ToolParams) => void;
  /** 在预览窗口中播放某个源视频。 */
  playSource: (item: QueueItem) => void;
  /** 修改片段范围并重新生成。 */
  setWindow: (start: number, duration: number) => void;
  rerender: () => void;
  close: () => void;
}

/** 预览片段的默认时长（秒）。 */
export const DEFAULT_PREVIEW_SECONDS = 5;

let generation = 0;
let ownedFile: string | null = null;

function releaseOwned() {
  if (ownedFile) releasePreviewFile(ownedFile);
  ownedFile = null;
}

export function moduleName(id: string | null): string {
  if (!id) return "整条流水线";
  return PIPELINE_MODULES.find((m) => m.id === id)?.name ?? id;
}

/** 预览里参与的步骤名（流水线模块或工具）。 */
export function stepName(id: string): string {
  return TOOLS.find((t) => t.id === id)?.name ?? moduleName(id);
}

export const usePreview = create<PreviewState>((set, get) => {
  const render = async () => {
    const { item, number, mode, start, duration } = get();
    if (!item || mode.kind === "source") return;
    const current = ++generation;
    set({ status: "rendering", percent: 0, error: null });
    try {
      const result =
        mode.kind === "tool"
          ? await renderToolPreview({
              tool: mode.toolId,
              params: mode.params,
              path: item.path,
              info: item.info,
              settings: useSettings.getState().settings,
              bitrate: useRecipe.getState().recipe.modules["pipe.bitrate"] ?? null,
              start,
              duration,
              number,
            })
          : await renderPreview({
              path: item.path,
              info: item.info,
              recipe: useRecipe.getState().recipe,
              settings: useSettings.getState().settings,
              onlyModule: mode.moduleId,
              output: { kind: "clip", start, duration },
              number,
              subtitle: item.subtitle,
            });
      if (current !== generation || !get().open) {
        releasePreviewFile(result.path);
        return;
      }
      releaseOwned();
      ownedFile = result.path;
      set({ status: "ready", result, url: mediaUrl(result.path), percent: 100 });
    } catch (e) {
      if (current !== generation) return;
      set({ status: "error", error: String(e).replace(/^Error: /, "") });
    }
  };

  onPreviewProgress(({ percent }) => {
    if (get().status === "rendering") set({ percent });
  });

  return {
    open: false,
    mode: { kind: "pipeline", moduleId: null },
    item: null,
    number: 1,
    status: "idle",
    percent: 0,
    url: null,
    result: null,
    error: null,
    start: 0,
    duration: DEFAULT_PREVIEW_SECONDS,

    previewPipeline: (moduleId) => {
      const selected = getSelectedVideo();
      if (!selected) {
        set({
          open: true,
          mode: { kind: "pipeline", moduleId },
          item: null,
          status: "error",
          error: "队列为空：请先添加视频",
        });
        return;
      }
      const sameVideo = get().item?.id === selected.item.id;
      set({
        open: true,
        mode: { kind: "pipeline", moduleId },
        item: selected.item,
        number: selected.number,
        url: null,
        result: null,
        start: sameVideo ? get().start : 0,
      });
      if (!isTauri) {
        set({ status: "error", error: "预览需要在桌面应用中使用" });
        return;
      }
      void render();
    },

    previewTool: (toolId, params) => {
      const selected = getSelectedVideo();
      const mode: PreviewMode = { kind: "tool", toolId, params };
      if (!selected) {
        set({ open: true, mode, item: null, status: "error", error: "队列为空：请先添加视频" });
        return;
      }
      const sameVideo = get().item?.id === selected.item.id;
      set({
        open: true,
        mode,
        item: selected.item,
        number: selected.number,
        url: null,
        result: null,
        start: sameVideo ? get().start : 0,
      });
      if (!isTauri) {
        set({ status: "error", error: "预览需要在桌面应用中使用" });
        return;
      }
      void render();
    },

    playSource: (item) => {
      generation++;
      void cancelPreview().catch(() => {});
      releaseOwned();
      set({
        open: true,
        mode: { kind: "source" },
        item,
        status: "rendering",
        url: null,
        result: null,
        error: null,
      });
      if (!isTauri) {
        set({ status: "error", error: "播放需要在桌面应用中使用" });
        return;
      }
      const current = generation;
      allowMediaFile(item.path)
        .then(() => {
          if (current === generation) set({ status: "ready", url: mediaUrl(item.path) });
        })
        .catch((e) => {
          if (current === generation) set({ status: "error", error: String(e) });
        });
    },

    setWindow: (start, duration) => {
      set({ start: Math.max(0, start), duration: Math.max(0.5, duration) });
      void render();
    },

    rerender: () => void render(),

    close: () => {
      generation++;
      if (get().status === "rendering") void cancelPreview().catch(() => {});
      releaseOwned();
      set({ open: false, status: "idle", url: null, result: null, error: null, percent: 0 });
    },
  };
});
