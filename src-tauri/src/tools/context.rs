//! 执行一个工作项时工具可用的全部服务（[`ToolContext`]）。
//!
//! | 需要 | 用法 |
//! |---|---|
//! | 输入文件与媒体信息 | `ctx.inputs()`、`ctx.input_path(i)`、`ctx.info(i)`（按需探测并缓存）、`ctx.probe(路径)` |
//! | 输出文件 | `ctx.output(&OutputSpec::file("mp3").suffix("_audio"))` 返回**写入用的临时路径**；成功后统一改名为最终文件，失败 / 取消时删除 |
//! | 把临时目录里生成的文件作为输出 | `ctx.keep_file(&生成的文件, &spec)` |
//! | 临时文件 | `ctx.temp_file("list.txt")`：工作项专用目录，结束时（成功、失败、取消）整体删除 |
//! | 运行 ffmpeg | `ctx.run_ffmpeg(说明, 参数, 预计输出时长)`（不重编码 / 不涉及视频编码器） |
//! | 视频编码 | `ctx.run_encode(说明, 预计时长, \|enc\| 参数)`：`enc.video_args(容器)` 已含编码器、码率（沿用 `pipe.bitrate`；未启用时源的每像素码率 + 质量下限）；硬件编码失败自动改用软件编码重试。码率来自别的输入 / 输出画面变了：`ctx.run_encode_from(.., &EncodeBasis::inputs(..).frame(w, h, fps), ..)` |
//! | 先分析再处理 | `ctx.analyze_input(0, &SceneDetect { .. })` 等，见 [`crate::tools::analysis`] |
//! | 进度 | 每次运行 ffmpeg 消耗一步：`ctx.set_steps(&[权重..])`；分析之后才知道步数时 `ctx.split_next_step(&[..])` |
//! | 取消 | ffmpeg 运行中自动响应；自己的循环里调用 `ctx.check_cancelled()?` |
//! | 日志 | `ctx.note(..)`（信息）、`ctx.warn(..)`、`ctx.log(级别, ..)` |
//! | 随机数 | `ctx.rng`：由种子 + 序号 + 工具 ID 派生，可复现 |

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::ffmpeg::encoders::{default_video_kbps, BitrateSource, FrameSize};
use crate::ffmpeg::{
    self, CancelToken, FfmpegPaths, MediaInfo, Progress, RunError, VideoCodec, VideoEncoder,
    DEFAULT_STALL_TIMEOUT,
};
use crate::fsutil;
use crate::jobs::{JobEvent, LogLevel};
use crate::model::rng::mix_seed;
use crate::model::{SeededRng, Settings};
use crate::pipeline::{aac_kbps, output_extension, BuildContext, PipelineModule};

use super::analysis::{self, Analysis, MetadataParser};
use super::output::{key, output_stem, part_path, plan_tool_output, OutputSpec, ToolOutputRequest};
use super::plan::{ItemInput, WorkItem};
use super::runner::{lock, RunShared};
use super::ToolError;

/// 通用的开头参数：不交互、覆盖、指定日志级别、进度输出到 stdout。
pub fn head_args(loglevel: &str) -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        loglevel,
        "-progress",
        "pipe:1",
        "-nostats",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

