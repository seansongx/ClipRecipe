// tool.simple-mix 简单混剪。后端：src-tauri/src/tools/simple_mix.rs

import AssetSourceInput from "@/components/AssetSourceInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { VIDEO_EXTENSIONS, type AssetSource } from "@/lib/backend";
import { WithUnit } from "./shared/with-unit";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type MixLayout = "fill" | "center";

export type SimpleMixParams = {
  clipsEnabled: boolean;
  clips: AssetSource;
  layout: MixLayout;
  /** 居中时副视频宽度占主视频的百分比 */
  centerSize: number;
  /** 每段副视频显示的秒数 */
  showSeconds: number;
  /** 两段副视频之间只显示主视频的秒数 */
  gapSeconds: number;
  bgmEnabled: boolean;
  bgm: AssetSource;
  bgmVolume: number;
  originalVolume: number;
  subtitleEnabled: boolean;
  subtitle: AssetSource;
};

/** 与后端 av_mux::AUDIO_EXTENSIONS 一致。 */
const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma"];
const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa"];

function SimpleMixForm({ value, onChange }: ToolFormProps<SimpleMixParams>) {
  return (
    <>
      <ParamField label="副视频" hint="先显示主视频“间隔”秒，再叠加一段副视频“每段”秒，循环到结束；目录时轮流使用。">
        <Switch
          aria-label="叠加副视频"
          checked={value.clipsEnabled}
          onCheckedChange={(clipsEnabled) => onChange({ clipsEnabled })}
        />
      </ParamField>
      {value.clipsEnabled && (
        <>
          <AssetSourceInput
            value={value.clips}
            onChange={(clips) => onChange({ clips })}
            extensions={VIDEO_EXTENSIONS}
            kindLabel="副视频"
          />
          <ParamField label="叠加方式">
            <ChoiceGroup<MixLayout>
              label="叠加方式"
              value={value.layout}
              onChange={(layout) => onChange({ layout })}
              options={[
                { value: "center", label: "居中" },
                { value: "fill", label: "铺满" },
              ]}
            />
          </ParamField>
          {value.layout === "center" && (
            <ParamField label="大小" hint="副视频宽度占主视频宽度的比例，高度按比例。">
              <WithUnit unit="%">
                <NumberInput
                  label="副视频大小"
                  value={value.centerSize}
                  onChange={(centerSize) => onChange({ centerSize })}
                  min={10}
                  max={100}
                  integer
                />
              </WithUnit>
            </ParamField>
          )}
          <ParamField label="每段">
            <WithUnit unit="秒">
              <NumberInput
                label="每段副视频时长"
                value={value.showSeconds}
                onChange={(showSeconds) => onChange({ showSeconds })}
                min={0.5}
                max={600}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="间隔" hint="为 0 时副视频一段接一段。">
            <WithUnit unit="秒">
              <NumberInput
                label="间隔时长"
                value={value.gapSeconds}
                onChange={(gapSeconds) => onChange({ gapSeconds })}
                min={0}
                max={600}
                step="any"
              />
            </WithUnit>
          </ParamField>
        </>
      )}
      <Separator className="my-1" />
      <ParamField label="背景音频" hint="循环到主视频结束，与原声混合。">
        <Switch
          aria-label="添加背景音频"
          checked={value.bgmEnabled}
          onCheckedChange={(bgmEnabled) => onChange({ bgmEnabled })}
        />
      </ParamField>
      {value.bgmEnabled && (
        <>
          <AssetSourceInput
            value={value.bgm}
            onChange={(bgm) => onChange({ bgm })}
            extensions={AUDIO_EXTENSIONS}
            kindLabel="音频"
          />
          <ParamField label="音频音量">
            <WithUnit unit="%">
              <NumberInput
                label="背景音频音量"
                value={value.bgmVolume}
                onChange={(bgmVolume) => onChange({ bgmVolume })}
                min={0}
                max={400}
                integer
              />
            </WithUnit>
          </ParamField>
          <ParamField label="原声音量">
            <WithUnit unit="%">
              <NumberInput
                label="原声音量"
                value={value.originalVolume}
                onChange={(originalVolume) => onChange({ originalVolume })}
                min={0}
                max={400}
                integer
              />
            </WithUnit>
          </ParamField>
        </>
      )}
      <Separator className="my-1" />
      <ParamField label="字幕" hint="srt / ass 字幕烧录到画面，显示在副视频之上。">
        <Switch
          aria-label="烧录字幕"
          checked={value.subtitleEnabled}
          onCheckedChange={(subtitleEnabled) => onChange({ subtitleEnabled })}
        />
      </ParamField>
      {value.subtitleEnabled && (
        <AssetSourceInput
          value={value.subtitle}
          onChange={(subtitle) => onChange({ subtitle })}
          extensions={SUBTITLE_EXTENSIONS}
          kindLabel="字幕"
        />
      )}
    </>
  );
}

const EMPTY: AssetSource = { mode: "file", path: "" };

export default defineTool<SimpleMixParams>({
  id: "tool.simple-mix",
  defaults: {
    clipsEnabled: true,
    clips: EMPTY,
    layout: "center",
    centerSize: 50,
    showSeconds: 3,
    gapSeconds: 3,
    bgmEnabled: false,
    bgm: EMPTY,
    bgmVolume: 100,
    originalVolume: 100,
    subtitleEnabled: false,
    subtitle: EMPTY,
  },
  inputs: VIDEO_INPUTS,
  Form: SimpleMixForm,
});
