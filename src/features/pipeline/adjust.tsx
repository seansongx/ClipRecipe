// pipe.adjust 画面调整。后端：src-tauri/src/pipeline/modules/adjust.rs

import { useId } from "react";
import AssetSourceInput from "@/components/AssetSourceInput";
import ColorField from "@/components/ColorField";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import type { AssetSource, Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

/** 可单独开关的范围值参数 */
export interface Tweak {
  enabled: boolean;
  value: Range;
}

export interface LutParams {
  enabled: boolean;
  source: AssetSource;
}

export interface BorderParams {
  enabled: boolean;
  /** 边框宽度（像素） */
  width: Range;
  /** #rrggbb */
  color: string;
  /** 每个视频随机颜色 */
  randomColor: boolean;
}

export interface AdjustParams {
  enabled: boolean;
  brightness: Tweak;
  contrast: Tweak;
  saturation: Tweak;
  sharpen: Tweak;
  denoise: Tweak;
  lut: LutParams;
  /** 入场（淡入）时长，秒 */
  fadeIn: Tweak;
  /** 光晕镜头角，度 */
  vignette: Tweak;
  blackWhite: boolean;
  /** 渐出（淡出）时长，秒 */
  fadeOut: Tweak;
  border: BorderParams;
  /** 开幕动画时长，秒 */
  opening: Tweak;
  /** 马赛克方块边长，像素 */
  mosaic: Tweak;
  /** 蒙板倒置不透明度 */
  maskInvert: Tweak;
}

type TweakKey = {
  [K in keyof AdjustParams]: AdjustParams[K] extends Tweak ? K : never;
}[keyof AdjustParams];

const LUT_EXTENSIONS = ["cube", "3dl", "dat", "m3d", "csp"];

function TweakRow({
  label,
  hint,
  tweak,
  onChange,
  min,
  max,
  step,
  integer,
  unit,
}: {
  label: string;
  hint?: string;
  tweak: Tweak;
  onChange: (tweak: Tweak) => void;
  min: number;
  max: number;
  step: number;
  integer?: boolean;
  unit?: string;
}) {
  return (
    <ParamField label={label} hint={tweak.enabled ? hint : undefined}>
      <div className="flex items-start gap-2">
        <span className="flex h-7 items-center">
          <Switch aria-label={label} checked={tweak.enabled} onCheckedChange={(enabled) => onChange({ ...tweak, enabled })} />
        </span>
        {tweak.enabled && (
          <RangeInput
            value={tweak.value}
            onChange={(value) => onChange({ ...tweak, value })}
            min={min}
            max={max}
            step={step}
            integer={integer}
            unit={unit}
          />
        )}
      </div>
    </ParamField>
  );
}

function SectionTitle({ children }: { children: string }) {
  return <h4 className="mt-2 border-t pt-2 text-xs font-medium text-muted-foreground first:mt-0 first:border-0 first:pt-0">{children}</h4>;
}

function AdjustForm({ value, onChange }: ModuleFormProps<AdjustParams>) {
  const tweak = (key: TweakKey) => ({
    tweak: value[key],
    onChange: (next: Tweak) => onChange({ [key]: next } as Partial<AdjustParams>),
  });
  const border = value.border;
  const randomColorId = useId();
  const setBorder = (patch: Partial<BorderParams>) => onChange({ border: { ...border, ...patch } });

  return (
    <>
      <SectionTitle>调色</SectionTitle>
      <TweakRow label="亮度" hint="0 为不变，-1～1。" min={-1} max={1} step={0.01} {...tweak("brightness")} />
      <TweakRow label="对比度" hint="1 为不变，0～3。" min={0} max={3} step={0.01} {...tweak("contrast")} />
      <TweakRow label="饱和度" hint="1 为不变，0 为灰度，0～3。" min={0} max={3} step={0.01} {...tweak("saturation")} />
      <TweakRow label="锐化" hint="强度 0～5。" min={0} max={5} step={0.1} {...tweak("sharpen")} />
      <TweakRow label="降噪" hint="强度 0～30，常用 2～6。" min={0} max={30} step={0.5} {...tweak("denoise")} />
      <ParamField label="LUT 滤镜" hint={value.lut.enabled ? "支持 .cube / .3dl / .dat / .m3d / .csp。" : undefined}>
        <div className="space-y-2">
          <div className="flex h-7 items-center">
            <Switch
              aria-label="LUT 滤镜"
              checked={value.lut.enabled}
              onCheckedChange={(enabled) => onChange({ lut: { ...value.lut, enabled } })}
            />
          </div>
          {value.lut.enabled && (
            <AssetSourceInput
              value={value.lut.source}
              onChange={(source) => onChange({ lut: { ...value.lut, source } })}
              extensions={LUT_EXTENSIONS}
              kindLabel="LUT "
            />
          )}
        </div>
      </ParamField>

      <SectionTitle>更多效果</SectionTitle>
      <TweakRow label="入场" hint="开头从黑场淡入的时长。" min={0.1} max={60} step={0.1} unit="秒" {...tweak("fadeIn")} />
      <TweakRow label="渐出" hint="结尾淡出到黑场的时长。" min={0.1} max={60} step={0.1} unit="秒" {...tweak("fadeOut")} />
      <TweakRow label="开幕" hint="黑幕从中间向两侧拉开的时长。" min={0.1} max={60} step={0.1} unit="秒" {...tweak("opening")} />
      <TweakRow label="光晕" hint="四周压暗的镜头角，越大越暗。" min={1} max={89} step={1} unit="°" {...tweak("vignette")} />
      <TweakRow label="马赛克" hint="整幅画面的方块边长。" min={2} max={256} step={1} integer unit="px" {...tweak("mosaic")} />
      <TweakRow
        label="蒙板倒置"
        hint="把画面上下倒置后以此不透明度叠加在原画面上。"
        min={0.01}
        max={1}
        step={0.05}
        {...tweak("maskInvert")}
      />
      <ParamField label="黑白">
        <Switch aria-label="黑白" checked={value.blackWhite} onCheckedChange={(blackWhite) => onChange({ blackWhite })} />
      </ParamField>
      <ParamField label="边框">
        <div className="space-y-2">
          <div className="flex h-7 items-center">
            <Switch aria-label="边框" checked={border.enabled} onCheckedChange={(enabled) => setBorder({ enabled })} />
          </div>
          {border.enabled && (
            <>
              <RangeInput
                value={border.width}
                onChange={(width) => setBorder({ width })}
                min={1}
                max={500}
                step={1}
                integer
                unit="px"
              />
              <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                <span className="flex items-center gap-1.5">
                  颜色
                  <ColorField
                    label="边框颜色"
                    value={border.color}
                    disabled={border.randomColor}
                    onChange={(color) => setBorder({ color })}
                  />
                </span>
                <span className="flex items-center gap-1.5">
                  <Switch
                    id={randomColorId}
                    checked={border.randomColor}
                    onCheckedChange={(randomColor) => setBorder({ randomColor })}
                  />
                  <Label htmlFor={randomColorId}>
                    每个视频随机颜色
                  </Label>
                </span>
              </div>
            </>
          )}
        </div>
      </ParamField>
    </>
  );
}

const off = (min: number, max: number): Tweak => ({ enabled: false, value: { min, max } });

export default defineModule<AdjustParams>({
  id: "pipe.adjust",
  defaults: {
    enabled: false,
    brightness: off(0, 0.05),
    contrast: off(1, 1.1),
    saturation: off(1, 1.2),
    sharpen: off(0.5, 1),
    denoise: off(2, 4),
    lut: { enabled: false, source: { mode: "file", path: "" } },
    fadeIn: off(1, 1),
    vignette: off(30, 30),
    blackWhite: false,
    fadeOut: off(1, 1),
    border: { enabled: false, width: { min: 20, max: 20 }, color: "#ffffff", randomColor: false },
    opening: off(1.5, 1.5),
    mosaic: off(10, 10),
    maskInvert: off(0.3, 0.3),
  },
  Form: AdjustForm,
});
