// tool.subtitle-asr 语音生成字幕（本机 whisper.cpp）。后端：src-tauri/src/tools/subtitle_asr.rs

import { FileInput, TriangleAlert, X } from "lucide-react";
import ChoiceGroup from "@/components/ChoiceGroup";
import ColorField from "@/components/ColorField";
import Hint from "@/components/Hint";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { isTauri, pickFile } from "@/lib/backend";
import { cn } from "@/lib/utils";
import { useFfmpegStatus } from "@/state/ffmpeg";
import { WithUnit } from "./merge";
import { defineTool, VIDEO_INPUTS, type ToolFormProps } from "./types";

export type AsrEngine = "auto" | "builtin" | "cli";
export type SrtLocation = "output" | "source";

export type SubtitleAsrParams = {
  engine: AsrEngine;
  /** ggml 模型文件 */
  modelPath: string;
  /** whisper-cli 程序 */
  cliPath: string;
  /** auto 或语言代码 */
  language: string;
  /** 内置滤镜每次识别的音频长度（秒） */
  queueSeconds: number;
  useGpu: boolean;
  /** 可选的 VAD 模型（只用于内置滤镜） */
  vadModelPath: string;
  vadThreshold: number;
  /** 每条字幕最多字符数，0 不限 */
  maxLength: number;
  srtLocation: SrtLocation;
  burn: boolean;
  /** 字号（画面高度的百分比） */
  fontSize: number;
  fontColor: string;
  outlineColor: string;
  outlineWidth: number;
  /** 距底部（画面高度的百分比） */
  marginBottom: number;
};

/** 选择一个文件：按钮打开系统对话框，也可以直接粘贴路径。由 shadcn InputGroup 组合而成。 */
export function FilePathInput({
  value,
  onChange,
  label,
  kind,
  extensions,
  placeholder,
}: {
  value: string;
  onChange: (path: string) => void;
  /** 无障碍名称，如“模型文件” */
  label: string;
  /** 文件对话框里的类型名 */
  kind: string;
  extensions: string[];
  placeholder?: string;
}) {
  return (
    <InputGroup>
      <InputGroupInput
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder ?? `选择或粘贴${label}路径`}
        aria-label={label}
      />
      <InputGroupAddon align="inline-end">
        {value && (
          <Hint label="清除">
            <InputGroupButton size="icon-xs" aria-label={`清除${label}`} onClick={() => onChange("")}>
              <X />
            </InputGroupButton>
          </Hint>
        )}
        <Hint label="浏览">
          <InputGroupButton
            size="icon-xs"
            aria-label={`选择${label}`}
            disabled={!isTauri}
            onClick={() => void pickFile(kind, extensions).then((path) => path && onChange(path))}
          >
            <FileInput />
          </InputGroupButton>
        </Hint>
      </InputGroupAddon>
    </InputGroup>
  );
}

const EXE_EXTENSIONS = ["exe", "*"];

/** 当前 FFmpeg 没有 whisper 滤镜时提前说明（应用内下载的 essentials 构建就没有）。 */
function WhisperFilterNotice({ engine }: { engine: AsrEngine }) {
  const status = useFfmpegStatus();
  if (!status?.available || status.whisper || engine === "cli") return null;
  return (
    <p
      role="note"
      className={cn(
        "mb-1 flex gap-1.5 rounded-md border px-2 py-1.5 text-xs leading-snug",
        engine === "builtin" ? "border-warning/40 bg-warning/10" : "text-muted-foreground",
      )}
    >
      <TriangleAlert className="mt-px size-3.5 shrink-0 text-warning" />
      <span>
        当前 FFmpeg 没有 whisper 滤镜（应用内下载的精简构建不含它），
        {engine === "builtin"
          ? "“FFmpeg 内置”引擎无法使用："
          : "“自动”会改用 whisper-cli，必须选择 whisper-cli 程序；"}
        也可以通过 PATH 或环境变量 CLIPRECIPE_FFMPEG 使用带 whisper 的完整版 FFmpeg（如 gyan.dev full 构建）。
      </span>
    </p>
  );
}

