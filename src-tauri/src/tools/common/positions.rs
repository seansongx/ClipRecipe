//! 位置与时间：解析用户填写的时间 / 位置列表（[`parse_time`] / [`parse_positions`]）、
//! 位置参数（时间点 / 百分比 / 均分，[`resolve_positions`]）换算成秒、时间点对齐到帧（[`position_frames`]）、
//! 时间段 → 滤镜开关表达式（[`windows_expr`]）。
//!
//! 使用者：定格播放、插帧、进度条、拆分重组、移除重组（原先放在 `freeze_play.rs` / `split_remix.rs`，
//! 那里保留了同名的再导出）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::MediaInfo;
use crate::pipeline::fmt_num;
use crate::tools::common::filter_job::Rate;
/// 解析一个时间：秒数、`分:秒`、`时:分:秒` 或百分比（相对 `duration`）。
pub fn parse_time(token: &str, duration: f64) -> Result<f64, String> {
    let t = token.trim();
    let bad = || format!("无法识别的位置：{t}");
    let value = if let Some(p) = t.strip_suffix('%').or_else(|| t.strip_suffix('％')) {
        let p: f64 = p.trim().parse().map_err(|_| bad())?;
        if !(0.0..=100.0).contains(&p) {
            return Err(format!("百分比必须在 0～100 之间：{t}"));
        }
        duration * p / 100.0
    } else if t.contains([':', '：']) {
        let parts: Vec<&str> = t.split([':', '：']).collect();
        if parts.len() > 3 {
            return Err(bad());
        }
        let mut total = 0.0;
        for part in &parts {
            let v: f64 = part.trim().parse().map_err(|_| bad())?;
            if v < 0.0 {
                return Err(bad());
            }
            total = total * 60.0 + v;
        }
        total
    } else {
        t.parse().map_err(|_| bad())?
    };
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(bad())
    }
}

/// 解析位置列表：返回升序的时间（秒）与提示（超出时长而忽略的位置）。一个有效位置都没有时报错。
pub fn parse_positions(text: &str, duration: f64) -> Result<(Vec<f64>, Vec<String>), String> {
    let mut times = Vec::new();
    let mut warnings = Vec::new();
    let tokens = text
        .split([',', '，', ';', '；', '、', '\n', '\r', '\t', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty());
    for token in tokens {
        let t = parse_time(token, duration)?;
        if t < duration {
            times.push(t);
        } else {
            warnings.push(format!("位置 {token} 超出视频时长，已忽略"));
        }
    }
    if times.is_empty() {
        return Err(if warnings.is_empty() {
            "请填写至少一个位置".into()
        } else {
            "没有在视频时长内的位置".into()
        });
    }
    times.sort_by(f64::total_cmp);
    Ok((times, warnings))
}

/// 时间点 → 帧序号（升序、去重、不超过最后一帧）；合并的位置记一条提示。
pub fn position_frames(
    times: &[f64],
    rate: &Rate,
    info: &MediaInfo,
    warnings: &mut Vec<String>,
) -> Vec<i64> {
    let last = rate.total_frames(info) - 1;
    let mut frames: Vec<i64> = times.iter().map(|t| rate.frame_at(*t).min(last)).collect();
    let before = frames.len();
    frames.dedup();
    if frames.len() < before {
        warnings.push(format!(
            "有 {} 个位置落在同一帧上，已合并",
            before - frames.len()
        ));
    }
    frames
}

/// 时间段列表 → 滤镜时间轴开关表达式（`t` 在任一段内时为真）。段为输出时间轴上的帧区间
/// `[start, end)`，比较时各减半帧，避免边界帧因时间戳误差多显示或少显示。
pub fn windows_expr(windows: &[(f64, f64)], rate: &Rate) -> String {
    let half = 0.5 / rate.fps();
    windows
        .iter()
        .map(|(a, b)| format!("gt(t,{})*lt(t,{})", fmt_num(a - half), fmt_num(b - half)))
        .collect::<Vec<_>>()
        .join("+")
}
/// 位置个数上限。
pub const MAX_POSITIONS: usize = 100;

/// 位置的给法。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PositionMode {
    /// 时间点（秒）
    Times,
    /// 占总时长的百分比
    Percents,
    /// 均分成 N 份，取 N-1 个分界点
    #[default]
    Parts,
}

/// 解析出的位置（秒，升序、去重）与因超出视频时长被忽略的个数。
#[derive(Debug, Clone, PartialEq)]
pub struct Positions {
    pub points: Vec<f64>,
    pub ignored: usize,
}

/// 把位置参数换算成秒。位置可以在 `[0, 时长)` 内；超出时长的忽略（[`Positions::ignored`]），一个都不剩时报错。
pub fn resolve_positions(
    mode: PositionMode,
    times: &[f64],
    percents: &[f64],
    parts: u32,
    duration: f64,
) -> Result<Positions, String> {
    if !(duration.is_finite() && duration > 0.0) {
        return Err("无法读取视频时长".into());
    }
    let raw: Vec<f64> = match mode {
        PositionMode::Times => {
            if times.is_empty() {
                return Err("请至少填写一个时间点".into());
            }
            if times.iter().any(|t| !(t.is_finite() && *t >= 0.0)) {
                return Err("时间点不能为负数".into());
            }
            times.to_vec()
        }
        PositionMode::Percents => {
            if percents.is_empty() {
                return Err("请至少填写一个百分比".into());
            }
            if percents
                .iter()
                .any(|p| !(p.is_finite() && (0.0..=100.0).contains(p)))
            {
                return Err("百分比必须在 0～100 之间".into());
            }
            percents.iter().map(|p| p / 100.0 * duration).collect()
        }
        PositionMode::Parts => {
            if !(2..=MAX_POSITIONS as u32 + 1).contains(&parts) {
                return Err(format!("份数必须在 2～{} 之间", MAX_POSITIONS + 1));
            }
            (1..parts)
                .map(|k| duration * f64::from(k) / f64::from(parts))
                .collect()
        }
    };
    if raw.len() > MAX_POSITIONS {
        return Err(format!("位置太多（最多 {MAX_POSITIONS} 个）"));
    }
    let mut points: Vec<f64> = raw
        .iter()
        .copied()
        .filter(|t| *t < duration - 1e-6)
        .collect();
    let ignored = raw.len() - points.len();
    points.sort_by(f64::total_cmp);
    points.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    if points.is_empty() {
        return Err(format!(
            "没有落在视频时长（{} 秒）以内的位置",
            fmt_num(duration)
        ));
    }
    Ok(Positions { points, ignored })
}
