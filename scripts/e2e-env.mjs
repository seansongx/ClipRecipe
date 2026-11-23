// 端到端测试：测试素材（FFmpeg lavfi 现场生成）、ffprobe、进程启动与清理。

import { execFile, execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { sleep } from "./e2e-cdp.mjs";

export const REPO = path.resolve(import.meta.dirname, "..");
/**
 * e2e 用独立的 Cargo 目标目录：`cargo test` 也会在 target/debug 下编译同名程序（不带 custom-protocol，
 * 即加载 devUrl 的开发版），共用目录时会把生产形态的程序覆盖掉。
 */
export const E2E_TARGET_DIR = path.join(REPO, "src-tauri", "target", "e2e");
export const DEBUG_EXE = path.join(E2E_TARGET_DIR, "debug", "cliprecipe.exe");
/**
 * `tauri dev` 也编译到 E2E_TARGET_DIR（共用依赖缓存），会把 DEBUG_EXE 覆盖成开发版（加载 devUrl），
 * 所以生产形态构建完成后立即复制一份到 PROD_EXE，`--no-build` 时用它。
 */
export const PROD_EXE = path.join(E2E_TARGET_DIR, "cliprecipe-prod.exe");

const EXE = process.platform === "win32" ? ".exe" : "";
/** `npm install` 装好的 @seankit 开发依赖（本平台）。 */
export const DEV_FFMPEG = path.join(REPO, "node_modules", "@seankit", `ffmpeg-${process.platform}-${process.arch}`, "bin", `ffmpeg${EXE}`);
export const DEV_FFPROBE = path.join(REPO, "node_modules", "@seankit", `ffprobe-${process.platform}-${process.arch}`, "bin", `ffprobe${EXE}`);

/**
 * 测试脚本自己用的 ffmpeg / ffprobe（生成素材、检查输出）：CLIPRECIPE_FFMPEG，否则 node_modules 里的 @seankit 包
 * （与调试构建的程序找到的相同），都没有时用 PATH。
 */
export function ffmpegPaths() {
  const env = process.env.CLIPRECIPE_FFMPEG;
  if (env) {
    const probe = path.join(path.dirname(env), `ffprobe${EXE}`);
    return { ffmpeg: env, ffprobe: fs.existsSync(probe) ? probe : "ffprobe", source: "CLIPRECIPE_FFMPEG" };
  }
  if (fs.existsSync(DEV_FFMPEG) && fs.existsSync(DEV_FFPROBE)) {
    return { ffmpeg: DEV_FFMPEG, ffprobe: DEV_FFPROBE, source: "node_modules" };
  }
  return { ffmpeg: "ffmpeg", ffprobe: "ffprobe", source: "PATH" };
}

const children = new Set();

function run(file, args, { timeout = 120000, encoding = "utf8" } = {}) {
  return new Promise((resolve, reject) => {
    const child = execFile(file, args, { timeout, encoding, maxBuffer: 64 * 1024 * 1024, windowsHide: true }, (err, stdout, stderr) => {
      children.delete(child);
      if (err) reject(new Error(`${path.basename(file)} ${args.join(" ")}\n${stderr || err.message}`));
      else resolve(stdout);
    });
    children.add(child);
  });
}

export async function ffmpeg(args) {
  const { ffmpeg: bin } = ffmpegPaths();
  return run(bin, ["-hide_banner", "-loglevel", "error", "-y", ...args]);
}

export async function probe(file) {
  const { ffprobe } = ffmpegPaths();
  const out = await run(ffprobe, ["-v", "error", "-show_format", "-show_streams", "-of", "json", file]);
  const json = JSON.parse(out);
  const video = json.streams.find((s) => s.codec_type === "video" && !s.disposition?.attached_pic);
  const audio = json.streams.find((s) => s.codec_type === "audio");
  const [num, den] = (video?.avg_frame_rate ?? "0/1").split("/").map(Number);
  return {
    duration: Number(json.format?.duration ?? 0),
    width: video?.width ?? 0,
    height: video?.height ?? 0,
    fps: den ? num / den : 0,
    vcodec: video?.codec_name ?? null,
    acodec: audio?.codec_name ?? null,
    streams: json.streams.length,
  };
}

/** 取一帧缩小成 w×h 的灰度像素。 */
export async function grayFrame(file, time, w = 160, h = 90) {
  const { ffmpeg: bin } = ffmpegPaths();
  const buf = await run(
    bin,
    ["-hide_banner", "-loglevel", "error", "-ss", String(time), "-i", file, "-frames:v", "1", "-vf", `scale=${w}:${h},format=gray`, "-f", "rawvideo", "-"],
    { encoding: "buffer" },
  );
  return { w, h, data: buf };
}

/** 灰度帧中一个水平带（行区间 [y0, y1)）的标准差。 */
export function bandStddev(frame, y0, y1) {
  let sum = 0;
  let sq = 0;
  let n = 0;
  for (let y = y0; y < y1; y++) {
    for (let x = 0; x < frame.w; x++) {
      const v = frame.data[y * frame.w + x];
      sum += v;
      sq += v * v;
      n++;
    }
  }
  const mean = sum / n;
  return Math.sqrt(Math.max(0, sq / n - mean * mean));
}

/** 生成全部测试素材，返回路径表。 */
export async function makeMedia(dir) {
  const m = (...p) => path.join(dir, ...p);
  for (const sub of ["clips", "portrait", "images", "subs", "long", "extra"]) fs.mkdirSync(m(sub), { recursive: true });
  const x264 = ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"];
  const clip = (out, video, seconds, freq) =>
    ffmpeg(["-f", "lavfi", "-i", `${video}:d=${seconds}`, "-f", "lavfi", "-i", `sine=frequency=${freq}:duration=${seconds}`, ...x264, out]);
  const media = {
    a: m("clips", "alpha clip.mp4"),
    b: m("clips", "bravo_clip.mp4"),
    portrait: m("portrait", "portrait.mp4"),
    imageDir: m("images"),
    watermark: m("watermark.png"),
    subbed: m("subs", "subbed.mp4"),
    srt: m("subs", "subbed.srt"),
    long: m("long", "long.mp4"),
    extra: m("extra", "extra.mp4"),
  };
  await Promise.all([
    clip(media.a, "testsrc2=size=640x360:rate=25", 4, 440),
    clip(media.b, "smptebars=size=640x360:rate=25", 4, 660),
    clip(media.portrait, "testsrc2=size=360x640:rate=25", 3, 550),
    clip(media.subbed, "color=c=0x336699:size=640x360:rate=25", 4, 330),
    clip(media.extra, "testsrc=size=320x240:rate=25", 2, 770),
    clip(media.long, "testsrc2=size=1920x1080:rate=30", 90, 220),
    ffmpeg(["-f", "lavfi", "-i", "testsrc2=size=800x600", "-frames:v", "1", m("images", "one.png")]),
    ffmpeg(["-f", "lavfi", "-i", "smptebars=size=600x800", "-frames:v", "1", m("images", "two.png")]),
    ffmpeg(["-f", "lavfi", "-i", "color=c=red:size=200x120", "-frames:v", "1", media.watermark]),
  ]);
  fs.writeFileSync(
    media.srt,
    "1\r\n00:00:00,000 --> 00:00:04,000\r\nSUBTITLE BURN TEST 字幕测试\r\n\r\n",
    "utf8",
  );
  return media;
}

/** 找一个空闲的本地端口。 */
export function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
    srv.on("error", reject);
  });
}

