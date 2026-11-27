// 复制文字到剪贴板。

import { useEffect, useRef, useState } from "react";

/** 剪贴板 API 不可用时（某些内嵌浏览器、没有焦点）：借一个临时文本框复制选区。 */
function copyBySelection(text: string): boolean {
  const box = document.createElement("textarea");
  box.value = text;
  box.setAttribute("readonly", "");
  box.style.cssText = "position:fixed;top:0;left:0;opacity:0;pointer-events:none";
  document.body.appendChild(box);
  box.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  box.remove();
  return ok;
}

/** 复制成功返回 true。 */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return copyBySelection(text);
  }
}

/** 复制按钮的“已复制”状态：成功后保持一小段时间。 */
export function useCopied(holdMs = 1600): [boolean, () => void] {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const mark = () => {
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), holdMs);
  };
  return [copied, mark];
}
