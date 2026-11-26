import { useEffect, useState, type Ref } from "react";
import { ClipboardPaste, FolderOpen, TriangleAlert, X } from "lucide-react";
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from "@/components/ui/input-group";
import Hint from "@/components/Hint";
import { checkOutputDir, isTauri, pickFolder } from "@/lib/backend";
import { formatCombo } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useLog } from "@/state/log";
import { useQueue } from "@/state/queue";
import { useSettings } from "@/state/settings";

/** 去掉粘贴路径两端的空白与引号（资源管理器“复制文件地址”会带引号）。 */
export function cleanPastedPath(text: string): string {
  return text.trim().replace(/^["']+|["']+$/g, "").trim();
}

/** 输出位置（pipe.output）：选择 / 粘贴 / 拖入文件夹；与源目录相同时警告。 */
export default function OutputPicker({
  boxRef,
  dropTarget = false,
  className,
}: {
  /** 窗口拖放时用来判断是否落在输出位置上 */
  boxRef?: Ref<HTMLDivElement>;
  dropTarget?: boolean;
  className?: string;
}) {
  const outputDir = useSettings((s) => s.settings.outputDir);
  const update = useSettings((s) => s.update);
  const items = useQueue((s) => s.items);
  const [sameAsSource, setSameAsSource] = useState(false);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    if (!isTauri || !outputDir.trim()) {
      setSameAsSource(false);
      setMissing(false);
      return;
    }
    let alive = true;
    const timer = setTimeout(() => {
      checkOutputDir(outputDir, items.map((i) => i.path))
        .then((check) => {
          if (!alive) return;
          setSameAsSource(check.sameAsSource);
          setMissing(!check.exists);
        })
        .catch(() => {});
    }, 200);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [outputDir, items]);

  const pasteFromClipboard = async () => {
    try {
      const text = cleanPastedPath(await navigator.clipboard.readText());
      if (text) update({ outputDir: text });
    } catch (e) {
      useLog.getState().add("warn", `无法读取剪贴板（可在输入框中按 ${formatCombo({ mod: true, label: "V" })} 粘贴）：${String(e)}`);
    }
  };

  const warning = !outputDir.trim()
    ? "未选择输出位置：将输出到各视频所在目录"
    : sameAsSource
      ? "输出目录与部分视频的源目录相同"
      : missing
        ? "该文件夹不存在，开始处理时会自动创建"
        : null;

  return (
    <div className={cn("min-w-0 flex-1", className)}>
      <InputGroup
        ref={boxRef}
        className={cn(dropTarget && "border-ring ring-2 ring-ring/30")}
      >
        <InputGroupAddon align="inline-start">
          <Hint label="选择输出文件夹">
            <InputGroupButton
              size="sm"
              onClick={() => void pickFolder().then((dir) => dir && update({ outputDir: dir }))}
              disabled={!isTauri}
            >
              <FolderOpen /> 输出到
            </InputGroupButton>
          </Hint>
        </InputGroupAddon>
        {/* 路径放不下时以省略号结尾（输入框失去焦点时生效），悬停可看完整路径 */}
        <Hint label={outputDir || null} side="top">
          <InputGroupInput
            value={outputDir}
            aria-label="输出文件夹"
            className="text-ellipsis"
            onChange={(e) => update({ outputDir: e.target.value })}
            onPaste={(e) => {
              const text = e.clipboardData.getData("text");
              if (!text) return;
              e.preventDefault();
              update({ outputDir: cleanPastedPath(text) });
            }}
            placeholder={dropTarget ? "松开以设为输出文件夹" : "选择、粘贴或拖入输出文件夹"}
          />
        </Hint>
        <InputGroupAddon align="inline-end">
          <Hint label="粘贴剪贴板中的路径">
            <InputGroupButton size="icon-xs" aria-label="粘贴剪贴板中的路径" onClick={() => void pasteFromClipboard()}>
              <ClipboardPaste />
            </InputGroupButton>
          </Hint>
          {outputDir && (
            <Hint label="清除">
              <InputGroupButton size="icon-xs" aria-label="清除输出位置" onClick={() => update({ outputDir: "" })}>
                <X />
              </InputGroupButton>
            </Hint>
          )}
        </InputGroupAddon>
      </InputGroup>
      {warning && (
        <p className="mt-1 flex items-center gap-1 text-xs text-warning">
          <TriangleAlert className="size-3" /> {warning}
        </p>
      )}
    </div>
  );
}
