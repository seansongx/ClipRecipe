//! `pipe.trim` 去头尾：按时长去掉片头 / 片尾，或按“起点 + 长度”截取一段。
//!
//! 参数与取值约定（FEATURES 未规定的部分在此说明）：
//! - 两种方式：`headTail` 去掉片头 `head` 秒与片尾 `tail` 秒；`segment` 从 `start` 秒起截取 `length` 秒。
//!   四个时长都是范围值，取值保留两位小数，单位秒，允许 0～86400。
//! - `minDuration`（秒，0 表示不限制）：截取后的时长不短于它。
//!   - 去头尾方式下，若去掉的总时长会让结果短于下限，片头片尾按原比例同时缩减，使结果恰好等于下限；
//!   - 截取方式下，先把终点向后延长，到达视频末尾后再把起点前移；
//!   - 源视频本身不长于下限时不做截取，只给出警告。
//! - 没有设置下限时，去掉的时长不短于视频时长、或起点超出视频时长，视为参数无效（返回错误）。
//!
//! 实现：在主输入前加 `-ss 起点 -t 时长`。FFmpeg 转码时的输入定位是精确到帧的（accurate seek），
//! 音视频从同一起点开始、同时截止，时间戳从 0 开始，后续模块不受影响；
//! 而且只解码需要的部分，比 `trim` 滤镜快。
//!
//! 这些计算基于 `ctx.stream.duration`；完成后把它更新为截取后的时长。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.trim";

/// 时长参数允许的最大值（秒）。
pub const MAX_SECONDS: f64 = 86_400.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrimMode {
    /// 去掉片头与片尾
    #[default]
    HeadTail,
    /// 起点 + 长度截取
    Segment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrimParams {
    pub enabled: bool,
    pub mode: TrimMode,
    /// 去片头时长（秒）
    pub head: Range<f64>,
    /// 去片尾时长（秒）
    pub tail: Range<f64>,
    /// 截取起点（秒）
    pub start: Range<f64>,
    /// 截取长度（秒）
    pub length: Range<f64>,
    /// 截取后的最短时长（秒），0 表示不限制
    pub min_duration: f64,
}

impl Default for TrimParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: TrimMode::HeadTail,
            head: Range::fixed(1.0),
            tail: Range::fixed(1.0),
            start: Range::fixed(0.0),
            length: Range::fixed(10.0),
            min_duration: 0.0,
        }
    }
}

/// 保留三位小数并向下取整（截取量向下取，保证结果不短于下限）。
fn floor3(value: f64) -> f64 {
    (value * 1000.0).floor() / 1000.0
}

fn check_range(range: &Range<f64>, name: &str) -> Result<(), String> {
    let (lo, hi) = range.ordered();
    if !lo.is_finite() || !hi.is_finite() || lo < 0.0 || hi > MAX_SECONDS {
        return Err(format!("{name}需在 0～{MAX_SECONDS} 秒之间"));
    }
    Ok(())
}

impl TrimParams {
    fn validate(&self) -> Result<(), String> {
        match self.mode {
            TrimMode::HeadTail => {
                check_range(&self.head, "片头时长")?;
                check_range(&self.tail, "片尾时长")?;
            }
            TrimMode::Segment => {
                check_range(&self.start, "截取起点")?;
                check_range(&self.length, "截取长度")?;
                if self.length.ordered().0 <= 0.0 {
                    return Err("截取长度必须大于 0".into());
                }
            }
        }
        if !self.min_duration.is_finite() || !(0.0..=MAX_SECONDS).contains(&self.min_duration) {
            return Err(format!("最短时长需在 0～{MAX_SECONDS} 秒之间"));
        }
        Ok(())
    }

