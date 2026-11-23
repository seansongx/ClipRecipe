// tool.screenshots 批量截图。后端：src-tauri/src/tools/screenshots.rs
// 按规则给每个视频截图（每隔 N 秒 / 平均 N 张 / 指定时间 / 一张），放进以视频名命名的文件夹；可把文件名画在图片上。

import type { ReactNode } from "react";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type ShotMode = "interval" | "count" | "timestamps" | "single";
export type ShotFormat = "jpg" | "png";
export type ShotNaming = "index" | "time";
export type NamePosition = "topLeft" | "top" | "topRight" | "center" | "bottomLeft" | "bottom" | "bottomRight";

export type ScreenshotsParams = {
  mode: ShotMode;
  intervalSeconds: number;
  count: number;
  /** 指定时间：逗号 / 空格 / 换行分隔 */
  timestamps: string;
  /** 一张：位置（时长的 %） */
  position: number;
  format: ShotFormat;
  /** jpg 质量 1～100 */
  quality: number;
  /** 缩放宽度，0 = 原尺寸 */
  width: number;
  naming: ShotNaming;
  drawName: boolean;
  /** 字号（图片高度 %） */
  nameSize: number;
  nameColor: string;
  /** 描边宽度（字号 %） */
  nameOutline: number;
  nameOutlineColor: string;
  namePosition: NamePosition;
};

const MODES: { value: ShotMode; label: string }[] = [
  { value: "interval", label: "每隔 N 秒" },
  { value: "count", label: "平均 N 张" },
  { value: "timestamps", label: "指定时间" },
  { value: "single", label: "一张" },
];

const POSITIONS: { value: NamePosition; label: string }[] = [
  { value: "topLeft", label: "左上" },
  { value: "top", label: "上方居中" },
  { value: "topRight", label: "右上" },
  { value: "center", label: "正中" },
  { value: "bottomLeft", label: "左下" },
  { value: "bottom", label: "下方居中" },
  { value: "bottomRight", label: "右下" },
];

function WithUnit({ unit, children }: { unit: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-1.5">
      {children}
      <span className="text-xs text-muted-foreground">{unit}</span>
    </div>
  );
}

