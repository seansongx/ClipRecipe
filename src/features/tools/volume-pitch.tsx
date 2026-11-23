// tool.volume-pitch 音量音调。后端：src-tauri/src/tools/volume_pitch.rs

import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Separator } from "@/components/ui/separator";
import { WithUnit } from "./merge";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type VolumeMode = "keep" | "gain" | "normalize";
export type GainUnit = "db" | "percent";

export type VolumePitchParams = {
  volumeMode: VolumeMode;
  gainUnit: GainUnit;
  /** 增益（dB） */
  gainDb: number;
  /** 增益（百分比，100 为不变） */
  gainPercent: number;
  /** 目标响度（LUFS） */
  targetLufs: number;
  /** 真峰值上限（dBTP） */
  truePeak: number;
  /** 响度范围（LU） */
  loudnessRange: number;
  /** 音调（半音），速度不变 */
  pitchSemitones: number;
};

function VolumePitchForm({ value, onChange }: ToolFormProps<VolumePitchParams>) {
  return (
    <>
      <ParamField
        label="音量"
        hint="统一响度：先测量每个视频的响度（EBU R128），再调到同一目标值，适合让一批视频音量一致。视频画面直接复制，只重编码音频（第一条音轨）。"
      >
        <ChoiceGroup<VolumeMode>
          label="音量调整方式"
          value={value.volumeMode}
          onChange={(volumeMode) => onChange({ volumeMode })}
          options={[
            { value: "normalize", label: "统一响度" },
            { value: "gain", label: "增益" },
            { value: "keep", label: "不变" },
          ]}
        />
      </ParamField>
      {value.volumeMode === "gain" && (
        <>
          <ParamField label="单位">
            <ChoiceGroup<GainUnit>
              label="增益单位"
              value={value.gainUnit}
              onChange={(gainUnit) => onChange({ gainUnit })}
              options={[
                { value: "db", label: "分贝" },
                { value: "percent", label: "百分比" },
              ]}
            />
          </ParamField>
          {value.gainUnit === "db" ? (
            <ParamField label="增益" hint="正数变响、负数变轻；+6 dB 约为两倍振幅。">
              <WithUnit unit="dB">
                <NumberInput
                  label="增益分贝"
                  value={value.gainDb}
                  onChange={(gainDb) => onChange({ gainDb })}
                  min={-60}
                  max={30}
                  step="any"
                />
              </WithUnit>
            </ParamField>
          ) : (
            <ParamField label="音量" hint="100% 为原音量，0% 为静音。">
              <WithUnit unit="%">
                <NumberInput
                  label="音量百分比"
                  value={value.gainPercent}
                  onChange={(gainPercent) => onChange({ gainPercent })}
                  min={0}
                  max={1000}
                  step="any"
                />
              </WithUnit>
            </ParamField>
          )}
        </>
      )}
      {value.volumeMode === "normalize" && (
        <>
          <ParamField label="目标响度" hint="短视频平台常用 -14 ～ -16 LUFS，广播 -23 LUFS。">
            <WithUnit unit="LUFS">
              <NumberInput
                label="目标响度"
                value={value.targetLufs}
                onChange={(targetLufs) => onChange({ targetLufs })}
                min={-70}
                max={-5}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="峰值上限">
            <WithUnit unit="dBTP">
              <NumberInput
                label="真峰值上限"
                value={value.truePeak}
                onChange={(truePeak) => onChange({ truePeak })}
                min={-9}
                max={0}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="响度范围" hint="允许的响度起伏，越小越平。">
            <WithUnit unit="LU">
              <NumberInput
                label="响度范围"
                value={value.loudnessRange}
                onChange={(loudnessRange) => onChange({ loudnessRange })}
                min={1}
                max={50}
                step="any"
              />
            </WithUnit>
          </ParamField>
        </>
      )}
      <Separator className="my-1" />
      <ParamField label="音调" hint="以半音为单位升高或降低音调，速度和时长不变；12 为升高一个八度。">
        <WithUnit unit="半音">
          <NumberInput
            label="音调半音数"
            value={value.pitchSemitones}
            onChange={(pitchSemitones) => onChange({ pitchSemitones })}
            min={-12}
            max={12}
            step="any"
          />
        </WithUnit>
      </ParamField>
    </>
  );
}

export default defineTool<VolumePitchParams>({
  id: "tool.volume-pitch",
  defaults: {
    volumeMode: "normalize",
    gainUnit: "db",
    gainDb: 0,
    gainPercent: 100,
    targetLufs: -16,
    truePeak: -1.5,
    loudnessRange: 11,
    pitchSemitones: 0,
  },
  inputs: VIDEO_INPUTS,
  Form: VolumePitchForm,
});
