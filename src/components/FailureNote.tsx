import { Check, Copy, MessageSquareWarning } from "lucide-react";
import { Button } from "@/components/ui/button";
import { isTauri } from "@/lib/backend";
import { useCopied } from "@/lib/clipboard";
import { copyFailure, openFeedback } from "@/state/feedback";

/**
 * 失败任务的错误信息，下面带“复制错误信息”（环境信息 + 错误 + 这条任务的处理记录）与“反馈问题”
 * （打开预填好的 GitHub issue）。放在可点击的行里也不会触发行的点击 / 双击。
 */
export default function FailureNote({
  name,
  error,
  taskId,
  tabIndex,
}: {
  /** 文件名或工作项名称 */
  name: string;
  error: string;
  /** 处理记录里这条任务的 ID */
  taskId?: string;
  /** 所在列表自己管理键盘焦点时传 -1 */
  tabIndex?: number;
}) {
  const [copied, markCopied] = useCopied();
  return (
    <div className="mt-1" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
      <p className="line-clamp-2 text-xs text-destructive select-text">{error}</p>
      <div className="mt-0.5 -ml-2 flex flex-wrap items-center">
        <Button
          variant="ghost"
          size="sm"
          tabIndex={tabIndex}
          onClick={() => void copyFailure({ name, error, taskId }).then((ok) => ok && markCopied())}
        >
          {copied ? <Check /> : <Copy />}
          <span aria-live="polite">{copied ? "已复制" : "复制错误信息"}</span>
        </Button>
        <Button variant="ghost" size="sm" tabIndex={tabIndex} disabled={!isTauri} onClick={() => void openFeedback(error)}>
          <MessageSquareWarning /> 反馈问题
        </Button>
      </div>
    </div>
  );
}