    /// 计算保留区间 `[起点, 终点)`；返回 None 表示不截取。
    fn plan(&self, ctx: &mut BuildContext, duration: f64) -> Result<Option<(f64, f64)>, String> {
        let floor = self.min_duration;
        // 先取样，保证同一种子下的取值与是否触发下限无关
        let (a, b) = match self.mode {
            TrimMode::HeadTail => (
                self.head.sample_rounded(&mut ctx.rng, 2),
                self.tail.sample_rounded(&mut ctx.rng, 2),
            ),
            TrimMode::Segment => (
                self.start.sample_rounded(&mut ctx.rng, 2),
                self.length.sample_rounded(&mut ctx.rng, 2),
            ),
        };
        if floor > 0.0 && duration <= floor {
            ctx.warn(format!(
                "视频时长 {} 秒不超过最短时长 {} 秒，未做截取",
                fmt_num(duration),
                fmt_num(floor)
            ));
            return Ok(None);
        }
        match self.mode {
            TrimMode::HeadTail => {
                let (mut head, mut tail) = (a, b);
                let budget = duration - floor;
                if head + tail >= budget {
                    if floor <= 0.0 {
                        return Err(format!(
                            "去掉的片头 {} 秒与片尾 {} 秒合计不短于视频时长 {} 秒",
                            fmt_num(head),
                            fmt_num(tail),
                            fmt_num(duration)
                        ));
                    }
                    let k = budget / (head + tail);
                    head = floor3(head * k);
                    tail = floor3(tail * k);
                }
                Ok(Some((head, duration - tail)))
            }
            TrimMode::Segment => {
                let (start, length) = (a, b);
                if start >= duration && floor <= 0.0 {
                    return Err(format!(
                        "截取起点 {} 秒超出视频时长 {} 秒",
                        fmt_num(start),
                        fmt_num(duration)
                    ));
                }
                let mut start = start.min(duration);
                let mut end = (start + length).min(duration);
                if end - start < floor {
                    end = (start + floor).min(duration);
                    start = floor3((end - floor).max(0.0));
                }
                Ok(Some((start, end)))
            }
        }
    }
}