function SubtitleAsrForm({ value, onChange }: ToolFormProps<SubtitleAsrParams>) {
  return (
    <>
      <WhisperFilterNotice engine={value.engine} />
      <p className="pb-1 text-xs leading-snug text-muted-foreground">
        在本机用 whisper.cpp 识别语音，不上传任何数据。需要先自行下载 ggml 模型文件（如 ggml-base.bin、ggml-small.bin），
        下载地址：huggingface.co/ggerganov/whisper.cpp（本程序不会自动下载）。模型越大越准、越慢。
      </p>
      <ParamField label="模型" hint="whisper.cpp 的 ggml 模型文件（ggml-*.bin）。">
        <FilePathInput
          value={value.modelPath}
          onChange={(modelPath) => onChange({ modelPath })}
          label="模型文件"
          kind="whisper 模型"
          extensions={["bin"]}
        />
      </ParamField>
      <ParamField label="语言" hint="auto 自动识别；也可填语言代码，如 zh、en、ja。">
        <Input
          value={value.language}
          onChange={(e) => onChange({ language: e.target.value })}
          placeholder="auto"
          aria-label="识别语言"
          className="w-24"
        />
      </ParamField>
      <ParamField
        label="引擎"
        hint="自动：FFmpeg 自带 whisper 滤镜时用它，否则用 whisper-cli。whisper-cli 来自 whisper.cpp 的发布包（github.com/ggml-org/whisper.cpp）。"
      >
        <ChoiceGroup<AsrEngine>
          label="识别引擎"
          value={value.engine}
          onChange={(engine) => onChange({ engine })}
          options={[
            { value: "auto", label: "自动" },
            { value: "builtin", label: "FFmpeg 内置" },
            { value: "cli", label: "whisper-cli" },
          ]}
        />
      </ParamField>
      {value.engine !== "builtin" && (
        <ParamField label="whisper-cli" hint={value.engine === "auto" ? "FFmpeg 没有 whisper 滤镜时才需要。" : undefined}>
          <FilePathInput
            value={value.cliPath}
            onChange={(cliPath) => onChange({ cliPath })}
            label="whisper-cli 程序"
            kind="程序"
            extensions={EXE_EXTENSIONS}
          />
        </ParamField>
      )}
      {value.engine !== "cli" && (
        <>
          <ParamField label="片段长度" hint="内置滤镜每次识别的音频长度，越长上下文越完整。">
            <WithUnit unit="秒">
              <NumberInput
                label="识别片段长度"
                value={value.queueSeconds}
                onChange={(queueSeconds) => onChange({ queueSeconds })}
                min={1}
                max={60}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="VAD 模型" hint="可选：语音活动检测模型（如 ggml-silero-*.bin），跳过无人声片段。">
            <FilePathInput
              value={value.vadModelPath}
              onChange={(vadModelPath) => onChange({ vadModelPath })}
              label="VAD 模型文件"
              kind="VAD 模型"
              extensions={["bin"]}
              placeholder="不使用"
            />
          </ParamField>
          {value.vadModelPath.trim() !== "" && (
            <ParamField label="VAD 阈值">
              <NumberInput
                label="VAD 阈值"
                value={value.vadThreshold}
                onChange={(vadThreshold) => onChange({ vadThreshold })}
                min={0}
                max={1}
                step={0.05}
              />
            </ParamField>
          )}
        </>
      )}
      <ParamField label="GPU" hint="FFmpeg / whisper.cpp 没有 GPU 支持时自动使用 CPU。">
        <Switch aria-label="使用 GPU" checked={value.useGpu} onCheckedChange={(useGpu) => onChange({ useGpu })} />
      </ParamField>
      <ParamField label="每条字数" hint="每条字幕最多字符数，0 为不限制。">
        <NumberInput
          label="每条字幕最多字符数"
          value={value.maxLength}
          onChange={(maxLength) => onChange({ maxLength })}
          min={0}
          max={200}
          integer
        />
      </ParamField>
      <ParamField label="字幕位置" hint="源视频旁：保存为“源文件名.srt”，便于播放器自动加载。">
        <ChoiceGroup<SrtLocation>
          label="字幕保存位置"
          value={value.srtLocation}
          onChange={(srtLocation) => onChange({ srtLocation })}
          options={[
            { value: "output", label: "输出位置" },
            { value: "source", label: "源视频旁" },
          ]}
        />
      </ParamField>
      <Separator className="my-1" />
      <ParamField label="烧录" hint="另外生成“视频名_subtitled”新视频，字幕烧录进画面（视频重编码）。">
        <Switch aria-label="烧录字幕" checked={value.burn} onCheckedChange={(burn) => onChange({ burn })} />
      </ParamField>
      {value.burn && (
        <>
          <ParamField label="字号" hint="占画面高度的百分比。">
            <WithUnit unit="%">
              <NumberInput
                label="字号"
                value={value.fontSize}
                onChange={(fontSize) => onChange({ fontSize })}
                min={1}
                max={20}
                step="any"
              />
            </WithUnit>
          </ParamField>
          <ParamField label="文字颜色">
            <ColorField label="文字颜色" value={value.fontColor} onChange={(fontColor) => onChange({ fontColor })} />
          </ParamField>
          <ParamField label="描边颜色">
            <ColorField
              label="描边颜色"
              value={value.outlineColor}
              onChange={(outlineColor) => onChange({ outlineColor })}
            />
          </ParamField>
          <ParamField label="描边粗细">
            <NumberInput
              label="描边粗细"
              value={value.outlineWidth}
              onChange={(outlineWidth) => onChange({ outlineWidth })}
              min={0}
              max={10}
              step="any"
            />
          </ParamField>
          <ParamField label="距底部" hint="占画面高度的百分比。">
            <WithUnit unit="%">
              <NumberInput
                label="距底部"
                value={value.marginBottom}
                onChange={(marginBottom) => onChange({ marginBottom })}
                min={0}
                max={40}
                step="any"
              />
            </WithUnit>
          </ParamField>
        </>
      )}
    </>
  );
}

export default defineTool<SubtitleAsrParams>({
  id: "tool.subtitle-asr",
  defaults: {
    engine: "auto",
    modelPath: "",
    cliPath: "",
    language: "auto",
    queueSeconds: 10,
    useGpu: false,
    vadModelPath: "",
    vadThreshold: 0.5,
    maxLength: 0,
    srtLocation: "output",
    burn: false,
    fontSize: 5,
    fontColor: "#ffffff",
    outlineColor: "#000000", // token-ignore：视频参数默认值，不是界面颜色
    outlineWidth: 2,
    marginBottom: 5,
  },
  inputs: VIDEO_INPUTS,
  Form: SubtitleAsrForm,
});