/// 便于日志显示的命令行（参数含空格等时加引号）。
pub fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_owned())
        .chain(args.iter().map(|a| {
            if a.is_empty() || a.contains([' ', ';', '[', '\'', '"']) {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `set.copyright`：命令的输出（最后一个参数）是视频文件时，在它前面写入版权元数据
/// （`-metadata copyright=…`，见 `pipeline::steps::copyright_args`）。独立工具的 `run_ffmpeg` / `run_encode`
/// 统一经过这里，工具不用自己处理；图片、GIF、HLS 播放列表、分析命令（输出到 `-`）不受影响。
pub fn with_copyright(mut args: Vec<String>, copyright: &str) -> Vec<String> {
    let extra = crate::pipeline::steps::copyright_args(copyright);
    let is_video = args.last().is_some_and(|out| {
        crate::fsutil::has_extension(Path::new(out), crate::fsutil::VIDEO_EXTENSIONS)
    });
    if extra.is_empty() || !is_video {
        return args;
    }
    let output = args.pop().unwrap_or_default();
    args.extend(extra);
    args.push(output);
    args
}

/// 视频编码设置：编码器（按设置选择硬件 / 软件、H.264 / H.265）与目标码率。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    pub encoder: VideoEncoder,
    /// 目标视频码率（kbit/s）；None 时用恒定质量
    pub video_kbps: Option<u64>,
}

impl Encoding {
    /// `-c:v …` 编码器、像素格式、码率 / 质量参数；H.265 写进 mp4/mov 时加 `hvc1` 标记。
    pub fn video_args(&self, container: &str) -> Vec<String> {
        let mut args = self.encoder.base_args();
        match self.video_kbps {
            Some(kbps) => args.extend(self.encoder.bitrate_args(kbps)),
            None => args.extend(self.encoder.quality_args()),
        }
        if self.encoder.codec == VideoCodec::Hevc
            && matches!(
                container.to_ascii_lowercase().as_str(),
                "mp4" | "mov" | "m4v"
            )
        {
            args.extend(["-tag:v".to_owned(), "hvc1".to_owned()]);
        }
        args
    }

    /// AAC 音频参数（码率按声道数，见 `pipeline::aac_kbps`）。
    pub fn audio_args(channels: u32) -> Vec<String> {
        vec![
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            format!("{}k", aac_kbps(channels)),
        ]
    }
}

/// 一次编码的码率依据（[`ToolContext::run_encode_from`] / [`ToolContext::encoding_from`]）：
/// 码率取自哪些输入（按时长加权平均每像素码率），输出画面是什么（按像素量换算、计算质量下限）。
/// 默认：第 0 个输入，输出画面与它相同。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EncodeBasis {
    /// 码率来源的输入序号；空时为第 0 个输入
    pub inputs: Vec<usize>,
    /// 输出画面（宽、高、帧率）；None 时与第一个来源输入相同
    pub frame: Option<(u32, u32, f64)>,
    /// 不以任何输入的码率为准（输入是图片等没有可参照码率的素材）：“码率”未启用时用恒定质量
    pub no_source_bitrate: bool,
}

impl EncodeBasis {
    /// 以第 `index` 个输入为准。
    pub fn input(index: usize) -> Self {
        Self::inputs([index])
    }

    /// 以这些输入为准（如合并时组内的全部输入）。
    pub fn inputs(indices: impl IntoIterator<Item = usize>) -> Self {
        Self {
            inputs: indices.into_iter().collect(),
            frame: None,
            no_source_bitrate: false,
        }
    }

    /// 只指定输出画面、不参照任何输入的码率（如图片生成视频）。“码率”设置启用时仍按它（倍率相对第 0 个输入）。
    pub fn output_only(width: u32, height: u32, fps: f64) -> Self {
        Self {
            no_source_bitrate: true,
            ..Self::default()
        }
        .frame(width, height, fps)
    }

    /// 输出画面与来源不同时（缩放、拼接、统一到某个规格）指定输出画面。
    pub fn frame(mut self, width: u32, height: u32, fps: f64) -> Self {
        self.frame = Some((width, height, fps));
        self
    }

    fn source_inputs(&self) -> Vec<usize> {
        if self.inputs.is_empty() {
            vec![0]
        } else {
            self.inputs.clone()
        }
    }
}

/// 媒体信息的显示画面（宽、高、帧率）。
fn frame_of(info: &MediaInfo) -> FrameSize {
    info.video.as_ref().map_or(FrameSize::new(0, 0, 0.0), |v| {
        let (w, h) = v.display_size();
        FrameSize::new(w, h, v.fps)
    })
}

/// 媒体信息作为码率来源（权重为时长）；读不到视频码率时 None。
fn bitrate_source(info: &MediaInfo) -> Option<BitrateSource> {
    let kbps = info.video_bit_rate().map(|bps| (bps / 1000).max(1))?;
    Some(BitrateSource {
        kbps,
        frame: frame_of(info),
        weight: if info.duration.is_finite() && info.duration > 0.0 {
            info.duration
        } else {
            1.0
        },
    })
}

/// 分步进度：每次运行 ffmpeg 占一段百分比。
#[derive(Debug, Clone, PartialEq)]
pub struct StepProgress {
    spans: Vec<(f64, f64)>,
    next: usize,
}

impl Default for StepProgress {
    fn default() -> Self {
        Self {
            spans: vec![(0.0, 100.0)],
            next: 0,
        }
    }
}

/// 一步在总进度中的位置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepSpan {
    /// 第几步（从 1 起）与总步数
    pub step: usize,
    pub steps: usize,
    pub start: f64,
    pub end: f64,
}

impl StepSpan {
    pub fn at(&self, fraction: f64) -> f64 {
        let fraction = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.start + (self.end - self.start) * fraction
    }
}

fn shares(weights: &[f64]) -> Vec<f64> {
    let clean: Vec<f64> = weights
        .iter()
        .map(|w| if w.is_finite() && *w > 0.0 { *w } else { 0.0 })
        .collect();
    let sum: f64 = clean.iter().sum();
    if sum <= 0.0 {
        let n = weights.len().max(1) as f64;
        return vec![1.0 / n; weights.len()];
    }
    clean.iter().map(|w| w / sum).collect()
}

fn spans_in(start: f64, end: f64, weights: &[f64]) -> Vec<(f64, f64)> {
    let mut cursor = start;
    let mut spans: Vec<(f64, f64)> = shares(weights)
        .into_iter()
        .map(|share| {
            let span = (cursor, cursor + (end - start) * share);
            cursor = span.1;
            span
        })
        .collect();
    // 消除浮点误差：最后一步正好结束在 end
    if let Some(last) = spans.last_mut() {
        last.1 = end;
    }
    spans
}

impl StepProgress {
    /// 剩余的步骤：按权重划分尚未开始的那部分进度。
    pub fn set_steps(&mut self, weights: &[f64]) {
        if weights.is_empty() {
            return;
        }
        let start = self.done_until();
        self.spans.truncate(self.next);
        self.spans.extend(spans_in(start, 100.0, weights));
    }

    /// 把下一步再分成若干小步（分析之后才知道要处理几段时用）。
    pub fn split_next(&mut self, weights: &[f64]) {
        if weights.is_empty() {
            return;
        }
        let (start, end) = match self.spans.get(self.next) {
            Some(span) => *span,
            None => {
                let at = self.done_until();
                (at, at)
            }
        };
        let sub = spans_in(start, end, weights);
        if self.next < self.spans.len() {
            self.spans.splice(self.next..=self.next, sub);
        } else {
            self.spans.extend(sub);
        }
    }

    /// 开始下一步；超出计划时返回一个不推进进度的步骤。
    pub fn begin(&mut self) -> StepSpan {
        let steps = self.spans.len().max(1);
        match self.spans.get(self.next).copied() {
            Some((start, end)) => {
                self.next += 1;
                StepSpan {
                    step: self.next,
                    steps,
                    start,
                    end,
                }
            }
            None => {
                let at = self.done_until();
                StepSpan {
                    step: steps,
                    steps,
                    start: at,
                    end: at,
                }
            }
        }
    }

    fn done_until(&self) -> f64 {
        match self.next {
            0 => 0.0,
            n => self.spans.get(n - 1).map_or(100.0, |s| s.1),
        }
    }
}

/// 计划中的一个输出：先写临时名，成功后改名。
#[derive(Debug, Clone)]
struct PendingOutput {
    part: PathBuf,
    final_path: PathBuf,
    folder: bool,
}

/// 执行一个工作项的上下文。由调度器创建，工具在 [`crate::tools::Tool::run`] 中使用。
pub struct ToolContext<'a> {
    shared: &'a RunShared,
    item: &'a WorkItem,
    token: CancelToken,
    /// 本工作项专用的随机数（种子 + 序号 + 工具 ID 派生）。不要自己建随机数。
    pub rng: SeededRng,
    item_seed: u64,
    infos: Vec<Option<MediaInfo>>,
    temp: Option<PathBuf>,
    temp_counter: usize,
    pending: Vec<PendingOutput>,
    reservations: Vec<String>,
    progress: StepProgress,
    force_software: bool,
    warned: HashSet<String>,
    finished: bool,
}

