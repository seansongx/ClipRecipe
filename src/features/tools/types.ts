import type { ComponentType } from "react";
import type { ToolParams } from "../../lib/tools";

export type { ToolParams };

/** 工具需要选择的一个文件夹（键与后端 FolderSlot.key 一致）。 */
export interface FolderSlot {
  key: string;
  label: string;
}

/** 工具的输入：队列中的视频，和 / 或若干文件夹（与后端 Tool::inputs() 一致，cargo test 会检查）。 */
export interface ToolInputs {
  videos: boolean;
  folders: FolderSlot[];
}

/** 只处理队列中的视频（大多数工具）。 */
export const VIDEO_INPUTS: ToolInputs = { videos: true, folders: [] };

/** 参数表单的属性：当前参数与局部更新函数。 */
export interface ToolFormProps<P extends ToolParams> {
  value: P;
  onChange: (patch: Partial<P>) => void;
}

/** 一个工具在前端的定义。 */
export interface ToolDef<P extends ToolParams = ToolParams> {
  /** 功能 ID，与 registry.ts、后端 tools/<工具>.rs 的 ID 一致 */
  id: string;
  /** 默认参数，必须与后端参数结构体的 Default 一致 */
  defaults: P;
  /** 输入声明，必须与后端 inputs() 一致 */
  inputs: ToolInputs;
  /** 参数表单；占位工具为 null（没有参数的工具可以用返回说明文字的组件） */
  Form: ComponentType<ToolFormProps<P>> | null;
  /**
   * 支持预览（对话框里的“预览”按钮：第一个选中视频的几秒低清片段经本工具处理后在预览窗口播放）。
   * 必须与后端 Tool::supports_preview() 一致（cargo test 会检查）；默认不支持。
   */
  preview?: boolean;
}

/** 定义工具；把具体参数类型擦除成通用类型，便于放进统一的注册表。 */
export function defineTool<P extends ToolParams>(def: ToolDef<P>): ToolDef {
  return def as unknown as ToolDef;
}
