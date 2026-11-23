// 合并引擎的公共参数与表单（后端 tools/merge.rs 的 MergeOptions；视频合并、文件夹合并共用）。

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { VIDEO_EXTENSIONS, type AssetSource } from "@/lib/backend";
import { WithUnit } from "./with-unit";

export type MergeMode = "lossless" | "reencode";

/** 合并引擎的公共参数（与后端 MergeOptions 一致，平铺在各工具参数里）。 */
export type MergeOptions = {
  mode: MergeMode;
  /** 无损不兼容时自动改为重编码 */
  fallbackReencode: boolean;
  /** none / random / 转场名 */
  transition: string;
  transitionSeconds: number;
  middleEnabled: boolean;
  middle: AssetSource;
  /** 输出名为各源文件名以空格连接 */
  joinNames: boolean;
};

export const MERGE_OPTION_DEFAULTS: MergeOptions = {
  mode: "lossless",
  fallbackReencode: true,
  transition: "none",
  transitionSeconds: 1,
  middleEnabled: false,
  middle: { mode: "file", path: "" },
  joinNames: false,
};

/** 与后端 TRANSITIONS 一致。 */
const TRANSITIONS: { value: string; label: string }[] = [
  { value: "none", label: "无转场" },
  { value: "random", label: "随机" },
  { value: "fade", label: "淡入淡出" },
  { value: "fadeblack", label: "黑场过渡" },
  { value: "fadewhite", label: "白场过渡" },
  { value: "dissolve", label: "溶解" },
  { value: "wipeleft", label: "向左擦除" },
  { value: "wiperight", label: "向右擦除" },
  { value: "wipeup", label: "向上擦除" },
  { value: "wipedown", label: "向下擦除" },
  { value: "slideleft", label: "向左滑动" },
  { value: "slideright", label: "向右滑动" },
  { value: "slideup", label: "向上滑动" },
  { value: "slidedown", label: "向下滑动" },
  { value: "circleopen", label: "圆形展开" },
  { value: "circleclose", label: "圆形收拢" },
  { value: "radial", label: "径向扫过" },
  { value: "pixelize", label: "像素化" },
  { value: "smoothleft", label: "平滑左移" },
  { value: "smoothright", label: "平滑右移" },
  { value: "zoomin", label: "放大" },
  { value: "hblur", label: "水平模糊" },
];

/** 合并方式、转场、中间视频、输出名（tool.merge 与 tool.folder-merge 共用）。 */
export function MergeOptionsFields({
  value,
  onChange,
  defaultName = "第一个视频名 + “_merged”",
}: {
  value: MergeOptions;
  onChange: (patch: Partial<MergeOptions>) => void;
  /** 不连接文件名时的输出名说明 */
  defaultName?: string;
}) {
  const reencode = value.mode === "reencode";
  return (
    <>
      <ParamField
        label="方式"
        hint={
          reencode
            ? "统一到第一个视频的尺寸、帧率与音频格式后重新编码，可加转场。"
            : "直接复制、速度快、不损失画质；要求各视频编码、分辨率、帧率、音频格式相同。"
        }
      >
        <ChoiceGroup<MergeMode>
          label="合并方式"
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "lossless", label: "无损合并" },
            { value: "reencode", label: "重编码" },
          ]}
        />
      </ParamField>
      {!reencode && (
        <ParamField label="不兼容时" hint="开启：自动改为重编码；关闭：报错并说明原因。">
          <Switch
            aria-label="不兼容时自动重编码"
            checked={value.fallbackReencode}
            onCheckedChange={(fallbackReencode) => onChange({ fallbackReencode })}
          />
        </ParamField>
      )}
      {reencode && (
        <ParamField label="转场" hint="视频之间的过渡效果，音频同步交叉淡化；每处转场使总时长减少一个转场时长。">
          <Select value={value.transition} onValueChange={(transition) => onChange({ transition })}>
            <SelectTrigger aria-label="转场" className="w-32">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {TRANSITIONS.map((t) => (
                <SelectItem key={t.value} value={t.value}>
                  {t.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </ParamField>
      )}
      {reencode && value.transition !== "none" && (
        <ParamField label="转场时长">
          <WithUnit unit="秒">
            <NumberInput
              label="转场时长"
              value={value.transitionSeconds}
              onChange={(transitionSeconds) => onChange({ transitionSeconds })}
              min={0.1}
              max={10}
              step={0.1}
            />
          </WithUnit>
        </ParamField>
      )}
      <Separator className="my-1" />
      <ParamField label="中间视频" hint="插入在每两个相邻视频之间；目录时每处各取一个。">
        <Switch
          aria-label="插入中间视频"
          checked={value.middleEnabled}
          onCheckedChange={(middleEnabled) => onChange({ middleEnabled })}
        />
      </ParamField>
      {value.middleEnabled && (
        <AssetSourceInput
          value={value.middle}
          onChange={(middle) => onChange({ middle })}
          extensions={VIDEO_EXTENSIONS}
          kindLabel="中间视频"
        />
      )}
      <ParamField label="输出名" hint={`开启：各源文件名以空格连接作为输出名；关闭：${defaultName}。`}>
        <Switch
          aria-label="以空格连接文件名作为输出名"
          checked={value.joinNames}
          onCheckedChange={(joinNames) => onChange({ joinNames })}
        />
      </ParamField>
    </>
  );
}
