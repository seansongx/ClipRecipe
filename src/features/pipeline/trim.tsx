// pipe.trim 去头尾。后端：src-tauri/src/pipeline/modules/trim.rs

import ParamField from "@/components/ParamField";
import NumberInput from "@/components/NumberInput";
import RangeInput from "@/components/RangeInput";
import ChoiceGroup from "@/components/ChoiceGroup";
import type { Range } from "@/lib/backend";
import { join, range } from "./summary";
import { defineModule, type ModuleFormProps } from "./types";

export type TrimMode = "headTail" | "segment";

export interface TrimParams {
  enabled: boolean;
  mode: TrimMode;
  /** 去片头时长（秒） */
  head: Range;
  /** 去片尾时长（秒） */
  tail: Range;
  /** 截取起点（秒） */
  start: Range;
  /** 截取长度（秒） */
  length: Range;
  /** 截取后的最短时长（秒），0 表示不限制 */
  minDuration: number;
}

const MAX_SECONDS = 86400;

function TrimForm({ value, onChange }: ModuleFormProps<TrimParams>) {
  return (
    <>
      <ParamField label="方式">
        <ChoiceGroup<TrimMode>
          value={value.mode}
          onChange={(mode) => onChange({ mode })}
          options={[
            { value: "headTail", label: "去片头片尾" },
            { value: "segment", label: "起点 + 长度" },
          ]}
        />
      </ParamField>
      {value.mode === "headTail" ? (
        <>
          <ParamField label="去片头">
            <RangeInput
              value={value.head}
              onChange={(head) => onChange({ head })}
              min={0}
              max={MAX_SECONDS}
              step={0.1}
              unit="秒"
            />
          </ParamField>
          <ParamField label="去片尾">
            <RangeInput
              value={value.tail}
              onChange={(tail) => onChange({ tail })}
              min={0}
              max={MAX_SECONDS}
              step={0.1}
              unit="秒"
            />
          </ParamField>
        </>
      ) : (
        <>
          <ParamField label="起点">
            <RangeInput
              value={value.start}
              onChange={(start) => onChange({ start })}
              min={0}
              max={MAX_SECONDS}
              step={0.1}
              unit="秒"
            />
          </ParamField>
          <ParamField label="长度" hint="超出视频末尾时截到末尾为止。">
            <RangeInput
              value={value.length}
              onChange={(length) => onChange({ length })}
              min={0.1}
              max={MAX_SECONDS}
              step={0.1}
              unit="秒"
            />
          </ParamField>
        </>
      )}
      <ParamField
        label="最短时长"
        hint="0 表示不限制。截取后不足此时长时自动少截一些；视频本身更短则不截取。"
      >
        <div className="flex items-center gap-1.5">
          <NumberInput
            value={value.minDuration}
            onChange={(minDuration) => onChange({ minDuration })}
            min={0}
            max={MAX_SECONDS}
            step={0.5}
            label="最短时长"
          />
          <span className="text-xs text-muted-foreground">秒</span>
        </div>
      </ParamField>
    </>
  );
}

export default defineModule<TrimParams>({
  id: "pipe.trim",
  defaults: {
    enabled: false,
    mode: "headTail",
    head: { min: 1, max: 1 },
    tail: { min: 1, max: 1 },
    start: { min: 0, max: 0 },
    length: { min: 10, max: 10 },
    minDuration: 0,
  },
  Form: TrimForm,
  summary: (p) => {
    if (p.mode === "segment") return `从 ${range(p.start, " 秒")}起截取 ${range(p.length, " 秒")}`;
    const head = p.head.max > 0 && `开头 ${range(p.head, " 秒")}`;
    const tail = p.tail.max > 0 && `结尾 ${range(p.tail, " 秒")}`;
    return join(head, tail) || "不去除";
  },
});
