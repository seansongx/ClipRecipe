//! **片段列表 → 输出** 引擎（[`Timeline`] → [`render`]）：移除重组、镜像重组、转场重组、拆分重组、
//! 静止帧移除、静音移除、智能去头尾共用（原先放在 `remove_remix.rs`，那里保留了同名的再导出）。
//!
//! - 工具先算出片段列表：主视频的若干区间（可加速、可水平翻转）与插入的其它视频，片段之间直接拼接或用转场；
//! - 区间边界先对齐到源视频的帧网格（[`SourceFormat::snap`]），这样每段视频帧数 × 帧时长正好等于同一段音频的长度，
//!   拼接几百段也不会累积音画偏移；
//! - **单遍 filter_complex**：`split`/`asplit` 把解码后的主视频分给每段，`trim`/`atrim` 取出区间（音频精确到采样），
//!   `setpts`/`asetpts` 归零后用 `concat`（或 `xfade` + `acrossfade`）拼接，最后一次编码输出。
//!   之所以不用“先分段编码成临时文件再 concat 分离器”：每段 AAC 都有编码延迟，分段越多音频越往后错位；
//!   分段多时还要启动几百次 ffmpeg。也不用 `select='between(t,..)+..'`：`aselect` 只能按整个音频帧
//!   （约 21 ms）取舍，几百处剪切会累积明显的不同步。
//! - 滤镜图写进临时目录里的脚本文件，用 `-/filter_complex <文件>` 传入，几百段（静音移除常见）也不会超出
//!   Windows 命令行长度限制。`split` 出来的每一路在区间开始前就把帧丢掉、区间结束后关闭，不会囤积帧；
//!   只有转场时后一段要提前缓冲“转场时长”那么多帧。
//! - 主视频片段首尾相接、覆盖全片、不变速且没有转场时（镜像重组），音频不经过滤镜，直接 `-map 0:a:0`，保证连续。
//! - **非方形像素**（SAR ≠ 1 的变形源）：主视频在拼接前统一缩放成方形像素（[`SourceFormat`] 的宽高就是
//!   缩放后的显示尺寸，[`square_pixels`]），插入的视频同样先按自己的 SAR 换算，所以拼接后画面比例不变。

use std::path::PathBuf;

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::fmt_num;
use crate::tools::analysis::Interval;
use crate::tools::context::Encoding;
use crate::tools::{OutputSpec, ToolContext, ToolError};
/// 比较时间时的容差（秒）。
const EPS: f64 = 1e-6;

// ======================================================================
// 共享引擎：片段列表 → 输出
// ======================================================================

/// 时间线上的一段。
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    /// 主视频（第 0 个输入）的 `[start, end)`，按 `speed` 倍速播放，`mirror` 为水平翻转。
    Main {
        start: f64,
        end: f64,
        speed: f64,
        mirror: bool,
    },
    /// 插入一个其它视频（额外输入，从 0 起；命令里是第 `input + 1` 个 `-i`），统一成主视频的规格，
    /// 截取 / 补齐到 `duration` 秒；没有音频时补静音。
    Insert {
        input: usize,
        duration: f64,
        has_audio: bool,
    },
}

impl Piece {
    /// 主视频的一段，原速、不翻转。
    pub fn main(start: f64, end: f64) -> Self {
        Piece::Main {
            start,
            end,
            speed: 1.0,
            mirror: false,
        }
    }

    /// 输出中的时长（秒）。
    pub fn duration(&self) -> f64 {
        match self {
            Piece::Main {
                start, end, speed, ..
            } => (end - start).max(0.0) / speed,
            Piece::Insert { duration, .. } => *duration,
        }
    }
}

/// 片段之间怎样衔接。
#[derive(Debug, Clone, PartialEq)]
pub enum Join {
    /// 直接拼接
    Cut,
    /// 每个衔接处用一种 `xfade` 转场（`kinds` 比片段少一个），音频 `acrossfade`，相邻片段重叠 `duration` 秒
    Crossfade { kinds: Vec<String>, duration: f64 },
}

/// 一个输出的完整片段列表。
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub pieces: Vec<Piece>,
    pub join: Join,
}

impl Timeline {
    /// 直接拼接的时间线。
    pub fn cut(pieces: Vec<Piece>) -> Self {
        Self {
            pieces,
            join: Join::Cut,
        }
    }

