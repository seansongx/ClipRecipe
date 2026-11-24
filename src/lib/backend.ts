// 与 Rust 后端通信的封装。命令定义见 src-tauri/src/commands/，类型与后端 serde 结构一致（camelCase）。
// 在浏览器中直接打开（isTauri 为 false）时，命令会以“需要桌面应用”的错误拒绝，事件监听为空操作。

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import { findSimulated } from "./simulated";

/** 是否运行在 Tauri 窗口中（`npm run dev` 直接用浏览器打开时为 false）。 */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const NOT_IN_TAURI = "此功能需要在桌面应用中使用";

function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) return Promise.reject(new Error(NOT_IN_TAURI));
  if (import.meta.env.DEV) {
    // 开发环境调试面板的模拟数据不发给后端（lib/simulated.ts）；生产构建里这段不存在
    const hit = findSimulated(args);
    if (hit) return Promise.reject(new Error(`这是调试面板模拟的数据，不会发给后端（${command}：${hit}）`));
  }
  return invoke<T>(command, args);
}

// ---------- 通用模型（src-tauri/src/model） ----------

/** 范围值：两端相等即固定值。 */
export interface Range {
  min: number;
  max: number;
}

export type PickOrder = "random" | "sequential";

/** 素材来源：单文件或目录。 */
export type AssetSource =
  | { mode: "file"; path: string }
  | { mode: "directory"; path: string; recursive: boolean; order: PickOrder };

// ---------- 媒体信息（src-tauri/src/ffmpeg/probe.rs） ----------

export interface VideoStream {
  index: number;
  codec: string;
  width: number;
  height: number;
  fps: number;
  fpsText: string;
  bitRate: number | null;
  pixFmt: string;
  /** 播放时需顺时针旋转的角度 */
  rotation: number;
  frames: number | null;
  /** 色彩传输特性：smpte2084（PQ）/ arib-std-b67（HLG）为 HDR；未知为空 */
  colorTransfer: string;
  colorPrimaries: string;
  colorSpace: string;
}

export interface AudioStream {
  index: number;
  codec: string;
  channels: number;
  sampleRate: number;
  bitRate: number | null;
  language: string | null;
  title: string | null;
}

export interface SubtitleStream {
  index: number;
  codec: string;
  language: string | null;
  title: string | null;
}

export interface MediaInfo {
  path: string;
  formatName: string;
  duration: number;
  size: number;
  bitRate: number | null;
  video: VideoStream | null;
  audio: AudioStream[];
  subtitles: SubtitleStream[];
  hasCover: boolean;
}

// ---------- 设置（src-tauri/src/model/settings.rs） ----------

export type TargetFormat = "mp4" | "source";
export type OnExists = "rename" | "overwrite";
/** set.after-all：只对一次运行有效（每次启动恢复为 none，触发后也改回 none） */
export type AfterAll = "none" | "shutdown" | "quit";
export type ThemeSetting = "system" | "light" | "dark";

/** 烧录字幕的样式；尺寸以 1080 高的画面为基准，按实际画面等比缩放 */
export interface SubtitleStyle {
  /** 字体文件；空为自动选择支持中文的系统字体 */
  font: string;
  size: number;
  color: string;
  outlineColor: string;
  outline: number;
  marginBottom: number;
}

export interface Settings {
  version: number;
  // 4.1 转换
  targetFormat: TargetFormat;
  onExists: OnExists;
  nameStrip: { text: string; regex: boolean };
  parallel: number;
  hevc: boolean;
  skipDone: boolean;
  autoSubtitle: boolean;
  subtitleStyle: SubtitleStyle;
  autoCover: boolean;
  /** 封面嵌入帧数：不超过帧率时盖住开头这几帧，超过时生成这么多帧的片段接在开头 */
  coverFrames: number;
  coverStretch: boolean;
  /** set.hdr：HDR 转 SDR；false 时保留 HDR（10 bit H.265） */
  hdrToSdr: boolean;
  /** set.blur-center：模块参数 blurCenter 为 null 时使用 */
  blurCenter: boolean;
  // 4.2 通用
  gpu: boolean;
  preferIntegratedGpu: boolean;
  gpuHintDismissed: boolean;
  numberPrefix: boolean;
  /** 导入文件夹包含子目录；也是新建目录素材“包含子目录”的初始值 */
  recursive: boolean;
  outByFolder: boolean;
  outByName: boolean;
  afterAll: AfterAll;
  openOutput: boolean;
  sound: boolean;
  soundPath: string;
  deleteSource: boolean;
  deleteToTrash: boolean;
  moveBack: boolean;
  removeFinished: boolean;
  confirmRemove: boolean;
  keepQueue: boolean;
  autoUpdate: boolean;
  tray: boolean;
  mutePreview: boolean;
  theme: ThemeSetting;
  // 4.3 高级
  /** set.fission-folder：每次裂变输出到“第 x 次裂变”目录 */
  fissionFolder: boolean;
  speedFit: boolean;
  reduceSize: boolean;
  copyright: string;
  outputDir: string;
}

