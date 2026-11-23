// tool.cover 封面处理。后端：src-tauri/src/tools/cover.rs

import { useId } from "react";
import ChoiceGroup from "@/components/ChoiceGroup";
import ParamField from "@/components/ParamField";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { FilePathInput } from "./subtitle-asr";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type CoverMode = "embed" | "capture";
export type CoverSource = "image" | "sameName" | "frame";
export type ImageFormat = "jpg" | "png";

export type CoverParams = {
  mode: CoverMode;
  source: CoverSource;
  /** 指定的封面图片 */
  image: string;
  /** 截取位置：秒、分:秒或百分比 */
  position: string;
  /** 截图文件名样式：{name}、{index} */
  namePattern: string;
  imageFormat: ImageFormat;
  /** 截图后同时嵌入视频 */
  alsoEmbed: boolean;
};

/** 与后端 cover::IMAGE_EXTENSIONS 一致。 */
const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "webp", "bmp"];

function CoverForm({ value, onChange }: ToolFormProps<CoverParams>) {
  const embedId = useId();
  const needsPosition = value.mode === "capture" || value.source === "frame";
  return (
    <>
      <ParamField
        label="操作"
        hint="嵌入后输出“视频名_cover”新视频，画面与声音直接复制、不重编码；mp4 / mov 写入封面流，mkv 写入封面附件。"
      >
        <ChoiceGroup<CoverMode>
          label="封面操作"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "embed", label: "嵌入封面" },
            { value: "capture", label: "截取封面" },
          ]}
        />
      </ParamField>
      {value.mode === "embed" && (
        <ParamField label="封面来源" hint="同名图片：视频旁同名的 jpg / png / webp / bmp，如 clip.mp4 → clip.jpg。">
          <ChoiceGroup<CoverSource>
            label="封面来源"
            value={value.source}
            onChange={(source) => onChange({ source })}
            options={[
              { value: "frame", label: "视频画面" },
              { value: "image", label: "指定图片" },
              { value: "sameName", label: "同名图片" },
            ]}
          />
        </ParamField>
      )}
      {value.mode === "embed" && value.source === "image" && (
        <ParamField label="图片">
          <FilePathInput
            value={value.image}
            onChange={(image) => onChange({ image })}
            label="封面图片"
            kind="图片"
            extensions={IMAGE_EXTENSIONS}
          />
        </ParamField>
      )}
      {needsPosition && (
        <ParamField label="截取位置" hint="秒数（12.5）、分:秒（1:30）或百分比（10%）；超出时长时取最后一帧附近。">
          <Input
            value={value.position}
            onChange={(e) => onChange({ position: e.target.value })}
            placeholder="10%"
            aria-label="截取位置"
            className="h-7 w-28 px-2 text-xs md:text-xs"
          />
        </ParamField>
      )}
      {value.mode === "capture" && (
        <>
          <ParamField label="文件名" hint="{name} 为视频名，{index} 为队列序号；扩展名自动添加。">
            <Input
              value={value.namePattern}
              onChange={(e) => onChange({ namePattern: e.target.value })}
              placeholder="{name}_cover"
              aria-label="截图文件名样式"
              className="h-7 px-2 text-xs md:text-xs"
            />
          </ParamField>
          <ParamField label="格式">
            <ChoiceGroup<ImageFormat>
              label="截图格式"
              value={value.imageFormat}
              onChange={(imageFormat) => onChange({ imageFormat })}
              options={[
                { value: "jpg", label: "JPG" },
                { value: "png", label: "PNG" },
              ]}
            />
          </ParamField>
          <div className="flex items-center gap-1.5 py-1.5">
            <Checkbox
              id={embedId}
              checked={value.alsoEmbed}
              onCheckedChange={(checked) => onChange({ alsoEmbed: checked === true })}
            />
            <Label htmlFor={embedId} className="text-xs font-normal">
              同时把截图嵌入视频作为封面
            </Label>
          </div>
        </>
      )}
    </>
  );
}

export default defineTool<CoverParams>({
  id: "tool.cover",
  defaults: {
    mode: "embed",
    source: "frame",
    image: "",
    position: "10%",
    namePattern: "{name}_cover",
    imageFormat: "jpg",
    alsoEmbed: false,
  },
  inputs: VIDEO_INPUTS,
  Form: CoverForm,
});
