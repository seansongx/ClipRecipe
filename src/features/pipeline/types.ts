import type { ComponentType } from "react";
import type { ModuleParams } from "../../lib/backend";

export type { ModuleParams };

/** 参数表单的属性：当前参数与局部更新函数。 */
export interface ModuleFormProps<P extends ModuleParams> {
  value: P;
  onChange: (patch: Partial<P>) => void;
}

/** 一个流水线模块在前端的定义。 */
export interface PipelineModuleDef<P extends ModuleParams = ModuleParams> {
  /** 功能 ID，与 registry.ts、后端 modules/<模块>.rs 的 ID 一致 */
  id: string;
  /** 默认参数，必须与后端参数结构体的 Default 一致 */
  defaults: P;
  /** 参数表单；占位模块为 null */
  Form: ComponentType<ModuleFormProps<P>> | null;
}

/** 定义模块；把具体参数类型擦除成通用类型，便于放进统一的注册表。 */
export function defineModule<P extends ModuleParams>(def: PipelineModuleDef<P>): PipelineModuleDef {
  return def as unknown as PipelineModuleDef;
}