/** 与后端 `Settings::default()` 一致（tests/recipe_defaults.rs 会检查）。 */
export const DEFAULT_SETTINGS: Settings = {
  version: 1,
  targetFormat: "mp4",
  onExists: "rename",
  nameStrip: { text: "", regex: false },
  parallel: 2,
  hevc: false,
  skipDone: true,
  autoSubtitle: false,
  subtitleStyle: {
    font: "",
    size: 48,
    color: "#ffffff",
    outlineColor: "#000000",
    outline: 2,
    marginBottom: 60,
  },
  autoCover: false,
  coverFrames: 1,
  coverStretch: true,
  hdrToSdr: true,
  blurCenter: true,
  gpu: true,
  preferIntegratedGpu: false,
  gpuHintDismissed: false,
  numberPrefix: false,
  recursive: true,
  outByFolder: false,
  outByName: false,
  afterAll: "none",
  openOutput: false,
  sound: true,
  soundPath: "",
  deleteSource: false,
  deleteToTrash: true,
  moveBack: false,
  removeFinished: false,
  confirmRemove: true,
  keepQueue: true,
  autoUpdate: false,
  tray: false,
  mutePreview: false,
  theme: "system",
  fissionFolder: false,
  speedFit: false,
  reduceSize: false,
  copyright: "",
  outputDir: "",
};

// ---------- 配方（src-tauri/src/model/recipe.rs） ----------

/** 每个模块的参数至少有 enabled；其余字段见 features/pipeline/<模块>.tsx。 */
export interface ModuleParams {
  enabled: boolean;
}

export interface Recipe {
  version: number;
  /** pipe.fission：每个视频生成几个输出（1 表示不裂变） */
  fission: number;
  /** 键为功能 ID，如 "pipe.fps" */
  modules: Record<string, ModuleParams>;
}

export const RECIPE_VERSION = 1;
/** 裂变次数上限（与后端 MAX_FISSION 一致） */
export const MAX_FISSION = 100;

/** 读取配方文件的结果：缺失 / 无效的内容已取默认值，warnings 说明了哪些被忽略。 */
export interface LoadedRecipe {
  recipe: Recipe;
  warnings: string[];
}

// ---------- 任务（src-tauri/src/jobs） ----------

export type TaskState = "waiting" | "running" | "done" | "failed" | "cancelled";
export type LogLevel = "debug" | "info" | "warn" | "error";

export interface TaskSpec {
  id: string;
  path: string;
  sourceRoot: string | null;
  info: MediaInfo | null;
  /** 队列序号（从 1 起） */
  number: number;
  /** 关联的字幕（烧录进画面）与封面（queue.sidecar） */
  subtitle?: string | null;
  cover?: string | null;
}

export type JobEvent =
  | {
      type: "taskState";
      taskId: string;
      state: TaskState;
      /** 完成时的输出（裂变时为最后一个） */
      output: string | null;
      error: string | null;
      /** 全部输出（裂变时有多个） */
      outputs: string[];
    }
  | {
      type: "progress";
      taskId: string;
      /** 整个任务（含全部裂变轮次）的进度 */
      percent: number;
      outTime: number;
      speed: number;
      fps: number;
      /** 当前第几次裂变（从 1 起）与总次数 */
      round: number;
      rounds: number;
    }
  | { type: "log"; level: LogLevel; taskId: string | null; message: string }
  | { type: "runFinished"; done: number; failed: number; cancelled: number };

export interface ImportedItem {
  path: string;
  sourceRoot: string | null;
  info: MediaInfo | null;
  error: string | null;
  /** 自动关联的同名字幕 / 封面 */
  subtitle: string | null;
  cover: string | null;
}

