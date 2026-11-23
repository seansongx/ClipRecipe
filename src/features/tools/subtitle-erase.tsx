// tool.subtitle-erase 字幕擦除（本地近似处理）。后端：src-tauri/src/tools/subtitle_erase.rs

import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import RegionPicker, { type Rect } from "@/components/RegionPicker";
import { Textarea } from "@/components/ui/textarea";
import { WithUnit } from "./merge";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type EraseArea = "bottom" | "regions";
export type EraseMode = "delogo" | "blur" | "fill";

export type SubtitleEraseParams = {
  area: EraseArea;
  /** 底部字幕条上沿（画面高度的百分比） */
  bandTop: number;
  /** 底部字幕条下沿（画面高度的百分比） */
  bandBottom: number;
  /** 自定义区域（源视频像素） */
  regions: Rect[];
  mode: EraseMode;
  /** 模糊强度（gblur sigma） */
  blurStrength: number;
  fillColor: string;
  /** 只在这些时段擦除，空为整段 */
  ranges: string;
};

/** 与后端 MAX_REGIONS 一致。 */
const MAX_REGIONS = 4;

function SubtitleEraseForm({ value, onChange }: ToolFormProps<SubtitleEraseParams>) {
  return (
    <>
      <p className="pb-1 text-xs leading-snug text-muted-foreground">
        本地近似处理：用周围像素填补、模糊或纯色遮盖所选区域，不调用任何云服务。背景简单时效果较好，
        复杂画面会留下涂抹痕迹。不会自动识别字幕位置，请用预设字幕条或手动框选。
      </p>
      <ParamField label="区域">
        <ChoiceGroup<EraseArea>
          label="擦除区域"
          value={value.area}
          onChange={(area) => onChange({ area })}
          options={[
            { value: "bottom", label: "底部字幕条" },
            { value: "regions", label: "框选" },
          ]}
        />
      </ParamField>
      {value.area === "bottom" ? (
        <>
          <ParamField label="上沿" hint="字幕条占整个宽度，从画面高度的这个位置开始。">
            <WithUnit unit="%">
              <NumberInput
                label="字幕条上沿"
                value={value.bandTop}
                onChange={(bandTop) => onChange({ bandTop })}
                min={0}
                max={99}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="下沿">
            <WithUnit unit="%">
              <NumberInput
                label="字幕条下沿"
                value={value.bandBottom}
                onChange={(bandBottom) => onChange({ bandBottom })}
                min={1}
                max={100}
                step="any"
              />
            </WithUnit>
          </ParamField>
        </>
      ) : (
        <ParamField label="框选" hint={`在当前视频的画面上框出字幕位置，最多 ${MAX_REGIONS} 个；坐标按源视频像素。`}>
          <RegionPicker
            value={value.regions}
            onChange={(regions) => onChange({ regions })}
            max={MAX_REGIONS}
            label={value.regions.length > 0 ? `已框选 ${value.regions.length} 处` : "框选字幕区域"}
          />
        </ParamField>
      )}
      <ParamField
        label="方式"
        hint="修补：用区域边缘的颜色向内填补（delogo），适合背景平缓的画面；模糊：把文字糊成色块；纯色：用颜色盖住。"
      >
        <ChoiceGroup<EraseMode>
          label="擦除方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "delogo", label: "修补" },
            { value: "blur", label: "模糊" },
            { value: "fill", label: "纯色" },
          ]}
        />
      </ParamField>
      {value.mode === "blur" && (
        <ParamField label="模糊强度">
          <NumberInput
            label="模糊强度"
            value={value.blurStrength}
            onChange={(blurStrength) => onChange({ blurStrength })}
            min={1}
            max={100}
            step="any"
          />
        </ParamField>
      )}
      {value.mode === "fill" && (
        <ParamField label="颜色">
          <ColorField label="遮盖颜色" value={value.fillColor} onChange={(fillColor) => onChange({ fillColor })} />
        </ParamField>
      )}
      <ParamField label="时段" hint="只在这些时段擦除，每段“开始-结束”（秒或分:秒），用逗号或换行分隔；留空为整段视频。">
        <Textarea
          value={value.ranges}
          onChange={(e) => onChange({ ranges: e.target.value })}
          placeholder={"如 0-12\n1:05-1:30"}
          aria-label="擦除时段"
          rows={2}
          className="min-h-12"
        />
      </ParamField>
    </>
  );
}

export default defineTool<SubtitleEraseParams>({
  id: "tool.subtitle-erase",
  defaults: {
    area: "bottom",
    bandTop: 78,
    bandBottom: 94,
    regions: [],
    mode: "delogo",
    blurStrength: 20,
    fillColor: "#000000",
    ranges: "",
  },
  inputs: VIDEO_INPUTS,
  Form: SubtitleEraseForm,
});
