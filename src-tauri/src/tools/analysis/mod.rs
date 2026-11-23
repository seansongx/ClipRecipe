//! 分析类 FFmpeg 运行：先检测（场景切换、静音、黑边、静止画面），再按结果处理。
//!
//! 每种检测是一个实现 [`Analysis`] 的参数结构体：给出流选择与滤镜参数，并从
//! `metadata=mode=print` / `ametadata=mode=print` 打印到日志里的逐帧元数据（[`MetadataFrame`]）解析结果。
//! 解析与命令拼装都是纯函数（有单元测试）；真实运行用 [`run_analysis`]，工具里用 `ctx.analyze(..)`。
//!
//! | 检测 | 结构体 | 结果 |
//! |---|---|---|
//! | 场景切换 | [`SceneDetect`] | 切换点时间 `Vec<f64>`（秒，升序） |
//! | 静音 | [`SilenceDetect`] | 静音区间 `Vec<Interval>` |
//! | 黑边 | [`BlackBarDetect`] | 去黑边后的画面 `Option<CropRect>`（None 表示没有黑边） |
//! | 静止画面 | [`FreezeDetect`] | 静止区间 `Vec<Interval>` |
//!
//! 时间都是从 0 开始的输入时间轴（ffmpeg 默认减去了输入的起始时间）。

mod black_bars;
mod freeze;
mod scene;
mod silence;

use std::path::Path;

use serde::Serialize;

use crate::ffmpeg::{self, path_arg, CancelToken, MediaInfo, Progress, DEFAULT_STALL_TIMEOUT};
use crate::tools::ToolError;

pub use black_bars::{BlackBarDetect, CropRect};
pub use freeze::FreezeDetect;
pub use scene::SceneDetect;
pub use silence::SilenceDetect;

/// 一种检测。
pub trait Analysis {
    type Output;

    /// 中文名称（日志、进度用），如“场景检测”。
    fn name(&self) -> &'static str;

    /// 输入之后、`-f null -` 之前的参数：流选择与滤镜（滤镜链最后要有 `metadata=mode=print`
    /// 或 `ametadata=mode=print`）。参数无效或媒体不合适（如没有音频）时返回错误。
    fn args(&self, info: &MediaInfo) -> Result<Vec<String>, String>;

    /// 从逐帧元数据得到结果。
    fn parse(&self, frames: &[MetadataFrame], info: &MediaInfo) -> Self::Output;
}

/// `metadata=mode=print` 打印的一帧：时间与全部键值。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetadataFrame {
    pub pts_time: f64,
    pub values: Vec<(String, String)>,
}

impl MetadataFrame {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key)?
            .trim()
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite())
    }
}

/// 逐行解析 ffmpeg 日志中的 `[Parsed_metadata_N @ 0x..] frame:.. pts:.. pts_time:..` 与
/// `[Parsed_metadata_N @ 0x..] key=value` 行（`ametadata` 同理），忽略其它输出。
#[derive(Debug, Default)]
pub struct MetadataParser {
    frames: Vec<MetadataFrame>,
}