    /// 输出总时长（转场重叠的部分只算一次）。
    pub fn duration(&self) -> f64 {
        let total: f64 = self.pieces.iter().map(Piece::duration).sum();
        match &self.join {
            Join::Cut => total,
            Join::Crossfade { duration, .. } => {
                total - duration * self.pieces.len().saturating_sub(1) as f64
            }
        }
    }

    /// 是否只是把主视频原样（首尾相接、覆盖全片、不变速、无转场）重新排了一遍——此时音频可以直接沿用。
    pub fn is_continuous(&self, source_duration: f64) -> bool {
        if self.join != Join::Cut || self.pieces.is_empty() {
            return false;
        }
        let mut cursor = 0.0;
        for piece in &self.pieces {
            match piece {
                Piece::Main {
                    start, end, speed, ..
                } if (speed - 1.0).abs() < EPS && (start - cursor).abs() < EPS => cursor = *end,
                _ => return false,
            }
        }
        cursor >= source_duration - EPS
    }

    /// 输出中各片段（及其片段号）的起止时间（转场时相邻片段重叠）。
    pub fn output_spans(&self) -> Vec<Interval> {
        let overlap = match &self.join {
            Join::Cut => 0.0,
            Join::Crossfade { duration, .. } => *duration,
        };
        let mut cursor = 0.0;
        self.pieces
            .iter()
            .map(|p| {
                let span = Interval::new(cursor, cursor + p.duration());
                cursor = span.end - overlap;
                span
            })
            .collect()
    }
}

/// 把任意视频换算成方形像素（宽度乘以自身的 SAR 并取偶、`setsar=1`）的滤镜。方形像素的源上是空操作；
/// 拼接不同来源前先用它，变形（非方形像素）源拼接后显示比例不变。
pub const SQUARE_PIXELS: &str = "scale=trunc(iw*sar/2)*2:ih,setsar=1";

/// [`SQUARE_PIXELS`]。
pub fn square_pixels() -> &'static str {
    SQUARE_PIXELS
}

/// 主视频的规格（插入的视频统一成它）。
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFormat {
    /// 按方形像素显示的宽高（已考虑旋转与 SAR，见 `VideoStream::square_display_size`）
    pub width: u32,
    pub height: u32,
    /// 帧率；未知为 0（此时不对齐帧网格）
    pub fps: f64,
    /// 帧率在滤镜里的写法（如 `25`、`30000/1001`），未知时为 `25`
    pub rate: String,
    /// 时长（秒）
    pub duration: f64,
    pub audio: Option<AudioFormat>,
    /// 源是非方形像素（变形）时为 true：滤镜图开头先把主视频缩放成方形像素（`width × height`）
    pub anamorphic: bool,
}

/// 主视频第一条音轨的规格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u32,
}

impl SourceFormat {
    pub fn from_info(info: &MediaInfo) -> Result<Self, String> {
        let video = info.video.as_ref().ok_or("文件中没有视频流，无法处理")?;
        if !(info.duration.is_finite() && info.duration > 0.0) {
            return Err("无法读取视频时长".into());
        }
        let (width, height) = video.square_display_size();
        let text = video.fps_text.trim();
        let parsed = text.split_once('/').and_then(|(n, d)| {
            let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
            (n > 0.0 && d > 0.0).then(|| n / d)
        });
        let fps = parsed
            .or_else(|| (video.fps.is_finite() && video.fps > 0.0).then_some(video.fps))
            .unwrap_or(0.0);
        let rate = match parsed {
            Some(_) => text.trim_end_matches("/1").to_owned(),
            None if fps > 0.0 => fmt_num(fps),
            None => "25".to_owned(),
        };
        Ok(Self {
            width,
            height,
            fps,
            rate,
            duration: info.duration,
            audio: info.audio.first().map(|a| AudioFormat {
                sample_rate: if a.sample_rate > 0 {
                    a.sample_rate
                } else {
                    48_000
                },
                channels: a.channels.max(1),
            }),
            anamorphic: video.sar_ratio().is_some(),
        })
    }

    /// 把时间对齐到帧网格（开头、结尾保持不变；帧率未知时不变）。
    pub fn snap(&self, t: f64) -> f64 {
        if t <= EPS {
            return 0.0;
        }
        if t >= self.duration - EPS {
            return self.duration;
        }
        if self.fps <= 0.0 {
            return t;
        }
        ((t * self.fps).round() / self.fps).clamp(0.0, self.duration)
    }

