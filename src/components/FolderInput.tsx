import type { Ref } from "react";
import { FolderOpen, X } from "lucide-react";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import Hint from "@/components/Hint";
import { cleanPastedPath } from "@/components/OutputPicker";
import { isTauri, pickFolder } from "@/lib/backend";
import { cn } from "@/lib/utils";

/** 选择一个文件夹：按钮打开系统对话框，也可以直接粘贴路径或拖入（需由上层用 boxRef 做拖放判断）。由 shadcn InputGroup 组合而成。 */
export default function FolderInput({
  value,
  onChange,
  label,
  placeholder = "选择或粘贴文件夹",
  className,
  boxRef,
  dropTarget = false,
}: {
  value: string;
  onChange: (path: string) => void;
  /** 按钮文字与无障碍名称，如“视频文件夹” */
  label: string;
  placeholder?: string;
  className?: string;
  /** 窗口拖放时用来判断是否落在这个框上（见 components/useFolderDrop.ts） */
  boxRef?: Ref<HTMLDivElement>;
  /** 正有文件夹拖到这个框上（高亮） */
  dropTarget?: boolean;
}) {
  return (
    <InputGroup ref={boxRef} className={cn(dropTarget && "border-ring ring-2 ring-ring/30", className)}>
      <InputGroupAddon align="inline-start">
        <Hint label={`选择${label}`}>
          <InputGroupButton
            size="sm"
            onClick={() => void pickFolder().then((dir) => dir && onChange(dir))}
            disabled={!isTauri}
          >
            <FolderOpen /> {label}
          </InputGroupButton>
        </Hint>
      </InputGroupAddon>
      <InputGroupInput
        value={value}
        aria-label={label}
        placeholder={dropTarget ? "松开以设为此文件夹" : placeholder}
        onChange={(e) => onChange(e.target.value)}
        onPaste={(e) => {
          const text = e.clipboardData.getData("text");
          if (!text) return;
          e.preventDefault();
          onChange(cleanPastedPath(text));
        }}
      />
      {value && (
        <InputGroupAddon align="inline-end">
          <Hint label="清除">
            <InputGroupButton size="icon-xs" aria-label={`清除${label}`} onClick={() => onChange("")}>
              <X />
            </InputGroupButton>
          </Hint>
        </InputGroupAddon>
      )}
    </InputGroup>
  );
}
