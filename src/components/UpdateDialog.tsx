// 发现新版本时的对话框：显示版本号与更新说明；能自动更新时下载（进度、可取消）→ 安装并重启，
// 否则或失败时打开发布页。状态见 state/update.ts。

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
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { formatSize } from "@/lib/format";
import { canAutoInstall, progressPercent, useUpdate, type UpdateRun } from "@/state/update";

function DownloadStatus({ run }: { run: Extract<UpdateRun, { kind: "downloading" }> }) {
  const percent = progressPercent(run.progress);
  const downloaded = run.progress ? formatSize(run.progress.downloaded) : "";
  const total = run.progress?.total ? formatSize(run.progress.total) : "";
  return (
    <div className="flex flex-col gap-1.5" data-testid="update-download-progress">
      <Progress value={percent} aria-label="更新下载进度" />
      <span className="text-xs text-muted-foreground tabular-nums">
        {run.cancelling
          ? "正在取消…"
          : percent === null
            ? `正在下载…${downloaded ? ` ${downloaded}` : ""}`
            : `正在下载 ${Math.floor(percent)}%（${downloaded} / ${total}）`}
      </span>
    </div>
  );
}

/** 发现新版本时询问是否更新（或打开发布页）。 */
export default function UpdateDialog() {
  const prompt = useUpdate((s) => s.prompt);
  const info = useUpdate((s) => s.info);
  const run = useUpdate((s) => s.run);
  const { dismiss, install, cancel, openReleasePage } = useUpdate.getState();
  const auto = canAutoInstall(info);

  const title =
    run.kind === "installing"
      ? `正在安装 ${info?.latest ?? ""}`
      : run.kind === "failed"
        ? "更新失败"
        : `发现新版本 ${info?.latest ?? ""}`;

  return (
    <AlertDialog open={prompt && !!info} onOpenChange={(open) => !open && dismiss()}>
      <AlertDialogContent className="sm:max-w-md" data-testid="update-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>
            当前版本 {info?.current}，最新版本 {info?.latest}。
            {run.kind === "idle" &&
              (auto
                ? "可以在程序内下载并安装，完成后程序会自动重启。"
                : `${info?.manualReason ? `${info.manualReason}。` : ""}是否在浏览器中打开发布页下载？`)}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {info?.notes && run.kind !== "installing" && (
          <ScrollArea className="max-h-48 rounded-md bg-muted/50">
            <div className="p-3 text-xs whitespace-pre-wrap text-muted-foreground" aria-label="更新说明">
              {info.notes}
            </div>
          </ScrollArea>
        )}

        {run.kind === "downloading" && <DownloadStatus run={run} />}
        {run.kind === "installing" && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner /> 正在安装，完成后程序会自动重启…
          </div>
        )}
        {run.kind === "failed" && (
          <p role="alert" className="text-xs whitespace-pre-wrap text-destructive">
            {run.error}。可以重试，或打开发布页手动下载安装包。
          </p>
        )}

        {run.kind !== "installing" && (
          <AlertDialogFooter>
            {run.kind === "downloading" ? (
              <Button variant="outline" onClick={() => void cancel()} disabled={run.cancelling}>
                取消下载
              </Button>
            ) : (
              <>
                <AlertDialogCancel>{run.kind === "failed" ? "关闭" : "稍后"}</AlertDialogCancel>
                {auto ? (
                  <>
                    <Button variant="outline" onClick={openReleasePage}>
                      打开发布页
                    </Button>
                    <AlertDialogAction onClick={() => void install()}>
                      {run.kind === "failed" ? "重试" : "下载并安装"}
                    </AlertDialogAction>
                  </>
                ) : (
                  <AlertDialogAction onClick={openReleasePage}>打开发布页</AlertDialogAction>
                )}
              </>
            )}
          </AlertDialogFooter>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}
