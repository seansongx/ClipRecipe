//! 一进一出的“滤镜图 + 重编码”命令（[`FilterJob`] / [`run_job`]）与常用小函数：帧率 [`Rate`]、
//! 插入空档的视频 / 音频滤镜（[`gap_video_filter`] / [`gap_audio_filter`]）、偶数裁剪 [`even_crop`]、
//! 颜色 [`color_hex`]、图片输入参数 [`image_input_args`]。
//!
//! 使用者：定帧动画、定格播放、插帧、运镜、去黑边、进度条（原先放在 `freeze_anim.rs` / `freeze_play.rs`，
//! 那里保留了同名的再导出）。

use crate::ffmpeg::{path_arg, AudioStream, MediaInfo};
use crate::pipeline::modules::text::escape_value;
use crate::tools::context::Encoding;
use crate::tools::media::{container_accepts, secs, StreamKind};
use crate::tools::{OutputSpec, ToolContext, ToolError};
// ---------------------------------------------------------------------------------------------
// 本组工具共用的小函数
// ---------------------------------------------------------------------------------------------

/// 帧率：滤镜里的写法（`25`、`30000/1001`）与分子分母。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    pub text: String,
    pub num: u64,
    pub den: u64,
}

impl Rate {
    /// 源视频的帧率（优先用帧率原文；读不到时用平均帧率的近似分数）。
    pub fn of(info: &MediaInfo) -> Result<Rate, String> {
        let video = info.video.as_ref().ok_or("文件中没有视频流")?;
        let parsed = video.fps_text.trim().split_once('/').map_or_else(
            || video.fps_text.trim().parse::<u64>().ok().map(|n| (n, 1)),
            |(n, d)| Some((n.trim().parse::<u64>().ok()?, d.trim().parse::<u64>().ok()?)),
        );
        let (num, den) = match parsed {
            Some((n, d)) if n > 0 && d > 0 && (n as f64 / d as f64) <= 1000.0 => (n, d),
            _ if video.fps.is_finite() && video.fps > 0.0 && video.fps <= 1000.0 => {
                ((video.fps * 1000.0).round() as u64, 1000)
            }
            _ => return Err("无法读取视频帧率".into()),
        };
        let g = gcd(num, den);
        let (num, den) = (num / g, den / g);
        let text = if den == 1 {
            num.to_string()
        } else {
            format!("{num}/{den}")
        };
        Ok(Rate { text, num, den })
    }

