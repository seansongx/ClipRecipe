//! 按场景切换分段（镜像重组、转场重组共用；原先放在 `mirror_remix.rs`，那里保留了同名的再导出）。

use crate::tools::analysis::{segments_from_cuts, Interval};
/// 场景片段：有切换点时按切换点分段；没有时每 `fallback` 秒一段（末段不足一半并入上一段）。
/// 返回片段与是否用了固定分段。
pub fn scene_segments(cuts: &[f64], duration: f64, fallback: f64) -> (Vec<Interval>, bool) {
    if !cuts.is_empty() {
        return (segments_from_cuts(cuts, duration), false);
    }
    let mut fixed = Vec::new();
    let mut k = 1.0;
    while fallback > 0.0 && fallback * k < duration - fallback / 2.0 - 1e-9 {
        fixed.push(fallback * k);
        k += 1.0;
    }
    (segments_from_cuts(&fixed, duration), true)
}

/// 把短于 `min_len` 的片段并入相邻片段（并入前一段；第一段则吸收后一段）。
pub fn merge_short(segments: Vec<Interval>, min_len: f64) -> Vec<Interval> {
    let mut out: Vec<Interval> = Vec::with_capacity(segments.len());
    for seg in segments {
        match out.last_mut() {
            Some(last) if last.duration() < min_len || seg.duration() < min_len => {
                last.end = seg.end;
            }
            _ => out.push(seg),
        }
    }
    out
}
/// 场景分段的公共参数检查（镜像重组、转场重组）。
pub fn check_scene_params(threshold: f64, min_segment: f64, fallback: f64) -> Result<(), String> {
    if !(threshold.is_finite() && (0.0..=1.0).contains(&threshold)) {
        return Err("场景切换阈值必须在 0～1 之间".into());
    }
    if !(min_segment.is_finite() && min_segment >= 0.0) {
        return Err("最短片段不能为负数".into());
    }
    if !(fallback.is_finite() && (0.5..=3600.0).contains(&fallback)) {
        return Err("无场景切换时的分段时长必须在 0.5～3600 秒之间".into());
    }
    Ok(())
}
