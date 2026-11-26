// pipe.text 文本。后端：src-tauri/src/pipeline/modules/text.rs
// 最多 3 处，每处单独开关。字号为画面高度的百分比，描边 / 行距为字号的百分比，边距为画面宽 / 高的百分比；
// 自选位置为源视频像素（PointPicker，文字块中心）。时间指去头尾 / 变速之后的视频时间。

import { useId, useState, type ReactNode } from "react";
import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import FontSelect from "@/components/FontSelect";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { PointPicker, type Point } from "@/components/RegionPicker";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import type { AssetSource } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export type TextSource = "fileName" | "textFiles" | "custom";
export type TextAlign = "left" | "center" | "right";
export type TextPosition =
  | "topLeft"
  | "top"
  | "topRight"
  | "left"
  | "center"
  | "right"
  | "bottomLeft"
  | "bottom"
  | "bottomRight"
  | "custom";

export interface TextItem {
  enabled: boolean;
  source: TextSource;
  /** 文件名：要删去的子串 */
  strip: string[];
  /** 文件名：在这些字符处分行 */
  lineBreaks: string;
  /** 文本文件：单文件或目录 */
  textFiles: AssetSource;
  /** 自定义文本 */
  customText: string;
  /** 每行最多字符数，0 = 不限 */
  maxLineChars: number;
  /** 字体文件；空 = 自动选择中文字体 */
  fontFile: string;
  /** 字号：画面高度的百分比 */
  fontSize: number;
  /** 超出宽度时自动缩小 */
  fitWidth: boolean;
  color: string;
  /** 描边宽度：字号的百分比 */
  outlineWidth: number;
  outlineColor: string;
  shadow: boolean;
  shadowColor: string;
  background: boolean;
  backgroundColor: string;
  backgroundOpacity: number;
  /** 行距：字号的百分比 */
  lineSpacing: number;
  align: TextAlign;
  position: TextPosition;
  /** 左右边距：画面宽的百分比 */
  marginX: number;
  /** 上下边距：画面高的百分比 */
  marginY: number;
  /** 自选位置（源像素，文字块中心） */
  point: Point | null;
  /** 出现时间（秒） */
  start: number;
  /** 持续时间（秒），0 = 到结尾 */
  duration: number;
}

export interface TextParams {
  enabled: boolean;
  /** 最多 3 处 */
  items: TextItem[];
}

const TEXT_EXTENSIONS = ["txt"];

const POSITIONS: { value: TextPosition; label: string }[] = [
  { value: "topLeft", label: "左上" },
  { value: "top", label: "上方居中" },
  { value: "topRight", label: "右上" },
  { value: "left", label: "左侧居中" },
  { value: "center", label: "正中" },
  { value: "right", label: "右侧居中" },
  { value: "bottomLeft", label: "左下" },
  { value: "bottom", label: "下方居中" },
  { value: "bottomRight", label: "右下" },
  { value: "custom", label: "自选位置" },
];

// ---------------------------------------------------------------------------
// 字体

// ---------------------------------------------------------------------------
// 一处文本

function SwitchWithColor({
  label,
  checked,
  onCheckedChange,
  color,
  onColor,
  children,
}: {
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  color: string;
  onColor: (color: string) => void;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Switch aria-label={label} checked={checked} onCheckedChange={onCheckedChange} />
      {checked && <ColorField label={`${label}颜色`} value={color} onChange={onColor} />}
      {checked && children}
    </div>
  );
}