/** 端口是否已被占用（127.0.0.1）。 */
export function portInUse(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: "127.0.0.1", port });
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => resolve(false));
  });
}

/** 结束进程树（Windows 用 taskkill /T /F）。 */
export function killTree(pid) {
  if (!pid) return;
  try {
    if (process.platform === "win32") execFileSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
    else process.kill(-pid, "SIGKILL");
  } catch {
    // 进程已退出
  }
}

/** 列出可执行文件为 exe 的进程（Windows），返回 [{pid, created}]。 */
export function processesOf(exe) {
  if (process.platform !== "win32") return [];
  try {
    const script = `Get-CimInstance Win32_Process -Filter "Name='${path.basename(exe)}'" | Where-Object { $_.ExecutablePath -eq '${exe.replace(/'/g, "''")}' } | ForEach-Object { $_.ProcessId }`;
    const out = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", windowsHide: true, timeout: 20000 });
    return out.split(/\s+/).filter(Boolean).map(Number);
  } catch {
    return [];
  }
}

/** 启动的进程：记录日志，退出时可结束整个进程树。 */
export class Proc {
  constructor(name, child, logFile) {
    this.name = name;
    this.child = child;
    this.exited = false;
    this.exitCode = null;
    const log = fs.createWriteStream(logFile, { flags: "a" });
    child.stdout?.on("data", (d) => log.write(d));
    child.stderr?.on("data", (d) => log.write(d));
    child.on("exit", (code) => {
      this.exited = true;
      this.exitCode = code;
      log.end();
    });
    children.add(child);
  }

  get pid() {
    return this.child.pid;
  }

  async waitExit(ms) {
    const deadline = Date.now() + ms;
    while (!this.exited && Date.now() < deadline) await sleep(100);
    return this.exited;
  }

  kill() {
    if (!this.exited) killTree(this.child.pid);
  }
}

/** 启动进程；args 为 null 时 file 是交给 shell 的整条命令（如 `npm run tauri dev`）。 */
export function startProc(name, file, args, env, logFile, { cwd = REPO } = {}) {
  const options = { env, cwd, windowsHide: false, stdio: ["ignore", "pipe", "pipe"] };
  const child = args === null ? spawn(file, { ...options, shell: true }) : spawn(file, args, options);
  return new Proc(name, child, logFile);
}

/** 结束本模块启动过的全部子进程（ffmpeg、应用、vite）。 */
export function killAllChildren() {
  for (const child of children) {
    if (child.exitCode === null && child.pid) killTree(child.pid);
  }
  children.clear();
}

