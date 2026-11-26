import { useRef } from "react";
import { FfmpegMissingBanner } from "@/components/FfmpegDownload";
import LogPanel from "./batch/LogPanel";
import PreviewStage from "./batch/PreviewStage";
import QueuePanel from "./batch/QueuePanel";
import RecipePanel from "./batch/RecipePanel";
import { RemoveConfirmDialog } from "./batch/RemoveConfirm";
import { useWindowDrop } from "./batch/useWindowDrop";

/** 批量处理页：左栏队列、中栏效果预览与处理记录、右栏配方（含输出位置与开始按钮）。 */
export default function BatchView() {
  const outputBox = useRef<HTMLDivElement>(null);
  const hover = useWindowDrop(outputBox);

  return (
    <div className="flex h-full flex-col">
      <FfmpegMissingBanner />
      <div className="flex min-h-0 flex-1">
        <QueuePanel hover={hover} />
        <section className="flex min-w-0 flex-1 flex-col">
          <PreviewStage />
          <LogPanel />
        </section>
        <RecipePanel outputBox={outputBox} outputDropTarget={hover?.target === "output"} />
      </div>
      <RemoveConfirmDialog />
    </div>
  );
}
