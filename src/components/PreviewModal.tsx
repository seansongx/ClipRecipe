import { useEffect, useState } from "react";
import { RefreshCw, TriangleAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import { Spinner } from "@/components/ui/spinner";
import { formatDuration } from "@/lib/format";
import { moduleName, stepName, usePreview } from "@/state/preview";
import { useSettings } from "@/state/settings";
import NumberInput from "./NumberInput";

/** 预览窗口：显示预览片段或播放源视频。由 state/preview.ts 控制，在 App 中挂载一次。 */
export default function PreviewModal() {
  const open = usePreview((s) => s.open);
  const close = usePreview((s) => s.close);
  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      {open && <PreviewDialog />}
    </Dialog>
  );
}

function PreviewDialog() {
  const { mode, item, status, percent, url, result, error, start, duration } = usePreview();
  const mutePreview = useSettings((s) => s.settings.mutePreview);
  const { setWindow, rerender } = usePreview.getState();
  // 生成的预览（流水线或工具）；否则是直接播放源视频
  const pipeline = mode.kind !== "source";
  const name = mode.kind === "tool" ? stepName(mode.toolId) : mode.kind === "pipeline" ? moduleName(mode.moduleId) : "";
  const title = pipeline ? `预览 · ${name}${item ? ` · ${item.name}` : ""}` : `播放 · ${item?.name ?? ""}`;

  return (
    <DialogContent
      aria-describedby={undefined}
      className="flex max-h-[calc(100vh-3rem)] flex-col gap-0 overflow-hidden p-0 sm:max-w-4xl"
    >
      <DialogHeader className="border-b px-4 py-3 pr-12">
        <DialogTitle className="truncate text-sm">{title}</DialogTitle>
      </DialogHeader>
      <div className="grid min-h-[320px] place-items-center bg-black">
        {status === "ready" && url && (
          <video key={url} src={url} controls autoPlay loop={pipeline} muted={mutePreview} className="max-h-[65vh] w-full" />
        )}
        {status === "rendering" && (
          <div className="w-64 text-center text-sm text-white/80">
            <Spinner className="mx-auto mb-3 size-7" aria-label="加载中" />
            {pipeline ? "正在生成预览…" : "正在打开…"}
            {pipeline && percent > 0 && <Progress value={percent} className="mt-3 h-1 bg-white/15" />}
          </div>
        )}
        {status === "error" && (
          <div className="max-w-md px-6 text-center text-sm text-white/80 select-text">
            <TriangleAlert className="mx-auto mb-3 size-7 text-primary" />
            {error}
          </div>
        )}
      </div>
      {pipeline && result && status === "ready" && (
        <div className="space-y-1 px-4 py-2 text-xs text-muted-foreground select-text">
          <p>
            {result.width}×{result.height} · {formatDuration(result.duration)} · 参与预览的步骤：
            {result.modules.length > 0 ? result.modules.map(stepName).join("、") : "无（仅缩小画面）"}
          </p>
          {mode.kind === "pipeline" && mode.moduleId === null && (
            <p>变速、背景音、加头尾、帧率、码率不参与预览；裂变时每次的随机取值不同，可点“换一组随机值”。</p>
          )}
          {result.warnings.map((w) => (
            <p key={w} className="text-warning">
              {w}
            </p>
          ))}
          {mode.kind === "tool" && <p>工具预览处理的是这一小段（低清）：按时间点填写的参数以片段的时间为准。</p>}
        </div>
      )}
      {pipeline && (
        <DialogFooter className="border-t px-4 py-2.5 sm:justify-start">
          <PreviewControls
            start={start}
            duration={duration}
            busy={status === "rendering"}
            onApply={setWindow}
            onRerender={rerender}
          />
        </DialogFooter>
      )}
    </DialogContent>
  );
}

function PreviewControls({
  start,
  duration,
  busy,
  onApply,
  onRerender,
}: {
  start: number;
  duration: number;
  busy: boolean;
  onApply: (start: number, duration: number) => void;
  onRerender: () => void;
}) {
  const [from, setFrom] = useState(start);
  const [length, setLength] = useState(duration);
  useEffect(() => setFrom(start), [start]);
  useEffect(() => setLength(duration), [duration]);

  return (
    <div className="flex w-full flex-wrap items-center gap-2 text-xs text-muted-foreground">
      从
      <NumberInput value={from} onChange={setFrom} min={0} step="any" className="w-20" label="起点（秒）" />
      秒起，时长
      <NumberInput value={length} onChange={setLength} min={0.5} max={60} step="any" className="w-16" label="时长（秒）" />
      秒
      <Button variant="outline" size="xs" onClick={() => onApply(from, length)} disabled={busy} className="h-7 px-2.5">
        应用
      </Button>
      <Button
        variant="outline"
        size="xs"
        onClick={onRerender}
        disabled={busy}
        className="ml-auto h-7 px-2.5"
        title="用新的随机种子重新生成"
      >
        <RefreshCw /> 换一组随机值
      </Button>
    </div>
  );
}
