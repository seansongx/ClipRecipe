// tool.image-to-video 图转视频。后端：src-tauri/src/tools/image_to_video.rs
// 文件夹里的每张图片生成一个同名 mp4：分辨率与放置方式、帧率、时长、入场动画、静音音轨。

import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { defineTool, type ToolFormProps } from "./types";

export type ImageFit = "fit" | "fill" | "stretch";
export type ImageAnimation = "none" | "fadeIn" | "zoomIn" | "slideIn" | "kenBurns";

export type ImageToVideoParams = {
  width: number;
  height: number;
  fit: ImageFit;
  /** 背景色 #rrggbb */
  background: string;
  fps: number;
  /** 时长（秒） */
  duration: number;
  animation: ImageAnimation;
  /** 入场动画时长（秒） */
  animationSeconds: number;
  silentAudio: boolean;
};

const ANIMATIONS: { value: ImageAnimation; label: string }[] = [
  { value: "none", label: "无" },
  { value: "fadeIn", label: "淡入" },
  { value: "zoomIn", label: "放大入场" },
  { value: "slideIn", label: "从左滑入" },
  { value: "kenBurns", label: "缓慢推拉" },
];

const PRESETS: { label: string; width: number; height: number }[] = [
  { label: "竖屏", width: 1080, height: 1920 },
  { label: "横屏", width: 1920, height: 1080 },
  { label: "方形", width: 1080, height: 1080 },
];

function ImageToVideoForm({ value, onChange }: ToolFormProps<ImageToVideoParams>) {
  const preset = PRESETS.find((p) => p.width === value.width && p.height === value.height);
  return (
    <>
      <ParamField label="分辨率" hint="奇数会减 1 取偶。">
        <div className="flex flex-col gap-1.5">
          <ChoiceGroup<string>
            label="常用分辨率"
            value={preset?.label ?? "自定义"}
            onChange={(label) => {
              const p = PRESETS.find((x) => x.label === label);
              if (p) onChange({ width: p.width, height: p.height });
            }}
            options={[...PRESETS.map((p) => ({ value: p.label, label: p.label })), { value: "自定义", label: "自定义" }]}
          />
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="宽度"
              value={value.width}
              onChange={(width) => onChange({ width })}
              min={16}
              max={7680}
              integer
            />
            <span className="text-xs text-muted-foreground">×</span>
            <NumberInput
              label="高度"
              value={value.height}
              onChange={(height) => onChange({ height })}
              min={16}
              max={7680}
              integer
            />
          </div>
        </div>
      </ParamField>
      <ParamField label="放置" hint="完整放入时空白处为背景色；透明图片的透明部分也显示背景色。">
        <ChoiceGroup<ImageFit>
          label="图片放置方式"
          value={value.fit}
          onChange={(fit) => onChange({ fit })}
          options={[
            { value: "fit", label: "完整放入" },
            { value: "fill", label: "铺满裁切" },
            { value: "stretch", label: "拉伸" },
          ]}
        />
      </ParamField>
      <ParamField label="背景色">
        <ColorField label="背景色" value={value.background} onChange={(background) => onChange({ background })} />
      </ParamField>
      <ParamField label="帧率">
        <div className="flex items-center gap-1.5">
          <NumberInput label="帧率" value={value.fps} onChange={(fps) => onChange({ fps })} min={1} max={120} step={1} />
          <span className="text-xs text-muted-foreground">fps</span>
        </div>
      </ParamField>
      <ParamField label="时长">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="时长"
            value={value.duration}
            onChange={(duration) => onChange({ duration })}
            min={0.1}
            max={3600}
            step="any"
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
      <ParamField
        label="动画"
        hint="缓慢推拉：整段时长内缓慢放大并从左向右平移，不使用入场时长。"
      >
        <Select
          items={ANIMATIONS}
          value={value.animation}
          onValueChange={(animation) => animation && onChange({ animation })}
        >
          <SelectTrigger aria-label="入场动画" className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {ANIMATIONS.map((o) => (
              <SelectItem key={o.value} value={o.value}>
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      {value.animation !== "none" && value.animation !== "kenBurns" && (
        <ParamField label="入场时长" hint="超过总时长时按总时长。">
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="入场时长"
              value={value.animationSeconds}
              onChange={(animationSeconds) => onChange({ animationSeconds })}
              min={0.1}
              max={3600}
              step={0.1}
            />
            <span className="text-xs text-muted-foreground">秒</span>
          </div>
        </ParamField>
      )}
      <ParamField label="静音音轨" hint="加一条静音音轨，部分平台要求视频带音轨。">
        <Switch
          aria-label="静音音轨"
          checked={value.silentAudio}
          onCheckedChange={(silentAudio) => onChange({ silentAudio })}
        />
      </ParamField>
    </>
  );
}

export default defineTool<ImageToVideoParams>({
  id: "tool.image-to-video",
  defaults: {
    width: 1080,
    height: 1920,
    fit: "fit",
    background: "#000000",
    fps: 30,
    duration: 5,
    animation: "none",
    animationSeconds: 1,
    silentAudio: true,
  },
  inputs: { videos: false, folders: [{ key: "images", label: "图片文件夹" }] },
  Form: ImageToVideoForm,
});
