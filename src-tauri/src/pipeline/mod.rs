//! 流水线：把 MediaInfo + Recipe + Settings + 随机种子 变成一条完整的 FFmpeg 命令。
//!
//! - 每个 `pipe.*` 模块在 `modules/` 下有自己的文件，实现 [`PipelineModule`]；
//! - 模块之间互不依赖，应用顺序只在 [`ordered_modules`] 中定义；
//! - 每个模块拿到由“种子 + 模块 ID”派生的独立随机数（[`BuildContext::rng`]），
//!   增删其它模块不会改变它的取值。

pub mod graph;
pub mod modules;
pub mod speed_fit;
pub mod steps;
pub mod temp;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, EncoderCaps, MediaInfo, VideoCodec, VideoEncoder};
use crate::model::Recipe;
use crate::model::{RecipeModules, SeededRng, Settings, TargetFormat};

pub use graph::{FilterGraph, RenderedGraph};
pub use steps::Sidecar;
pub use temp::{TempFiles, TempGuard};

/// 一个流水线模块。实现者是该模块的参数结构体。
pub trait PipelineModule {
    /// 功能 ID，与 registry.ts、docs/specs/ 一致，如 `pipe.fps`。
    fn id(&self) -> &'static str;

    /// 参数里的开关。
    fn enabled(&self) -> bool;

    /// 是否已实现。占位模块返回 false，构建时会被跳过并给出警告。
    fn implemented(&self) -> bool {
        true
    }

    /// 把自己的效果加到构建上下文：追加滤镜、额外输入、编码参数，并更新 `ctx.stream`。
    /// 参数无效时返回中文错误信息（会显示给用户）。
    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String>;

    /// 预览时能否直接定位到要看的时间点（见 [`BuildContext::preview_seek`]）。
    /// 效果取决于“从视频开头数起的帧序号”、又没有按定位起点修正的模块返回 false，
    /// 此时预览退回到从头解码（结果仍然正确，只是时间点越靠后越慢）。
    fn preview_seekable(&self) -> bool {
        true
    }
}

/// 预览定位起点（秒）在构建过程中的占位标记：起点要等模块都应用完才能确定（受去头尾影响），
/// 构建结束时统一替换为数字（见 [`BuildContext::preview_seek`]）。
const SEEK_MARKER: &str = "@@cliprecipe-seek@@";
/// 素材（额外输入）在预览定位时的起点的占位标记：定位起点向下取整到整秒（见 [`extras_start`]）。
const SEEK_EXTRAS_MARKER: &str = "@@cliprecipe-seek-extras@@";

/// 预览定位时素材的起点：不晚于主视频流的定位起点，并且是整秒。
///
/// 素材必须在主视频流的第一帧之前（或同时）就有画面，否则那一帧上没有它（overlay 只在素材出过帧之后才叠加）。
/// 素材的时间戳要换算到它自己的时间基，起点不是整秒时（如 10.03 秒在 1/25 的时间基里是 250.75）会被取整，
/// 可能落到主视频流第一帧之后——所以取整秒：在任何常见的时间基里都能精确表示。多出来的不到一秒由 FFmpeg 跳过。
fn extras_start(start: f64) -> f64 {
    start.floor().max(0.0)
}

/// 全部模块 ID（顺序同 registry.ts）。
pub const MODULE_IDS: [&str; 18] = [
    "pipe.remove-watermark",
    "pipe.add-watermark",
    "pipe.pip",
    "pipe.text",
    "pipe.crop",
    "pipe.resolution",
    "pipe.rotate-flip",
    "pipe.adjust",
    "pipe.grid",
    "pipe.zoom",
    "pipe.trim",
    "pipe.intro-outro",
    "pipe.speed",
    "pipe.drop-frames",
    "pipe.dedupe-frames",
    "pipe.bgm",
    "pipe.fps",
    "pipe.bitrate",
];

/// 模块的应用顺序（集中定义于此）。
///
/// 1. 时间裁剪与帧处理（去头尾 → 重复帧 → 抽帧 → 变速），先减少后续要处理的帧；
/// 2. 在原始坐标系上去水印，再裁剪、旋转、缩放动画、调色、宫格、分辨率；
/// 3. 叠加类（画中画 → 水印 → 文本），坐标基于最终画面；
/// 4. 片头片尾与相框、背景音；
/// 5. 编码类（帧率、码率）最后。
pub fn ordered_modules(m: &RecipeModules) -> [&dyn PipelineModule; 18] {
    [
        &m.trim,
        &m.dedupe_frames,
        &m.drop_frames,
        &m.speed,
        &m.remove_watermark,
        &m.crop,
        &m.rotate_flip,
        &m.zoom,
        &m.adjust,
        &m.grid,
        &m.resolution,
        &m.pip,
        &m.add_watermark,
        &m.text,
        &m.intro_outro,
        &m.bgm,
        &m.fps,
        &m.bitrate,
    ]
}

/// 模块执行过程中流的当前状态。改变尺寸 / 帧率 / 时长的模块必须同步更新。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamState {
    /// 当前画面宽高（已考虑源视频的旋转）
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// 预计输出时长（秒），用于计算进度
    pub duration: f64,
    /// 输出音频的声道数（决定 AAC 码率）；改变声道布局的模块（如背景音统一为立体声）需要更新
    pub audio_channels: u32,
    /// 视频是否已变为可变帧率（如“重复帧移除”只丢帧不重排时间戳）。
    /// 假定输入为恒定帧率、按帧序号重排时间的模块应先调用 [`BuildContext::ensure_cfr`]。
    pub vfr: bool,
}

/// 读取素材媒体信息的函数（见 [`BuildContext::probe`]）。
pub type ProbeFn<'a> = &'a dyn Fn(&Path) -> Result<MediaInfo, String>;

/// 编码相关参数。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EncodeParams {
    /// 目标视频码率（kbit/s）；None 时沿用源码率，源码率未知则用恒定质量
    pub video_kbps: Option<u64>,
    /// 额外的输出参数，放在输出文件之前
    pub output_args: Vec<String>,
}

/// 一个输入文件及其前置参数（如 `-ss 3`、`-stream_loop -1`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputSpec {
    pub args: Vec<String>,
    pub path: String,
}

/// 构建上下文：模块通过它修改命令。
pub struct BuildContext<'a> {
    pub info: &'a MediaInfo,
    pub settings: &'a Settings,
    /// 队列中的序号（从 0 起），用于素材“依次取”
    pub task_index: usize,
    /// 当前模块专用的随机数（由种子 + 模块 ID 派生）
    pub rng: SeededRng,
    pub graph: FilterGraph,
    /// 主输入（第 0 个输入）的前置参数
    pub main_input_args: Vec<String>,
    /// 额外输入；第 i 个的 FFmpeg 输入序号为 i + 1
    pub extra_inputs: Vec<InputSpec>,
    pub encode: EncodeParams,
    pub stream: StreamState,
    warnings: Vec<String>,
    current_module: &'static str,
    /// 本次构建中启用且已实现的模块（按应用顺序）
    enabled_modules: Vec<&'static str>,
    probe_fn: Option<ProbeFn<'a>>,
    /// 模块登记的临时文件（见 [`temp`]）
    temp: TempFiles,
    preview: bool,
    /// 预览是否直接定位到要看的时间点（见 [`Self::preview_seek`]）
    seek: bool,
    /// 烧录字幕的滤镜（见 [`steps::begin_subtitle`]）
    subtitle_filter: Option<String>,
    /// 源画面按方形像素显示的宽高（区域框选等“源视频像素”坐标以此为准）
    source_size: (u32, u32),
}