function ItemForm({ item, onChange }: { item: TextItem; onChange: (patch: Partial<TextItem>) => void }) {
  const fitId = useId();
  const percentInput = (key: "fontSize" | "outlineWidth" | "lineSpacing", label: string, min: number, max: number) => (
    <NumberInput
      className="w-16"
      label={label}
      value={item[key]}
      min={min}
      max={max}
      step={key === "fontSize" ? 0.5 : 1}
      onChange={(v) => onChange({ [key]: v } as Partial<TextItem>)}
    />
  );

  return (
    <>
      <ParamField label="启用">
        <Switch aria-label="启用这一处文本" checked={item.enabled} onCheckedChange={(enabled) => onChange({ enabled })} />
      </ParamField>

      <ParamField label="内容">
        <ChoiceGroup<TextSource>
          label="内容来源"
          value={item.source}
          onChange={(source) => onChange({ source })}
          options={[
            { value: "fileName", label: "文件名" },
            { value: "textFiles", label: "文本文件" },
            { value: "custom", label: "自定义" },
          ]}
        />
      </ParamField>

      {item.source === "fileName" && (
        <>
          <ParamField label="删去" hint="每行一个，文件名中出现的这些文字会被删掉。">
            <Textarea
              value={item.strip.join("\n")}
              onChange={(e) => onChange({ strip: e.target.value.split("\n") })}
              aria-label="要删去的文字"
              placeholder={"如：_高清\n【合集】"}
              rows={2}
              className="min-h-0"
            />
          </ParamField>
          <ParamField label="分行符" hint="在这些字符处换行（字符本身去掉），如 #、|、空格。">
            <Input
              value={item.lineBreaks}
              onChange={(e) => onChange({ lineBreaks: e.target.value })}
              aria-label="分行符"
              placeholder="不分行"
              className="w-28"
            />
          </ParamField>
        </>
      )}
      {item.source === "textFiles" && (
        <ParamField label="文本" hint="UTF-8 编码的 .txt；目录模式下每个视频取一个文件。">
          <AssetSourceInput
            value={item.textFiles}
            onChange={(textFiles) => onChange({ textFiles })}
            extensions={TEXT_EXTENSIONS}
            kindLabel="文本"
          />
        </ParamField>
      )}
      {item.source === "custom" && (
        <ParamField label="文字">
          <Textarea
            value={item.customText}
            onChange={(e) => onChange({ customText: e.target.value })}
            aria-label="自定义文字"
            placeholder="可多行"
            rows={3}
            className="min-h-0"
          />
        </ParamField>
      )}

      <ParamField label="每行字数" hint="0 为不限；超出时按字数硬折行。">
        <NumberInput
          className="w-16"
          label="每行最多字数"
          value={item.maxLineChars}
          min={0}
          max={1000}
          integer
          onChange={(maxLineChars) => onChange({ maxLineChars })}
        />
      </ParamField>

      <ParamField label="字体" hint="默认优先使用微软雅黑等支持中文的字体。">
        <FontSelect value={item.fontFile} onChange={(fontFile) => onChange({ fontFile })} />
      </ParamField>

      <ParamField label="字号" hint="画面高度的百分比（1～30），随分辨率缩放。">
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          {percentInput("fontSize", "字号", 1, 30)}%
          <span className="flex items-center gap-1.5">
            <Checkbox
              id={fitId}
              checked={item.fitWidth}
              onCheckedChange={(checked) => onChange({ fitWidth: checked === true })}
            />
            <Label htmlFor={fitId}>
              太宽时自动缩小
            </Label>
          </span>
        </div>
      </ParamField>

      <ParamField label="颜色">
        <div className="flex">
          <ColorField label="文字颜色" value={item.color} onChange={(color) => onChange({ color })} />
        </div>
      </ParamField>

      <ParamField label="描边" hint="宽度为字号的百分比（0～25），0 为不描边。">
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          {percentInput("outlineWidth", "描边宽度", 0, 25)}%
          {item.outlineWidth > 0 && (
            <ColorField label="描边颜色" value={item.outlineColor} onChange={(outlineColor) => onChange({ outlineColor })} />
          )}
        </div>
      </ParamField>

      <ParamField label="阴影">
        <SwitchWithColor
          label="阴影"
          checked={item.shadow}
          onCheckedChange={(shadow) => onChange({ shadow })}
          color={item.shadowColor}
          onColor={(shadowColor) => onChange({ shadowColor })}
        />
      </ParamField>

      <ParamField label="背景框" hint={item.background ? "右侧为不透明度（0～1）。" : undefined}>
        <SwitchWithColor
          label="背景框"
          checked={item.background}
          onCheckedChange={(background) => onChange({ background })}
          color={item.backgroundColor}
          onColor={(backgroundColor) => onChange({ backgroundColor })}
        >
          <NumberInput
            className="w-14"
            label="背景不透明度"
            value={item.backgroundOpacity}
            min={0}
            max={1}
            step={0.1}
            onChange={(backgroundOpacity) => onChange({ backgroundOpacity })}
          />
        </SwitchWithColor>
      </ParamField>

      <ParamField label="多行" hint="行距为字号的百分比（0～200）。">
        {/* 分段选择的选中项是 muted 底，不能继承 muted 文字色：muted 色只给“行距” */}
        <div className="flex flex-wrap items-center gap-2 text-xs">
          <ChoiceGroup<TextAlign>
            label="对齐"
            value={item.align}
            onChange={(align) => onChange({ align })}
            options={[
              { value: "left", label: "左对齐" },
              { value: "center", label: "居中" },
              { value: "right", label: "右对齐" },
            ]}
          />
          <span className="flex items-center gap-1.5 text-muted-foreground">
            行距 {percentInput("lineSpacing", "行距", 0, 200)}%
          </span>
        </div>
      </ParamField>

      <ParamField
        label="位置"
        hint={
          item.position === "custom"
            ? "所选位置为文字中心（源视频像素），文字不会超出画面。"
            : "边距为画面宽 / 高的百分比（0～45），居中的方向上不使用。"
        }
      >
        <div className="space-y-2">
          <Select items={POSITIONS} value={item.position} onValueChange={(position) => position && onChange({ position })}>
            <SelectTrigger aria-label="位置" className="w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {POSITIONS.map((p) => (
                <SelectItem key={p.value} value={p.value}>
                  {p.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {item.position === "custom" ? (
            <PointPicker value={item.point} onChange={(point) => onChange({ point })} label="选择文字位置" />
          ) : (
            <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
              左右
              <NumberInput
                className="w-14"
                label="左右边距（%）"
                value={item.marginX}
                min={0}
                max={45}
                step={0.5}
                onChange={(marginX) => onChange({ marginX })}
              />
              上下
              <NumberInput
                className="w-14"
                label="上下边距（%）"
                value={item.marginY}
                min={0}
                max={45}
                step={0.5}
                onChange={(marginY) => onChange({ marginY })}
              />
              %
            </div>
          )}
        </div>
      </ParamField>

      <ParamField label="时间" hint="第几秒出现、持续几秒（0 为到结尾）；按去头尾 / 变速之后的视频时间计算。">
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          出现
          <NumberInput
            className="w-14"
            label="出现时间（秒）"
            value={item.start}
            min={0}
            step={0.1}
            onChange={(start) => onChange({ start })}
          />
          持续
          <NumberInput
            className="w-14"
            label="持续时间（秒）"
            value={item.duration}
            min={0}
            step={0.1}
            onChange={(duration) => onChange({ duration })}
          />
          秒
        </div>
      </ParamField>
    </>
  );
}

function TextForm({ value, onChange }: ModuleFormProps<TextParams>) {
  const [tab, setTab] = useState("0");
  const update = (index: number, patch: Partial<TextItem>) =>
    onChange({ items: value.items.map((item, i) => (i === index ? { ...item, ...patch } : item)) });

  return (
    <Tabs value={tab} onValueChange={setTab} className="gap-1">
      <TabsList className="w-full">
        {value.items.map((item, i) => (
          <TabsTrigger key={i} value={String(i)}>
            文本 {i + 1}
            {item.enabled && <span className="size-1.5 rounded-full bg-primary" aria-label="已启用" />}
          </TabsTrigger>
        ))}
      </TabsList>
      {value.items.map((item, i) => (
        <TabsContent key={i} value={String(i)}>
          <ItemForm item={item} onChange={(patch) => update(i, patch)} />
        </TabsContent>
      ))}
    </Tabs>
  );
}

const ITEM_DEFAULTS: TextItem = {
  enabled: false,
  source: "fileName",
  strip: [],
  lineBreaks: "",
  textFiles: { mode: "file", path: "" },
  customText: "",
  maxLineChars: 0,
  fontFile: "",
  fontSize: 5,
  fitWidth: true,
  color: "#ffffff", // token-ignore：视频参数默认值，不是界面颜色
  outlineWidth: 8,
  outlineColor: "#000000", // token-ignore
  shadow: false,
  shadowColor: "#000000",
  background: false,
  backgroundColor: "#000000", // token-ignore
  backgroundOpacity: 0.5,
  lineSpacing: 20,
  align: "center",
  position: "bottom",
  marginX: 5,
  marginY: 8,
  point: null,
  start: 0,
  duration: 0,
};

export default defineModule<TextParams>({
  id: "pipe.text",
  defaults: {
    enabled: false,
    items: [
      { ...ITEM_DEFAULTS, enabled: true },
      { ...ITEM_DEFAULTS, source: "custom", position: "top", marginY: 6 },
      { ...ITEM_DEFAULTS, source: "custom", position: "center" },
    ],
  },
  Form: TextForm,
});