    pub fn fps(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// 帧数 → 秒。
    pub fn seconds(&self, frames: f64) -> f64 {
        frames * self.den as f64 / self.num as f64
    }

    /// 时间点之后（含）的第一帧的序号。
    pub fn frame_at(&self, time: f64) -> i64 {
        (time * self.fps() - 1e-6).ceil().max(0.0) as i64
    }

    /// 时长对应的帧数（四舍五入，至少 1 帧）。
    pub fn frames_for(&self, seconds: f64) -> u32 {
        ((seconds * self.fps()).round() as u32).max(1)
    }

    /// 视频的总帧数（读不到帧数时按时长估算）。
    pub fn total_frames(&self, info: &MediaInfo) -> i64 {
        info.video
            .as_ref()
            .and_then(|v| v.frames)
            .filter(|f| *f > 0)
            .map_or_else(|| (info.duration * self.fps()).round() as i64, |f| f as i64)
            .max(1)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// 完整精度的小数（写进表达式，避免 4 位小数的累积误差）；整数写成整数。
pub fn float_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

pub fn check_duration(duration: f64) -> Result<(), String> {
    if duration.is_finite() && duration > 0.0 {
        Ok(())
    } else {
        Err("无法读取视频时长".into())
    }
}

/// 显示尺寸（已考虑旋转；FFmpeg 解码时自动转正）。
pub fn display_size(info: &MediaInfo) -> Result<(u32, u32), String> {
    let (w, h) = info
        .video
        .as_ref()
        .ok_or("文件中没有视频流")?
        .display_size();
    if w < 2 || h < 2 {
        return Err("无法读取视频分辨率".into());
    }
    Ok((w, h))
}

/// 宽或高为奇数时裁成偶数（yuv420p 编码要求），返回要接在滤镜链后的 `,crop=…`；偶数尺寸返回空串。
pub fn even_crop(w: u32, h: u32) -> String {
    if w % 2 == 1 || h % 2 == 1 {
        format!(",crop={}:{}:0:0", w & !1, h & !1)
    } else {
        String::new()
    }
}

/// 在第 `after` 帧之后（`-1` 表示最开头）各插入 `gap_frames` 帧的空档：先 `fps` 统一为恒定帧率，
/// `setpts` 把其后的帧整体后移，再用 `fps` 以前一帧（开头时为第一帧）填满空档。
pub fn gap_video_filter(after: &[i64], gap_frames: u32, rate: &Rate) -> String {
    let terms: Vec<String> = after.iter().map(|g| format!("gt(N,{g})")).collect();
    let expr = format!(
        "PTS+({})*{gap_frames}*{}/{}/TB",
        terms.join("+"),
        rate.den,
        rate.num
    );
    format!(
        "fps={r},setpts={},fps={r}:start_time=0",
        escape_value(&expr),
        r = rate.text
    )
}

/// 音频声道布局名（静音片段用）。
fn layout_name(channels: u32) -> String {
    match channels {
        0 | 1 => "mono".into(),
        2 => "stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n}c"),
    }
}

/// 在源音频的时间点 `cuts`（秒）各插入 `gap` 秒静音：`asplit` + `atrim` 切段，与 `anullsrc` 静音片段 `concat`。
/// `input` / `output` 是滤镜标签（如 `[0:a:0]`、`[a]`）。
pub fn gap_audio_filter(
    input: &str,
    cuts: &[f64],
    gap: f64,
    audio: &AudioStream,
    output: &str,
) -> String {
    let mut cuts: Vec<f64> = cuts.iter().copied().filter(|c| c.is_finite()).collect();
    cuts.sort_by(f64::total_cmp);
    if cuts.is_empty() {
        return format!("{input}anull{output}");
    }
    // 段的边界：0, cut1, cut2, …, 结尾；长度为 0 的段跳过
    let mut pieces: Vec<(Option<f64>, Option<f64>)> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut start: Option<f64> = None;
    let rate = if audio.sample_rate > 0 {
        audio.sample_rate
    } else {
        48_000
    };
    let silence = format!(
        "anullsrc=r={rate}:cl={},atrim=duration={}",
        layout_name(audio.channels),
        secs(gap)
    );
    let mut silences = Vec::new();
    for (k, cut) in cuts.iter().enumerate() {
        if start.map_or(true, |s| *cut > s + 1e-9) && *cut > 1e-9 {
            order.push(format!("[gs{}]", pieces.len()));
            pieces.push((start, Some(*cut)));
        }
        order.push(format!("[gz{k}]"));
        silences.push(format!("{silence}[gz{k}]"));
        start = Some(*cut);
    }
    order.push(format!("[gs{}]", pieces.len()));
    pieces.push((start, None));

    let mut parts = Vec::new();
    let labels: Vec<String> = (0..pieces.len()).map(|i| format!("[ga{i}]")).collect();
    parts.push(format!("{input}asplit={}{}", pieces.len(), labels.concat()));
    for (i, (from, to)) in pieces.iter().enumerate() {
        let mut opts = Vec::new();
        if let Some(from) = from {
            opts.push(format!("start={}", secs(*from)));
        }
        if let Some(to) = to {
            opts.push(format!("end={}", secs(*to)));
        }
        parts.push(format!(
            "[ga{i}]atrim={},asetpts=PTS-STARTPTS[gs{i}]",
            opts.join(":")
        ));
    }
    parts.extend(silences);
    parts.push(format!(
        "{}concat=n={}:v=0:a=1{output}",
        order.concat(),
        order.len()
    ));
    parts.join(";")
}

/// 输出的音频怎么处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOut {
    /// 没有音频
    None,
    /// 第一条音轨不经过滤镜：容器装得下就直接复制，否则转 AAC
    Source { channels: u32 },
    /// 滤镜输出 `[a]`，转 AAC
    Filtered { channels: u32 },
}

/// 音频不经过滤镜时：容器装得下就复制，否则转 AAC；没有音频为 None。
pub fn source_audio(info: &MediaInfo) -> AudioOut {
    match info.audio.first() {
        None => AudioOut::None,
        Some(a) => AudioOut::Source {
            channels: a.channels,
        },
    }
}

/// 一条“滤镜图 + 重编码”命令（不含输出路径与编码参数，二者在 [`FilterJob::args`] 里补上）。
#[derive(Debug, Clone, PartialEq)]
pub struct FilterJob {
    /// 输入参数（含 `-i`），第 0 个输入是源视频
    pub inputs: Vec<String>,
    /// `-filter_complex`：视频输出标签 `[v]`，滤镜处理音频时输出 `[a]`
    pub graph: String,
    pub audio: AudioOut,
    /// 源音频编码（判断能否直接复制）
    pub audio_codec: String,
    /// 预计输出时长（秒）
    pub duration: f64,
    /// 非致命提示
    pub warnings: Vec<String>,
}

impl FilterJob {
    pub fn new(input: &str) -> Self {
        Self {
            inputs: vec!["-i".into(), input.into()],
            graph: String::new(),
            audio: AudioOut::None,
            audio_codec: String::new(),
            duration: 0.0,
            warnings: Vec::new(),
        }
    }