    /// 半帧时长（帧率未知时 1 ms），短于它的片段视为空。
    fn half_frame(&self) -> f64 {
        if self.fps > 0.0 {
            0.5 / self.fps
        } else {
            1e-3
        }
    }
}

/// 把主视频片段的边界对齐到帧网格，去掉对齐后为空的片段。
pub fn snap_pieces(pieces: Vec<Piece>, format: &SourceFormat) -> Vec<Piece> {
    pieces
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Main {
                start,
                end,
                speed,
                mirror,
            } => {
                let (start, end) = (format.snap(start), format.snap(end));
                (end - start >= format.half_frame()).then_some(Piece::Main {
                    start,
                    end,
                    speed,
                    mirror,
                })
            }
            insert => Some(insert),
        })
        .collect()
}

/// 主视频中要保留的区间 → 原速片段（已对齐帧网格）。
pub fn keep_pieces(keep: &[Interval], format: &SourceFormat) -> Vec<Piece> {
    snap_pieces(
        keep.iter().map(|i| Piece::main(i.start, i.end)).collect(),
        format,
    )
}

/// 时间写进滤镜：最多 6 位小数（帧边界需要比 [`crate::tools::media::secs`] 更精确）。
pub fn ts(value: f64) -> String {
    let s = format!("{:.6}", value.max(0.0));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

/// 声道数 → FFmpeg 声道布局名。
pub fn channel_layout(channels: u32) -> &'static str {
    match channels {
        1 => "mono",
        3 => "2.1",
        4 => "quad",
        5 => "5.0",
        6 => "5.1",
        7 => "6.1",
        8 => "7.1",
        _ => "stereo",
    }
}

/// 音频怎样进入输出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioOut {
    /// 源视频没有音频
    None,
    /// 滤镜图的 `[aout]`
    Filtered,
    /// 直接用源视频的第一条音轨（片段首尾相接时）
    Source,
}

/// 生成的滤镜图：输出 `[vout]`（与 `[aout]`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub filter: String,
    pub audio: AudioOut,
}

/// `trim` / `atrim` 的区间参数；片段从开头开始、到结尾结束时省略对应的端点（不依赖探测到的时长是否精确）。
fn trim_filter(name: &str, start: f64, end: f64, format: &SourceFormat) -> Option<String> {
    let open_start = start <= EPS;
    let open_end = end >= format.duration - EPS;
    match (open_start, open_end) {
        (true, true) => None,
        (true, false) => Some(format!("{name}=end={}", ts(end))),
        (false, true) => Some(format!("{name}=start={}", ts(start))),
        (false, false) => Some(format!("{name}=start={}:end={}", ts(start), ts(end))),
    }
}

fn labels(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("[{prefix}{i}]")).collect()
}

fn check_crossfade(timeline: &Timeline, kinds: &[String], duration: f64) -> Result<(), String> {
    let n = timeline.pieces.len();
    if n < 2 {
        return Err("至少需要两个片段才能添加转场".into());
    }
    if kinds.len() != n - 1 {
        return Err(format!(
            "转场数量（{}）与衔接处数量（{}）不一致",
            kinds.len(),
            n - 1
        ));
    }
    if !(duration.is_finite() && duration > 0.0) {
        return Err("转场时长必须大于 0".into());
    }
    for (i, piece) in timeline.pieces.iter().enumerate() {
        // 首尾片段只和一侧重叠，中间片段两侧都重叠
        let needed = if i == 0 || i == n - 1 {
            duration
        } else {
            duration * 2.0
        };
        if piece.duration() + EPS < needed {
            return Err(format!(
                "第 {} 个片段只有 {} 秒，不够做 {} 秒的转场",
                i + 1,
                fmt_num(piece.duration()),
                fmt_num(duration)
            ));
        }
    }
    Ok(())
}