impl<'a> BuildContext<'a> {
    pub fn new(info: &'a MediaInfo, settings: &'a Settings, task_index: usize) -> Self {
        let video = info.video.clone().unwrap_or_default();
        // 变形（非方形像素）源在所有模块之前换算为方形像素（见 `steps::normalize_pixels`），尺寸以此为准
        let (width, height) = video.square_display_size();
        let audio_channels = info.audio.first().map_or(0, |a| a.channels.max(1));
        Self {
            info,
            settings,
            task_index,
            rng: SeededRng::new(0),
            graph: FilterGraph::new(!info.audio.is_empty()),
            main_input_args: Vec::new(),
            extra_inputs: Vec::new(),
            encode: EncodeParams::default(),
            stream: StreamState {
                width,
                height,
                fps: video.fps,
                duration: info.duration,
                audio_channels,
                vfr: false,
            },
            warnings: Vec::new(),
            current_module: "",
            enabled_modules: Vec::new(),
            probe_fn: None,
            temp: TempFiles::default(),
            preview: false,
            seek: false,
            subtitle_filter: None,
            source_size: (width, height),
        }
    }

    /// 源画面的宽高：已按旋转转正、按方形像素换算（变形源的宽度乘以 SAR）。区域框选、位置选取等
    /// “源视频像素”坐标都以此为准（与取帧 `preview::extract_frame` 返回的尺寸一致）。
    pub fn source_size(&self) -> (u32, u32) {
        self.source_size
    }

    /// 是否在构建预览（`pipe.preview` / 单模块预览）。只在预览里显示的辅助标记（如去水印的区域框线）据此添加，
    /// 正式输出里绝不能出现。
    pub fn is_preview(&self) -> bool {
        self.preview
    }

    /// 预览直接定位到要看的时间点时，返回定位起点（输出时间轴上的秒数）在滤镜文本里的占位写法，
    /// 可以直接写进表达式（如 `setpts=PTS+{起点}/TB`），构建结束时会换成数字；正式输出与从头解码的预览返回 None。
    ///
    /// 定位时主输入只从起点开始解码，但主视频流的时间戳已经加回起点，所以按时间 `t` 写的表达式不用管它；
    /// 只有**按帧序号**计算、或会把时间戳重排为从 0 开始的模块需要据此修正（例：`zoom.rs`），
    /// 修正不了的在 [`PipelineModule::preview_seekable`] 里返回 false。
    pub fn preview_seek(&self) -> Option<&'static str> {
        self.seek.then_some(SEEK_MARKER)
    }

    /// 把一条素材流的时间戳重排为“从视频开头开始”的滤镜：通常是 `setpts=PTS-STARTPTS`；
    /// 预览定位时主视频流从定位起点开始，素材也要从那附近开始（见 [`extras_start`]），
    /// 否则 FFmpeg 会先把素材从 0 处理到起点。
    pub fn restart_pts_filter(&self) -> String {
        if self.seek {
            format!("setpts=PTS-STARTPTS+{SEEK_EXTRAS_MARKER}/TB")
        } else {
            "setpts=PTS-STARTPTS".to_owned()
        }
    }

    /// 本次构建临时目录下的一个文件路径（目录第一次调用时创建；不创建文件本身）。
    /// 目录与其中的一切在任务（或预览）结束时删除——成功、失败、取消都一样，模块不用自己清理。
    /// `name` 只取文件名部分。
    pub fn temp_file(&mut self, name: &str) -> Result<PathBuf, String> {
        let file_name = Path::new(name)
            .file_name()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| format!("临时文件名无效：{name}"))?
            .to_owned();
        Ok(self.temp.dir()?.join(file_name))
    }

    /// 登记一个模块自己创建的临时文件或目录，任务（或预览）结束时删除。
    pub fn register_temp(&mut self, path: impl Into<PathBuf>) {
        self.temp.register(path.into());
    }

    /// 设置读取素材信息的方式（构建时由调度器 / 预览按 `RunConfig.paths` 注入；单元测试可传假函数）。
    pub fn set_probe(&mut self, probe: ProbeFn<'a>) {
        self.probe_fn = Some(probe);
    }

    /// 读取素材（片头片尾、背景音乐等）的媒体信息。没有注入读取方式时返回错误。
    pub fn probe(&self, path: &Path) -> Result<MediaInfo, String> {
        match self.probe_fn {
            Some(probe) => probe(path),
            None => Err("当前无法读取素材信息".into()),
        }
    }

    /// 是否能读取素材信息。
    pub fn can_probe(&self) -> bool {
        self.probe_fn.is_some()
    }

    /// 在当前模块之后、会被应用的模块 ID（启用且已实现，按应用顺序）。
    pub fn later_modules(&self) -> &[&'static str] {
        match self
            .enabled_modules
            .iter()
            .position(|id| *id == self.current_module)
        {
            Some(pos) => &self.enabled_modules[pos + 1..],
            None => &[],
        }
    }

    /// 前面的模块把视频变成了可变帧率时，用 `fps` 滤镜恢复为当前帧率的恒定帧率
    /// （被丢掉的帧由前一帧补上，时间轴不变）。按帧序号计算时间的模块（抽帧、动态缩放等）在开头调用。
    pub fn ensure_cfr(&mut self) {
        if !self.stream.vfr {
            return;
        }
        if let Some(rate) = self.rate_text() {
            self.graph.video(format!("fps={rate}"));
        }
        self.stream.vfr = false;
    }

    /// 当前帧率在滤镜参数里的写法（`fps=`、`r=` 等）：未被前面的模块改动时用源视频的分数原文
    /// （如 `30000/1001`；`25/1` 写作 `25`），保证时间戳精确；否则为十进制。帧率未知时为 None。
    pub fn rate_text(&self) -> Option<String> {
        if self.stream.fps <= 0.0 {
            return None;
        }
        if let Some(video) = &self.info.video {
            let text = video.fps_text.trim();
            let valid = text.contains('/') && !text.starts_with('0') && !text.ends_with("/0");
            if valid && (video.fps - self.stream.fps).abs() < 1e-6 {
                return Some(text.trim_end_matches("/1").to_owned());
            }
        }
        Some(fmt_num(self.stream.fps))
    }

    /// 添加一个额外输入，返回它的 FFmpeg 输入序号（主输入为 0）。
    ///
    /// 预览定位时（[`Self::preview_seek`]）素材的时间戳自动平移到定位起点之前最近的整秒（`-itsoffset`，见 [`extras_start`]）；
    /// 循环素材因此从第一帧开始播放，相位与正式输出可能不同。
    pub fn add_input(&mut self, mut args: Vec<String>, path: &Path) -> Result<usize, String> {
        if self.seek {
            args.splice(
                0..0,
                ["-itsoffset".to_owned(), SEEK_EXTRAS_MARKER.to_owned()],
            );
        }
        self.extra_inputs.push(InputSpec {
            args,
            path: path_arg(path)?,
        });
        Ok(self.extra_inputs.len())
    }

    /// 记录一条警告（会显示在处理记录里），自动带上模块 ID。
    pub fn warn(&mut self, message: impl AsRef<str>) {
        let message = message.as_ref();
        if self.current_module.is_empty() {
            self.warnings.push(message.to_owned());
        } else {
            self.warnings
                .push(format!("[{}] {message}", self.current_module));
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// 构建一条命令所需的全部输入。
pub struct BuildRequest<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub info: &'a MediaInfo,
    pub recipe: &'a Recipe,
    pub settings: &'a Settings,
    pub encoders: &'a EncoderCaps,
    pub seed: u64,
    /// 队列中的序号（从 0 起）
    pub task_index: usize,
    /// 强制软件编码（硬件编码失败后的重试）
    pub force_software: bool,
    /// 读取素材信息的方式（通常是 `ffmpeg::probe` + 本次运行的 ffprobe 路径）；None 时模块无法读取素材
    pub probe: Option<ProbeFn<'a>>,
    /// 队列项关联的字幕、封面（见 [`steps`]）
    pub sidecar: Sidecar<'a>,
}

/// 构建结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltCommand {
    /// 不含程序名的 ffmpeg 参数
    pub args: Vec<String>,
    /// 预计输出时长（秒）
    pub expected_duration: f64,
    pub encoder: VideoEncoder,
    pub warnings: Vec<String>,
    /// 模块登记的临时文件：命令要用到它们，所以要让它活到 ffmpeg 结束；最后一个克隆丢弃时删除。
    #[serde(skip)]
    pub temp_files: TempGuard,
}

