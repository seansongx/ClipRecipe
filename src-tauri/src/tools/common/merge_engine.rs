//! **合并引擎**：把若干片段（整个文件或其中一段）合成一个输出——无损（concat 分离器）或统一规格后重编码
//! （concat / xfade 滤镜）、中间视频、转场、分组与打乱。`tool.merge`、`tool.folder-merge`、`tool.clip-merge`
//! 共用（原先放在 `merge.rs`，那里保留了同名的再导出）。规则见 `merge.rs` 开头。
//!
//! 统一规格（[`conform_target`] / [`conform_graph`]）时每个片段先按自己的 SAR 换算成方形像素
//! （[`SQUARE_PIXELS`]），目标尺寸是第一个片段按方形像素显示的尺寸，所以变形（非方形像素）源拼接后显示比例不变；
//! 无损合并要求各片段 SAR 相同。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo, VideoStream};
use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::{AssetSource, SeededRng};
use crate::pipeline::fmt_num;
use crate::pipeline::speed_fit::{speed_fit, SpeedFit};
use crate::tools::analysis::Interval;
use crate::tools::common::cutlist::SQUARE_PIXELS;
use crate::tools::context::Encoding;
use crate::tools::media::{copy_container, secs, write_concat_list};
use crate::tools::plan::ItemInput;
use crate::tools::{EncodeBasis, OutputSpec, ToolContext, ToolError, WorkItem};
/// 以空格连接文件名时输出名的最大字符数。
pub const MAX_JOINED_CHARS: usize = 120;
pub const MIN_TRANSITION: f64 = 0.1;
pub const MAX_TRANSITION: f64 = 10.0;

/// 可选的转场（`xfade` 的 transition 名称）；`random` 从中随机取。
pub const TRANSITIONS: &[&str] = &[
    "fade",
    "fadeblack",
    "fadewhite",
    "dissolve",
    "wipeleft",
    "wiperight",
    "wipeup",
    "wipedown",
    "slideleft",
    "slideright",
    "slideup",
    "slidedown",
    "circleopen",
    "circleclose",
    "radial",
    "pixelize",
    "smoothleft",
    "smoothright",
    "zoomin",
    "hblur",
];

/// 合并方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeMode {
    /// 直接复制（concat 分离器）
    #[default]
    Lossless,
    /// 统一规格后重编码（concat / xfade 滤镜）
    Reencode,
}

/// 合并引擎的公共参数（`tool.merge`、`tool.folder-merge` 平铺在各自的参数里）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MergeOptions {
    pub mode: MergeMode,
    /// 无损不兼容时自动改为重编码（关闭则失败）
    pub fallback_reencode: bool,
    /// `none` / `random` / [`TRANSITIONS`] 之一（仅重编码）
    pub transition: String,
    /// 转场时长（秒）
    pub transition_seconds: f64,
    /// 在相邻视频之间插入中间视频
    pub middle_enabled: bool,
    pub middle: AssetSource,
    /// 输出名为各源文件名以空格连接
    pub join_names: bool,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            mode: MergeMode::Lossless,
            fallback_reencode: true,
            transition: "none".into(),
            transition_seconds: 1.0,
            middle_enabled: false,
            middle: AssetSource::default(),
            join_names: false,
        }
    }
}

impl MergeOptions {
    pub fn validate(&self) -> Result<(), String> {
        let t = self.transition.as_str();
        if t != "none" && t != "random" && !TRANSITIONS.contains(&t) {
            return Err(format!("未知的转场：{t}"));
        }
        if self.mode == MergeMode::Reencode
            && t != "none"
            && !(MIN_TRANSITION..=MAX_TRANSITION).contains(&self.transition_seconds)
        {
            return Err(format!(
                "转场时长必须在 {}～{} 秒之间",
                fmt_num(MIN_TRANSITION),
                fmt_num(MAX_TRANSITION)
            ));
        }
        if self.middle_enabled && self.middle.is_empty() {
            return Err("已开启中间视频，但没有选择文件或目录".into());
        }
        Ok(())
    }
}
// ---------- 规划 / 分组（纯函数） ----------

/// Fisher–Yates 打乱（由种子决定）。
pub fn shuffle<T>(items: &mut [T], rng: &mut SeededRng) {
    for i in (1..items.len()).rev() {
        let j = rng.index(i + 1);
        items.swap(i, j);
    }
}