/** 删除目录（WebView2 退出稍晚时重试）。 */
export async function removeDir(dir) {
  for (let i = 0; i < 20; i++) {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
      return true;
    } catch {
      await sleep(500);
    }
  }
  return false;
}

/** 递归列出目录下的文件（相对路径）。 */
export function listFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const out = [];
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else out.push(path.relative(dir, p));
    }
  };
  walk(dir);
  return out.sort();
}

// ------------------------------------------------------------------ 原生窗口（Win32，经 PowerShell）

/** 声明 user32 函数；先 SetProcessDPIAware，坐标都是物理像素（与 Tauri 的 PhysicalPosition 一致）。 */
const WIN32 = `
Add-Type -Namespace E2E -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
[DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
[DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
public delegate bool EnumProc(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
/// 进程的主窗口：该进程面积最大的顶层窗口（单实例插件的消息窗口、输入法窗口都很小；最小化的主窗口仍比它们大）。
public static IntPtr MainWindow(uint target) {
  IntPtr best = IntPtr.Zero; long bestArea = -1;
  EnumWindows((h, l) => {
    uint pid; GetWindowThreadProcessId(h, out pid);
    if (pid != target) return true;
    RECT r; GetWindowRect(h, out r);
    long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
    if (area > bestArea) { bestArea = area; best = h; }
    return true;
  }, IntPtr.Zero);
  return best;
}
'@
[void][E2E.Win]::SetProcessDPIAware()
`;

function powershell(script) {
  return execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], {
    encoding: "utf8",
    windowsHide: true,
    timeout: 30000,
  });
}

/** 进程的主窗口句柄（PowerShell 表达式）。不用 MainWindowHandle：单实例插件的消息窗口可能被当成主窗口。 */
const hwndOf = (pid) => `$h = [E2E.Win]::MainWindow(${Number(pid)})`;

/**
 * 进程主窗口的原生信息：标题、位置大小（物理像素）、是否最小化 / 最大化 / 可见。
 * 找不到窗口时返回 null。
 */
export function nativeWindow(pid) {
  if (process.platform !== "win32" || !pid) return null;
  const out = powershell(`${WIN32}
${hwndOf(pid)}
if ($h -eq 0) { 'null'; exit }
$r = New-Object E2E.Win+RECT
[void][E2E.Win]::GetWindowRect($h, [ref]$r)
$sb = New-Object System.Text.StringBuilder 256
[void][E2E.Win]::GetWindowText($h, $sb, 256)
@{ title = $sb.ToString(); x = $r.Left; y = $r.Top; width = $r.Right - $r.Left; height = $r.Bottom - $r.Top;
   minimized = [E2E.Win]::IsIconic($h); maximized = [E2E.Win]::IsZoomed($h); visible = [E2E.Win]::IsWindowVisible($h) } | ConvertTo-Json -Compress`);
  const text = out.trim();
  return text === "null" || !text ? null : JSON.parse(text);
}

/** 移动并缩放主窗口（物理像素），模拟用户拖动窗口。 */
export function moveNativeWindow(pid, x, y, width, height) {
  powershell(`${WIN32}
${hwndOf(pid)}
[void][E2E.Win]::ShowWindow($h, 9)
[void][E2E.Win]::MoveWindow($h, ${x}, ${y}, ${width}, ${height}, $true)`);
}

/** 最小化主窗口。 */
export function minimizeNativeWindow(pid) {
  powershell(`${WIN32}
${hwndOf(pid)}
[void][E2E.Win]::ShowWindow($h, 6)`);
}

/**
 * 系统标题栏实际显示的颜色：截取窗口（含标题栏）保存到 file，取标题栏中段一条横带（第 10 行物理像素、宽度的 40%～55%，
 * 避开左侧的标题文字与右侧的按钮）的平均颜色，返回 [r, g, b]。
 * （DwmGetWindowAttribute 不能读回 DWMWA_CAPTION_COLOR 等属性，返回 E_INVALIDARG，所以看真实像素。）
 */
export async function titlebarColor(pid, file) {
  const info = nativeWindow(pid);
  if (!info) throw new Error("找不到应用窗口");
  captureNativeWindow(pid, file);
  const x = Math.round(info.width * 0.4);
  const w = Math.round(info.width * 0.15);
  const { ffmpeg: bin } = ffmpegPaths();
  const buf = await run(
    bin,
    ["-hide_banner", "-loglevel", "error", "-i", file, "-vf", `crop=${w}:1:${x}:10,scale=1:1:flags=area,format=rgb24`, "-f", "rawvideo", "-"],
    { encoding: "buffer" },
  );
  return [...buf.subarray(0, 3)];
}

/** 截取屏幕上主窗口所在的矩形（含系统标题栏，真实显示的像素），保存为 PNG。 */
export function captureNativeWindow(pid, file) {
  powershell(`${WIN32}
Add-Type -AssemblyName System.Drawing
${hwndOf(pid)}
$r = New-Object E2E.Win+RECT
[void][E2E.Win]::GetWindowRect($h, [ref]$r)
$w = $r.Right - $r.Left; $hh = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save('${file.replace(/'/g, "''")}', [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()`);
}
