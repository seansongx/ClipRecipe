// tool.camera-move 运镜效果。后端：src-tauri/src/tools/camera_move.rs
// 裁取较小的窗口平滑往复移动（或缓慢推拉），再放大回源分辨率输出。

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Switch } from "@/components/ui/switch";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type CameraDirection = "horizontal" | "vertical" | "both" | "zoom";

export type CameraMoveParams = {
  direction: CameraDirection;
  /** 放大倍数：窗口 = 源尺寸 ÷ zoom（1.05～4） */
  zoom: number;
  /** 周期由时长自动决定（约 8 秒一个周期，正好整数个） */
  autoPeriod: boolean;
  /** 手动周期（秒） */
  periodSeconds: number;
};

const DIRECTION_HINTS: Record<CameraDirection, string> = {
  horizontal: "窗口从正中出发，左右来回平移。",
  vertical: "窗口从正中出发，上下来回平移。",
  both: "窗口从正中出发，沿斜线来回平移。",
  zoom: "画面居中，从全画面慢慢放大到设定倍数再缩回。",
};

function CameraMoveForm({ value, onChange }: ToolFormProps<CameraMoveParams>) {
  return (
    <>
      <ParamField label="运动方式" hint={DIRECTION_HINTS[value.direction]}>
        <ChoiceGroup<CameraDirection>
          label="运动方式"
          value={value.direction}
          onChange={(direction) => onChange({ direction })}
          options={[
            { value: "horizontal", label: "左右" },
            { value: "vertical", label: "上下" },
            { value: "both", label: "斜向" },
            { value: "zoom", label: "推拉" },
          ]}
        />
      </ParamField>
      <ParamField
        label="放大倍数"
        hint="裁取的窗口为画面的 1/倍数，再放大回原分辨率；倍数越大移动幅度越大，画面越模糊。"
      >
        <div className="flex items-center gap-1.5">
          <NumberInput
            label="放大倍数"
            value={value.zoom}
            onChange={(zoom) => onChange({ zoom })}
            min={1.05}
            max={4}
            step={0.05}
          />
          <span className="text-xs text-muted-foreground">倍</span>
        </div>
      </ParamField>
      <ParamField label="自动周期" hint="按视频时长决定：约 8 秒往返一次，结尾正好回到起点。">
        <Switch
          aria-label="自动周期"
          checked={value.autoPeriod}
          onCheckedChange={(autoPeriod) => onChange({ autoPeriod })}
        />
      </ParamField>
      {!value.autoPeriod && (
        <ParamField label="往返周期" hint="完成一次完整往返所用的时间。">
          <div className="flex items-center gap-1.5">
            <NumberInput
              label="往返周期"
              value={value.periodSeconds}
              onChange={(periodSeconds) => onChange({ periodSeconds })}
              min={0.5}
              max={3600}
              step="any"
            />
            <span className="text-xs text-muted-foreground">秒</span>
          </div>
        </ParamField>
      )}
    </>
  );
}

export default defineTool<CameraMoveParams>({
  id: "tool.camera-move",
  defaults: {
    direction: "horizontal",
    zoom: 1.25,
    autoPeriod: true,
    periodSeconds: 6,
  },
  inputs: VIDEO_INPUTS,
  Form: CameraMoveForm,
  preview: true,
});
