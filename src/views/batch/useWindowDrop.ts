import { useEffect, useRef, useState, type RefObject } from "react";
import { folderFromDrop } from "@/components/useFolderDrop";
import { isPointInElement, isRecipeFile, onDragDrop } from "@/lib/backend";
import { openPaths } from "@/state/open";
import { useSettings } from "@/state/settings";

/** 拖放悬停的位置：队列（recipe 为 true 时拖的是配方文件）或输出位置。 */
export type DropHover = { target: "queue" | "output"; recipe: boolean } | null;

/** 窗口级拖放：配方文件 → 加载配方；落在输出位置上 → 设为输出文件夹；其余 → 导入视频。只在批量处理页挂载。 */
export function useWindowDrop(outputBox: RefObject<HTMLDivElement | null>): DropHover {
  const [hover, setHover] = useState<DropHover>(null);
  const draggingRecipe = useRef(false);

  useEffect(
    () =>
      onDragDrop((event) => {
        if (event.type === "leave") {
          setHover(null);
          return;
        }
        if (event.type === "enter") {
          draggingRecipe.current = event.paths.length > 0 && event.paths.every(isRecipeFile);
        }
        const overOutput = isPointInElement(outputBox.current, event.x, event.y);
        const target = overOutput ? "output" : "queue";
        if (event.type !== "drop") {
          setHover({ target, recipe: draggingRecipe.current });
          return;
        }
        setHover(null);
        if (overOutput) {
          void folderFromDrop(event.paths).then((dir) => {
            if (dir) useSettings.getState().update({ outputDir: dir });
          });
          return;
        }
        openPaths(event.paths);
      }),
    [outputBox],
  );
  return hover;
}
