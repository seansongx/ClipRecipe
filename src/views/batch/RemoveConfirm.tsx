// set.remove-task：手动删除队列任务时的确认（可在设置中关闭）。

import { useId } from "react";
import { create } from "zustand";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { useQueue } from "@/state/queue";
import { useSettings } from "@/state/settings";

/** 要删除的任务：一组 ID，或清空整个队列。 */
type Pending = { kind: "ids"; ids: string[] } | { kind: "clear" };

const usePending = create<{ pending: Pending | null; dontAsk: boolean }>(() => ({
  pending: null,
  dontAsk: false,
}));

function perform(pending: Pending) {
  const queue = useQueue.getState();
  if (pending.kind === "clear") queue.clear();
  else queue.remove(pending.ids);
}

/** 删除任务（从队列移除，不删除文件）；开启“删除任务时确认”时先弹出确认。 */
export function requestRemove(pending: Pending) {
  if (pending.kind === "ids" && pending.ids.length === 0) return;
  if (!useSettings.getState().settings.confirmRemove) {
    perform(pending);
    return;
  }
  usePending.setState({ pending, dontAsk: false });
}

export function RemoveConfirmDialog() {
  const pending = usePending((s) => s.pending);
  const dontAsk = usePending((s) => s.dontAsk);
  const items = useQueue((s) => s.items);
  const checkboxId = useId();
  const count = pending?.kind === "ids" ? pending.ids.length : items.length;
  const close = () => usePending.setState({ pending: null });

  return (
    <AlertDialog open={pending !== null} onOpenChange={(open) => !open && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{pending?.kind === "clear" ? "清空队列？" : `从队列中删除 ${count} 个任务？`}</AlertDialogTitle>
          <AlertDialogDescription>
            只从队列中移除，不会删除磁盘上的视频文件。正在处理的任务不会被删除。
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="flex items-center gap-2">
          <Checkbox
            id={checkboxId}
            checked={dontAsk}
            onCheckedChange={(v) => usePending.setState({ dontAsk: v === true })}
          />
          <Label htmlFor={checkboxId}>
            以后不再询问（可在设置中重新开启）
          </Label>
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel>取消</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              if (pending) perform(pending);
              if (dontAsk) useSettings.getState().update({ confirmRemove: false });
              close();
            }}
          >
            删除
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
