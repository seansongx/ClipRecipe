// 窗口底部的状态栏：队列数量、批量处理 / 独立工具的运行状态与总进度、FFmpeg 与编码设置。
// 取代了原先散在“开始处理”按钮下方的运行汇总与队列标题旁的数量。

import { CircleAlert, Cpu, Loader } from "lucide-react";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { useEffect } from "react";
import type { EncoderInfo } from "@/lib/backend";
import { describeEncoder, useEncoders } from "@/state/encoders";
import { SOURCE_LABELS, useFfmpeg } from "@/state/ffmpeg";
import { useQueue } from "@/state/queue";
import { useSettings } from "@/state/settings";
import { batchPercent, toolPercent } from "@/state/taskbar";
import { toolName, useTools } from "@/state/tools";

function Divider() {
  return <Separator orientation="vertical" className="mx-2 h-3 data-vertical:self-center" />;
}

/** 批量处理：处理中的计数与总进度，或上次运行的汇总。 */
function BatchStatus() {
  const items = useQueue((s) => s.items);
  const running = useQueue((s) => s.running);
  const lastRun = useQueue((s) => s.lastRun);
  const runIds = useQueue((s) => s.runIds);
  const count = (state: string) => items.filter((i) => i.state === state).length;
  if (running) {
    const percent = batchPercent(runIds, items);
    return (
      <span className="flex items-center gap-2">
        <Loader className="size-3 animate-spin text-primary motion-reduce:animate-none" aria-hidden />
        <span>
          处理中 {count("running")} · 等待 {count("waiting")} · 完成 {count("done")}
          {count("failed") > 0 && ` · 失败 ${count("failed")}`}
        </span>
        <Progress value={percent} aria-label="批量处理总进度" className="w-24" />
        <span className="w-8">{percent.toFixed(0)}%</span>
      </span>
    );
  }
  if (lastRun) {
    return (
      <span className={lastRun.failed > 0 ? "text-destructive" : undefined}>
        上次：完成 {lastRun.done} · 失败 {lastRun.failed} · 取消 {lastRun.cancelled}
      </span>
    );
  }
  return <span>就绪</span>;
}

function ToolStatus() {
  const run = useTools((s) => s.run);
  if (!run?.running) return null;
  const done = run.items.filter((i) => i.state === "done").length;
  return (
    <>
      <Divider />
      <span className="flex items-center gap-2">
        <Loader className="size-3 animate-spin text-primary motion-reduce:animate-none" aria-hidden />
        工具“{toolName(run.toolId)}” {done}/{run.items.length} · {toolPercent(run.items).toFixed(0)}%
      </span>
    </>
  );
}

/**
 * 实际会用的视频编码器：关闭 GPU 加速时就是软件编码；开启时以探测结果为准（没有可用的硬件编码器时也是软件编码），
 * 还没探测过时如实说明。HDR 保留时改用 H.265 等少数情况以处理记录为准。
 */
function encoderText(hevc: boolean, gpu: boolean, info: EncoderInfo | null): string {
  const codec = hevc ? "H.265" : "H.264";
  if (!gpu) return `${codec} · ${describeEncoder(hevc ? "libx265" : "libx264")}`;
  if (!info) return `${codec} · GPU 加速（编码器将在开始处理时检测）`;
  return `${codec} · ${describeEncoder(hevc ? info.hevc : info.h264)}`;
}

function FfmpegStatus() {
  const status = useFfmpeg((s) => s.status);
  const gpu = useSettings((s) => s.settings.gpu);
  const hevc = useSettings((s) => s.settings.hevc);
  const info = useEncoders((s) => s.info);
  const path = status?.available ? status.path : null;
  useEffect(() => {
    // FFmpeg 换了（下载、删除、重新检测）之后，缓存的探测结果也随之改变
    void useEncoders.getState().refresh();
  }, [path]);
  if (!status) return <span>正在检测 FFmpeg…</span>;
  return (
    <>
      {status.available ? (
        <span>
          FFmpeg {/version\s+(\d[\w.]*)/.exec(status.version ?? "")?.[1] ?? ""} · {SOURCE_LABELS[status.source]}
        </span>
      ) : (
        <span className="flex items-center gap-1 text-warning">
          <CircleAlert className="size-3" aria-hidden /> 未找到 FFmpeg
        </span>
      )}
      <Divider />
      <span className="flex items-center gap-1">
        <Cpu className="size-3" aria-hidden />
        {status.available ? encoderText(hevc, gpu, info) : "无法编码"}
      </span>
    </>
  );
}

export default function StatusBar() {
  const items = useQueue((s) => s.items);
  const selected = useQueue((s) => s.selected);
  const importing = useQueue((s) => s.importing);
  return (
    <footer
      id="statusbar"
      aria-label="状态栏"
      className="flex h-6 shrink-0 items-center border-t bg-card px-3 text-xs whitespace-nowrap text-muted-foreground"
    >
      <span>
        队列 {items.length} 个{selected.size > 0 && ` · 选中 ${selected.size}`}
      </span>
      {importing && (
        <>
          <Divider />
          <span>{importing.total > 0 ? `正在导入 ${importing.done} / ${importing.total}` : "正在导入…"}</span>
        </>
      )}
      <Divider />
      <BatchStatus />
      <ToolStatus />
      <span className="ml-auto flex items-center">
        <FfmpegStatus />
      </span>
    </footer>
  );
}
