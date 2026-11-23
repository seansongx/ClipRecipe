import { useEffect, useState } from "react";
import { extractFrame, isTauri, mediaUrl, releasePreviewFile, type FrameResult } from "../lib/backend";
import type { QueueItem } from "../state/queue";

export interface VideoFrame {
  /** 可给 <img src> 的地址 */
  url: string;
  /** 源视频（旋转后）的显示宽高：区域坐标以此为准 */
  width: number;
  height: number;
  time: number;
}

export interface VideoFrameState {
  frame: VideoFrame | null;
  loading: boolean;
  error: string | null;
}

/**
 * 取队列视频 `time` 秒处的一帧（未经流水线处理）。参数变化时重新取帧（250ms 防抖），
 * 旧图片与卸载时的图片会通知后端删除。`item` 为 null 时不取帧。
 */
export function useVideoFrame(item: QueueItem | null, time: number): VideoFrameState {
  const [state, setState] = useState<VideoFrameState>({ frame: null, loading: false, error: null });
  const path = item?.path ?? null;
  const info = item?.info ?? null;

  useEffect(() => {
    if (!path) {
      setState({ frame: null, loading: false, error: "队列为空：请先添加视频" });
      return;
    }
    if (!isTauri) {
      setState({ frame: null, loading: false, error: "取帧需要在桌面应用中使用" });
      return;
    }
    let alive = true;
    let owned: FrameResult | null = null;
    setState((s) => ({ ...s, loading: true, error: null }));
    const timer = setTimeout(() => {
      extractFrame(path, info, time)
        .then((result) => {
          if (!alive) {
            releasePreviewFile(result.path);
            return;
          }
          owned = result;
          setState({
            frame: { url: mediaUrl(result.path), width: result.width, height: result.height, time: result.time },
            loading: false,
            error: null,
          });
        })
        .catch((e) => alive && setState({ frame: null, loading: false, error: String(e) }));
    }, 250);
    return () => {
      alive = false;
      clearTimeout(timer);
      // 新图片加载前保留旧图片的显示，稍后再删除文件
      const old = owned;
      if (old) setTimeout(() => releasePreviewFile(old.path), 1000);
    };
    // info 只是加速用的缓存（缺失时后端会探测），不必作为依赖
  }, [path, time]);

  return state;
}