impl<'a> ToolContext<'a> {
    pub(crate) fn new(shared: &'a RunShared, item: &'a WorkItem, token: CancelToken) -> Self {
        let item_seed = mix_seed(shared.config.seed, item.number as u64);
        Self {
            shared,
            item,
            token,
            rng: SeededRng::derive(item_seed, shared.tool.id()),
            item_seed,
            infos: item.inputs.iter().map(|i| i.info.clone()).collect(),
            temp: None,
            temp_counter: 0,
            pending: Vec::new(),
            reservations: Vec::new(),
            progress: StepProgress::default(),
            force_software: false,
            warned: HashSet::new(),
            finished: false,
        }
    }

    // ---------- 输入 ----------

    pub fn item(&self) -> &WorkItem {
        self.item
    }

    pub fn inputs(&self) -> &[ItemInput] {
        &self.item.inputs
    }

    pub fn input_path(&self, index: usize) -> Result<&Path, String> {
        self.item
            .inputs
            .get(index)
            .map(|i| i.path.as_path())
            .ok_or_else(|| format!("没有第 {} 个输入", index + 1))
    }

    /// 第 `index` 个输入的媒体信息（没有时用 ffprobe 探测并缓存）。
    pub fn info(&mut self, index: usize) -> Result<MediaInfo, String> {
        if let Some(Some(info)) = self.infos.get(index) {
            return Ok(info.clone());
        }
        let path = self.input_path(index)?.to_path_buf();
        if !path.is_file() {
            return Err(format!("源文件不存在：{}", path.display()));
        }
        let info = self.probe(&path)?;
        if let Some(slot) = self.infos.get_mut(index) {
            *slot = Some(info.clone());
        }
        Ok(info)
    }