/// 由时间线生成滤镜图（纯函数）。
pub fn build_graph(timeline: &Timeline, format: &SourceFormat) -> Result<Graph, String> {
    let pieces = &timeline.pieces;
    if pieces.is_empty() {
        return Err("处理后没有剩余内容".into());
    }
    if let Join::Crossfade { kinds, duration } = &timeline.join {
        check_crossfade(timeline, kinds, *duration)?;
    }
    for piece in pieces {
        if let Piece::Main { speed, .. } = piece {
            if !(speed.is_finite() && (0.5..=100.0).contains(speed)) {
                return Err("播放倍速必须在 0.5～100 之间".into());
            }
        }
    }
    let n = pieces.len();
    let audio = match format.audio {
        None => AudioOut::None,
        Some(_) if timeline.is_continuous(format.duration) => AudioOut::Source,
        Some(_) => AudioOut::Filtered,
    };
    let with_audio = audio == AudioOut::Filtered;
    let has_inserts = pieces.iter().any(|p| matches!(p, Piece::Insert { .. }));
    let sped = pieces
        .iter()
        .any(|p| matches!(p, Piece::Main { speed, .. } if (speed - 1.0).abs() > EPS));
    let mains = pieces
        .iter()
        .filter(|p| matches!(p, Piece::Main { .. }))
        .count();

    let mut chains: Vec<String> = Vec::new();
    // 变形源：先统一缩放成方形像素（只做一次，各片段共用）
    let main_video = if format.anamorphic && mains > 0 {
        chains.push(format!(
            "[0:V:0]scale={}:{},setsar=1[sq]",
            format.width, format.height
        ));
        "[sq]"
    } else {
        "[0:V:0]"
    };
    let (video_sources, audio_sources) = match mains {
        0 => (Vec::new(), Vec::new()),
        1 => (vec![main_video.to_owned()], vec!["[0:a:0]".to_owned()]),
        m => {
            let v = labels("s", m);
            chains.push(format!("{main_video}split={m}{}", v.concat()));
            let a = labels("t", m);
            if with_audio {
                chains.push(format!("[0:a:0]asplit={m}{}", a.concat()));
            }
            (v, a)
        }
    };

    let layout = format
        .audio
        .map_or("stereo", |a| channel_layout(a.channels));
    let sample_rate = format.audio.map_or(48_000, |a| a.sample_rate);
    let (w, h, rate) = (format.width, format.height, &format.rate);
    let mut main_index = 0;
    for (k, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Main {
                start,
                end,
                speed,
                mirror,
            } => {
                let mut video: Vec<String> = trim_filter("trim", *start, *end, format)
                    .into_iter()
                    .collect();
                video.push(if (speed - 1.0).abs() > EPS {
                    format!("setpts=(PTS-STARTPTS)/{}", fmt_num(*speed))
                } else {
                    "setpts=PTS-STARTPTS".to_owned()
                });
                if *mirror {
                    video.push("hflip".into());
                }
                if has_inserts {
                    video.push("setsar=1".into());
                }
                chains.push(format!(
                    "{}{}[v{k}]",
                    video_sources[main_index],
                    video.join(",")
                ));
                if with_audio {
                    let mut sound: Vec<String> = trim_filter("atrim", *start, *end, format)
                        .into_iter()
                        .collect();
                    sound.push("asetpts=PTS-STARTPTS".into());
                    if (speed - 1.0).abs() > EPS {
                        sound.push(format!("atempo={}", fmt_num(*speed)));
                    }
                    chains.push(format!(
                        "{}{}[a{k}]",
                        audio_sources[main_index],
                        sound.join(",")
                    ));
                }
                main_index += 1;
            }
            Piece::Insert {
                input,
                duration,
                has_audio,
            } => {
                let i = input + 1;
                let d = ts(*duration);
                chains.push(format!(
                    "[{i}:V:0]{SQUARE_PIXELS},scale={w}:{h}:force_original_aspect_ratio=decrease,\
                     pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={rate},\
                     trim=duration={d},setpts=PTS-STARTPTS[v{k}]"
                ));
                if with_audio {
                    chains.push(if *has_audio {
                        format!(
                            "[{i}:a:0]aformat=sample_rates={sample_rate}:channel_layouts={layout},\
                             apad,atrim=duration={d},asetpts=PTS-STARTPTS[a{k}]"
                        )
                    } else {
                        format!("anullsrc=r={sample_rate}:cl={layout},atrim=duration={d}[a{k}]")
                    });
                }
            }
        }
    }

    // 变速后帧时间戳变密，最后统一回源帧率
    let video_out = if sped { "[vc]" } else { "[vout]" };
    match &timeline.join {
        Join::Cut if n == 1 => {
            // 只有一段：把这一段的输出标签直接改成最终标签
            rename_last(&mut chains, "[v0]", video_out);
            if with_audio {
                rename_last(&mut chains, "[a0]", "[aout]");
            }
        }
        Join::Cut => {
            let inputs: String = (0..n)
                .map(|k| {
                    if with_audio {
                        format!("[v{k}][a{k}]")
                    } else {
                        format!("[v{k}]")
                    }
                })
                .collect();
            chains.push(format!(
                "{inputs}concat=n={n}:v=1:a={}{video_out}{}",
                u8::from(with_audio),
                if with_audio { "[aout]" } else { "" }
            ));
        }
        Join::Crossfade { kinds, duration } => {
            let d = ts(*duration);
            let mut previous = "[v0]".to_owned();
            let mut length = pieces[0].duration();
            for (k, kind) in kinds.iter().enumerate() {
                let next = k + 1;
                let out = if next == n - 1 {
                    video_out.to_owned()
                } else {
                    format!("[x{next}]")
                };
                chains.push(format!(
                    "{previous}[v{next}]xfade=transition={kind}:duration={d}:offset={}{out}",
                    ts(length - duration)
                ));
                length += pieces[next].duration() - duration;
                previous = out;
            }
            if with_audio {
                chains.push(format!(
                    "{}acrossfade=n={n}:d={d}[aout]",
                    labels("a", n).concat()
                ));
            }
        }
    }
    if sped {
        chains.push(format!("[vc]fps={rate}[vout]"));
    }
    Ok(Graph {
        filter: chains.join(";\n"),
        audio,
    })
}