export interface ImportResult {
  items: ImportedItem[];
  skipped: number;
  errors: string[];
}

export interface ImportProgress {
  done: number;
  total: number;
}

// ---------- 命令 ----------

export interface AppInfo {
  name: string;
  version: string;
  cpuCores: number;
  /** 编译目标的操作系统（Rust 的 std::env::consts::OS）；前端只经 lib/platform.ts 读取平台 */
  platform: string;
}

/** 实际使用的 FFmpeg 从哪里找到（顺序见 src-tauri/src/ffmpeg/mod.rs 的 locate_detailed）。 */
export type FfmpegSource = "env" | "managed" | "bundled" | "path" | "dev" | "missing";

/** 应用内下载 FFmpeg 的状态（ffmpeg::ManagedStatus）。 */
export interface ManagedFfmpegStatus {
  /** 本平台能否下载 */
  supported: boolean;
  unsupportedReason: string | null;
  /** 本程序固定的 FFmpeg 版本与 npm 包版本 */
  pinnedVersion: string;
  packageVersion: string;
  /** 需要下载的字节数（两个压缩包） */
  downloadSize: number;
  /** 已下载的副本（不论是否正在使用） */
  installedVersion: string | null;
  installedDir: string | null;
  /** 已下载的副本比固定版本旧 */
  updateAvailable: boolean;
  root: string | null;
}

export interface FfmpegStatus {
  available: boolean;
  path: string;
  ffprobePath: string;
  version: string | null;
  /** `ffprobe -version` 的首行 */
  ffprobeVersion: string | null;
  error: string | null;
  source: FfmpegSource;
  /** 当前 FFmpeg 有 whisper 滤镜（语音生成字幕的内置引擎） */
  whisper: boolean;
  managed: ManagedFfmpegStatus;
}

/** `ffmpeg-download` 事件：下载进度（ffmpeg::managed::DownloadProgress）。 */
export interface FfmpegDownloadProgress {
  phase: "download" | "extract" | "install";
  tool: "ffmpeg" | "ffprobe";
  /** 当前下载地址的主机名 */
  source: string;
  downloaded: number;
  total: number;
  bytesPerSec: number;
}

export interface FfmpegDownloadOutcome {
  cancelled: boolean;
  status: FfmpegStatus;
}

export interface EncoderInfo {
  hardware: string[];
  h264: string;
  hevc: string;
  /** 装了显卡驱动但硬件编码器试编码全部失败的厂商（可能驱动过旧） */
  failedVendors: string[];
}

export interface PreviewResponse {
  commandLine: string;
  args: string[];
  output: string;
  encoder: string;
  warnings: string[];
}

export interface OutputDirCheck {
  exists: boolean;
  sameAsSource: boolean;
}

export const getAppInfo = () => call<AppInfo>("app_info");
export const getFfmpegStatus = () => call<FfmpegStatus>("ffmpeg_status");
export const getEncoderInfo = (refresh = false) => call<EncoderInfo>("encoder_info", { refresh });
/** 已经探测过的编码器信息；还没探测过时为 null（不会触发探测）。 */
export const getCachedEncoderInfo = () => call<EncoderInfo | null>("encoder_info_cached");

// ---------- 应用内下载 FFmpeg（src-tauri/src/commands/ffmpeg.rs） ----------

/** 下载并安装固定版本的 FFmpeg；只在用户点击时调用。失败时以错误信息拒绝。 */
export const downloadFfmpeg = () => call<FfmpegDownloadOutcome>("download_ffmpeg");
export const cancelFfmpegDownload = () => call<boolean>("cancel_ffmpeg_download");
export const removeManagedFfmpeg = () => call<FfmpegStatus>("remove_managed_ffmpeg");
/** 重新查找 FFmpeg（清除编码器探测缓存）。 */
export const redetectFfmpeg = () => call<FfmpegStatus>("redetect_ffmpeg");
export const onFfmpegDownload = (handler: (progress: FfmpegDownloadProgress) => void) =>
  subscribe("ffmpeg-download", handler);

export interface SidecarOptions {
  subtitle: boolean;
  cover: boolean;
}

export const importPaths = (paths: string[], recursive: boolean, sidecars?: SidecarOptions) =>
  call<ImportResult>("import_paths", { paths, recursive, sidecars: sidecars ?? null });
export const probeMedia = (path: string) => call<MediaInfo>("probe_media", { path });