impl BuiltCommand {
    /// 第一次出现 `flag` 后面的那个参数。
    pub fn arg_after(&self, flag: &str) -> Option<&str> {
        let pos = self.args.iter().position(|a| a == flag)?;
        self.args.get(pos + 1).map(String::as_str)
    }

    /// 便于日志显示的命令行（参数含空格时加引号）。
    pub fn command_line(&self, program: &str) -> String {
        std::iter::once(program.to_owned())
            .chain(self.args.iter().map(|a| {
                if a.is_empty() || a.contains([' ', ';', '[', '\'', '"']) {
                    format!("\"{}\"", a.replace('"', "\\\""))
                } else {
                    a.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 数字转滤镜参数：去掉多余的小数位，如 30.0 → `30`、29.970 → `29.97`。
pub fn fmt_num(value: f64) -> String {
    let s = format!("{value:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

/// 能装 H.264 / H.265 的容器（保持源格式时使用）。
const H26X_CONTAINERS: &[&str] = &["mp4", "mkv", "mov", "m4v", "ts", "flv", "avi", "3gp"];

/// 输出文件扩展名（小写，不带点）。
pub fn output_extension(settings: &Settings, input: &Path) -> String {
    if settings.target_format == TargetFormat::Source {
        if let Some(ext) = input.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if H26X_CONTAINERS.contains(&ext.as_str()) {
                return ext;
            }
        }
    }
    "mp4".to_owned()
}

/// 不重编码时能否直接复制音频流到目标容器。
fn can_copy_audio(codec: &str, container: &str) -> bool {
    match container {
        "mkv" => true,
        "mp4" | "mov" | "m4v" | "ts" => matches!(codec, "aac" | "mp3" | "ac3" | "eac3"),
        "flv" | "3gp" | "avi" => matches!(codec, "aac" | "mp3"),
        _ => false,
    }
}

/// 生成预览（`pipe.preview` 与单模块预览）时不参与的模块：
/// 变速、背景音、片头尾不适合截取片段预览，帧率、码率在低清预览里看不出效果。
pub const PREVIEW_EXCLUDED: [&str; 5] = [
    "pipe.speed",
    "pipe.bgm",
    "pipe.intro-outro",
    "pipe.fps",
    "pipe.bitrate",
];

/// 预览的输出形式。时间都是**输出**时间轴上的秒数（已考虑去头尾等模块）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewOutput {
    /// 从 `start` 起 `duration` 秒的低清 H.264 片段（mp4）
    Clip { start: f64, duration: f64 },
    /// `time` 处的一帧（jpg）
    Frame { time: f64 },
}

/// 预览构建选项，见 [`build_preview`]。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewOptions {
    pub output: PreviewOutput,
    /// 画面高度超过此值时缩小（宽度等比、取偶数）
    pub max_height: u32,
}

/// 构建完整的 ffmpeg 命令。
pub fn build(req: &BuildRequest) -> Result<BuiltCommand, String> {
    build_inner(req, None)
}

/// 构建预览命令：同样依次应用配方里的模块，但输出为低清片段或单帧，
/// 固定用 libx264 ultrafast（WebView 都能播放），不使用硬件编码。
///
/// 调用方负责决定哪些模块参与（见 [`PREVIEW_EXCLUDED`]）。
pub fn build_preview(req: &BuildRequest, options: &PreviewOptions) -> Result<BuiltCommand, String> {
    build_inner(req, Some(options))
}

/// AAC 编码码率（kbit/s）：按声道数给定，单声道不超过 96k。
///
/// FFmpeg 9.1 自带的 AAC 编码器在单声道约 176k～208k 时，遇到“数字静音后接纯音”会死循环、
/// 整条命令卡住；单声道 96k、立体声 192k 都不受影响，音质也足够。
pub fn aac_kbps(channels: u32) -> u32 {
    match channels {
        0 | 1 => 96,
        2 => 192,
        _ => 320,
    }
}

/// 把文件路径写进滤镜参数（如 `lut3d=file=…`、`subtitles=…`）：统一用正斜杠，
/// 再依次做“选项值”和“滤镜图”两级转义。
pub fn escape_filter_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut option_level = String::new();
    for ch in path.chars() {
        if matches!(ch, '\\' | '\'' | ':') {
            option_level.push('\\');
        }
        option_level.push(ch);
    }
    let mut graph_level = String::new();
    for ch in option_level.chars() {
        if matches!(ch, '\\' | '\'' | '[' | ']' | ',' | ';') {
            graph_level.push('\\');
        }
        graph_level.push(ch);
    }
    graph_level
}

/// yuv420p 编码要求偶数宽高：最终尺寸为奇数时裁掉最右 / 最下一像素（只在需要时添加）。
fn ensure_even_size(ctx: &mut BuildContext) {
    let (w, h) = (ctx.stream.width, ctx.stream.height);
    if w % 2 == 0 && h % 2 == 0 {
        return;
    }
    let (ew, eh) = ((w - w % 2).max(2), (h - h % 2).max(2));
    ctx.graph.video(format!("crop={ew}:{eh}:0:0"));
    ctx.stream.width = ew;
    ctx.stream.height = eh;
}

fn apply_modules<'a>(ctx: &mut BuildContext<'a>, req: &BuildRequest<'a>) -> Result<(), String> {
    if let Some(probe) = req.probe {
        ctx.set_probe(probe);
    }
    ctx.enabled_modules = ordered_modules(&req.recipe.modules)
        .iter()
        .filter(|m| m.enabled() && m.implemented())
        .map(|m| m.id())
        .collect();
    for module in ordered_modules(&req.recipe.modules) {
        if !module.enabled() {
            continue;
        }
        ctx.current_module = module.id();
        if !module.implemented() {
            ctx.warn("该模块尚未实现，已跳过");
            continue;
        }
        ctx.rng = SeededRng::derive(req.seed, module.id());
        module
            .apply(ctx)
            .map_err(|e| format!("[{}] {e}", module.id()))?;
    }
    ctx.current_module = "";
    Ok(())
}

/// 预览：缩小画面并生成输出参数。返回（输出参数，预计时长，片段起点 / 取帧时间）。
fn preview_args(
    ctx: &mut BuildContext,
    options: &PreviewOptions,
) -> Result<(Vec<String>, f64, f64), String> {
    let max_height = options.max_height.max(16);
    if ctx.stream.height > max_height {
        ctx.graph.video(format!("scale=-2:{max_height}"));
        let width = (f64::from(ctx.stream.width) * f64::from(max_height)
            / f64::from(ctx.stream.height.max(1)))
        .round() as u32;
        ctx.stream.width = width.max(2) & !1;
        ctx.stream.height = max_height;
    }
    let total = ctx.stream.duration;
    match options.output {
        PreviewOutput::Clip { start, duration } => {
            if duration.is_nan() || duration <= 0.0 {
                return Err("预览时长必须大于 0".into());
            }
            let start = start.max(0.0);
            let start = if total > 0.0 && start >= total {
                (total - duration).max(0.0)
            } else {
                start
            };
            let length = if total > 0.0 {
                duration.min(total - start)
            } else {
                duration
            };
            let args = [
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "26",
                "-pix_fmt",
                "yuv420p",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .chain([
                "-ss".to_owned(),
                fmt_num(start),
                "-t".to_owned(),
                fmt_num(length),
            ])
            .collect();
            Ok((args, length, start))
        }
        PreviewOutput::Frame { time } => {
            let time = if total > 0.0 {
                time.clamp(0.0, (total - 0.05).max(0.0))
            } else {
                time.max(0.0)
            };
            let mut args = Vec::new();
            // 已定位到这一刻时，第一帧就是要的画面
            if !ctx.seek {
                args.extend(["-ss".to_owned(), fmt_num(time)]);
            }
            args.extend(["-frames:v", "1", "-q:v", "3"].map(str::to_owned));
            Ok((args, 0.0, time))
        }
    }
}

fn build_inner(
    req: &BuildRequest,
    preview: Option<&PreviewOptions>,
) -> Result<BuiltCommand, String> {
    if req.info.video.is_none() {
        return Err("文件中没有视频流".into());
    }
    let mut ctx = BuildContext::new(req.info, req.settings, req.task_index);
    ctx.preview = preview.is_some();
    if let Some(options) = preview {
        begin_preview_seek(&mut ctx, req, options);
    }
    // 出错时 ctx 随之丢弃，已登记的临时文件也一并删除
    steps::normalize_pixels(&mut ctx);
    let keep_hdr = steps::apply_hdr(&mut ctx);
    if let Some(subtitle) = req.sidecar.subtitle {
        steps::begin_subtitle(&mut ctx, subtitle)?;
    }
    apply_modules(&mut ctx, req)?;
    steps::finish_subtitle(&mut ctx)?;
    ensure_even_size(&mut ctx);
    if let Some(options) = preview {
        return finish_preview(ctx, req, options);
    }

    let container = req
        .output
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "mp4".into());
    let cover = match req.sidecar.cover {
        Some(cover) => steps::apply_cover(&mut ctx, cover, &container)?,
        None => None,
    };

    let codec = if req.settings.hevc || keep_hdr {
        VideoCodec::Hevc
    } else {
        VideoCodec::H264
    };
    if keep_hdr && !req.settings.hevc {
        ctx.warn("源视频为 HDR 且设置为保留 HDR：H.264 无法保留 HDR，已改用 10 bit H.265");
    }
    let encoder = if req.force_software {
        VideoEncoder::software(codec)
    } else {
        req.encoders
            .select_with(codec, req.settings.gpu, req.settings.prefer_integrated_gpu)
    };
    let slim = req.settings.reduce_size && ctx.encode.video_kbps.is_none();
    if req.settings.reduce_size && !slim {
        ctx.warn("已启用码率模块，按码率模块的设置编码，“体积压缩”不生效");
    }

    let BuildContext {
        graph,
        main_input_args,
        extra_inputs,
        encode,
        stream,
        warnings,
        temp,
        ..
    } = ctx;
    let rendered = graph.render()?;

    let mut args = head_args();
    args.extend(main_input_args);
    args.push("-i".into());
    args.push(path_arg(req.input)?);
    for input in extra_inputs {
        args.extend(input.args);
        args.push("-i".into());
        args.push(input.path);
    }

    if let Some(fc) = &rendered.filter_complex {
        args.push("-filter_complex".into());
        args.push(fc.clone());
    }
    args.push("-map".into());
    args.push(rendered.video_map.clone());
    if let Some(audio) = &rendered.audio_map {
        args.push("-map".into());
        args.push(audio.clone());
    }

    let mut base = encoder.base_args();
    if keep_hdr {
        replace_arg(&mut base, "-pix_fmt", encoder.pix_fmt_10bit());
    }
    if slim && !encoder.is_hardware() {
        replace_arg(&mut base, "-preset", "slow");
    }
    args.extend(base);
    let source_kbps = req.info.video_bit_rate().map(|bps| (bps / 1000).max(1));
    if slim {
        args.extend(encoder.slim_args(source_kbps));
    } else {
        // 未启用“码率”时：沿用源的每像素码率，并保证不低于质量下限（见 `encoders::default_video_kbps`）
        let default_kbps = || {
            let video = req.info.video.as_ref()?;
            let (w, h) = video.display_size();
            crate::ffmpeg::encoders::default_video_kbps(
                codec,
                &[crate::ffmpeg::encoders::BitrateSource {
                    kbps: source_kbps?,
                    frame: crate::ffmpeg::encoders::FrameSize::new(w, h, video.fps),
                    weight: 1.0,
                }],
                crate::ffmpeg::encoders::FrameSize::new(stream.width, stream.height, stream.fps),
            )
        };
        match encode.video_kbps.or_else(default_kbps) {
            Some(kbps) => args.extend(encoder.bitrate_args(kbps)),
            None => args.extend(encoder.quality_args()),
        }
    }
    let tag = if cover.is_some() {
        "-tag:v:0"
    } else {
        "-tag:v"
    };
    if codec == VideoCodec::Hevc && matches!(container.as_str(), "mp4" | "mov" | "m4v") {
        args.extend([tag.into(), "hvc1".into()]);
    }
    args.extend(color_args(req.info, keep_hdr));

    if rendered.audio_map.is_some() {
        let source_codec = req
            .info
            .audio
            .first()
            .map(|a| a.codec.as_str())
            .unwrap_or("");
        if !rendered.audio_filtered && can_copy_audio(source_codec, &container) {
            args.extend(["-c:a".into(), "copy".into()]);
        } else {
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{}k", aac_kbps(stream.audio_channels)),
            ]);
        }
    } else {
        args.push("-an".into());
    }
    if let Some(cover) = &cover {
        args.extend(steps::cover_output_args(cover));
    }

    if matches!(container.as_str(), "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend(steps::copyright_args(&req.settings.copyright));
    args.extend(encode.output_args);
    args.push(path_arg(req.output)?);

    Ok(BuiltCommand {
        args,
        expected_duration: stream.duration,
        encoder,
        warnings,
        temp_files: TempGuard::new(temp),
    })
}

/// 把参数列表里 `flag` 后面的值换成 `value`（没有该参数时追加）。
fn replace_arg(args: &mut Vec<String>, flag: &str, value: &str) {
    match args.iter().position(|a| a == flag) {
        Some(i) if i + 1 < args.len() => args[i + 1] = value.to_owned(),
        _ => args.extend([flag.to_owned(), value.to_owned()]),
    }
}

/// 输出的色彩标记：HDR 源转为 SDR 后标记为 BT.709；保留 HDR 时沿用源的色彩信息。只作用于主视频流。
fn color_args(info: &MediaInfo, keep_hdr: bool) -> Vec<String> {
    let Some(video) = info.video.as_ref().filter(|v| v.is_hdr()) else {
        return Vec::new();
    };
    let (primaries, trc, space) = if keep_hdr {
        (
            video.color_primaries.as_str(),
            video.color_transfer.as_str(),
            video.color_space.as_str(),
        )
    } else {
        ("bt709", "bt709", "bt709")
    };
    let mut args = Vec::new();
    for (flag, value) in [
        ("-color_primaries:v:0", primaries),
        ("-color_trc:v:0", trc),
        ("-colorspace:v:0", space),
    ] {
        if !value.is_empty() {
            args.extend([flag.to_owned(), value.to_owned()]);
        }
    }
    args
}

/// 通用的开头参数：静默、覆盖、进度输出到 stdout。
fn head_args() -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "warning",
        "-progress",
        "pipe:1",
        "-nostats",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// 预览要看的时间点不在开头时，直接定位过去而不是从头解码（否则时间点越靠后越慢，几分钟处要等十几秒）：
/// 主输入用输入端 `-ss` 定位（起点在 [`finish_preview_seek`] 里确定），主视频 / 音频流开头把时间戳加回起点，
/// 之后的滤镜看到的时间与从头解码时一样。有模块不支持（[`PipelineModule::preview_seekable`]）时不定位。
fn begin_preview_seek(ctx: &mut BuildContext, req: &BuildRequest, options: &PreviewOptions) {
    let (wanted, frame) = match options.output {
        PreviewOutput::Clip { start, .. } => (start, false),
        PreviewOutput::Frame { time } => (time, true),
    };
    let seekable = ordered_modules(&req.recipe.modules)
        .iter()
        .filter(|m| m.enabled() && m.implemented())
        .all(|m| m.preview_seekable());
    // 写成 partial_cmp：NaN 也不定位
    if wanted.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || !seekable {
        return;
    }
    ctx.seek = true;
    ctx.graph.video(format!("setpts=PTS+{SEEK_MARKER}/TB"));
    // 单帧不带音频，不必为它加滤镜
    if !frame {
        ctx.graph.audio(format!("asetpts=PTS+{SEEK_MARKER}/TB"));
    }
}

/// 定位起点确定后：替换占位标记，并把主输入的 `-ss` / `-t`（去头尾可能已经写了）推后 / 缩短 `start` 秒。
fn finish_preview_seek(ctx: &mut BuildContext, start: f64) {
    let text = fmt_num(start);
    let extras = fmt_num(extras_start(start));
    // 先换较长的那个标记（两个标记互不包含，顺序其实无关）
    ctx.graph.replace_marker(SEEK_EXTRAS_MARKER, &extras);
    ctx.graph.replace_marker(SEEK_MARKER, &text);
    for arg in ctx.extra_inputs.iter_mut().flat_map(|i| i.args.iter_mut()) {
        if arg == SEEK_EXTRAS_MARKER {
            arg.clone_from(&extras);
        }
    }
    fn shift(args: &mut [String], flag: &str, by: f64, min: f64) -> bool {
        let Some(i) = args.iter().rposition(|a| a == flag) else {
            return false;
        };
        let Some(value) = args.get(i + 1).and_then(|v| v.parse::<f64>().ok()) else {
            return false;
        };
        args[i + 1] = fmt_num((value + by).max(min));
        true
    }
    let args = &mut ctx.main_input_args;
    if !shift(args, "-ss", start, 0.0) {
        args.splice(0..0, ["-ss".to_owned(), text]);
    }
    shift(args, "-t", -start, 0.01);
}

fn finish_preview(
    mut ctx: BuildContext,
    req: &BuildRequest,
    options: &PreviewOptions,
) -> Result<BuiltCommand, String> {
    let (output_args, expected_duration, start) = preview_args(&mut ctx, options)?;
    if ctx.seek {
        finish_preview_seek(&mut ctx, start);
    }
    let frame = matches!(options.output, PreviewOutput::Frame { .. });
    if frame {
        // 单帧不带音频：模块加过音频滤镜时（如抽帧的 atempo）要把那条链接走，否则 FFmpeg 报“滤镜输出未连接”
        ctx.graph.drop_audio();
    }
    let audio_kbps = aac_kbps(ctx.stream.audio_channels).min(128);
    let BuildContext {
        graph,
        main_input_args,
        extra_inputs,
        encode,
        warnings,
        temp,
        ..
    } = ctx;
    let rendered = graph.render()?;

    let mut args = head_args();
    args.extend(main_input_args);
    args.push("-i".into());
    args.push(path_arg(req.input)?);
    for input in extra_inputs {
        args.extend(input.args);
        args.push("-i".into());
        args.push(input.path);
    }
    if let Some(fc) = &rendered.filter_complex {
        args.push("-filter_complex".into());
        args.push(fc.clone());
    }
    args.push("-map".into());
    args.push(rendered.video_map.clone());
    match (&rendered.audio_map, frame) {
        (Some(audio), false) => {
            args.extend(["-map".into(), audio.clone()]);
            args.extend([
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                format!("{audio_kbps}k"),
            ]);
        }
        _ => args.push("-an".into()),
    }
    args.extend(output_args);
    if !frame {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend(encode.output_args);
    args.push(path_arg(req.output)?);

    Ok(BuiltCommand {
        args,
        expected_duration,
        encoder: VideoEncoder::software(VideoCodec::H264),
        warnings,
        temp_files: TempGuard::new(temp),
    })
}

#[cfg(test)]
pub(crate) mod testing {
    //! 单元测试共用的样例数据。
    use super::*;
    use crate::ffmpeg::{AudioStream, VideoStream};

    /// 1280x720、25fps、10 秒、视频 4000 kbit/s、AAC 音频。
    pub fn sample_info() -> MediaInfo {
        MediaInfo {
            path: "in.mp4".into(),
            format_name: "mov,mp4,m4a,3gp,3g2,mj2".into(),
            duration: 10.0,
            size: 5_000_000,
            bit_rate: Some(4_128_000),
            video: Some(VideoStream {
                index: 0,
                codec: "h264".into(),
                width: 1280,
                height: 720,
                fps: 25.0,
                fps_text: "25/1".into(),
                bit_rate: Some(4_000_000),
                pix_fmt: "yuv420p".into(),
                rotation: 0,
                frames: Some(250),
                ..VideoStream::default()
            }),
            audio: vec![AudioStream {
                index: 1,
                codec: "aac".into(),
                channels: 2,
                sample_rate: 48_000,
                bit_rate: Some(128_000),
                language: None,
                title: None,
            }],
            subtitles: Vec::new(),
            has_cover: false,
        }
    }

    /// 用默认设置、仅软件编码构建 `in.mp4 → out.mp4`。
    pub fn build_with(
        info: &MediaInfo,
        recipe: &Recipe,
        seed: u64,
    ) -> Result<BuiltCommand, String> {
        build_full(
            info,
            recipe,
            &Settings::default(),
            &EncoderCaps::software_only(),
            seed,
        )
    }

    pub fn build_full(
        info: &MediaInfo,
        recipe: &Recipe,
        settings: &Settings,
        encoders: &EncoderCaps,
        seed: u64,
    ) -> Result<BuiltCommand, String> {
        build(&BuildRequest {
            input: Path::new("in.mp4"),
            output: Path::new("out.mp4"),
            info,
            recipe,
            settings,
            encoders,
            seed,
            task_index: 0,
            force_software: false,
            probe: None,
            sidecar: Sidecar::NONE,
        })
    }

    /// 同 [`build_with`]，但构建 5 秒预览片段（`ctx.is_preview()` 为 true，画面最高 360）。
    pub fn build_preview_with(
        info: &MediaInfo,
        recipe: &Recipe,
        seed: u64,
    ) -> Result<BuiltCommand, String> {
        build_preview(
            &BuildRequest {
                input: Path::new("in.mp4"),
                output: Path::new("preview.mp4"),
                info,
                recipe,
                settings: &Settings::default(),
                encoders: &EncoderCaps::software_only(),
                seed,
                task_index: 0,
                force_software: true,
                probe: None,
                sidecar: Sidecar::NONE,
            },
            &PreviewOptions {
                output: PreviewOutput::Clip {
                    start: 0.0,
                    duration: 5.0,
                },
                max_height: 360,
            },
        )
    }

    /// 同 [`build_with`]，并注入假的素材读取函数（供 `ctx.probe` 使用）。
    pub fn build_with_probe(
        info: &MediaInfo,
        recipe: &Recipe,
        seed: u64,
        probe: ProbeFn,
    ) -> Result<BuiltCommand, String> {
        build(&BuildRequest {
            input: Path::new("in.mp4"),
            output: Path::new("out.mp4"),
            info,
            recipe,
            settings: &Settings::default(),
            encoders: &EncoderCaps::software_only(),
            seed,
            task_index: 0,
            force_software: false,
            probe: Some(probe),
            sidecar: Sidecar::NONE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::ffmpeg::{HwVendor, VideoEncoder};

    #[test]
    fn all_registry_ids_are_ordered_exactly_once() {
        let modules = RecipeModules::default();
        let mut ordered: Vec<&str> = ordered_modules(&modules).iter().map(|m| m.id()).collect();
        ordered.sort();
        let mut ids = MODULE_IDS.to_vec();
        ids.sort();
        assert_eq!(ordered, ids);
    }

    #[test]
    fn reference_modules_are_implemented() {
        // 其它模块在 M2 中陆续实现，这里只检查参考实现
        let modules = RecipeModules::default();
        let implemented: Vec<&str> = ordered_modules(&modules)
            .iter()
            .filter(|m| m.implemented())
            .map(|m| m.id())
            .collect();
        assert!(implemented.contains(&"pipe.fps"));
        assert!(implemented.contains(&"pipe.bitrate"));
    }

    #[test]
    fn disabled_recipe_reencodes_with_source_bitrate_and_copies_audio() {
        let built = build_with(&sample_info(), &Recipe::disabled(), 1).unwrap();
        assert_eq!(
            built.args,
            [
                "-hide_banner",
                "-nostdin",
                "-y",
                "-loglevel",
                "warning",
                "-progress",
                "pipe:1",
                "-nostats",
                "-i",
                "in.mp4",
                "-map",
                "0:v:0",
                "-map",
                "0:a:0",
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-pix_fmt",
                "yuv420p",
                "-b:v",
                "4000k",
                "-maxrate",
                "6000k",
                "-bufsize",
                "8000k",
                "-c:a",
                "copy",
                "-movflags",
                "+faststart",
                "out.mp4",
            ]
        );
        assert_eq!(built.expected_duration, 10.0);
        assert!(built.warnings.is_empty());
    }

    #[test]
    fn placeholder_modules_are_skipped_with_warning() {
        let modules = RecipeModules::default();
        let Some(placeholder) = ordered_modules(&modules)
            .iter()
            .find(|m| !m.implemented())
            .map(|m| m.id())
        else {
            return; // 全部模块都已实现
        };
        let mut recipe = Recipe::disabled();
        recipe.set_module_enabled(placeholder, true).unwrap();
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(
            built.warnings,
            [format!("[{placeholder}] 该模块尚未实现，已跳过")]
        );
    }

    fn preview_with(info: &MediaInfo, recipe: &Recipe, output: PreviewOutput) -> BuiltCommand {
        build_preview(
            &BuildRequest {
                input: Path::new("in.mp4"),
                output: Path::new("preview.mp4"),
                info,
                recipe,
                settings: &Settings {
                    hevc: true,
                    ..Settings::default()
                },
                encoders: &EncoderCaps {
                    hardware: vec![VideoEncoder::hardware(VideoCodec::H264, HwVendor::Nvenc)],
                },
                seed: 1,
                task_index: 0,
                force_software: false,
                probe: None,
                sidecar: Sidecar::NONE,
            },
            &PreviewOptions {
                output,
                max_height: 360,
            },
        )
        .unwrap()
    }

    #[test]
    fn preview_clip_is_small_software_h264() {
        let built = preview_with(
            &sample_info(),
            &Recipe::disabled(),
            PreviewOutput::Clip {
                start: 0.0,
                duration: 5.0,
            },
        );
        let args = built.args.join(" ");
        assert!(
            args.ends_with(
                "-nostats -i in.mp4 -filter_complex [0:v:0]scale=-2:360[v0] -map [v0] -map 0:a:0 \
                 -c:a aac -b:a 128k -c:v libx264 -preset ultrafast -crf 26 -pix_fmt yuv420p -ss 0 -t 5 \
                 -movflags +faststart preview.mp4"
            ),
            "{args}"
        );
        assert_eq!(built.encoder, VideoEncoder::software(VideoCodec::H264));
        assert_eq!(built.expected_duration, 5.0);
        // 起点不在开头：输入端定位，音视频时间戳加回起点（输出端的 -ss 把起点之前的丢掉，并让输出从 0 开始）
        let seeked = preview_with(
            &sample_info(),
            &Recipe::disabled(),
            PreviewOutput::Clip {
                start: 2.0,
                duration: 5.0,
            },
        );
        let args = seeked.args.join(" ");
        assert!(
            args.ends_with(
                "-nostats -ss 2 -i in.mp4 \
                 -filter_complex [0:v:0]setpts=PTS+2/TB,scale=-2:360[v0];[0:a:0]asetpts=PTS+2/TB[a1] \
                 -map [v0] -map [a1] -c:a aac -b:a 128k -c:v libx264 -preset ultrafast -crf 26 \
                 -pix_fmt yuv420p -ss 2 -t 5 -movflags +faststart preview.mp4"
            ),
            "{args}"
        );
        // 片段超出结尾时截短；起点超出时退回最后一段
        let tail = preview_with(
            &sample_info(),
            &Recipe::disabled(),
            PreviewOutput::Clip {
                start: 8.0,
                duration: 5.0,
            },
        );
        assert_eq!(tail.arg_after("-t"), Some("2"));
        let late = preview_with(
            &sample_info(),
            &Recipe::disabled(),
            PreviewOutput::Clip {
                start: 30.0,
                duration: 4.0,
            },
        );
        assert_eq!(late.arg_after("-ss"), Some("6"));
    }

    #[test]
    fn preview_frame_and_small_sources() {
        let mut info = sample_info();
        info.video.as_mut().unwrap().height = 240;
        info.video.as_mut().unwrap().width = 320;
        let built = preview_with(
            &info,
            &Recipe::default(),
            PreviewOutput::Frame { time: 99.0 },
        );
        let args = built.args.join(" ");
        assert!(
            args.ends_with(
                "-nostats -ss 9.95 -i in.mp4 -filter_complex [0:v:0]setpts=PTS+9.95/TB,fps=30[v0] \
                 -map [v0] -an -frames:v 1 -q:v 3 preview.mp4"
            ),
            "不需要缩小，单帧不带音频，直接定位到取帧时间：{args}"
        );
        // 开头的一帧不需要定位
        let first = preview_with(
            &info,
            &Recipe::default(),
            PreviewOutput::Frame { time: 0.0 },
        );
        let args = first.args.join(" ");
        assert!(
            args.ends_with(
                "-nostats -i in.mp4 -filter_complex [0:v:0]fps=30[v0] -map [v0] -an -ss 0 -frames:v 1 -q:v 3 preview.mp4"
            ),
            "{args}"
        );
    }

    #[test]
    fn preview_seek_follows_trim_and_modules() {
        use crate::pipeline::modules::trim::TrimMode;
        // 去头 1.5 秒、去尾 1 秒后的第 4 秒 = 源视频的 5.5 秒；剩下的 7.5 秒里还要读 3.5 秒
        let mut recipe = Recipe::disabled();
        recipe.modules.trim.enabled = true;
        recipe.modules.trim.mode = TrimMode::HeadTail;
        recipe.modules.trim.head = crate::model::Range::fixed(1.5);
        recipe.modules.trim.tail = crate::model::Range::fixed(1.0);
        // 动态缩放按帧序号计算，zoompan 的时间戳又从 0 开始：两处都按定位起点修正
        recipe.modules.zoom.enabled = true;
        let built = preview_with(&sample_info(), &recipe, PreviewOutput::Frame { time: 4.0 });
        let args = built.args.join(" ");
        assert!(args.contains("-ss 5.5 -t 3.5 -i in.mp4"), "{args}");
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(fc.starts_with("[0:v:0]setpts=PTS+4/TB,zoompan="), "{fc}");
        assert!(fc.contains("cos(2*PI*(in+4*25)/100)"), "{fc}");
        assert!(fc.contains(":fps=25,setpts=PTS+4/TB,"), "{fc}");
        assert!(!args.contains("@@"), "{args}");
        assert!(
            args.ends_with("-an -frames:v 1 -q:v 3 preview.mp4"),
            "{args}"
        );

        // 抽帧并同步加速音频时时间轴被压缩：不定位，从头解码
        recipe.modules.drop_frames.enabled = true;
        recipe.modules.drop_frames.sync_audio = true;
        let slow = preview_with(&sample_info(), &recipe, PreviewOutput::Frame { time: 4.0 });
        let args = slow.args.join(" ");
        assert!(args.contains("-ss 1.5 -t 7.5 -i in.mp4"), "{args}");
        assert!(!args.contains("setpts=PTS+4/TB"), "{args}");
        // 单帧不带音频：抽帧加的 atempo 链接到 anullsink，不留未连接的输出
        let fc = slow.arg_after("-filter_complex").unwrap();
        assert!(fc.contains("atempo=") && fc.contains("]anullsink;"), "{fc}");
        assert!(
            args.ends_with("-an -ss 4 -frames:v 1 -q:v 3 preview.mp4"),
            "{args}"
        );
    }

    #[test]
    fn preview_seek_aligns_extra_inputs() {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.seek = true;
        let input = ctx
            .add_input(vec!["-loop".into(), "1".into()], Path::new("bg.png"))
            .unwrap();
        assert_eq!(input, 1);
        ctx.graph.video(ctx.restart_pts_filter());
        finish_preview_seek(&mut ctx, 12.5);
        assert_eq!(ctx.main_input_args, ["-ss", "12.5"]);
        // 素材从定位起点之前的整秒开始：12.5 在素材的时间基里可能取整到主视频流第一帧之后，那一帧上就没有素材
        assert_eq!(ctx.extra_inputs[0].args, ["-itsoffset", "12", "-loop", "1"]);
        let fc = ctx.graph.render().unwrap().filter_complex.unwrap();
        assert_eq!(fc, "[0:v:0]setpts=PTS-STARTPTS+12/TB[v0]");
        assert_eq!(extras_start(0.4), 0.0);
        assert_eq!(extras_start(250.0), 250.0);
    }

    #[test]
    fn hevc_gpu_and_no_audio() {
        let mut info = sample_info();
        info.audio.clear();
        let settings = Settings {
            hevc: true,
            ..Settings::default()
        };
        let caps = EncoderCaps {
            hardware: vec![VideoEncoder::hardware(VideoCodec::Hevc, HwVendor::Nvenc)],
        };
        let built = build_full(&info, &Recipe::disabled(), &settings, &caps, 1).unwrap();
        assert_eq!(built.arg_after("-c:v"), Some("hevc_nvenc"));
        assert_eq!(built.arg_after("-tag:v"), Some("hvc1"));
        assert!(built.args.contains(&"-an".to_owned()));
        assert!(built.encoder.is_hardware());
    }

    #[test]
    fn odd_final_size_is_cropped_to_even() {
        let mut info = sample_info();
        let video = info.video.as_mut().unwrap();
        video.width = 321;
        video.height = 241;
        let built = build_with(&info, &Recipe::disabled(), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]crop=320:240:0:0[v0]")
        );
        // 旋转 90° 的源按显示尺寸计算
        info.video.as_mut().unwrap().rotation = 90;
        let built = build_with(&info, &Recipe::default(), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]fps=30,crop=240:320:0:0[v0]")
        );
        // 偶数尺寸不加任何滤镜
        let built = build_with(&sample_info(), &Recipe::disabled(), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
    }

    #[test]
    fn aac_bitrate_depends_on_channels() {
        let mut info = sample_info();
        info.audio[0].channels = 1;
        info.audio[0].codec = "pcm_s16le".into(); // 不能直接复制到 mp4
        let built = build_with(&info, &Recipe::disabled(), 1).unwrap();
        assert_eq!(built.arg_after("-c:a"), Some("aac"));
        assert_eq!(built.arg_after("-b:a"), Some("96k"));
        info.audio[0].channels = 2;
        let built = build_with(&info, &Recipe::disabled(), 1).unwrap();
        assert_eq!(built.arg_after("-b:a"), Some("192k"));
        assert_eq!(aac_kbps(6), 320);
    }

    #[test]
    fn context_helpers() {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        assert_eq!(ctx.rate_text().as_deref(), Some("25"));
        ctx.stream.fps = 29.97;
        assert_eq!(ctx.rate_text().as_deref(), Some("29.97"));
        ctx.stream.fps = 0.0;
        assert_eq!(ctx.rate_text(), None);
        assert!(!ctx.can_probe());
        assert!(ctx.probe(Path::new("x.mp4")).is_err());
        let fake = |_: &Path| -> Result<MediaInfo, String> { Ok(sample_info()) };
        ctx.set_probe(&fake);
        assert_eq!(ctx.probe(Path::new("x.mp4")).unwrap().duration, 10.0);

        ctx.enabled_modules = vec!["pipe.crop", "pipe.adjust", "pipe.fps"];
        ctx.current_module = "pipe.crop";
        assert_eq!(ctx.later_modules(), ["pipe.adjust", "pipe.fps"]);
        ctx.current_module = "pipe.fps";
        assert!(ctx.later_modules().is_empty());

        ctx.stream.fps = 25.0;
        ctx.ensure_cfr(); // 不是可变帧率：什么也不做
        ctx.stream.vfr = true;
        ctx.ensure_cfr();
        assert!(!ctx.stream.vfr);
        assert_eq!(
            ctx.graph
                .clone()
                .render()
                .unwrap()
                .filter_complex
                .as_deref(),
            Some("[0:v:0]fps=25[v0]")
        );
    }

    #[test]
    fn anamorphic_source_is_scaled_to_square_pixels_first() {
        let mut info = sample_info();
        let video = info.video.as_mut().unwrap();
        video.width = 720;
        video.height = 576;
        video.sar = "64:45".into(); // PAL 16:9
        let settings = Settings::default();
        let ctx = BuildContext::new(&info, &settings, 0);
        assert_eq!((ctx.stream.width, ctx.stream.height), (1024, 576));
        assert_eq!(ctx.source_size(), (1024, 576));
        let built = build_with(&info, &Recipe::default(), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]scale=1024:576,setsar=1,fps=30[v0]")
        );
        // 方形像素（1:1）不加任何滤镜
        info.video.as_mut().unwrap().sar = "1:1".into();
        let built = build_with(&info, &Recipe::disabled(), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
    }

    fn hdr_info() -> MediaInfo {
        let mut info = sample_info();
        let video = info.video.as_mut().unwrap();
        video.codec = "hevc".into();
        video.pix_fmt = "yuv420p10le".into();
        video.color_transfer = "smpte2084".into();
        video.color_primaries = "bt2020".into();
        video.color_space = "bt2020nc".into();
        info
    }

    #[test]
    fn hdr_source_is_tone_mapped_by_default() {
        let built = build_with(&hdr_info(), &Recipe::disabled(), 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        assert!(
            fc.starts_with("[0:v:0]zscale=tin=smpte2084:pin=bt2020:min=bt2020nc:t=linear"),
            "{fc}"
        );
        assert!(fc.contains("tonemap=tonemap=hable"));
        assert_eq!(built.arg_after("-c:v"), Some("libx264"));
        assert_eq!(built.arg_after("-color_trc:v:0"), Some("bt709"));
        assert_eq!(built.warnings.len(), 1);
        // 非 HDR 源不加任何色彩参数
        let sdr = build_with(&sample_info(), &Recipe::disabled(), 1).unwrap();
        assert!(!sdr.args.iter().any(|a| a.starts_with("-color")));
    }

    #[test]
    fn hdr_can_be_kept_as_10bit_hevc() {
        let settings = Settings {
            hdr_to_sdr: false,
            ..Settings::default()
        };
        let built = build_full(
            &hdr_info(),
            &Recipe::disabled(),
            &settings,
            &EncoderCaps::software_only(),
            1,
        )
        .unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None, "不做色调映射");
        assert_eq!(built.arg_after("-c:v"), Some("libx265"));
        assert_eq!(built.arg_after("-pix_fmt"), Some("yuv420p10le"));
        assert_eq!(built.arg_after("-color_trc:v:0"), Some("smpte2084"));
        assert_eq!(built.arg_after("-color_primaries:v:0"), Some("bt2020"));
        assert_eq!(built.arg_after("-colorspace:v:0"), Some("bt2020nc"));
        assert!(built.warnings[0].contains("H.264 无法保留 HDR"));
        // 硬件编码用 p010le
        let caps = EncoderCaps {
            hardware: vec![VideoEncoder::hardware(VideoCodec::Hevc, HwVendor::Nvenc)],
        };
        let hw = build_full(&hdr_info(), &Recipe::disabled(), &settings, &caps, 1).unwrap();
        assert_eq!(hw.arg_after("-pix_fmt"), Some("p010le"));
        // 预览总是转换为 SDR
        let preview = preview_with(
            &hdr_info(),
            &Recipe::disabled(),
            PreviewOutput::Frame { time: 1.0 },
        );
        assert!(preview
            .arg_after("-filter_complex")
            .unwrap()
            .contains("tonemap"));
    }

    #[test]
    fn reduce_size_and_copyright_args() {
        let settings = Settings {
            reduce_size: true,
            copyright: "© 测试".into(),
            ..Settings::default()
        };
        let built = build_full(
            &sample_info(),
            &Recipe::disabled(),
            &settings,
            &EncoderCaps::software_only(),
            1,
        )
        .unwrap();
        let args = built.args.join(" ");
        assert!(
            args.contains(
                "-c:v libx264 -preset slow -pix_fmt yuv420p -crf 28 -maxrate 4000k -bufsize 8000k"
            ),
            "{args}"
        );
        assert!(
            args.contains("-metadata copyright=© 测试 out.mp4"),
            "{args}"
        );
        // 启用码率模块时以码率模块为准
        let mut recipe = Recipe::disabled();
        recipe.modules.bitrate.enabled = true;
        let built = build_full(
            &sample_info(),
            &recipe,
            &settings,
            &EncoderCaps::software_only(),
            1,
        )
        .unwrap();
        assert!(built.arg_after("-crf").is_none());
        assert!(built.warnings.iter().any(|w| w.contains("体积压缩")));
    }

    #[test]
    fn integrated_gpu_preference_comes_from_settings() {
        let caps = EncoderCaps {
            hardware: vec![
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Nvenc),
                VideoEncoder::hardware(VideoCodec::H264, HwVendor::Qsv),
            ],
        };
        let settings = Settings {
            prefer_integrated_gpu: true,
            ..Settings::default()
        };
        let built = build_full(&sample_info(), &Recipe::disabled(), &settings, &caps, 1).unwrap();
        assert_eq!(built.arg_after("-c:v"), Some("h264_qsv"));
    }

    #[test]
    fn subtitle_and_cover_steps_in_command() {
        let dir = std::env::temp_dir().join(format!("cr-steps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = dir.join("a.srt");
        let png = dir.join("a.png");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nhi\n").unwrap();
        std::fs::write(&png, b"png").unwrap();
        let mut recipe = Recipe::disabled();
        recipe.modules.trim.enabled = true; // 默认去头 1 秒
        let settings = Settings {
            cover_frames: 3,
            ..Settings::default()
        };
        let built = build(&BuildRequest {
            input: Path::new("in.mp4"),
            output: Path::new("out.mp4"),
            info: &sample_info(),
            recipe: &recipe,
            settings: &settings,
            encoders: &EncoderCaps::software_only(),
            seed: 1,
            task_index: 0,
            force_software: false,
            probe: None,
            sidecar: Sidecar {
                subtitle: Some(&srt),
                cover: Some(&png),
            },
        })
        .unwrap();
        let fc = built.arg_after("-filter_complex").unwrap().to_owned();
        assert!(
            fc.starts_with("[0:v:0]setpts=PTS+1/TB,subtitles=filename="),
            "{fc}"
        );
        assert!(fc.contains(",setpts=PTS-1/TB"), "{fc}");
        assert!(
            fc.contains("force_style=PlayResX=1920\\,PlayResY=1080"),
            "{fc}"
        );
        assert!(
            fc.ends_with("[1:v]scale=1280:720,setsar=1[cv1];[v0][cv1]overlay=0:0:eof_action=repeat:enable='lt(n,3)'[cv2]"),
            "{fc}"
        );
        let args = built.args.join(" ");
        assert!(
            args.contains("-map 2:v:0 -c:v:1 mjpeg -pix_fmt:v:1 yuvj420p -q:v:1 2 -disposition:v:1 attached_pic"),
            "{args}"
        );
        // 字体复制到构建临时目录，随构建结果一起删除
        assert!(
            !built.temp_files.is_empty()
                || crate::pipeline::modules::text::default_font().is_none()
        );
        drop(built);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filter_paths_are_escaped() {
        assert_eq!(
            escape_filter_path(r"C:\LUTs\a b,c's.cube"),
            r"C\\:/LUTs/a b\,c\\\'s.cube"
        );
    }

    #[test]
    fn no_video_is_an_error() {
        let mut info = sample_info();
        info.video = None;
        assert!(build_with(&info, &Recipe::disabled(), 1).is_err());
    }

    #[test]
    fn number_formatting() {
        assert_eq!(fmt_num(30.0), "30");
        assert_eq!(fmt_num(29.97), "29.97");
        assert_eq!(fmt_num(0.5), "0.5");
        assert_eq!(fmt_num(-0.0), "0");
    }

    #[test]
    fn output_extension_rules() {
        let mut settings = Settings::default();
        assert_eq!(output_extension(&settings, Path::new("a.MKV")), "mp4");
        settings.target_format = TargetFormat::Source;
        assert_eq!(output_extension(&settings, Path::new("a.MKV")), "mkv");
        assert_eq!(output_extension(&settings, Path::new("a.webm")), "mp4");
    }

    #[test]
    fn command_line_quotes_complex_args() {
        let built = build_with(&sample_info(), &Recipe::default(), 1).unwrap();
        let line = built.command_line("ffmpeg");
        assert!(line.starts_with("ffmpeg -hide_banner"));
        assert!(line.contains("\"[0:v:0]fps=30[v0]\""));
    }
}