    /// 读取任意文件的媒体信息（使用本次运行的 ffprobe）。
    pub fn probe(&self, path: &Path) -> Result<MediaInfo, String> {
        ffmpeg::probe(&self.shared.config.paths.ffprobe, path)
    }

    pub fn settings(&self) -> &Settings {
        &self.shared.config.settings
    }

    pub fn paths(&self) -> &FfmpegPaths {
        &self.shared.config.paths
    }

    /// 序号（从 1 起）。
    pub fn number(&self) -> usize {
        self.item.number
    }

    /// 本工作项的种子（需要多个独立随机序列时用 `SeededRng::derive(ctx.seed(), "键")`）。
    pub fn seed(&self) -> u64 {
        self.item_seed
    }

    /// 第 `index` 个输入去除后的文件名主干（自己拼输出名时用）。
    pub fn stem(&self, index: usize) -> Result<String, String> {
        output_stem(self.input_path(index)?, self.settings())
    }

    /// 视频输出的默认扩展名：按设置“目标格式”为 mp4 或保持源格式。
    pub fn video_extension(&self, index: usize) -> Result<String, String> {
        Ok(output_extension(self.settings(), self.input_path(index)?))
    }

    // ---------- 取消与日志 ----------

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn check_cancelled(&self) -> Result<(), ToolError> {
        if self.is_cancelled() {
            Err(ToolError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn cancel_token(&self) -> &CancelToken {
        &self.token
    }

    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        self.shared.emit(JobEvent::Log {
            level,
            task_id: Some(self.item.id.clone()),
            message: message.into(),
        });
    }

    /// 信息级日志。
    pub fn note(&self, message: impl Into<String>) {
        self.log(LogLevel::Info, message);
    }

    /// 警告（同样的内容只记一次）。
    pub fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.warned.insert(message.clone()) {
            self.log(LogLevel::Warn, message);
        }
    }

    // ---------- 临时文件 ----------

    /// 本工作项的临时目录（第一次调用时创建；工作项结束时整体删除）。
    pub fn temp_dir(&mut self) -> Result<PathBuf, String> {
        if let Some(dir) = &self.temp {
            return Ok(dir.clone());
        }
        let unique = self.shared.unique.fetch_add(1, Ordering::SeqCst);
        let dir = self
            .shared
            .temp_root()
            .join(format!("{}-{unique}", self.item.number));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("无法创建临时目录 {}：{e}", dir.display()))?;
        self.temp = Some(dir.clone());
        Ok(dir)
    }

