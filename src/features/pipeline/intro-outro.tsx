// pipe.intro-outro 加头尾与边框。后端：src-tauri/src/pipeline/modules/intro_outro.rs

import AssetSourceInput from "@/components/AssetSourceInput";
import ParamField from "@/components/ParamField";
import RangeInput from "@/components/RangeInput";
import { Switch } from "@/components/ui/switch";
import { VIDEO_EXTENSIONS, type AssetSource, type Range } from "@/lib/backend";
import { defineModule, type ModuleFormProps } from "./types";

export interface IntroOutroParams {
  enabled: boolean;
  introEnabled: boolean;
  /** 片头视频 */
  intro: AssetSource;
  outroEnabled: boolean;
  /** 片尾视频 */
  outro: AssetSource;
  frameEnabled: boolean;
  /** 相框图片（中间透明） */
  frame: AssetSource;
  /** 画面与相框边缘的间距（占宽高的百分比） */
  frameMargin: Range;
}

const FRAME_EXTENSIONS = ["png", "webp"];

function IntroOutroForm({ value, onChange }: ModuleFormProps<IntroOutroParams>) {
  return (
    <>
      <ParamField label="片头">
        <Switch
          checked={value.introEnabled}
          onCheckedChange={(introEnabled) => onChange({ introEnabled })}
          aria-label="启用片头"
        />
      </ParamField>
      {value.introEnabled && (
        <ParamField label="片头视频" hint="自动缩放、补黑边并转换为正片的帧率与声道；没有声音时补静音。">
          <AssetSourceInput
            value={value.intro}
            onChange={(intro) => onChange({ intro })}
            extensions={VIDEO_EXTENSIONS}
            kindLabel="片头"
          />
        </ParamField>
      )}
      <ParamField label="片尾">
        <Switch
          checked={value.outroEnabled}
          onCheckedChange={(outroEnabled) => onChange({ outroEnabled })}
          aria-label="启用片尾"
        />
      </ParamField>
      {value.outroEnabled && (
        <ParamField label="片尾视频">
          <AssetSourceInput
            value={value.outro}
            onChange={(outro) => onChange({ outro })}
            extensions={VIDEO_EXTENSIONS}
            kindLabel="片尾"
          />
        </ParamField>
      )}
      <ParamField label="相框">
        <Switch
          checked={value.frameEnabled}
          onCheckedChange={(frameEnabled) => onChange({ frameEnabled })}
          aria-label="启用相框"
        />
      </ParamField>
      {value.frameEnabled && (
        <>
          <ParamField label="相框图片" hint="中间透明的 PNG / WebP，拉伸到画面大小盖在最上层；只套在正片上。">
            <AssetSourceInput
              value={value.frame}
              onChange={(frame) => onChange({ frame })}
              extensions={FRAME_EXTENSIONS}
              kindLabel="相框"
            />
          </ParamField>
          <ParamField label="间距" hint="画面缩小后与相框边缘的距离，按宽、高的百分比。">
            <RangeInput
              value={value.frameMargin}
              onChange={(frameMargin) => onChange({ frameMargin })}
              min={0}
              max={40}
              step={0.5}
              unit="%"
            />
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineModule<IntroOutroParams>({
  id: "pipe.intro-outro",
  defaults: {
    enabled: false,
    introEnabled: false,
    intro: { mode: "file", path: "" },
    outroEnabled: false,
    outro: { mode: "file", path: "" },
    frameEnabled: false,
    frame: { mode: "file", path: "" },
    frameMargin: { min: 5, max: 5 },
  },
  Form: IntroOutroForm,
});
