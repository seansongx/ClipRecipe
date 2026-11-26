// 配方面板步骤行左侧的模块图标（lucide）。新增模块时在这里补上，没有时显示通用图标。

import {
  ChartNoAxesColumn,
  Clapperboard,
  CopyMinus,
  Crop,
  Eraser,
  Film,
  Gauge,
  LayoutGrid,
  Music,
  PictureInPicture2,
  RotateCw,
  Scaling,
  Scissors,
  SlidersHorizontal,
  Stamp,
  StepForward,
  Type,
  Workflow,
  ZoomIn,
  type LucideIcon,
} from "lucide-react";

const ICONS: Record<string, LucideIcon> = {
  "pipe.remove-watermark": Eraser,
  "pipe.add-watermark": Stamp,
  "pipe.pip": PictureInPicture2,
  "pipe.text": Type,
  "pipe.crop": Crop,
  "pipe.resolution": Scaling,
  "pipe.rotate-flip": RotateCw,
  "pipe.adjust": SlidersHorizontal,
  "pipe.grid": LayoutGrid,
  "pipe.zoom": ZoomIn,
  "pipe.trim": Scissors,
  "pipe.intro-outro": Clapperboard,
  "pipe.speed": Gauge,
  "pipe.drop-frames": StepForward,
  "pipe.dedupe-frames": CopyMinus,
  "pipe.bgm": Music,
  "pipe.fps": Film,
  "pipe.bitrate": ChartNoAxesColumn,
};

export function moduleIcon(id: string): LucideIcon {
  return ICONS[id] ?? Workflow;
}
