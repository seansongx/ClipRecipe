// 把文件夹拖到界面上的某个输入框（输出位置、工具的文件夹输入）：Tauri 窗口级拖放事件 + 按位置判断落在哪个框上。
// 拖入的是文件时取它所在的文件夹。

import { useCallback, useEffect, useRef, useState, type RefCallback } from "react";
import { checkOutputDir, isPointInElement, onDragDrop } from "@/lib/backend";
import { dirName } from "@/lib/format";

/** 拖入的第一个路径：是文件夹则用它，否则用文件所在的文件夹。 */
export async function folderFromDrop(paths: string[]): Promise<string | null> {
  const first = paths[0];
  if (!first) return null;
  try {
    const check = await checkOutputDir(first, []);
    return check.exists ? first : dirName(first) || null;
  } catch {
    return null;
  }
}

/**
 * 监听拖放：把 `dropBox(键)` 作为 ref 交给若干输入框；落点在某个框上时把拖入的文件夹交给 `onDrop(键, 文件夹)`。
 * `hover` 为当前悬停的框的键（用于高亮），不在任何框上时为 null。
 */
export function useFolderDrop(onDrop: (key: string, folder: string) => void): {
  hover: string | null;
  dropBox: (key: string) => RefCallback<HTMLElement>;
} {
  const [hover, setHover] = useState<string | null>(null);
  const boxes = useRef(new Map<string, HTMLElement>());
  const refs = useRef(new Map<string, RefCallback<HTMLElement>>());
  const onDropRef = useRef(onDrop);
  useEffect(() => {
    onDropRef.current = onDrop;
  });

  const dropBox = useCallback((key: string) => {
    let ref = refs.current.get(key);
    if (!ref) {
      ref = (element) => {
        if (element) boxes.current.set(key, element);
        else boxes.current.delete(key);
      };
      refs.current.set(key, ref);
    }
    return ref;
  }, []);

  useEffect(() => {
    const boxAt = (x: number, y: number) => {
      for (const [key, element] of boxes.current) {
        if (isPointInElement(element, x, y)) return key;
      }
      return null;
    };
    return onDragDrop((event) => {
      if (event.type === "leave") {
        setHover(null);
        return;
      }
      const key = boxAt(event.x, event.y);
      if (event.type !== "drop") {
        setHover(key);
        return;
      }
      setHover(null);
      if (!key) return;
      void folderFromDrop(event.paths).then((dir) => {
        if (dir) onDropRef.current(key, dir);
      });
    });
  }, []);

  return { hover, dropBox };
}
