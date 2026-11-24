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
import { useQuitConfirm } from "@/state/quit";

/** 正在处理视频或运行工具时关闭窗口 / 从托盘退出：确认后停止处理并退出。 */
export default function QuitConfirmDialog() {
  const open = useQuitConfirm((s) => s.open);
  const dismiss = useQuitConfirm((s) => s.dismiss);
  const confirm = useQuitConfirm((s) => s.confirm);
  return (
    <AlertDialog open={open} onOpenChange={(next) => !next && dismiss()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>正在处理，确定退出吗？</AlertDialogTitle>
          <AlertDialogDescription>
            退出会停止正在处理的视频或运行中的工具，未完成的任务下次需要重新处理。
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>继续处理</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={confirm}>
            停止并退出
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