    /// 临时目录中的一个新文件路径（不创建文件），文件名前加序号避免重名，如 `3-list.txt`。
    pub fn temp_file(&mut self, name: &str) -> Result<PathBuf, String> {
        let dir = self.temp_dir()?;
        self.temp_counter += 1;
        Ok(dir.join(format!("{}-{name}", self.temp_counter)))
    }

    // ---------- 输出 ----------

    /// 规划一个输出，返回**写入用的临时路径**（文件夹输出时已创建该文件夹）。
    /// 工作项成功后统一改名为最终路径（事件与界面里显示最终路径）。
    pub fn output(&mut self, spec: &OutputSpec) -> Result<PathBuf, String> {
        let input = self
            .item
            .inputs
            .get(spec.input)
            .ok_or_else(|| format!("没有第 {} 个输入", spec.input + 1))?;
        let protected: Vec<PathBuf> = self.item.inputs.iter().map(|i| i.path.clone()).collect();
        let shared = self.shared;
        let config = &shared.config;
        let plan = {
            let mut reserved = lock(&self.shared.reserved);
            let plan = plan_tool_output(
                &ToolOutputRequest {
                    input: &input.path,
                    source_root: input.source_root.as_deref(),
                    output_dir: config.output_dir.as_deref(),
                    settings: &config.settings,
                    number: self.item.number,
                    total: self.shared.total_numbers(),
                    spec,
                    protected: &protected,
                },
                Path::exists,
                |p| reserved.contains(&key(p)),
            )?;
            let k = key(&plan.path);
            reserved.insert(k.clone());
            self.reservations.push(k);
            plan
        };
        if plan.same_dir_as_source && !self.shared.warned_same_dir.swap(true, Ordering::SeqCst) {
            self.shared.emit(JobEvent::Log {
                level: LogLevel::Warn,
                task_id: None,
                message: "输出目录与源文件目录相同，输出文件会和源文件放在一起".into(),
            });
        }
        if let Some(dir) = plan.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("无法创建输出目录 {}：{e}", dir.display()))?;
        }
        let unique = self.shared.unique.fetch_add(1, Ordering::SeqCst);
        let part = part_path(&plan.path, unique, spec.folder);
        if spec.folder {
            std::fs::create_dir_all(&part)
                .map_err(|e| format!("无法创建输出文件夹 {}：{e}", part.display()))?;
        }
        self.pending.push(PendingOutput {
            part: part.clone(),
            final_path: plan.path,
            folder: spec.folder,
        });
        Ok(part)
    }

    /// 把已经生成的文件（通常在临时目录里，如 segment 分割的结果）作为一个输出。
    pub fn keep_file(&mut self, produced: &Path, spec: &OutputSpec) -> Result<PathBuf, String> {
        let part = self.output(spec)?;
        fsutil::move_file(produced, &part)
            .map_err(|e| format!("无法移动 {} 到输出目录：{e}", produced.display()))?;
        Ok(part)
    }

    /// 已规划的输出个数。
    pub fn output_count(&self) -> usize {
        self.pending.len()
    }

    /// 成功：把全部临时输出改名为最终文件，返回最终路径。
    pub(crate) fn commit(&mut self) -> Result<Vec<String>, String> {
        self.finished = true;
        if let Some(missing) = self.pending.iter().find(|o| !o.part.exists()) {
            let name = missing.final_path.display().to_string();
            self.discard();
            return Err(format!("没有生成输出：{name}"));
        }
        let mut done = Vec::with_capacity(self.pending.len());
        let pending = std::mem::take(&mut self.pending);
        for (i, out) in pending.iter().enumerate() {
            if let Err(e) = std::fs::rename(&out.part, &out.final_path) {
                for rest in &pending[i..] {
                    remove_path(&rest.part, rest.folder);
                }
                return Err(format!("无法写入输出 {}：{e}", out.final_path.display()));
            }
            done.push(out.final_path.to_string_lossy().into_owned());
        }
        Ok(done)
    }

    /// 失败 / 取消：删除全部临时输出。
    pub(crate) fn discard(&mut self) {
        self.finished = true;
        for out in self.pending.drain(..) {
            remove_path(&out.part, out.folder);
        }
    }

    // ---------- 进度 ----------

    /// 设定接下来各步（每次运行 ffmpeg 为一步）的权重，如 `&[1.0, 3.0]`。默认只有一步。
    pub fn set_steps(&mut self, weights: &[f64]) {
        self.progress.set_steps(weights);
    }

    /// 把下一步再按权重分成若干步（例如分析后得到 N 个片段，每段一步）。
    pub fn split_next_step(&mut self, weights: &[f64]) {
        self.progress.split_next(weights);
    }

    /// 跳过一步（不需要运行 ffmpeg 时），进度直接到这一步的结尾。
    pub fn skip_step(&mut self) {
        let span = self.progress.begin();
        self.emit_progress(span, 1.0, &Progress::default());
    }

    fn emit_progress(&self, span: StepSpan, fraction: f64, p: &Progress) {
        self.shared.emit(JobEvent::Progress {
            task_id: self.item.id.clone(),
            percent: span
                .at(fraction)
                .min(if fraction >= 1.0 { 100.0 } else { 99.9 }),
            out_time: p.out_time,
            speed: p.speed,
            fps: p.fps,
            round: span.step,
            rounds: span.steps,
        });
    }

    // ---------- 运行 ffmpeg ----------

    /// 运行一条 ffmpeg 命令（参数不含开头的通用参数，会自动加上）。`expected_duration` 是预计的
    /// 输出时长（秒），用于计算这一步的进度；未知时传 0。
    pub fn run_ffmpeg(
        &mut self,
        label: &str,
        args: Vec<String>,
        expected_duration: f64,
    ) -> Result<(), ToolError> {
        let mut full = head_args("warning");
        full.extend(with_copyright(args, &self.shared.config.settings.copyright));
        let span = self.progress.begin();
        self.execute(span, label, &full, expected_duration, |_| {})
    }

    /// 编码设置：按设置选择编码器，码率沿用流水线的 `pipe.bitrate`；未启用时沿用第一个输入的
    /// 每像素码率并保证质量下限（见 [`crate::ffmpeg::encoders::default_video_kbps`]）。
    pub fn encoding(&mut self) -> Result<Encoding, String> {
        self.encoding_from(&EncodeBasis::default())
    }

    /// 以某个媒体信息为准的编码设置（码率倍率、默认码率都相对它计算；输出画面与它相同）。
    pub fn encoding_for(&mut self, info: &MediaInfo) -> Result<Encoding, String> {
        let output = frame_of(info);
        let sources: Vec<BitrateSource> = bitrate_source(info).into_iter().collect();
        self.encoding_with(info, &sources, output)
    }

    /// 按 [`EncodeBasis`] 的编码设置：码率来自哪些输入（按时长加权）、输出画面是什么。
    pub fn encoding_from(&mut self, basis: &EncodeBasis) -> Result<Encoding, String> {
        let indices = basis.source_inputs();
        let mut infos = Vec::with_capacity(indices.len());
        for &i in &indices {
            infos.push(self.info(i)?);
        }
        let first = infos.first().cloned().ok_or("没有输入")?;
        let output = match basis.frame {
            Some((w, h, fps)) => FrameSize::new(w, h, fps),
            None => frame_of(&first),
        };
        let sources: Vec<BitrateSource> = if basis.no_source_bitrate {
            Vec::new()
        } else {
            infos.iter().filter_map(bitrate_source).collect()
        };
        self.encoding_with(&first, &sources, output)
    }

    fn encoding_with(
        &mut self,
        info: &MediaInfo,
        sources: &[BitrateSource],
        output: FrameSize,
    ) -> Result<Encoding, String> {
        let shared = self.shared;
        let settings = &shared.config.settings;
        let codec = if settings.hevc {
            VideoCodec::Hevc
        } else {
            VideoCodec::H264
        };
        let encoder = if self.force_software {
            VideoEncoder::software(codec)
        } else {
            self.shared.config.encoders.select(codec, settings.gpu)
        };
        let bitrate = &shared.config.bitrate;
        let video_kbps = if bitrate.enabled {
            let mut build = BuildContext::new(info, settings, self.item.number.saturating_sub(1));
            build.rng = SeededRng::derive(self.item_seed, bitrate.id());
            bitrate
                .apply(&mut build)
                .map_err(|e| format!("[码率] {e}"))?;
            let warnings: Vec<String> = build.warnings().to_vec();
            for warning in warnings {
                self.warn(format!("[码率] {warning}"));
            }
            build.encode.video_kbps
        } else {
            default_video_kbps(codec, sources, output)
        };
        Ok(Encoding {
            encoder,
            video_kbps,
        })
    }

    /// 运行一条需要视频编码的命令：`build` 根据编码设置生成参数（`enc.video_args(容器)`）。
    /// 码率以第 0 个输入为准；输出由别的输入组成（合并的分组）或画面尺寸变了时用 [`Self::run_encode_from`]。
    /// 硬件编码失败时自动改用软件编码重试一次，之后本工作项都用软件编码。
    pub fn run_encode(
        &mut self,
        label: &str,
        expected_duration: f64,
        build: impl Fn(&Encoding) -> Result<Vec<String>, String>,
    ) -> Result<(), ToolError> {
        self.run_encode_from(label, expected_duration, &EncodeBasis::default(), build)
    }

    /// 同 [`Self::run_encode`]，码率依据由 `basis` 指定（来自哪些输入、输出画面）。
    pub fn run_encode_from(
        &mut self,
        label: &str,
        expected_duration: f64,
        basis: &EncodeBasis,
        build: impl Fn(&Encoding) -> Result<Vec<String>, String>,
    ) -> Result<(), ToolError> {
        let span = self.progress.begin();
        let encoding = self.encoding_from(basis)?;
        let mut full = head_args("warning");
        full.extend(with_copyright(
            build(&encoding)?,
            &self.shared.config.settings.copyright,
        ));
        let result = self.execute(span, label, &full, expected_duration, |_| {});
        match result {
            Err(ToolError::Failed(_)) if encoding.encoder.is_hardware() && !self.is_cancelled() => {
                self.warn(format!(
                    "硬件编码（{}）失败，改用软件编码重试",
                    encoding.encoder.name()
                ));
                self.force_software = true;
                let encoding = self.encoding_from(basis)?;
                let mut full = head_args("warning");
                full.extend(with_copyright(
                    build(&encoding)?,
                    &self.shared.config.settings.copyright,
                ));
                self.execute(span, label, &full, expected_duration, |_| {})
            }
            other => other,
        }
    }

    /// 对第 `index` 个输入运行一次检测，返回结果。占一步进度。
    pub fn analyze_input<A: Analysis>(
        &mut self,
        index: usize,
        analysis: &A,
    ) -> Result<A::Output, ToolError> {
        let info = self.info(index)?;
        let path = self.input_path(index)?.to_path_buf();
        self.analyze(&path, &info, analysis)
    }

    /// 对任意文件（如临时目录里的中间结果）运行一次检测。占一步进度。
    pub fn analyze<A: Analysis>(
        &mut self,
        path: &Path,
        info: &MediaInfo,
        analysis: &A,
    ) -> Result<A::Output, ToolError> {
        let args = analysis::command_args(path, info, analysis)?;
        let span = self.progress.begin();
        let mut parser = MetadataParser::default();
        self.execute(span, analysis.name(), &args, info.duration, |line| {
            parser.feed(line)
        })?;
        Ok(analysis.parse(&parser.finish(), info))
    }

    fn execute(
        &self,
        span: StepSpan,
        label: &str,
        args: &[String],
        expected_duration: f64,
        on_stderr: impl FnMut(&str),
    ) -> Result<(), ToolError> {
        self.check_cancelled()?;
        let ffmpeg_path = &self.shared.config.paths.ffmpeg;
        let step = if span.steps > 1 {
            format!("（{}/{}）", span.step, span.steps)
        } else {
            String::new()
        };
        self.note(format!("{label}{step}：{}", self.item.label));
        self.log(
            LogLevel::Debug,
            command_line(&ffmpeg_path.to_string_lossy(), args),
        );
        let result = ffmpeg::run_with_stderr(
            ffmpeg_path,
            args,
            &self.token,
            DEFAULT_STALL_TIMEOUT,
            |p| {
                let fraction = if expected_duration > 0.0 {
                    p.out_time / expected_duration
                } else {
                    0.0
                };
                self.emit_progress(span, fraction.min(0.999), p);
            },
            on_stderr,
        );
        match result {
            Ok(_) => {
                self.emit_progress(span, 1.0, &Progress::default());
                Ok(())
            }
            Err(err) => {
                if let RunError::Failed { stderr_tail, .. }
                | RunError::Stalled { stderr_tail, .. } = &err
                {
                    let tail = stderr_tail
                        .iter()
                        .rev()
                        .take(10)
                        .rev()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !tail.is_empty() {
                        self.log(LogLevel::Error, format!("ffmpeg 输出：\n{tail}"));
                    }
                }
                Err(err.into())
            }
        }
    }
}

