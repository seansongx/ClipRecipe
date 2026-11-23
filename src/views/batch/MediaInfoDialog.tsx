import type { ReactNode } from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { formatBitrate, formatDuration, formatFps, formatSize } from "@/lib/format";
import type { QueueItem } from "@/state/queue";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex gap-3 py-1">
      <span className="w-24 shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 flex-1 break-all">{children || "—"}</span>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b py-2 first:pt-0 last:border-b-0 last:pb-0">
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">{title}</h3>
      {children}
    </section>
  );
}

/** 队列视频的媒体信息（queue.manage “查看媒体信息”）。 */
export default function MediaInfoDialog({ item, onClose }: { item: QueueItem; onClose: () => void }) {
  const info = item.info;
  const v = info?.video;
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent aria-describedby={undefined} className="overflow-hidden sm:max-w-xl">
        <DialogHeader>
          <DialogTitle className="truncate">媒体信息 · {item.name}</DialogTitle>
        </DialogHeader>
        <ScrollArea className="[&>[data-slot=scroll-area-viewport]]:max-h-[70vh]">
          <div className="text-xs select-text">
            <Section title="文件">
              <Row label="路径">{item.path}</Row>
              {info && (
                <>
                  <Row label="格式">{info.formatName}</Row>
                  <Row label="时长">{`${formatDuration(info.duration)}（${info.duration.toFixed(3)} 秒）`}</Row>
                  <Row label="大小">{formatSize(info.size)}</Row>
                  <Row label="总码率">{formatBitrate(info.bitRate)}</Row>
                  <Row label="封面">{info.hasCover ? "有" : "无"}</Row>
                </>
              )}
              {item.probeError && <Row label="读取失败">{item.probeError}</Row>}
            </Section>
            {v && (
              <Section title="视频">
                <Row label="编码">{`${v.codec}（${v.pixFmt}）`}</Row>
                <Row label="分辨率">
                  {`${v.width}×${v.height}`}
                  {v.rotation ? `，播放时旋转 ${v.rotation}°` : ""}
                </Row>
                <Row label="帧率">{`${formatFps(v.fps)}（${v.fpsText}）`}</Row>
                <Row label="码率">{formatBitrate(v.bitRate)}</Row>
                <Row label="帧数">{v.frames ?? ""}</Row>
              </Section>
            )}
            {info?.audio.map((a, i) => (
              <Section key={a.index} title={`音轨 ${i + 1}`}>
                <Row label="编码">{a.codec}</Row>
                <Row label="声道 / 采样率">{`${a.channels} 声道 · ${a.sampleRate} Hz`}</Row>
                <Row label="码率">{formatBitrate(a.bitRate)}</Row>
                {(a.language || a.title) && (
                  <Row label="语言 / 标题">{[a.language, a.title].filter(Boolean).join(" · ")}</Row>
                )}
              </Section>
            ))}
            {info?.subtitles.map((s, i) => (
              <Section key={s.index} title={`字幕轨 ${i + 1}`}>
                <Row label="编码">{s.codec}</Row>
                {(s.language || s.title) && (
                  <Row label="语言 / 标题">{[s.language, s.title].filter(Boolean).join(" · ")}</Row>
                )}
              </Section>
            ))}
            {item.outputs.length > 0 && (
              <Section title={`输出（${item.outputs.length} 个）`}>
                {item.outputs.map((o) => (
                  <p key={o} className="py-0.5 break-all">
                    {o}
                  </p>
                ))}
              </Section>
            )}
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  );
}