    /// 加一个输入（前置参数 + 路径），返回它的输入序号。
    pub fn add_input(&mut self, before: &[&str], path: &str) -> usize {
        let index = self.inputs.iter().filter(|a| *a == "-i").count();
        self.inputs.extend(before.iter().map(|s| (*s).to_owned()));
        self.inputs.extend(["-i".to_owned(), path.to_owned()]);
        index
    }

    /// 完整参数（不含开头的通用参数）。
    pub fn args(&self, encoding: &Encoding, container: &str, output: &str) -> Vec<String> {
        let mut args = self.inputs.clone();
        args.extend([
            "-filter_complex".into(),
            self.graph.clone(),
            "-map".into(),
            "[v]".into(),
        ]);
        let audio_args = match self.audio {
            AudioOut::None => {
                args.push("-an".into());
                Vec::new()
            }
            AudioOut::Source { channels } => {
                args.extend(["-map".into(), "0:a:0".into()]);
                if container_accepts(container, StreamKind::Audio, &self.audio_codec) == Some(true)
                {
                    vec!["-c:a".into(), "copy".into()]
                } else {
                    Encoding::audio_args(channels)
                }
            }
            AudioOut::Filtered { channels } => {
                args.extend(["-map".into(), "[a]".into()]);
                Encoding::audio_args(channels)
            }
        };
        args.extend(encoding.video_args(container));
        args.extend(audio_args);
        args.extend(["-sn".into(), "-dn".into()]);
        if matches!(container, "mp4" | "mov" | "m4v") {
            args.extend(["-movflags".into(), "+faststart".into()]);
        }
        args.push(output.into());
        args
    }
}

/// 运行一条 [`FilterJob`]：输出与源视频同名（按设置的目标格式），沿用码率设置，记录提示。
pub fn run_job(ctx: &mut ToolContext, label: &str, job: &FilterJob) -> Result<(), ToolError> {
    for warning in &job.warnings {
        ctx.warn(warning.clone());
    }
    let container = ctx.video_extension(0)?;
    let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
    let mut job = job.clone();
    if job.audio_codec.is_empty() {
        if let Some(audio) = ctx.info(0)?.audio.first() {
            job.audio_codec = audio.codec.clone();
        }
    }
    ctx.run_encode(label, job.duration, |enc| {
        Ok(job.args(enc, &container, &output))
    })
}

/// `#rrggbb`（或 `#rgb`）→ FFmpeg 颜色 `0xrrggbb`。
pub fn color_hex(text: &str, what: &str) -> Result<String, String> {
    let hex = text.trim().trim_start_matches('#');
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_owned(),
        _ => String::new(),
    };
    if full.len() == 6 && full.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(format!("0x{}", full.to_ascii_lowercase()))
    } else {
        Err(format!("{what}不是有效的颜色：{text}"))
    }
}

/// 图片输入的前置参数：gif 循环播放，静态图片重复成视频流（叠加时配合 `shortest=1` 随主视频结束）。
pub fn image_input_args(path: &str) -> &'static [&'static str] {
    if path.to_ascii_lowercase().ends_with(".gif") {
        &["-stream_loop", "-1"]
    } else {
        &["-loop", "1"]
    }
}
/// 可用作图标 / 插入图片的格式。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
