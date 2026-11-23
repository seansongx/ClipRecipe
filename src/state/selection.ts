// “当前视频”：预览、区域框选等以哪个队列视频为准。
// 规则：队列中第一个被选中的视频；没有选中时取队列第一个；队列为空时为 null。
// 模块表单可以用 useSelectedVideo() 读取（只读），例如显示源视频尺寸或为 RegionPicker 提供画面。

import { useQueue, type QueueItem } from "./queue";

export interface SelectedVideo {
  item: QueueItem;
  /** 队列序号（从 1 起），用于素材“依次取” */
  number: number;
}

function pick(items: QueueItem[], selected: Set<string>): SelectedVideo | null {
  const index = items.findIndex((i) => selected.has(i.id));
  const at = index >= 0 ? index : 0;
  const item = items[at];
  return item ? { item, number: at + 1 } : null;
}

/** 当前视频（非 React 场景使用）。 */
export function getSelectedVideo(): SelectedVideo | null {
  const { items, selected } = useQueue.getState();
  return pick(items, selected);
}

/** 当前视频；队列或选择变化时更新。 */
export function useSelectedVideo(): SelectedVideo | null {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  return pick(items, selected);
}