impl MetadataParser {
    pub fn feed(&mut self, line: &str) {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("[Parsed_metadata_")
            .or_else(|| line.strip_prefix("[Parsed_ametadata_"))
        else {
            return;
        };
        let Some((_, content)) = rest.split_once("] ") else {
            return;
        };
        let content = content.trim();
        if content.starts_with("frame:") {
            let pts_time = content
                .split_whitespace()
                .find_map(|part| part.strip_prefix("pts_time:"))
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(f64::NAN);
            self.frames.push(MetadataFrame {
                pts_time,
                values: Vec::new(),
            });
        } else if let Some((key, value)) = content.split_once('=') {
            if let Some(frame) = self.frames.last_mut() {
                frame
                    .values
                    .push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
    }

    pub fn finish(self) -> Vec<MetadataFrame> {
        self.frames
    }

    /// 一次解析多行（测试用）。
    pub fn parse_all<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<MetadataFrame> {
        let mut parser = Self::default();
        for line in lines {
            parser.feed(line);
        }
        parser.finish()
    }
}

/// 一个时间区间（秒）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Interval {
    pub start: f64,
    pub end: f64,
}

impl Interval {
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// 整理区间：限制在 `[0, duration]`（duration ≤ 0 表示未知，不限上界），去掉空区间，排序并合并重叠的。
pub fn normalize_intervals(mut intervals: Vec<Interval>, duration: f64) -> Vec<Interval> {
    let upper = if duration > 0.0 {
        duration
    } else {
        f64::INFINITY
    };
    for i in &mut intervals {
        i.start = i.start.clamp(0.0, upper);
        i.end = i.end.clamp(0.0, upper);
    }
    intervals.retain(|i| i.end > i.start && i.start.is_finite());
    intervals.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<Interval> = Vec::with_capacity(intervals.len());
    for i in intervals {
        match merged.last_mut() {
            Some(last) if i.start <= last.end => last.end = last.end.max(i.end),
            _ => merged.push(i),
        }
    }
    merged
}

/// `[0, duration]` 中不属于 `intervals` 的部分（例如去掉静音后要保留的片段），短于 `min_len` 的丢弃。
pub fn complement(intervals: &[Interval], duration: f64, min_len: f64) -> Vec<Interval> {
    let mut keep = Vec::new();
    let mut cursor = 0.0;
    for i in normalize_intervals(intervals.to_vec(), duration) {
        if i.start - cursor >= min_len && i.start > cursor {
            keep.push(Interval::new(cursor, i.start));
        }
        cursor = cursor.max(i.end);
    }
    if duration - cursor >= min_len && duration > cursor {
        keep.push(Interval::new(cursor, duration));
    }
    keep
}

/// 按切点把 `[0, duration]` 分成连续的片段（切点会去重、排序，越界的忽略）。
pub fn segments_from_cuts(cuts: &[f64], duration: f64) -> Vec<Interval> {
    let mut points: Vec<f64> = cuts
        .iter()
        .copied()
        .filter(|t| t.is_finite() && *t > 0.0 && *t < duration)
        .collect();
    points.sort_by(f64::total_cmp);
    points.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let mut segments = Vec::with_capacity(points.len() + 1);
    let mut start = 0.0;
    for t in points {
        segments.push(Interval::new(start, t));
        start = t;
    }
    if duration > start {
        segments.push(Interval::new(start, duration));
    }
    segments
}

/// 分析命令的完整参数（不含程序名）：日志级别为 info（元数据打印在这一级），进度输出到 stdout。
pub fn command_args<A: Analysis + ?Sized>(
    input: &Path,
    info: &MediaInfo,
    analysis: &A,
) -> Result<Vec<String>, String> {
    let mut args = crate::tools::context::head_args("info");
    args.push("-i".into());
    args.push(path_arg(input)?);
    args.extend(analysis.args(info)?);
    args.extend(["-f".into(), "null".into(), "-".into()]);
    Ok(args)
}

/// 真实运行一次检测（不依赖工具上下文，集成测试与工具都可用；工具里优先用 `ctx.analyze`）。
pub fn run_analysis<A: Analysis>(
    ffmpeg: &Path,
    input: &Path,
    info: &MediaInfo,
    analysis: &A,
    cancel: &CancelToken,
    on_progress: impl FnMut(&Progress),
) -> Result<A::Output, ToolError> {
    let args = command_args(input, info, analysis)?;
    let mut parser = MetadataParser::default();
    ffmpeg::run_with_stderr(
        ffmpeg,
        &args,
        cancel,
        DEFAULT_STALL_TIMEOUT,
        on_progress,
        |line| parser.feed(line),
    )?;
    Ok(analysis.parse(&parser.finish(), info))
}

/// 检查 0..=1 的比例参数。
fn check_ratio(value: f64, what: &str) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what}必须在 0～1 之间"))
    }
}

/// 检查非负时长参数。
fn check_seconds(value: f64, what: &str) -> Result<(), String> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(format!("{what}不能为负数"))
    }
}

/// 检查噪声阈值（dB，必须为负）。
fn check_db(value: f64, what: &str) -> Result<(), String> {
    if value.is_finite() && value < 0.0 {
        Ok(())
    } else {
        Err(format!("{what}必须小于 0 dB"))
    }
}