function ScreenshotsForm({ value, onChange }: ToolFormProps<ScreenshotsParams>) {
  return (
    <>
      <ParamField label="截图方式">
        <Select value={value.mode} onValueChange={(mode) => onChange({ mode: mode as ShotMode })}>
          <SelectTrigger size="sm" aria-label="截图方式" className="h-7 w-32 px-2 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MODES.map((o) => (
              <SelectItem key={o.value} value={o.value} className="text-xs">
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </ParamField>
      {value.mode === "interval" && (
        <ParamField label="间隔" hint="从 0 秒开始，每隔这么久截一张。">
          <WithUnit unit="秒">
            <NumberInput
              label="截图间隔"
              value={value.intervalSeconds}
              onChange={(intervalSeconds) => onChange({ intervalSeconds })}
              min={0.1}
              max={86400}
              step="any"
            />
          </WithUnit>
        </ParamField>
      )}
      {value.mode === "count" && (
        <ParamField label="张数" hint="把视频等分成 N 段，截每段的正中间。">
          <NumberInput
            label="截图张数"
            value={value.count}
            onChange={(count) => onChange({ count })}
            min={1}
            max={1000}
            integer
          />
        </ParamField>
      )}
      {value.mode === "timestamps" && (
        <ParamField label="时间" hint="逗号、空格或换行分隔；可写秒数、分:秒、时:分:秒。超出时长的时间点会跳过。">
          <Textarea
            value={value.timestamps}
            onChange={(e) => onChange({ timestamps: e.target.value })}
            aria-label="截图时间"
            placeholder={"1.5, 30, 1:20\n0:02:03.5"}
            className="min-h-16 text-xs md:text-xs"
          />
        </ParamField>
      )}
      {value.mode === "single" && (
        <ParamField label="位置" hint="在视频时长的这个百分比处截一张。">
          <WithUnit unit="%">
            <NumberInput
              label="截图位置"
              value={value.position}
              onChange={(position) => onChange({ position })}
              min={0}
              max={100}
              step={1}
            />
          </WithUnit>
        </ParamField>
      )}
      <ParamField label="格式">
        <ChoiceGroup<ShotFormat>
          label="图片格式"
          value={value.format}
          onChange={(format) => onChange({ format })}
          options={[
            { value: "jpg", label: "JPG" },
            { value: "png", label: "PNG" },
          ]}
        />
      </ParamField>
      {value.format === "jpg" && (
        <ParamField label="质量" hint="1～100，越高越清晰、文件越大。">
          <NumberInput
            label="图片质量"
            value={value.quality}
            onChange={(quality) => onChange({ quality })}
            min={1}
            max={100}
            integer
          />
        </ParamField>
      )}
      <ParamField label="宽度" hint="高度按比例；0 表示原尺寸。">
        <WithUnit unit="像素">
          <NumberInput
            label="缩放宽度"
            value={value.width}
            onChange={(width) => onChange({ width })}
            min={0}
            max={7680}
            integer
          />
        </WithUnit>
      </ParamField>
      <ParamField label="命名" hint="图片放在以视频名命名的文件夹里。">
        <ChoiceGroup<ShotNaming>
          label="命名方式"
          value={value.naming}
          onChange={(naming) => onChange({ naming })}
          options={[
            { value: "index", label: "序号" },
            { value: "time", label: "时间" },
          ]}
        />
      </ParamField>
      <ParamField label="绘制文件名" hint="把视频文件名（不含扩展名）画在图片上，支持中文。">
        <Switch aria-label="绘制文件名" checked={value.drawName} onCheckedChange={(drawName) => onChange({ drawName })} />
      </ParamField>
      {value.drawName && (
        <>
          <ParamField label="位置">
            <Select
              value={value.namePosition}
              onValueChange={(namePosition) => onChange({ namePosition: namePosition as NamePosition })}
            >
              <SelectTrigger size="sm" aria-label="文件名位置" className="h-7 w-28 px-2 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {POSITIONS.map((o) => (
                  <SelectItem key={o.value} value={o.value} className="text-xs">
                    {o.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </ParamField>
          <ParamField label="字号" hint="图片高度的百分比；文字太长时自动缩小。">
            <WithUnit unit="%">
              <NumberInput
                label="文件名字号"
                value={value.nameSize}
                onChange={(nameSize) => onChange({ nameSize })}
                min={1}
                max={30}
                step={0.5}
              />
            </WithUnit>
          </ParamField>
          <ParamField label="文字颜色">
            <ColorField label="文字颜色" value={value.nameColor} onChange={(nameColor) => onChange({ nameColor })} />
          </ParamField>
          <ParamField label="描边" hint="描边宽度为字号的百分比，0 为不描边。">
            <div className="flex items-center gap-1.5">
              <ColorField
                label="描边颜色"
                value={value.nameOutlineColor}
                onChange={(nameOutlineColor) => onChange({ nameOutlineColor })}
              />
              <WithUnit unit="%">
                <NumberInput
                  label="描边宽度"
                  value={value.nameOutline}
                  onChange={(nameOutline) => onChange({ nameOutline })}
                  min={0}
                  max={25}
                  step={1}
                />
              </WithUnit>
            </div>
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineTool<ScreenshotsParams>({
  id: "tool.screenshots",
  defaults: {
    mode: "interval",
    intervalSeconds: 10,
    count: 9,
    timestamps: "",
    position: 50,
    format: "jpg",
    quality: 90,
    width: 0,
    naming: "index",
    drawName: false,
    nameSize: 5,
    nameColor: "#ffffff",
    nameOutline: 8,
    nameOutlineColor: "#000000",
    namePosition: "bottom",
  },
  inputs: VIDEO_INPUTS,
  Form: ScreenshotsForm,
});
