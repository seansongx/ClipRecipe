// tool.freeze-play 定格播放。后端：src-tauri/src/tools/freeze_play.rs
// 在指定位置暂停一段时间后继续播放；暂停期间静音，可在画面中央显示暂停图标（内置或自选图片）。

import AssetSourceInput from "@/components/AssetSourceInput";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import type { AssetSource } from "@/lib/backend";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

export type FreezePlayParams = {
  /** 暂停位置：秒、分:秒或百分比，逗号 / 空格分隔 */
  positions: string;
  /** 每次暂停的秒数 */
  pauseSeconds: number;
  /** 暂停期间显示图标 */
  showIcon: boolean;
  /** 图标图片；为空时用内置暂停图标 */
  icon: AssetSource;
  /** 图标大小：画面短边的百分比 */
  iconSize: number;
};

function FreezePlayForm({ value, onChange }: ToolFormProps<FreezePlayParams>) {
  return (
    <>
      <ParamField
        label="暂停位置"
        hint="多个位置用逗号或空格分隔；可写秒数（12.5）、分:秒（1:05）或百分比（50%）。画面停在该位置的帧上。"
      >
        <Input
          value={value.positions}
          onChange={(e) => onChange({ positions: e.target.value })}
          aria-label="暂停位置"
          placeholder="如 3, 50%, 1:20"
          className="h-7 w-40 text-xs md:text-xs"
        />
      </ParamField>
      <ParamField label="暂停时长" hint="按帧取整；暂停期间静音，视频相应变长。">
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="暂停时长"
            value={value.pauseSeconds}
            onChange={(pauseSeconds) => onChange({ pauseSeconds })}
            min={0.01}
            max={60}
            step="any"
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
      <ParamField label="暂停图标" hint="暂停期间在画面中央显示。">
        <Switch
          aria-label="暂停图标"
          checked={value.showIcon}
          onCheckedChange={(showIcon) => onChange({ showIcon })}
        />
      </ParamField>
      {value.showIcon && (
        <>
          <ParamField label="图标图片" hint="不选时使用内置的暂停图标；GIF 会循环播放。">
            <AssetSourceInput
              value={value.icon}
              onChange={(icon) => onChange({ icon })}
              extensions={IMAGE_EXTENSIONS}
              kindLabel="图标"
            />
          </ParamField>
          <ParamField label="图标大小" hint="画面短边的百分比。">
            <div className="flex items-center gap-1.5">
              <NumberInput
                label="图标大小"
                value={value.iconSize}
                onChange={(iconSize) => onChange({ iconSize })}
                min={5}
                max={80}
              />
              <span className="text-xs text-muted-foreground">%</span>
            </div>
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineTool<FreezePlayParams>({
  id: "tool.freeze-play",
  defaults: {
    positions: "50%",
    pauseSeconds: 1,
    showIcon: true,
    icon: { mode: "file", path: "" },
    iconSize: 20,
  },
  inputs: VIDEO_INPUTS,
  Form: FreezePlayForm,
  preview: true,
});
