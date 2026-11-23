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
import { openProjectPage } from "@/lib/backend";
import { useLog } from "@/state/log";
import { useUpdate } from "@/state/update";

/** 发现新版本时询问是否打开发布页。 */
export default function UpdateDialog() {
  const prompt = useUpdate((s) => s.prompt);
  const info = useUpdate((s) => s.info);
  const dismiss = useUpdate((s) => s.dismiss);
  return (
    <AlertDialog open={prompt && !!info} onOpenChange={(open) => !open && dismiss()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>发现新版本 {info?.latest}</AlertDialogTitle>
          <AlertDialogDescription>
            当前版本 {info?.current}。是否在浏览器中打开发布页下载？（不会自动下载或替换程序）
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>稍后</AlertDialogCancel>
          <AlertDialogAction
            onClick={() => {
              if (info)
                openProjectPage(info.url).catch((e) =>
                  useLog.getState().add("error", `无法打开发布页：${String(e)}`),
                );
              dismiss();
            }}
          >
            打开发布页
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
