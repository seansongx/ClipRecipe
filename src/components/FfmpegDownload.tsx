// 应用内下载 FFmpeg 的控件（设置页与批量处理页的提示共用）：下载 / 进度 / 取消 / 失败重试，
// 以及“未找到 FFmpeg”提示。只有点击按钮才会下载；状态见 state/ffmpeg.ts。

import { Download, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import Hint from "@/components/Hint";
import { formatSize } from "@/lib/format";
import { progressText, useFfmpeg } from "@/state/ffmpeg";

/** 下载按钮；下载中显示进度条、速度与“取消”；失败时显示原因与“重试”。 */
export function FfmpegDownloadControl({ idleLabel }: { idleLabel: string }) {
  const download = useFfmpeg((s) => s.download);
  const total = useFfmpeg((s) => s.status?.managed.downloadSize ?? 0);
  const { startDownload, cancelDownload } = useFfmpeg.getState();

  if (download.kind === "running") {
    const p = download.progress;
    const percent = p && p.total > 0 ? Math.min(100, (p.downloaded / p.total) * 100) : 0;
    return (
      <div className="flex flex-col gap-1.5" data-testid="ffmpeg-download-progress">
        <Progress value={percent} aria-label="FFmpeg 下载进度" />
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="min-w-0 flex-1 tabular-nums">
            {progressText(p, total)}
            {p?.phase === "download" && p.source ? `（来自 ${p.source}）` : ""}
          </span>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void cancelDownload()}
            disabled={download.cancelling}
          >
            {download.cancelling ? "正在取消…" : "取消"}
          </Button>
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-col items-start gap-1.5">
      {download.kind === "failed" && (
        <p role="alert" className="text-xs whitespace-pre-wrap text-destructive">
          下载失败：{download.error}
        </p>
      )}
      <Button size="sm" onClick={() => void startDownload()}>
        <Download /> {download.kind === "failed" ? "重试下载" : idleLabel}
      </Button>
    </div>
  );
}

/** 下载说明：版本、大小、来源、许可。 */
export function FfmpegDownloadNote() {
  const managed = useFfmpeg((s) => s.status?.managed);
  if (!managed) return null;
  return (
    <p className="text-xs text-muted-foreground">
      FFmpeg {managed.pinnedVersion}（ffmpeg + ffprobe，下载约 {formatSize(managed.downloadSize)}）来自 npm 包
      @seankit/ffmpeg 与 @seankit/ffprobe {managed.packageVersion}，经 npmmirror（失败时改用 npmjs）下载，是未经修改的上游构建
      （Windows 为 gyan.dev essentials）。下载后按程序内置的 SHA-256 逐一校验。FFmpeg 是按 GPL 授权的第三方软件，
      ClipRecipe 不包含也不分发它；只有点击下载按钮时才会下载。
    </p>
  );
}

/** 批量处理页：启动时没有可用的 FFmpeg 时提示一键下载（不阻塞界面，可关闭）。 */
export function FfmpegMissingBanner() {
  const status = useFfmpeg((s) => s.status);
  const dismissed = useFfmpeg((s) => s.bannerDismissed);
  const running = useFfmpeg((s) => s.download.kind === "running");
  if (!status || (status.available && !running) || (dismissed && !running)) return null;
  const managed = status.managed;
  return (
    <div
      role="region"
      aria-label="未找到 FFmpeg"
      className="mx-4 mt-2 flex items-start gap-2 rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-xs"
    >
      <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <p className="font-medium">未找到 FFmpeg，暂时无法导入和处理视频</p>
        {managed.supported ? (
          <>
            <p className="text-muted-foreground">
              可以一键下载 FFmpeg {managed.pinnedVersion}（约 {formatSize(managed.downloadSize)}，来自 npmmirror / npm 上的
              @seankit/ffmpeg，未经修改的上游构建，GPL 许可）。也可以自行安装 FFmpeg 后在“设置 → 关于”中重新检测。
            </p>
            <FfmpegDownloadControl idleLabel={`下载 FFmpeg（${formatSize(managed.downloadSize)}）`} />
          </>
        ) : (
          <p className="text-muted-foreground">
            {managed.unsupportedReason ?? "请安装 FFmpeg 并加入 PATH，或设置环境变量 CLIPRECIPE_FFMPEG"}
          </p>
        )}
      </div>
      {!running && (
        <Hint label="暂不下载">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="关闭 FFmpeg 提示"
            onClick={() => useFfmpeg.getState().dismissBanner()}
          >
            <X />
          </Button>
        </Hint>
      )}
    </div>
  );
}
