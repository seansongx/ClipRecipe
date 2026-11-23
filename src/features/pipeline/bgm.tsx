// pipe.bgm 背景音。后端：src-tauri/src/pipeline/modules/bgm.rs
// 不支持即时预览。

import AssetSourceInput from "@/components/AssetSourceInput";
import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import RangeInput from "@/components/RangeInput";
import { Switch } from "@/components/ui/switch";
import type { AssetSource, Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export interface BgmParams {
  enabled: boolean;
  /** 音乐来源：单文件或目录 */
  source: AssetSource;
  /** 音乐音量（%） */
  musicVolume: Range;
  /** 原声音量（%），0 为静音 */
  originalVolume: Range;
  /** 音乐比视频短时循环播放 */
  loopMusic: boolean;
  /** 音乐延迟开始（秒） */
  delay: Range;
  /** 淡入（秒） */
  fadeIn: number;
  /** 淡出（秒），在视频结尾前完成 */
  fadeOut: number;
}

/** 与后端 MUSIC_EXTENSIONS 一致 */
const MUSIC_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"];

function BgmForm({ value, onChange }: ModuleFormProps<BgmParams>) {
  return (
    <>
      <ParamField label="音乐" hint="目录模式下从 mp3 / m4a / wav 等音频中为每个视频取一首。">
        <AssetSourceInput
          value={value.source}
          onChange={(source) => onChange({ source })}
          extensions={MUSIC_EXTENSIONS}
          kindLabel="音乐"
        />
      </ParamField>
      <ParamField label="音乐音量">
        <RangeInput
          value={value.musicVolume}
          onChange={(musicVolume) => onChange({ musicVolume })}
          min={0}
          max={300}
          integer
          unit="%"
        />
      </ParamField>
      <ParamField label="原声音量" hint="设为 0 即去掉原声，只保留背景音乐。">
        <RangeInput
          value={value.originalVolume}
          onChange={(originalVolume) => onChange({ originalVolume })}
          min={0}
          max={300}
          integer
          unit="%"
        />
      </ParamField>
      <ParamField label="循环播放" hint={value.loopMusic ? "音乐比视频短时从头循环。" : "音乐只播一遍，之后静音。"}>
        <Switch checked={value.loopMusic} onCheckedChange={(loopMusic) => onChange({ loopMusic })} aria-label="循环播放" />
      </ParamField>
      <ParamField label="延迟">
        <RangeInput
          value={value.delay}
          onChange={(delay) => onChange({ delay })}
          min={0}
          max={600}
          step={0.1}
          unit="秒"
        />
      </ParamField>
      <ParamField label="淡入淡出" hint="淡出在视频结尾前完成；0 表示不淡入 / 不淡出。">
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span>淡入</span>
          <NumberInput
            className="w-16"
            label="淡入秒数"
            value={value.fadeIn}
            min={0}
            max={60}
            step={0.5}
            onChange={(fadeIn) => onChange({ fadeIn })}
          />
          <span className="pl-2">淡出</span>
          <NumberInput
            className="w-16"
            label="淡出秒数"
            value={value.fadeOut}
            min={0}
            max={60}
            step={0.5}
            onChange={(fadeOut) => onChange({ fadeOut })}
          />
          <span>秒</span>
        </div>
      </ParamField>
      <p className="pt-1 text-[11px] text-muted-foreground">输出时长始终与视频相同；背景音不支持即时预览。</p>
    </>
  );
}

export default defineModule<BgmParams>({
  id: "pipe.bgm",
  defaults: {
    enabled: false,
    source: { mode: "file", path: "" },
    musicVolume: { min: 50, max: 50 },
    originalVolume: { min: 100, max: 100 },
    loopMusic: true,
    delay: { min: 0, max: 0 },
    fadeIn: 1,
    fadeOut: 1,
  },
  Form: BgmForm,
});