export const loadSettings = () => call<Settings>("load_settings");
export const saveSettings = (settings: Settings) => call<void>("save_settings", { settings });
export const getDefaultRecipe = () => call<Recipe>("default_recipe");
export const getDataDir = () => call<string>("data_dir");

export const startJobs = (request: {
  tasks: TaskSpec[];
  recipe: Recipe;
  settings: Settings;
  outputDir: string;
  seed?: number;
}) => call<{ seed: number }>("start_jobs", { request });
export const stopJobs = () => call<void>("stop_jobs");
export const cancelTask = (id: string) => call<boolean>("cancel_task", { id });
export const jobsRunning = () => call<boolean>("jobs_running");
export const previewCommand = (request: {
  path: string;
  info: MediaInfo | null;
  recipe: Recipe;
  settings: Settings;
  outputDir: string;
  number: number;
}) => call<PreviewResponse>("preview_command", { request });
export const checkOutputDir = (outputDir: string, sources: string[]) =>
  call<OutputDirCheck>("check_output_dir", { outputDir, sources });
/** 插队：把等待中的任务移到最前面（处理中才有意义）。 */
export const prioritizeTask = (id: string) => call<boolean>("prioritize_task", { id });

// ---------- 配方文件（pipe.recipe） ----------

export const RECIPE_FILE_EXTENSION = "json";

export const saveRecipeFile = (path: string, recipe: Recipe) =>
  call<void>("save_recipe_file", { path, recipe });
export const loadRecipeFile = (path: string) => call<LoadedRecipe>("load_recipe_file", { path });

/** 是否是配方文件（拖放时与视频区分）。 */
export const isRecipeFile = (path: string) => path.toLowerCase().endsWith(`.${RECIPE_FILE_EXTENSION}`);

// ---------- 预览与取帧（pipe.preview、区域框选） ----------

/** 预览输出：片段（输出时间轴上 start 起 duration 秒）或单帧。 */
export type PreviewOutput =
  | { kind: "clip"; start: number; duration: number }
  | { kind: "frame"; time: number };

export interface PreviewSpec {
  path: string;
  info: MediaInfo | null;
  recipe: Recipe;
  settings: Settings;
  /** 只预览这一个模块；null 为整条流水线（变速、背景音、片头尾、帧率、码率除外） */
  onlyModule: string | null;
  output: PreviewOutput;
  seed?: number;
  /** 队列序号（从 1 起） */
  number: number;
  /** 画面最大高度；0 为默认（480） */
  maxHeight?: number;
  /** 队列项关联的字幕（预览时同样烧录） */
  subtitle?: string | null;
}

export interface PreviewResult {
  /** 预览文件（mp4 / jpg），用 mediaUrl() 转成可显示的地址 */
  path: string;
  duration: number;
  width: number;
  height: number;
  seed: number;
  /** 实际参与预览的模块 */
  modules: string[];
  warnings: string[];
}

export interface FrameResult {
  path: string;
  /** 源视频（旋转后）的显示宽高：区域坐标以此为准 */
  width: number;
  height: number;
  time: number;
}

export const renderPreview = (spec: PreviewSpec) => call<PreviewResult>("render_preview", { spec });
export const cancelPreview = () => call<void>("cancel_preview");
export const extractFrame = (path: string, info: MediaInfo | null, time: number) =>
  call<FrameResult>("extract_frame", { path, info, time });
/** 删除不再需要的预览 / 取帧文件（失败时忽略）。 */
export const releasePreviewFile = (path: string) => {
  if (isTauri) void call<void>("release_preview_file", { path }).catch(() => {});
};
/** 允许 WebView 读取某个源视频（播放用）。 */
export const allowMediaFile = (path: string) => call<string>("allow_media_file", { path });

/** 本地文件 → WebView 可加载的地址（asset 协议）。 */
export const mediaUrl = (path: string) => (isTauri ? convertFileSrc(path) : "");

export const onPreviewProgress = (handler: (progress: { percent: number }) => void) =>
  subscribe("preview-progress", handler);

// ---------- 事件 ----------

function subscribe<T>(event: string, handler: (payload: T) => void): () => void {
  if (!isTauri) return () => {};
  const unlisten = listen<T>(event, (e) => handler(e.payload));
  return () => {
    void unlisten.then((stop) => stop());
  };
}

