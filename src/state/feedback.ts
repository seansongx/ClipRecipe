// 反馈问题：环境信息、处理记录的文本、预填好的 GitHub issue 地址。
// 只在用户点击时生成文字或打开浏览器；程序本身不上传任何内容。

import { getAppInfo, isTauri, openProjectPage } from "../lib/backend";
import { copyText } from "../lib/clipboard";
import { formatTime } from "../lib/format";
import { useEncoders } from "./encoders";
import { SOURCE_LABELS, useFfmpeg } from "./ffmpeg";
import { useLog, type LogEntry } from "./log";
import { useSettings } from "./settings";

/** 新建 issue 的地址（仓库与 src-tauri/src/system.rs 的 REPO_URL 一致）；字段名见 .github/ISSUE_TEMPLATE/bug_report.yml。 */
const NEW_ISSUE_URL = "https://github.com/seansongx/ClipRecipe/issues/new";
const ISSUE_TEMPLATE = "bug_report.yml";
/** 预填地址的长度上限：再长有的系统或浏览器打不开；放不下的内容（处理记录）靠复制粘贴。 */
const MAX_URL_LENGTH = 2000;

const LEVEL_LABEL = { debug: "调试", info: "信息", warn: "警告", error: "错误" } as const;

/** 去掉路径里的系统用户名（issue 是公开的）：`C:\Users\张三\…` → `C:\Users\***\…`。 */
export function redactPaths(text: string): string {
  return text
    .replace(/([A-Za-z]:[\\/]+Users[\\/]+)[^\\/\r\n"'<>|:*?]+/g, "$1***")
    .replace(/(\/(?:home|Users)\/)[^/\s"']+/g, "$1***");
}

/** 处理记录的文字（含调试级别的 FFmpeg 命令行）。 */
export function formatLog(entries: LogEntry[]): string {
  return redactPaths(
    entries.map((e) => `${formatTime(e.time)} [${LEVEL_LABEL[e.level]}] ${e.message}`).join("\n"),
  );
}

/** 环境信息：应用版本、系统、FFmpeg、硬件加速。 */
export async function environmentInfo(): Promise<string> {
  const lines: string[] = [];
  if (isTauri) {
    try {
      const app = await getAppInfo();
      lines.push(`剪方 ClipRecipe ${app.version}`, `系统：${app.os}（${app.arch}）`);
    } catch {
      lines.push("剪方 ClipRecipe（版本读取失败）");
    }
  } else {
    lines.push("剪方 ClipRecipe（浏览器预览）");
  }
  const ffmpeg = useFfmpeg.getState().status;
  if (!ffmpeg) lines.push("FFmpeg：尚未检测");
  else if (ffmpeg.available) lines.push(`FFmpeg：${ffmpeg.version ?? "版本未知"}（来源：${SOURCE_LABELS[ffmpeg.source]}）`);
  else lines.push(`FFmpeg：未找到${ffmpeg.error ? `（${ffmpeg.error}）` : ""}`);
  const { gpu, hevc } = useSettings.getState().settings;
  const encoders = useEncoders.getState().info;
  const codec = hevc ? "H.265" : "H.264";
  const encoder = gpu && encoders ? `，${codec} → ${hevc ? encoders.hevc : encoders.h264}` : `，${codec}`;
  lines.push(`GPU 加速：${gpu ? "开" : "关"}${encoder}`);
  return redactPaths(lines.join("\n"));
}

/** 预填好环境信息（与错误信息）的新建 issue 地址；错误信息太长时截短。 */
export function issueUrl(environment: string, error = ""): string {
  const build = (err: string) => {
    const query = new URLSearchParams({ template: ISSUE_TEMPLATE, environment });
    if (err) query.set("error", err);
    return `${NEW_ISSUE_URL}?${query.toString()}`;
  };
  let text = redactPaths(error).trim();
  let url = build(text);
  while (url.length > MAX_URL_LENGTH && text.length > 0) {
    text = text.slice(0, Math.floor(text.length * 0.8));
    url = build(text ? `${text}…` : "");
  }
  return url;
}

/** 在浏览器里打开预填好的反馈页面。 */
export async function openFeedback(error?: string): Promise<void> {
  try {
    await openProjectPage(issueUrl(await environmentInfo(), error));
  } catch (e) {
    useLog.getState().add("error", `无法打开反馈页面：${String(e)}`);
  }
}

/** 一条失败任务的完整说明：环境信息、文件名、错误、这条任务的处理记录（含命令行）。 */
export async function failureReport(failure: { name: string; error: string; taskId?: string }): Promise<string> {
  const parts = [await environmentInfo(), `文件：${failure.name}\n错误：${redactPaths(failure.error)}`];
  const related = failure.taskId ? useLog.getState().entries.filter((e) => e.taskId === failure.taskId) : [];
  if (related.length > 0) parts.push(`处理记录：\n${formatLog(related)}`);
  return parts.join("\n\n");
}

/** 复制；写不进剪贴板时在处理记录里说明（按钮不会变成“已复制”）。 */
async function copyOrWarn(text: string): Promise<boolean> {
  const ok = await copyText(text);
  if (!ok) useLog.getState().add("warn", "无法写入剪贴板，请在处理记录里选中文字后手动复制");
  return ok;
}

export async function copyFailure(failure: { name: string; error: string; taskId?: string }): Promise<boolean> {
  return copyOrWarn(await failureReport(failure));
}

/** 复制处理记录，开头带环境信息。 */
export async function copyLog(entries: LogEntry[]): Promise<boolean> {
  return copyOrWarn(`${await environmentInfo()}\n\n${formatLog(entries)}`);
}
