// 在视频的若干位置上做事（拆开插入、开始移除…）的公共参数与表单（拆分重组、移除重组共用）。

import { useEffect, useState } from "react";
import ChoiceGroup from "@/components/ChoiceGroup";
import NumberInput from "@/components/NumberInput";
import ParamField from "@/components/ParamField";
import { Input } from "@/components/ui/input";

/** 位置的给法：时间点 / 百分比 / 均分（与后端 PositionMode 一致） */
export type PositionMode = "times" | "percents" | "parts";

/** 位置参数（平铺在各工具参数里，与后端字段一致）。 */
export type PositionParams = {
  positionMode: PositionMode;
  /** 时间点（秒） */
  times: number[];
  /** 百分比（0～100） */
  percents: number[];
  /** 均分成几份 */
  parts: number;
};

function formatList(values: number[]): string {
  return values.join(", ");
}

/**
 * 数字列表输入（逗号或空格分隔），基于 shadcn Input：失焦或回车时解析、限幅、排序去重后提交；
 * 没有任何有效数字时恢复原值。
 */
export function NumberListInput({
  value,
  onChange,
  min,
  max,
  label,
  placeholder,
}: {
  value: number[];
  onChange: (value: number[]) => void;
  min: number;
  max: number;
  label: string;
  placeholder?: string;
}) {
  const [text, setText] = useState(formatList(value));

  useEffect(() => setText(formatList(value)), [value]);

  const commit = () => {
    const parsed = text
      .split(/[\s,，、;；]+/)
      .filter((part) => part.trim() !== "")
      .map(Number)
      .filter((n) => Number.isFinite(n))
      .map((n) => Math.min(max, Math.max(min, n)));
    const unique = [...new Set(parsed)].sort((a, b) => a - b);
    if (unique.length === 0) {
      setText(formatList(value));
      return;
    }
    setText(formatList(unique));
    if (formatList(unique) !== formatList(value)) onChange(unique);
  };

  return (
    <Input
      aria-label={label}
      value={text}
      placeholder={placeholder}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
      className="h-7 px-2 text-xs tabular-nums md:text-xs"
    />
  );
}

/** 位置设置：方式 + 时间点 / 百分比列表 / 份数。 */
export function PositionFields({
  value,
  onChange,
  verb,
}: {
  value: PositionParams;
  onChange: (patch: Partial<PositionParams>) => void;
  /** 在位置处做什么，用于说明文字，如“拆开”“开始移除” */
  verb: string;
}) {
  return (
    <>
      <ParamField label="位置">
        <ChoiceGroup<PositionMode>
          label="位置的给法"
          value={value.positionMode}
          onChange={(positionMode) => onChange({ positionMode })}
          options={[
            { value: "parts", label: "均分" },
            { value: "times", label: "时间点" },
            { value: "percents", label: "百分比" },
          ]}
        />
      </ParamField>
      {value.positionMode === "parts" && (
        <ParamField label="份数" hint={`把视频平均分成几份，在每个分界处${verb}。`}>
          <NumberInput
            label="份数"
            value={value.parts}
            onChange={(parts) => onChange({ parts })}
            min={2}
            max={101}
            integer
          />
        </ParamField>
      )}
      {value.positionMode === "times" && (
        <ParamField label="时间点" hint={`单位秒，逗号分隔，如 10, 25.5；在这些时间${verb}，超出视频时长的忽略。`}>
          <NumberListInput
            label="时间点（秒）"
            value={value.times}
            onChange={(times) => onChange({ times })}
            min={0}
            max={86400}
            placeholder="10, 25.5"
          />
        </ParamField>
      )}
      {value.positionMode === "percents" && (
        <ParamField label="百分比" hint={`占总时长的百分比（0～100），逗号分隔，如 30, 70；在这些位置${verb}。`}>
          <NumberListInput
            label="百分比"
            value={value.percents}
            onChange={(percents) => onChange({ percents })}
            min={0}
            max={100}
            placeholder="30, 70"
          />
        </ParamField>
      )}
    </>
  );
}