/// 从配对的开始 / 结束键中取出区间（结束缺失时到视频结尾）。
fn paired_intervals(
    frames: &[MetadataFrame],
    start_key: &str,
    end_key: &str,
    duration: f64,
) -> Vec<Interval> {
    let mut intervals = Vec::new();
    let mut open: Option<f64> = None;
    for frame in frames {
        if let Some(start) = frame.number(start_key) {
            if open.is_none() {
                open = Some(start);
            }
        }
        if let Some(end) = frame.number(end_key) {
            intervals.push(Interval::new(open.take().unwrap_or(0.0), end));
        }
    }
    if let Some(start) = open {
        if duration > 0.0 {
            intervals.push(Interval::new(start, duration));
        }
    }
    normalize_intervals(intervals, duration)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn lines(text: &str) -> Vec<MetadataFrame> {
        MetadataParser::parse_all(text.lines())
    }

    #[test]
    fn parser_groups_values_by_frame_and_ignores_noise() {
        let frames = lines(
            "Input #0, lavfi, from 'x':\n\
             [Parsed_metadata_2 @ 000001ced3e04480] frame:0    pts:2000000 pts_time:2\n\
             [Parsed_metadata_2 @ 000001ced3e04480] lavfi.scene_score=1.000000\n\
             [Parsed_silencedetect_0 @ 0000] silence_start: 1\n\
             [Parsed_ametadata_1 @ 0x55d] frame:64   pts:65536   pts_time:1.486077\n\
             [Parsed_ametadata_1 @ 0x55d] lavfi.silence_start=1\n\
             [Parsed_ametadata_1 @ 0x55d] lavfi.silence_duration=1.000023\n\
             [Parsed_metadata_2 @ 0] lavfi.orphan=1\n",
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].pts_time, 2.0);
        assert_eq!(frames[0].number("lavfi.scene_score"), Some(1.0));
        assert_eq!(frames[1].pts_time, 1.486077);
        assert_eq!(frames[1].get("lavfi.silence_start"), Some("1"));
        assert_eq!(frames[1].values.len(), 3, "孤立的键值归到上一帧");
        assert_eq!(
            MetadataParser::parse_all(["[Parsed_metadata_1 @ 0] x=1"]),
            []
        );
    }

    #[test]
    fn interval_helpers() {
        let merged = normalize_intervals(
            vec![
                Interval::new(5.0, 7.0),
                Interval::new(-1.0, 1.0),
                Interval::new(6.0, 8.0),
                Interval::new(3.0, 3.0),
                Interval::new(9.0, 20.0),
            ],
            10.0,
        );
        assert_eq!(
            merged,
            [
                Interval::new(0.0, 1.0),
                Interval::new(5.0, 8.0),
                Interval::new(9.0, 10.0)
            ]
        );
        assert_eq!(
            complement(&merged, 10.0, 0.5),
            [Interval::new(1.0, 5.0), Interval::new(8.0, 9.0)]
        );
        assert_eq!(complement(&[], 4.0, 0.0), [Interval::new(0.0, 4.0)]);
        assert_eq!(
            segments_from_cuts(&[4.0, 2.0, 2.0, 0.0, 11.0], 6.0),
            [
                Interval::new(0.0, 2.0),
                Interval::new(2.0, 4.0),
                Interval::new(4.0, 6.0)
            ]
        );
        assert_eq!(Interval::new(2.0, 1.0).duration(), 0.0);
    }

    #[test]
    fn paired_intervals_close_at_end() {
        let frames = lines(
            "[Parsed_metadata_1 @ 0] frame:1 pts:1 pts_time:1.4\n\
             [Parsed_metadata_1 @ 0] k.start=1\n\
             [Parsed_metadata_1 @ 0] frame:2 pts:2 pts_time:2\n\
             [Parsed_metadata_1 @ 0] k.end=2\n\
             [Parsed_metadata_1 @ 0] frame:3 pts:3 pts_time:3.5\n\
             [Parsed_metadata_1 @ 0] k.start=3\n",
        );
        assert_eq!(
            paired_intervals(&frames, "k.start", "k.end", 5.0),
            [Interval::new(1.0, 2.0), Interval::new(3.0, 5.0)]
        );
    }
}
