// 打开文件：拖入窗口、添加视频 / 文件夹按钮与快捷键、再次启动程序时命令行里的文件都走这里。
// 配方文件（.json）加载为配方，其余按视频导入（文件夹展开）。

import { isRecipeFile, isTauri, pickFolder, pickVideos } from "../lib/backend";
import { useLog } from "./log";
import { useQueue } from "./queue";
import { useRecipe } from "./recipe";

export function openPaths(paths: string[]): void {
  const recipes = paths.filter(isRecipeFile);
  const videos = paths.filter((p) => !isRecipeFile(p));
  if (recipes.length > 0) {
    void useRecipe.getState().importFile(recipes[0]);
    if (recipes.length > 1) useLog.getState().add("warn", "一次只能加载一个配方文件，已加载第一个");
  }
  if (videos.length > 0) void useQueue.getState().addPaths(videos);
}

/** 系统对话框选择视频后导入（“添加视频”、Ctrl+O）。 */
export async function addVideos(): Promise<void> {
  if (!isTauri || useQueue.getState().importing) return;
  const paths = await pickVideos();
  if (paths.length > 0) await useQueue.getState().addPaths(paths);
}

/** 系统对话框选择文件夹后导入（“添加文件夹”、Ctrl+Shift+O）。 */
export async function addFolder(): Promise<void> {
  if (!isTauri || useQueue.getState().importing) return;
  const dir = await pickFolder();
  if (dir) await useQueue.getState().addPaths([dir]);
}
