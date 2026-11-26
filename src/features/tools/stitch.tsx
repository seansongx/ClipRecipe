// tool.stitch 画面拼接。后端：src-tauri/src/tools/stitch.rs
// 文件夹里的视频按顺序每 N 个一组拼成分屏：同时播放、依次播放或（四宫格）依次填充播放；音频可混合 / 只用第一个 / 跟随当前播放。

import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { defineTool, type ToolFormProps } from "./types";

export type StitchLayout = "sideBySide" | "stacked" | "row3" | "column3" | "grid4";
export type StitchPlayback = "together" | "sequential" | "fillTurns";
export type StitchAudio = "mix" | "first" | "current";
export type StitchSize = "auto" | "custom";
export type CellFit = "fit" | "fill" | "stretch";

export type StitchParams = {
  layout: StitchLayout;
  playback: StitchPlayback;
  /** 依次填充播放：每次回到四宫格停留的秒数 */
  gridSeconds: number;
  audio: StitchAudio;
  size: StitchSize;
  /** 自定义整幅画面宽高 */
  width: number;
  height: number;
  cellFit: CellFit;
  background: string;
  fps: number;
};

const LAYOUTS: { value: StitchLayout; label: string }[] = [
  { value: "sideBySide", label: "左右 2 屏" },
  { value: "stacked", label: "上下 2 屏" },
  { value: "row3", label: "横排 3 屏" },
  { value: "column3", label: "竖排 3 屏" },
  { value: "grid4", label: "四宫格" },
];

const PLAYBACK_HINTS: Record<StitchPlayback, string> = {
  together: "全部同时播放，总时长为最长的视频，先播完的停在最后一帧。",
  sequential: "上一个播完下一个才开始；未轮到的停在第一帧，播完的停在最后一帧。总时长为各视频之和。",
  fillTurns:
    "先显示静止的四宫格，然后每个视频依次放大到整个画面播放，播完回到四宫格（该格停在最后一帧）再轮到下一个。",
};

function StitchForm({ value, onChange }: ToolFormProps<StitchParams>) {
  const sequential = value.playback !== "together";
  const setLayout = (layout: StitchLayout) =>
    onChange(
      layout !== "grid4" && value.playback === "fillTurns" ? { layout, playback: "sequential" } : { layout },
    );
  const setPlayback = (playback: StitchPlayback) =>
    onChange(playback === "together" && value.audio === "current" ? { playback, audio: "mix" } : { playback });

  const playbackOptions: { value: StitchPlayback; label: string }[] = [
    { value: "together", label: "同时" },
    { value: "sequential", label: "依次" },
  ];
  if (value.layout === "grid4") playbackOptions.push({ value: "fillTurns", label: "依次填充" });

  const audioOptions: { value: StitchAudio; label: string }[] = [
    { value: "mix", label: "全部混合" },
    { value: "first", label: "第一个" },
  ];
  if (sequential) audioOptions.push({ value: "current", label: "当前播放" });

  return (
    <>
      <ParamField label="布局" hint="文件按名称排序后每 N 个一组，最后一组不足时空位填充背景色。">
        <Select items={LAYOUTS} value={value.layout} onValueChange={(layout) => layout && setLayout(layout)}>
          <SelectTrigger aria-label="拼接布局" className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {LAYOUTS.map((o) => (
              <SelectItem key={o.value} value={o.value}>
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      <ParamField label="播放" hint={PLAYBACK_HINTS[value.playback]}>
        <ChoiceGroup<StitchPlayback>
          label="播放方式"
          value={value.playback}
          onChange={setPlayback}
          options={playbackOptions}
        />
      </ParamField>
      {value.playback === "fillTurns" && (
        <ParamField label="宫格停留" hint="每次回到四宫格停留的时间。">
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="宫格停留"
              value={value.gridSeconds}
              onChange={(gridSeconds) => onChange({ gridSeconds })}
              min={0}
              max={10}
              step={0.1}
            />
            <span className="text-xs text-muted-foreground">秒</span>
          </div>
        </ParamField>
      )}
      <ParamField
        label="音频"
        hint={
          value.audio === "current"
            ? "只播放正在播放的那个视频的声音。"
            : "全部混合：所有视频的声音从头同时播放。"
        }
      >
        <ChoiceGroup<StitchAudio>
          label="音频"
          value={value.audio}
          onChange={(audio) => onChange({ audio })}
          options={audioOptions}
        />
      </ParamField>
      <ParamField
        label="画面尺寸"
        hint={
          value.size === "auto"
            ? "每格为第一个视频的尺寸，整幅画面最长边超过 1920 时等比缩小。"
            : "整幅画面的宽高，每格均分。"
        }
      >
        <ChoiceGroup<StitchSize>
          label="画面尺寸"
          value={value.size}
          onChange={(size) => onChange({ size })}
          options={[
            { value: "auto", label: "自动" },
            { value: "custom", label: "自定义" },
          ]}
        />
      </ParamField>
      {value.size === "custom" && (
        <ParamField label="宽 × 高">
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
        </ParamField>
      )}
      <ParamField label="格内画面">
        <ChoiceGroup<CellFit>
          label="格内画面"
          value={value.cellFit}
          onChange={(cellFit) => onChange({ cellFit })}
          options={[
            { value: "fill", label: "铺满裁切" },
            { value: "fit", label: "完整放入" },
            { value: "stretch", label: "拉伸" },
          ]}
        />
      </ParamField>
      <ParamField label="背景色" hint="完整放入时的空白处与空位的颜色。">
        <ColorField label="背景色" value={value.background} onChange={(background) => onChange({ background })} />
      </ParamField>
      <ParamField label="帧率">
        <div className="flex items-center gap-1.5">
          <NumberInput label="帧率" value={value.fps} onChange={(fps) => onChange({ fps })} min={1} max={120} step={1} />
          <span className="text-xs text-muted-foreground">fps</span>
        </div>
      </ParamField>
    </>
  );
}

export default defineTool<StitchParams>({
  id: "tool.stitch",
  defaults: {
    layout: "sideBySide",
    playback: "together",
    gridSeconds: 1,
    audio: "mix",
    size: "auto",
    width: 1920,
    height: 1080,
    cellFit: "fill",
    background: "#000000", // token-ignore：视频参数默认值，不是界面颜色
    fps: 30,
  },
  inputs: { videos: false, folders: [{ key: "source", label: "视频文件夹" }] },
  Form: StitchForm,
});