/// 把顺序切成每 `size` 个一组；None 或 0 表示全部一组。空输入返回空。
pub fn chunk_order(order: Vec<usize>, size: Option<usize>) -> Vec<Vec<usize>> {
    match size {
        Some(size) if size > 0 => order.chunks(size).map(<[usize]>::to_vec).collect(),
        _ if order.is_empty() => Vec::new(),
        _ => vec![order],
    }
}

/// 把输入按每 `size` 个分成工作项（ID `<工具>#<组号>`，标签“第 k 组（m 个）”）。
pub fn chunk_items(tool_id: &str, inputs: Vec<ItemInput>, size: usize) -> Vec<WorkItem> {
    let size = size.max(1);
    inputs
        .chunks(size)
        .enumerate()
        .map(|(k, chunk)| {
            WorkItem::new(
                format!("{tool_id}#{}", k + 1),
                format!("第 {} 组（{} 个视频）", k + 1, chunk.len()),
                k + 1,
                chunk.to_vec(),
            )
        })
        .collect()
}

/// 输出名主干：`joinNames` 时各名字以空格连接（过长截断），否则用 `fallback`。
pub fn output_stem(stems: &[String], join_names: bool, fallback: &str) -> String {
    if !join_names || stems.is_empty() {
        return fallback.to_owned();
    }
    let joined = stems.join(" ");
    if joined.chars().count() <= MAX_JOINED_CHARS {
        return joined;
    }
    joined
        .chars()
        .take(MAX_JOINED_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// 每个衔接处的转场：`none` 为空列表；`random` 每处随机取一个。
pub fn pick_transitions(name: &str, junctions: usize, rng: &mut SeededRng) -> Vec<String> {
    match name {
        "none" => Vec::new(),
        "random" => (0..junctions)
            .map(|_| TRANSITIONS[rng.index(TRANSITIONS.len())].to_owned())
            .collect(),
        other => vec![other.to_owned(); junctions],
    }
}

/// 实际使用的转场时长：不小于最短片段时缩短为最短片段的一半。返回（时长，是否缩短）。
pub fn effective_fade(fade: f64, durations: &[f64]) -> (f64, bool) {
    let shortest = durations.iter().copied().fold(f64::INFINITY, f64::min);
    if shortest.is_finite() && fade >= shortest {
        ((shortest / 2.0 * 1000.0).floor() / 1000.0, true)
    } else {
        (fade, false)
    }
}

// ---------- 片段与规格 ----------

/// 合并的一个片段：整个文件，或文件中的一段（`cut`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub path: PathBuf,
    pub info: MediaInfo,
    /// 截取的时间段（None：整个文件）
    pub cut: Option<Interval>,
}

impl Segment {
    pub fn whole(path: impl Into<PathBuf>, info: MediaInfo) -> Self {
        Self {
            path: path.into(),
            info,
            cut: None,
        }
    }

    pub fn duration(&self) -> f64 {
        self.cut.map_or(self.info.duration, |c| c.duration())
    }

    pub fn has_audio(&self) -> bool {
        !self.info.audio.is_empty()
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// 无损合并的障碍；全部兼容时为 None。
pub fn lossless_problem(segments: &[Segment]) -> Option<String> {
    let first = segments.first()?;
    let Some(fv) = first.info.video.as_ref() else {
        return Some(format!("{} 没有视频流", first.name()));
    };
    let fa = first.info.audio.first();
    for seg in &segments[1..] {
        let pair = format!("{} 与 {}", first.name(), seg.name());
        let Some(v) = seg.info.video.as_ref() else {
            return Some(format!("{} 没有视频流", seg.name()));
        };
        if !v.codec.eq_ignore_ascii_case(&fv.codec) {
            return Some(format!(
                "{pair} 的视频编码不同（{} / {}）",
                fv.codec, v.codec
            ));
        }
        if v.display_size() != fv.display_size() {
            let (a, b) = (fv.display_size(), v.display_size());
            return Some(format!(
                "{pair} 的分辨率不同（{}x{} / {}x{}）",
                a.0, a.1, b.0, b.1
            ));
        }
        if v.sar_ratio() != fv.sar_ratio() {
            return Some(format!(
                "{pair} 的像素宽高比不同（{} / {}）",
                if fv.sar.is_empty() { "1:1" } else { &fv.sar },
                if v.sar.is_empty() { "1:1" } else { &v.sar }
            ));
        }
        if v.pix_fmt != fv.pix_fmt {
            return Some(format!(
                "{pair} 的像素格式不同（{} / {}）",
                fv.pix_fmt, v.pix_fmt
            ));
        }
        if (v.fps - fv.fps).abs() > 0.01 {
            return Some(format!(
                "{pair} 的帧率不同（{} / {}）",
                fmt_num(fv.fps),
                fmt_num(v.fps)
            ));
        }
        match (fa, seg.info.audio.first()) {
            (Some(_), None) | (None, Some(_)) => {
                return Some(format!("{pair} 一个有音频、一个没有音频"));
            }
            (Some(a), Some(b))
                if !a.codec.eq_ignore_ascii_case(&b.codec)
                    || a.sample_rate != b.sample_rate
                    || a.channels != b.channels =>
            {
                return Some(format!(
                    "{pair} 的音频格式不同（{} {}Hz {}声道 / {} {}Hz {}声道）",
                    a.codec, a.sample_rate, a.channels, b.codec, b.sample_rate, b.channels
                ));
            }
            _ => {}
        }
    }
    None
}

/// 统一后的音频布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTarget {
    pub sample_rate: u32,
    pub channels: u32,
    pub layout: &'static str,
}

impl AudioTarget {
    pub fn from_stream(sample_rate: u32, channels: u32) -> Self {
        let (channels, layout) = match channels {
            1 => (1, "mono"),
            6 => (6, "5.1"),
            8 => (8, "7.1"),
            _ => (2, "stereo"),
        };
        Self {
            sample_rate: if sample_rate == 0 {
                48_000
            } else {
                sample_rate
            },
            channels,
            layout,
        }
    }
}

/// 重编码时各片段统一到的规格。
#[derive(Debug, Clone, PartialEq)]
pub struct ConformTarget {
    pub width: u32,
    pub height: u32,
    /// 帧率在滤镜里的写法（`25`、`30000/1001`）
    pub rate: String,
    /// 帧率数值
    pub fps: f64,
    /// None：全部片段都没有音频
    pub audio: Option<AudioTarget>,
}

/// 帧率在滤镜里的写法：优先用分数原文（与平均帧率相符时），否则用平均帧率；未知时 25。
pub fn rate_text(video: &VideoStream) -> String {
    if let Some((n, d)) = video.fps_text.split_once('/') {
        if let (Ok(n), Ok(d)) = (n.trim().parse::<u64>(), d.trim().parse::<u64>()) {
            if n > 0 && d > 0 {
                let value = n as f64 / d as f64;
                if value <= 240.0 && (video.fps <= 0.0 || (value - video.fps).abs() < 0.5) {
                    return if d == 1 {
                        n.to_string()
                    } else {
                        format!("{n}/{d}")
                    };
                }
            }
        }
    }
    if video.fps.is_finite() && video.fps > 0.0 && video.fps <= 240.0 {
        fmt_num(video.fps)
    } else {
        "25".into()
    }
}

/// 以第一个片段为准的统一规格。
pub fn conform_target(first: &MediaInfo, any_audio: bool) -> Result<ConformTarget, String> {
    let video = first
        .video
        .as_ref()
        .ok_or_else(|| format!("{} 没有视频流", first.path))?;
    let (w, h) = video.square_display_size();
    if w == 0 || h == 0 {
        return Err(format!("无法读取 {} 的分辨率", first.path));
    }
    let even = |v: u32| (v - v % 2).max(2);
    let audio = match first.audio.first() {
        Some(a) => Some(AudioTarget::from_stream(a.sample_rate, a.channels)),
        None if any_audio => Some(AudioTarget::from_stream(48_000, 2)),
        None => None,
    };
    let rate = rate_text(video);
    Ok(ConformTarget {
        width: even(w),
        height: even(h),
        fps: rate_value(&rate),
        rate,
        audio,
    })
}

/// 帧率写法（`25`、`30000/1001`）的数值；无效时 25。
pub fn rate_value(rate: &str) -> f64 {
    let value = match rate.split_once('/') {
        Some((n, d)) => match (n.trim().parse::<f64>(), d.trim().parse::<f64>()) {
            (Ok(n), Ok(d)) if d > 0.0 => n / d,
            _ => 0.0,
        },
        None => rate.trim().parse().unwrap_or(0.0),
    };
    if value.is_finite() && value > 0.0 {
        value
    } else {
        25.0
    }
}

/// 时长对应的整帧数（至少 1 帧）。
pub fn frame_count(duration: f64, fps: f64) -> u64 {
    (duration * fps).round().max(1.0) as u64
}

/// 取整到整帧的时长。
pub fn quantize(duration: f64, fps: f64) -> f64 {
    frame_count(duration, fps) as f64 / fps
}

/// 片段在滤镜图里的描述：时长与是否有音频。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphSegment {
    pub duration: f64,
    pub has_audio: bool,
}

/// 重编码合并的滤镜图（第 i 个片段是第 i 个输入），输出 `[vout]`（与有音频时的 `[aout]`）。
/// `transitions` 为空时用 concat 滤镜；否则长度为片段数 − 1，用 xfade / acrossfade 逐个衔接。
pub fn conform_graph(
    segments: &[GraphSegment],
    target: &ConformTarget,
    transitions: &[String],
    fade: f64,
) -> String {
    let (w, h, r) = (target.width, target.height, &target.rate);
    // 每段取整到整帧：视频正好 N 帧，音频正好 N / 帧率 秒，拼接后音画不累积偏差
    let segments: Vec<GraphSegment> = segments
        .iter()
        .map(|s| GraphSegment {
            duration: quantize(s.duration, target.fps),
            ..*s
        })
        .collect();
    let mut chains = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let d = secs(seg.duration);
        let frames = frame_count(seg.duration, target.fps);
        chains.push(format!(
            "[{i}:V:0]{SQUARE_PIXELS},scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps={r},format=yuv420p,\
             tpad=stop_mode=clone:stop_duration={d},trim=end_frame={frames},setpts=PTS-STARTPTS[v{i}]"
        ));
        if let Some(a) = &target.audio {
            let fmt = format!(
                "aformat=sample_fmts=fltp:sample_rates={}:channel_layouts={}",
                a.sample_rate, a.layout
            );
            if seg.has_audio {
                chains.push(format!(
                    "[{i}:a:0]aresample={},{fmt},apad=whole_dur={d},atrim=duration={d},asetpts=PTS-STARTPTS[a{i}]",
                    a.sample_rate
                ));
            } else {
                chains.push(format!(
                    "anullsrc=r={}:cl={},{fmt},atrim=duration={d}[a{i}]",
                    a.sample_rate, a.layout
                ));
            }
        }
    }
    let n = segments.len();
    let audio = target.audio.is_some();
    if transitions.is_empty() || n < 2 {
        let inputs: String = (0..n)
            .map(|i| {
                if audio {
                    format!("[v{i}][a{i}]")
                } else {
                    format!("[v{i}]")
                }
            })
            .collect();
        let outputs = if audio { "[vout][aout]" } else { "[vout]" };
        chains.push(format!(
            "{inputs}concat=n={n}:v=1:a={}{outputs}",
            u8::from(audio)
        ));
        return chains.join(";");
    }
    let fade_text = secs(fade);
    let mut elapsed = segments[0].duration;
    let (mut vprev, mut aprev) = ("v0".to_owned(), "a0".to_owned());
    for (k, seg) in segments.iter().enumerate().skip(1) {
        let last = k == n - 1;
        let vout = if last {
            "vout".to_owned()
        } else {
            format!("vx{k}")
        };
        let offset = elapsed - fade;
        let transition = transitions.get(k - 1).map_or("fade", String::as_str);
        chains.push(format!(
            "[{vprev}][v{k}]xfade=transition={transition}:duration={fade_text}:offset={}[{vout}]",
            secs(offset)
        ));
        if audio {
            let aout = if last {
                "aout".to_owned()
            } else {
                format!("ax{k}")
            };
            chains.push(format!(
                "[{aprev}][a{k}]acrossfade=d={fade_text}:c1=tri:c2=tri[{aout}]"
            ));
            aprev = aout;
        }
        vprev = vout;
        elapsed += seg.duration - fade;
    }
    chains.join(";")
}