fn rename_last(chains: &mut [String], from: &str, to: &str) {
    if let Some(chain) = chains.iter_mut().rev().find(|c| c.ends_with(from)) {
        chain.truncate(chain.len() - from.len());
        chain.push_str(to);
    }
}

/// 编码命令的参数（不含开头的通用参数）。
#[allow(clippy::too_many_arguments)]
pub fn encode_args(
    input: &str,
    inserts: &[String],
    script: &str,
    graph: &Graph,
    format: &SourceFormat,
    encoding: &Encoding,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args = vec!["-i".to_owned(), input.to_owned()];
    for insert in inserts {
        args.extend(["-i".to_owned(), insert.clone()]);
    }
    args.extend([
        "-/filter_complex".to_owned(),
        script.to_owned(),
        "-map".to_owned(),
        "[vout]".to_owned(),
    ]);
    match graph.audio {
        AudioOut::Filtered => args.extend(["-map".to_owned(), "[aout]".to_owned()]),
        AudioOut::Source => args.extend(["-map".to_owned(), "0:a:0".to_owned()]),
        AudioOut::None => {}
    }
    args.extend(encoding.video_args(container));
    match (graph.audio, format.audio) {
        (AudioOut::None, _) | (_, None) => args.push("-an".into()),
        (_, Some(audio)) => args.extend(Encoding::audio_args(audio.channels)),
    }
    args.extend(["-sn", "-dn", "-map_chapters", "-1"].map(String::from));
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".to_owned(), "+faststart".to_owned()]);
    }
    args.push(output.to_owned());
    args
}

/// 把时间线编码成第 0 个输入的同名输出（占一步进度）。`inserts` 是 [`Piece::Insert`] 引用的额外输入。
pub fn render(
    ctx: &mut ToolContext,
    timeline: &Timeline,
    inserts: &[PathBuf],
    label: &str,
) -> Result<(), ToolError> {
    ctx.check_cancelled()?;
    let info = ctx.info(0)?;
    let format = SourceFormat::from_info(&info)?;
    let graph = build_graph(timeline, &format)?;
    let script = ctx.temp_file("filter.txt")?;
    std::fs::write(&script, &graph.filter).map_err(|e| format!("无法写入滤镜脚本：{e}"))?;
    let script = path_arg(&script)?;
    let input = path_arg(ctx.input_path(0)?)?;
    let inserts = inserts
        .iter()
        .map(|p| path_arg(p))
        .collect::<Result<Vec<_>, _>>()?;
    let container = ctx.video_extension(0)?;
    let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
    let duration = timeline.duration();
    ctx.note(format!(
        "输出 {} 个片段，约 {} 秒",
        timeline.pieces.len(),
        fmt_num(duration)
    ));
    ctx.run_encode(label, duration, |enc| {
        Ok(encode_args(
            &input, &inserts, &script, &graph, &format, enc, &container, &output,
        ))
    })
}