fn remove_path(path: &Path, folder: bool) {
    if folder {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for ToolContext<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.discard();
        }
        {
            let mut reserved = lock(&self.shared.reserved);
            for k in &self.reservations {
                reserved.remove(k);
            }
        }
        if let Some(dir) = self.temp.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::HwVendor;

    #[test]
    fn copyright_goes_before_video_outputs_only() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            with_copyright(v(&["-i", "a.mp4", "-c", "copy", "o.part.mkv"]), "© X"),
            v(&[
                "-i",
                "a.mp4",
                "-c",
                "copy",
                "-metadata",
                "copyright=© X",
                "o.part.mkv"
            ])
        );
        let gif = v(&["-i", "a.mp4", "o.gif"]);
        assert_eq!(with_copyright(gif.clone(), "© X"), gif);
        let null = v(&["-i", "a.mp4", "-f", "null", "-"]);
        assert_eq!(with_copyright(null.clone(), "© X"), null);
        let plain = v(&["-i", "a.mp4", "o.mp4"]);
        assert_eq!(with_copyright(plain.clone(), " "), plain);
    }

    #[test]
    fn steps_split_and_overflow() {
        let mut p = StepProgress::default();
        p.set_steps(&[1.0, 3.0]);
        let first = p.begin();
        assert_eq!(
            (first.step, first.steps, first.start, first.end),
            (1, 2, 0.0, 25.0)
        );
        assert_eq!(first.at(0.5), 12.5);
        p.split_next(&[1.0, 1.0, 2.0]);
        let spans: Vec<(f64, f64)> = (0..3)
            .map(|_| p.begin())
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(spans, [(25.0, 43.75), (43.75, 62.5), (62.5, 100.0)]);
        // 超出计划：停在 100，不倒退
        let extra = p.begin();
        assert_eq!(
            (extra.start, extra.end, extra.step, extra.steps),
            (100.0, 100.0, 4, 4)
        );
    }

    #[test]
    fn default_is_one_step_and_zero_weights_are_equal() {
        let mut p = StepProgress::default();
        let only = p.begin();
        assert_eq!((only.start, only.end, only.steps), (0.0, 100.0, 1));
        let mut q = StepProgress::default();
        q.set_steps(&[0.0, f64::NAN]);
        assert_eq!(q.begin().end, 50.0);
        assert_eq!(
            StepSpan {
                step: 1,
                steps: 1,
                start: 10.0,
                end: 20.0
            }
            .at(f64::NAN),
            10.0
        );
    }

    #[test]
    fn encoding_args() {
        let soft = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1200),
        };
        assert_eq!(
            soft.video_args("mp4").join(" "),
            "-c:v libx264 -preset medium -pix_fmt yuv420p -b:v 1200k -maxrate 1800k -bufsize 2400k"
        );
        let hevc = Encoding {
            encoder: VideoEncoder::hardware(VideoCodec::Hevc, HwVendor::Nvenc),
            video_kbps: None,
        };
        let args = hevc.video_args("MP4").join(" ");
        assert!(args.starts_with("-c:v hevc_nvenc"), "{args}");
        assert!(args.ends_with("-tag:v hvc1"), "{args}");
        assert!(!hevc.video_args("mkv").join(" ").contains("hvc1"));
        assert_eq!(Encoding::audio_args(2), ["-c:a", "aac", "-b:a", "192k"]);
    }

    #[test]
    fn command_lines_quote_special_args() {
        let line = command_line(
            "ffmpeg",
            &[
                "-vf".into(),
                "select='gt(scene,0.3)'".into(),
                "a b.mp4".into(),
            ],
        );
        assert_eq!(line, "ffmpeg -vf \"select='gt(scene,0.3)'\" \"a b.mp4\"");
    }
}
