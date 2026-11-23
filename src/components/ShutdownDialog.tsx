import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { useShutdown } from "@/state/finish";

/** set.after-all 为“关机”时的 30 秒倒计时，可取消。 */
export default function ShutdownDialog() {
  const remaining = useShutdown((s) => s.remaining);
  const cancel = useShutdown((s) => s.cancel);
  return (
    <AlertDialog open={remaining !== null}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>全部完成，即将关机</AlertDialogTitle>
          <AlertDialogDescription>
            {remaining === 0 ? "正在关机…" : `将在 ${remaining ?? 0} 秒后关机。请保存其它程序中的工作。`}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogAction onClick={cancel} disabled={remaining === 0}>
            取消关机
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