/// 合并后的总时长。
pub fn merged_duration(durations: &[f64], transitions: usize, fade: f64) -> f64 {
    let sum: f64 = durations.iter().sum();
    if transitions == 0 {
        sum
    } else {
        sum - fade * transitions as f64
    }
}

/// 重编码命令的一个输入。
#[derive(Debug, Clone, PartialEq)]
pub struct InputArg {
    pub path: String,
    pub cut: Option<Interval>,
}

/// 重编码合并的参数（不含通用开头参数）：滤镜图从 `script` 文件读取（`-/filter_complex`）。
#[allow(clippy::too_many_arguments)]
pub fn encode_args(
    inputs: &[InputArg],
    script: &str,
    audio: Option<&AudioTarget>,
    encoding: &Encoding,
    container: &str,
    trim: Option<f64>,
    output: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    for input in inputs {
        if let Some(cut) = input.cut {
            args.extend([
                "-ss".into(),
                secs(cut.start),
                "-t".into(),
                secs(cut.duration()),
            ]);
        }
        args.extend(["-i".into(), input.path.clone()]);
    }
    args.extend([
        "-/filter_complex".into(),
        script.into(),
        "-map".into(),
        "[vout]".into(),
    ]);
    if audio.is_some() {
        args.extend(["-map".into(), "[aout]".into()]);
    }
    args.extend(encoding.video_args(container));
    if let Some(a) = audio {
        args.extend(Encoding::audio_args(a.channels));
    }
    if let Some(t) = trim {
        args.extend(["-t".into(), secs(t)]);
    }
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

/// 无损合并（concat 分离器）的参数。
pub fn concat_copy_args(list: &str, has_audio: bool, container: &str, output: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-f", "concat", "-safe", "0", "-i", list, "-map", "0:v:0"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    if has_audio {
        args.extend(["-map".into(), "0:a:0".into()]);
    }
    args.extend(["-c".into(), "copy".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

// ---------- 执行（ToolContext） ----------

/// 输出的命名：主干与“以第几个输入为准”（决定输出目录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputName {
    pub stem: String,
    pub input: usize,
}

/// 在每两个相邻片段之间插入一个中间视频（目录时每处各取一个）。
pub fn insert_middles(
    ctx: &mut ToolContext,
    source: &AssetSource,
    clips: Vec<Segment>,
) -> Result<Vec<Segment>, String> {
    let mut cache: HashMap<PathBuf, MediaInfo> = HashMap::new();
    let mut out = Vec::with_capacity(clips.len() * 2);
    let total = clips.len();
    for (i, clip) in clips.into_iter().enumerate() {
        out.push(clip);
        if i + 1 < total {
            let path = source.pick(VIDEO_EXTENSIONS, i, &mut ctx.rng)?;
            let info = match cache.get(&path) {
                Some(info) => info.clone(),
                None => {
                    let info = ctx
                        .probe(&path)
                        .map_err(|e| format!("无法读取中间视频 {}：{e}", path.display()))?;
                    cache.insert(path.clone(), info.clone());
                    info
                }
            };
            out.push(Segment::whole(path, info));
        }
    }
    Ok(out)
}

/// 按合并参数把片段合成一个输出（中间视频、无损 / 重编码、转场）。占一步进度。
pub fn merge_segments(
    ctx: &mut ToolContext,
    opts: &MergeOptions,
    segments: Vec<Segment>,
    name: &OutputName,
) -> Result<(), ToolError> {
    if segments.is_empty() {
        return Err("没有可合并的视频".into());
    }
    let segments = if opts.middle_enabled {
        insert_middles(ctx, &opts.middle, segments)?
    } else {
        segments
    };
    if opts.mode == MergeMode::Lossless {
        match lossless_problem(&segments) {
            None => {
                if opts.transition != "none" {
                    ctx.warn("无损合并不支持转场，已忽略转场");
                }
                return concat_copy(ctx, &segments, name);
            }
            Some(reason) if opts.fallback_reencode => {
                ctx.warn(format!("无法无损合并（{reason}），已改为重编码"));
                return encode_segments(ctx, &segments, 0, &[], 0.0, None, name);
            }
            Some(reason) => {
                return Err(format!(
                    "无法无损合并：{reason}。请改用重编码，或开启“不兼容时自动重编码”"
                )
                .into());
            }
        }
    }
    let junctions = segments.len().saturating_sub(1);
    let transitions = pick_transitions(&opts.transition, junctions, &mut ctx.rng);
    let mut fade = opts.transition_seconds;
    if !transitions.is_empty() {
        let durations: Vec<f64> = segments.iter().map(Segment::duration).collect();
        let (effective, shortened) = effective_fade(fade, &durations);
        if shortened {
            ctx.warn(format!(
                "转场时长不能超过最短片段，已缩短为 {} 秒",
                fmt_num(effective)
            ));
        }
        fade = effective;
    }
    encode_segments(ctx, &segments, 0, &transitions, fade, None, name)
}

/// 无损合并：写 concat 列表，直接复制。
pub fn concat_copy(
    ctx: &mut ToolContext,
    segments: &[Segment],
    name: &OutputName,
) -> Result<(), ToolError> {
    let first = &segments[0];
    let codec = first
        .info
        .video
        .as_ref()
        .map(|v| v.codec.clone())
        .unwrap_or_default();
    let container = copy_container(ctx.settings(), &first.path, &codec);
    let files: Vec<PathBuf> = segments.iter().map(|s| s.path.clone()).collect();
    let list = ctx.temp_file("concat.txt")?;
    write_concat_list(&list, &files)?;
    let out = ctx.output(
        &OutputSpec::file(&container)
            .stem(&name.stem)
            .from_input(name.input),
    )?;
    let total: f64 = segments.iter().map(Segment::duration).sum();
    ctx.run_ffmpeg(
        "无损合并",
        concat_copy_args(
            &path_arg(&list)?,
            first.has_audio(),
            &container,
            &path_arg(&out)?,
        ),
        total,
    )
}

/// 重编码合并：统一到第 `reference` 个片段的规格（合并时为第一个；有片头时为第一个正片），
/// 可选转场，可选裁到 `trim` 秒。
#[allow(clippy::too_many_arguments)]
pub fn encode_segments(
    ctx: &mut ToolContext,
    segments: &[Segment],
    reference: usize,
    transitions: &[String],
    fade: f64,
    trim: Option<f64>,
    name: &OutputName,
) -> Result<(), ToolError> {
    let end = trim.map_or(OutputEnd::Natural, OutputEnd::Trim);
    encode_segments_to(ctx, segments, reference, transitions, fade, end, name)
}

/// 合并结果的时长处理。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputEnd {
    /// 保持拼接后的自然时长
    Natural,
    /// 截到这么多秒（超出部分丢弃）
    Trim(f64),
    /// 变速适应（`set.speed-fit`）：整体变速到这么多秒；倍数超出 0.25～4 时退回 `Trim` 并警告
    SpeedFit(f64),
}

/// 把合并滤镜图的最终输出 `[vout]` / `[aout]` 接上变速适应滤镜（标签不变）。
pub fn append_speed_fit(graph: &str, fit: &SpeedFit, with_audio: bool) -> String {
    let mut graph = graph.replacen("[vout]", "[vfit]", 1);
    graph.push_str(&format!(";[vfit]{}[vout]", fit.video));
    if with_audio {
        graph = graph.replacen("[aout]", "[afit]", 1);
        graph.push_str(&format!(";[afit]{}[aout]", fit.audio));
    }
    graph
}

/// 同 [`encode_segments`]，时长处理由 `end` 指定。
pub fn encode_segments_to(
    ctx: &mut ToolContext,
    segments: &[Segment],
    reference: usize,
    transitions: &[String],
    fade: f64,
    end: OutputEnd,
    name: &OutputName,
) -> Result<(), ToolError> {
    let any_audio = segments.iter().any(Segment::has_audio);
    let reference = segments
        .get(reference)
        .or_else(|| segments.first())
        .ok_or("没有可合并的视频")?;
    let target = conform_target(&reference.info, any_audio)?;
    let graph_segments: Vec<GraphSegment> = segments
        .iter()
        .map(|s| GraphSegment {
            duration: s.duration(),
            has_audio: s.has_audio(),
        })
        .collect();
    let mut graph = conform_graph(&graph_segments, &target, transitions, fade);
    let natural: Vec<f64> = graph_segments
        .iter()
        .map(|s| quantize(s.duration, target.fps))
        .collect();
    let natural = merged_duration(&natural, transitions.len(), fade);
    let mut fitted = false;
    let trim = match end {
        OutputEnd::Natural => None,
        OutputEnd::Trim(t) => Some(t),
        OutputEnd::SpeedFit(t) => match speed_fit(natural, t) {
            Ok(Some(fit)) => {
                ctx.note(format!(
                    "变速适应：{} 秒 → {} 秒（{} 倍时长）",
                    fmt_num(natural),
                    fmt_num(t),
                    fmt_num(fit.factor)
                ));
                graph = append_speed_fit(&graph, &fit, target.audio.is_some());
                fitted = true;
                // 变速后的时长有帧级误差：截到目标，保证正好是目标时长
                Some(t)
            }
            Ok(None) => Some(t),
            Err(e) => {
                ctx.warn(format!("{e}，改为截到目标时长"));
                Some(t)
            }
        },
    };
    let script = ctx.temp_file("merge-filter.txt")?;
    std::fs::write(&script, graph).map_err(|e| format!("无法写入滤镜脚本：{e}"))?;
    let inputs = segments
        .iter()
        .map(|s| {
            Ok(InputArg {
                path: path_arg(&s.path)?,
                cut: s.cut,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let container = ctx.video_extension(name.input)?;
    let out = ctx.output(
        &OutputSpec::file(&container)
            .stem(&name.stem)
            .from_input(name.input),
    )?;
    let durations: Vec<f64> = graph_segments
        .iter()
        .map(|s| quantize(s.duration, target.fps))
        .collect();
    let total = merged_duration(&durations, transitions.len(), fade);
    let expected = match trim {
        Some(t) if fitted => t,
        _ => trim.map_or(total, |t| t.min(total)),
    };
    let script = path_arg(&script)?;
    let out = path_arg(&out)?;
    // 码率依据：组成这个输出的那些输入（不一定是第 0 个），输出画面为统一后的规格
    let mut sources: Vec<usize> = Vec::new();
    for seg in segments {
        if let Some(i) = ctx.inputs().iter().position(|input| input.path == seg.path) {
            if !sources.contains(&i) {
                sources.push(i);
            }
        }
    }
    let basis = EncodeBasis::inputs(sources).frame(target.width, target.height, target.fps);
    ctx.run_encode_from("合并", expected, &basis, |enc| {
        Ok(encode_args(
            &inputs,
            &script,
            target.audio.as_ref(),
            enc,
            &container,
            trim,
            &out,
        ))
    })
}

/// 合并一个工作项里的若干组（每组按输入序号），每组一个输出、一步进度。
/// `fallback_stem(ctx, 组内序号)` 给出不连接文件名时的输出名。
pub fn run_groups(
    ctx: &mut ToolContext,
    opts: &MergeOptions,
    groups: Vec<Vec<usize>>,
    fallback_stem: impl Fn(&ToolContext, &[usize]) -> Result<String, String>,
) -> Result<(), ToolError> {
    let mut weights = Vec::with_capacity(groups.len());
    for group in &groups {
        let mut sum = 0.0;
        for &i in group {
            sum += ctx.info(i)?.duration;
        }
        weights.push(sum.max(0.1));
    }
    if groups.len() > 1 {
        ctx.set_steps(&weights);
    }
    for group in groups {
        ctx.check_cancelled()?;
        let mut segments = Vec::with_capacity(group.len());
        let mut stems = Vec::with_capacity(group.len());
        for &i in &group {
            let info = ctx.info(i)?;
            segments.push(Segment::whole(ctx.input_path(i)?.to_path_buf(), info));
            stems.push(ctx.stem(i)?);
        }
        let fallback = fallback_stem(ctx, &group)?;
        let name = OutputName {
            stem: output_stem(&stems, opts.join_names, &fallback),
            input: group[0],
        };
        merge_segments(ctx, opts, segments, &name)?;
    }
    Ok(())
}

/// 默认输出名：`<第一个视频名>_merged`。
pub fn merged_stem(ctx: &ToolContext, group: &[usize]) -> Result<String, String> {
    Ok(format!("{}_merged", ctx.stem(group[0])?))
}

/// 一个工作项的输入顺序（需要时按种子打乱）。
pub fn item_order(ctx: &mut ToolContext, shuffle_order: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..ctx.inputs().len()).collect();
    if shuffle_order {
        shuffle(&mut order, &mut ctx.rng);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_fit_is_appended_to_the_final_outputs() {
        let fit = speed_fit(10.0, 5.0).unwrap().unwrap();
        let graph = "[0:v]scale=2:2[v0];[v0][a0]concat=n=1:v=1:a=1[vout][aout]";
        assert_eq!(
            append_speed_fit(graph, &fit, true),
            "[0:v]scale=2:2[v0];[v0][a0]concat=n=1:v=1:a=1[vfit][afit];[vfit]setpts=0.5*PTS[vout];[afit]atempo=2[aout]"
        );
        assert!(!append_speed_fit("[x]null[vout]", &fit, false).contains("afit"));
    }
}