impl PipelineModule for TrimParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        let duration = ctx.stream.duration;
        if duration <= 0.0 {
            return Err("源视频时长未知，无法去头尾".into());
        }
        let Some((start, end)) = self.plan(ctx, duration)? else {
            return Ok(());
        };
        let start = (start * 10_000.0).round() / 10_000.0;
        let kept = ((end - start) * 10_000.0).round() / 10_000.0;
        if start > 0.0 {
            ctx.main_input_args
                .extend(["-ss".to_owned(), fmt_num(start)]);
        }
        if start + kept < duration {
            ctx.main_input_args.extend(["-t".to_owned(), fmt_num(kept)]);
        }
        ctx.stream.duration = kept;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};
    use crate::pipeline::BuiltCommand;

    fn recipe_with(params: TrimParams) -> crate::model::Recipe {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.trim = params;
        recipe
    }

    fn build(params: TrimParams, seed: u64) -> Result<BuiltCommand, String> {
        build_with(&sample_info(), &recipe_with(params), seed)
    }

    /// `-i` 之前的输入参数。
    fn input_args(built: &BuiltCommand) -> Vec<String> {
        let i = built.args.iter().position(|a| a == "-i").unwrap();
        let start = built.args.iter().position(|a| a == "-nostats").unwrap() + 1;
        built.args[start..i].to_vec()
    }

    fn head_tail(head: Range<f64>, tail: Range<f64>, min_duration: f64) -> TrimParams {
        TrimParams {
            enabled: true,
            mode: TrimMode::HeadTail,
            head,
            tail,
            min_duration,
            ..TrimParams::default()
        }
    }

    fn segment(start: Range<f64>, length: Range<f64>, min_duration: f64) -> TrimParams {
        TrimParams {
            enabled: true,
            mode: TrimMode::Segment,
            start,
            length,
            min_duration,
            ..TrimParams::default()
        }
    }

    #[test]
    fn head_and_tail_become_input_seek_args() {
        // sample_info 时长 10 秒
        let built = build(head_tail(Range::fixed(1.5), Range::fixed(2.0), 0.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "1.5", "-t", "6.5"]);
        assert_eq!(built.expected_duration, 6.5);
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.arg_after("-c:a"), Some("copy"));
        assert!(built.warnings.is_empty());
    }

    #[test]
    fn head_only_and_tail_only() {
        let built = build(head_tail(Range::fixed(3.0), Range::fixed(0.0), 0.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "3"]);
        assert_eq!(built.expected_duration, 7.0);
        let built = build(head_tail(Range::fixed(0.0), Range::fixed(3.0), 0.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-t", "7"]);
        assert_eq!(built.expected_duration, 7.0);
    }

    #[test]
    fn ranged_values_are_reproducible_and_bounded() {
        let params = head_tail(Range::new(0.5, 2.0), Range::new(1.0, 3.0), 0.0);
        let a = build(params.clone(), 42).unwrap();
        let b = build(params.clone(), 42).unwrap();
        assert_eq!(a.args, b.args);
        // 固定参考值：种子 42 下的取值
        assert_eq!(input_args(&a), ["-ss", "0.66", "-t", "7"]);
        for seed in 0..50 {
            let built = build(params.clone(), seed).unwrap();
            let args = input_args(&built);
            let head: f64 = args[1].parse().unwrap();
            let kept: f64 = args[3].parse().unwrap();
            assert!((0.5..=2.0).contains(&head), "{head}");
            let tail = 10.0 - head - kept;
            assert!((1.0 - 1e-9..=3.0 + 1e-9).contains(&tail), "{tail}");
            assert!((built.expected_duration - kept).abs() < 1e-9);
        }
    }

    #[test]
    fn min_duration_scales_cuts_proportionally() {
        // 去 4 + 4 秒会剩 2 秒，下限 6 秒 → 共只能去 4 秒，按 1:1 各去 2 秒
        let built = build(head_tail(Range::fixed(4.0), Range::fixed(4.0), 6.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "2", "-t", "6"]);
        assert_eq!(built.expected_duration, 6.0);
        // 3:1 的比例保持
        let built = build(head_tail(Range::fixed(6.0), Range::fixed(2.0), 6.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "3", "-t", "6"]);
    }

    #[test]
    fn source_shorter_than_min_duration_is_untouched() {
        let built = build(head_tail(Range::fixed(1.0), Range::fixed(1.0), 12.0), 1).unwrap();
        assert!(input_args(&built).is_empty());
        assert_eq!(built.expected_duration, 10.0);
        assert_eq!(built.warnings.len(), 1);
        assert!(
            built.warnings[0].contains("最短时长"),
            "{:?}",
            built.warnings
        );
    }

    #[test]
    fn segment_mode() {
        let built = build(segment(Range::fixed(2.0), Range::fixed(3.5), 0.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "2", "-t", "3.5"]);
        assert_eq!(built.expected_duration, 3.5);
        // 长度超过末尾：截到末尾，不需要 -t
        let built = build(segment(Range::fixed(8.0), Range::fixed(5.0), 0.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "8"]);
        assert_eq!(built.expected_duration, 2.0);
    }

    #[test]
    fn segment_min_duration_extends_end_then_moves_start() {
        // 1 秒长度不足下限 3 秒：终点延长到 5
        let built = build(segment(Range::fixed(2.0), Range::fixed(1.0), 3.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "2", "-t", "3"]);
        // 起点 9 秒：终点到末尾 10 后起点前移到 7
        let built = build(segment(Range::fixed(9.0), Range::fixed(1.0), 3.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "7"]);
        assert_eq!(built.expected_duration, 3.0);
        // 起点超出时长也能靠下限得到结果
        let built = build(segment(Range::fixed(20.0), Range::fixed(1.0), 4.0), 1).unwrap();
        assert_eq!(input_args(&built), ["-ss", "6"]);
        assert_eq!(built.expected_duration, 4.0);
    }

    #[test]
    fn invalid_params_are_rejected() {
        let cases = [
            head_tail(Range::fixed(6.0), Range::fixed(4.0), 0.0),
            head_tail(Range::fixed(-1.0), Range::fixed(0.0), 0.0),
            head_tail(Range::fixed(1.0), Range::fixed(f64::NAN), 0.0),
            head_tail(Range::fixed(1.0), Range::fixed(1.0), -2.0),
            segment(Range::fixed(10.0), Range::fixed(1.0), 0.0),
            segment(Range::fixed(1.0), Range::fixed(0.0), 0.0),
        ];
        for params in cases {
            let err = build(params.clone(), 1).unwrap_err();
            assert!(err.starts_with("[pipe.trim]"), "{params:?}: {err}");
        }
        let mut info = sample_info();
        info.duration = 0.0;
        let err = build_with(
            &info,
            &recipe_with(head_tail(Range::fixed(1.0), Range::fixed(1.0), 0.0)),
            1,
        )
        .unwrap_err();
        assert!(err.contains("时长未知"), "{err}");
    }

    #[test]
    fn serde_defaults_and_shape() {
        let parsed: TrimParams =
            serde_json::from_str(r#"{"enabled":true,"mode":"segment","minDuration":3}"#).unwrap();
        assert_eq!(parsed.mode, TrimMode::Segment);
        assert_eq!(parsed.min_duration, 3.0);
        assert_eq!(parsed.length, Range::fixed(10.0));
        let json = serde_json::to_value(TrimParams::default()).unwrap();
        assert_eq!(json["mode"], "headTail");
        assert!(json.get("minDuration").is_some());
    }
}