export const onJobEvent = (handler: (event: JobEvent) => void) => subscribe("job-event", handler);
export const onImportProgress = (handler: (progress: ImportProgress) => void) =>
  subscribe("import-progress", handler);

// ---------- 文件对话框与拖放 ----------

export const VIDEO_EXTENSIONS = [
  "mp4", "mkv", "mov", "avi", "flv", "wmv", "webm", "ts", "m4v", "mpg", "mpeg", "3gp",
];

export async function pickVideos(): Promise<string[]> {
  if (!isTauri) return [];
  const picked = await open({
    multiple: true,
    filters: [{ name: "视频", extensions: VIDEO_EXTENSIONS }],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

export async function pickFile(name: string, extensions: string[]): Promise<string | null> {
  if (!isTauri) return null;
  const picked = await open({ multiple: false, filters: [{ name, extensions }] });
  return typeof picked === "string" ? picked : null;
}

export async function pickFolder(): Promise<string | null> {
  if (!isTauri) return null;
  const picked = await open({ directory: true });
  return typeof picked === "string" ? picked : null;
}

/** 监听拖入窗口的文件；返回取消监听的函数。 */
export function onFilesDropped(
  onDrop: (paths: string[]) => void,
  onHover: (hovering: boolean) => void,
): () => void {
  return onDragDrop((event) => {
    onHover(event.type === "enter" || event.type === "over");
    if (event.type === "drop") onDrop(event.paths);
  });
}

/** 拖放事件；`x`、`y` 为相对窗口内容区的 CSS 像素（可与 getBoundingClientRect 比较）。 */
export type DragDropEvent =
  | { type: "enter"; paths: string[]; x: number; y: number }
  | { type: "over"; x: number; y: number }
  | { type: "drop"; paths: string[]; x: number; y: number }
  | { type: "leave" };

/** 监听拖放（带位置），用于把文件拖到特定区域（如输出位置）。返回取消监听的函数。 */
export function onDragDrop(handler: (event: DragDropEvent) => void): () => void {
  if (!isTauri) return () => {};
  const unlisten = getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "leave") {
      handler({ type: "leave" });
      return;
    }
    // Tauri 给出物理像素，换算成 CSS 像素
    const ratio = window.devicePixelRatio || 1;
    const x = p.position.x / ratio;
    const y = p.position.y / ratio;
    if (p.type === "over") handler({ type: "over", x, y });
    else handler({ type: p.type, paths: p.paths, x, y });
  });
  return () => {
    void unlisten.then((stop) => stop());
  };
}

/** 点 (x, y) 是否落在元素内。 */
export function isPointInElement(element: Element | null, x: number, y: number): boolean {
  if (!element) return false;
  const r = element.getBoundingClientRect();
  return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
}

export async function pickRecipeFile(): Promise<string | null> {
  return pickFile("ClipRecipe 配方", [RECIPE_FILE_EXTENSION]);
}

export async function pickRecipeSavePath(defaultName: string): Promise<string | null> {
  if (!isTauri) return null;
  const picked = await save({
    defaultPath: defaultName,
    filters: [{ name: "ClipRecipe 配方", extensions: [RECIPE_FILE_EXTENSION] }],
  });
  return picked ?? null;
}

// ---------- 系统字体（src-tauri/src/commands/fonts.rs） ----------

export interface SystemFont {
  /** 显示名称（优先中文） */
  name: string;
  /** 字体族（英文） */
  family: string;
  path: string;
  /** 是否支持中文 */
  cjk: boolean;
}

/** 列出系统字体（支持中文的在前）。 */
export function listSystemFonts(): Promise<SystemFont[]> {
  return call("list_system_fonts");
}

// ---------- 队列保存与同名资源（queue.persist、queue.sidecar） ----------

export interface SavedTask {
  path: string;
  sourceRoot: string | null;
  info: MediaInfo | null;
  subtitle: string | null;
  cover: string | null;
}

export interface RestoredQueue {
  items: ImportedItem[];
  /** 已不存在、被丢弃的视频 */
  missing: string[];
  notes: string[];
}

export const saveQueue = (tasks: SavedTask[]) => call<void>("save_queue", { tasks });
export const restoreQueue = () => call<RestoredQueue>("restore_queue");
export const findSidecars = (paths: string[], options: SidecarOptions) =>
  call<{ subtitle: string | null; cover: string | null }[]>("find_sidecars", { paths, options });

export const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa", "vtt"];
export const COVER_EXTENSIONS = ["jpg", "jpeg", "png", "webp", "bmp"];

// ---------- 系统（src-tauri/src/commands/system.rs） ----------

export interface UpdateInfo {
  current: string;
  latest: string;
  newer: boolean;
  /** 发布页（本项目 GitHub 下的地址） */
  url: string;
  /** 更新说明（发布说明，Markdown 原文；没有时为 null） */
  notes: string | null;
  /** 能否在程序内下载并安装；否则只能打开发布页 */
  auto: boolean;
  /** 不能自动更新的原因（未配置公钥的构建、deb / rpm 安装、开发版，或读取 latest.json 失败） */
  manualReason: string | null;
}

/** 自动更新的下载进度（字节；total 未知时为 null）。 */
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

export const scheduleShutdown = () => call<void>("schedule_shutdown");
export const cancelShutdown = () => call<void>("cancel_shutdown");
export const quitApp = () => call<void>("quit_app");
/** 正在处理时关闭主窗口或从托盘退出：后端已阻止退出，由界面确认后调用 quitApp。 */
export const onCloseRequested = (handler: () => void) => subscribe("close-requested", handler);
/** 已显示退出确认框（告诉后端界面正常；否则短时间内再次关闭会直接退出）。 */
export const closePromptShown = () => call<void>("close_prompt_shown");
/** 用户在退出确认框里选择继续处理。 */
export const closePromptDismissed = () => call<void>("close_prompt_dismissed");
/** 打开文件夹（传入文件时打开其所在文件夹）。 */
export const openFolder = (path: string) => call<void>("open_folder", { path });
export const openDataDir = () => call<void>("open_data_dir");
/** 打开本项目的 GitHub 页面（默认仓库首页）。 */
export const openProjectPage = (url?: string) => call<void>("open_project_page", { url: url ?? null });
export const checkUpdate = () => call<UpdateInfo>("check_update");
/** 下载并安装检查到的更新（立即返回；进度与结果见 onUpdateProgress / onUpdateInstalling / onUpdateFailed），安装后程序重启。 */
export const installUpdate = () => call<void>("install_update");
/** 取消正在进行的更新下载，返回是否取消了（安装开始后不能取消）。 */
export const cancelUpdate = () => call<boolean>("cancel_update");
export const onUpdateProgress = (handler: (progress: UpdateProgress) => void) => subscribe("update-progress", handler);
export const onUpdateInstalling = (handler: () => void) => subscribe("update-installing", handler);
export const onUpdateFailed = (handler: (error: string) => void) => subscribe("update-failed", handler);
/** 允许 WebView 播放自定义提示音（wav），返回原路径。 */
export const allowSoundFile = (path: string) => call<string>("allow_sound_file", { path });

export const onShutdownCountdown = (handler: (tick: { remaining: number }) => void) =>
  subscribe("shutdown-countdown", handler);
export const onShutdownFailed = (handler: (error: string) => void) => subscribe("shutdown-failed", handler);

// ---------- 主窗口（src-tauri/src/commands/window.rs，实现见 src-tauri/src/desktop.rs） ----------

/** 窗口配色：设置里的主题偏好、当前是否深色、主题变量的实际值（#rrggbb）。 */
export interface WindowChrome {
  preference: ThemeSetting;
  dark: boolean;
  titlebar: string;
  text: string;
  border: string;
  background: string;
}

export type TaskbarProgressState = "none" | "normal" | "indeterminate" | "paused" | "error";

/** 首帧已画好：第一次调用时显示主窗口。 */
export const windowReady = () => call<void>("window_ready");
/** 按应用主题设置窗口主题、底色与标题栏颜色。 */
export const setWindowChrome = (chrome: WindowChrome) => call<void>("set_window_chrome", { chrome });
/** 任务栏按钮上的进度（progress 为 0～100）。 */
export const setTaskbarProgress = (state: TaskbarProgressState, progress?: number) =>
  call<void>("set_taskbar_progress", { state, progress: progress === undefined ? null : Math.round(progress) });
/** 首次启动时命令行带的文件（只返回一次）。 */
export const takeLaunchPaths = () => call<string[]>("take_launch_paths");
/** 再次启动程序时命令行里的文件（单实例转交）。 */
export const onOpenPaths = (handler: (paths: string[]) => void) => subscribe("open-paths", handler);
