//! `tool.smart-trim` 智能去头尾：依据场景切换点判断片头、片尾并去掉。
//!
//! - 第 1 步场景检测（[`SceneDetect`]）；
//! - 片头：开头 `maxHeadSeconds` 秒内的**第一个**切换点之前的部分；片尾：结尾 `maxTailSeconds` 秒内的
//!   **最后一个**切换点之后的部分（[`plan_trim`]）。某一端的范围内没有切换点时，这一端保持不变，并在日志里说明；
//!   范围设为 0 表示不处理这一端。两端用的是同一个切换点（视频很短、范围重叠）时，只去掉离它更近的那一端；
//! - 第 2 步截取（切点对齐帧网格）用共享引擎（[`crate::tools::remove_remix`]）编码：只有一段，
//!   `trim` + `atrim`，音画同一区间，保持同步。

use serde::{Deserialize, Serialize};

use crate::pipeline::fmt_num;
use crate::tools::analysis::{Interval, SceneDetect};
use crate::tools::common::cutlist::{keep_pieces, render, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.smart-trim";

/// 场景切换点与开头 / 结尾的最小间隔（秒）：更近的切换点通常是淡入淡出的抖动。
pub const MIN_GAP: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmartTrimParams {
    /// 场景切换阈值（0～1，越小越敏感）
    pub scene_threshold: f64,
    /// 片头最长（秒）：在开头这么长的范围内找切换点；0 表示不去片头
    pub max_head_seconds: f64,
    /// 片尾最长（秒）：在结尾这么长的范围内找切换点；0 表示不去片尾
    pub max_tail_seconds: f64,
}

impl Default for SmartTrimParams {
    fn default() -> Self {
        Self {
            scene_threshold: 0.3,
            max_head_seconds: 10.0,
            max_tail_seconds: 10.0,
        }
    }
}

impl SmartTrimParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.scene_threshold.is_finite() && (0.0..=1.0).contains(&self.scene_threshold)) {
            return Err("场景切换阈值必须在 0～1 之间".into());
        }
        for (value, what) in [
            (self.max_head_seconds, "片头范围"),
            (self.max_tail_seconds, "片尾范围"),
        ] {
            if !(value.is_finite() && (0.0..=3600.0).contains(&value)) {
                return Err(format!("{what}必须在 0～3600 秒之间"));
            }
        }
        if self.max_head_seconds <= 0.0 && self.max_tail_seconds <= 0.0 {
            return Err("片头和片尾的范围不能都为 0".into());
        }
        Ok(())
    }

    fn detect(&self) -> SceneDetect {
        SceneDetect {
            threshold: self.scene_threshold,
            min_gap: MIN_GAP,
        }
    }
}

/// 去头尾的位置：片头结束于 `head`、片尾开始于 `tail`（None 表示这一端不动）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrimPlan {
    pub head: Option<f64>,
    pub tail: Option<f64>,
}

impl TrimPlan {
    /// 保留的区间。
    pub fn kept(&self, duration: f64) -> Interval {
        Interval::new(self.head.unwrap_or(0.0), self.tail.unwrap_or(duration))
    }
}

/// 由切换点决定片头、片尾。
pub fn plan_trim(cuts: &[f64], duration: f64, max_head: f64, max_tail: f64) -> TrimPlan {
    let inside = |c: &f64| *c > 0.0 && *c < duration;
    let head = if max_head > 0.0 {
        cuts.iter().copied().filter(inside).find(|c| *c <= max_head)
    } else {
        None
    };
    let tail = if max_tail > 0.0 {
        cuts.iter()
            .rev()
            .copied()
            .filter(inside)
            .find(|c| *c >= duration - max_tail)
    } else {
        None
    };
    match (head, tail) {
        // 两端落在同一个（或交错的）切换点上：只去掉离切换点更近的一端
        (Some(h), Some(t)) if t <= h => {
            if h <= duration - t {
                TrimPlan {
                    head: Some(h),
                    tail: None,
                }
            } else {
                TrimPlan {
                    head: None,
                    tail: Some(t),
                }
            }
        }
        (head, tail) => TrimPlan { head, tail },
    }
}

impl Tool for SmartTrimParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let format = SourceFormat::from_info(&info)?;
        ctx.set_steps(&[1.0, 3.0]);
        let cuts = ctx.analyze_input(0, &self.detect())?;
        let plan = plan_trim(
            &cuts,
            format.duration,
            self.max_head_seconds,
            self.max_tail_seconds,
        );
        match plan.head {
            Some(h) => ctx.note(format!("片头：去掉开头 {} 秒（场景切换点）", fmt_num(h))),
            None if self.max_head_seconds > 0.0 => ctx.warn(format!(
                "片头：开头 {} 秒内没有合适的场景切换点，开头保持不变",
                fmt_num(self.max_head_seconds)
            )),
            None => {}
        }
        match plan.tail {
            Some(t) => ctx.note(format!(
                "片尾：去掉 {} 秒之后的 {} 秒（场景切换点）",
                fmt_num(t),
                fmt_num(format.duration - t)
            )),
            None if self.max_tail_seconds > 0.0 => ctx.warn(format!(
                "片尾：结尾 {} 秒内没有合适的场景切换点，结尾保持不变",
                fmt_num(self.max_tail_seconds)
            )),
            None => {}
        }
        let pieces = keep_pieces(&[plan.kept(format.duration)], &format);
        render(ctx, &Timeline::cut(pieces), &[], "智能去头尾")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::tests::sample_format;
    use crate::tools::common::cutlist::{build_graph, Piece};

    #[test]
    fn head_and_tail_from_cuts() {
        let cuts = [1.5, 3.0, 6.0, 8.5];
        assert_eq!(
            plan_trim(&cuts, 10.0, 4.0, 4.0),
            TrimPlan {
                head: Some(1.5),
                tail: Some(8.5)
            },
            "片头取范围内第一个切换点，片尾取最后一个"
        );
        assert_eq!(
            plan_trim(&cuts, 10.0, 1.0, 1.0),
            TrimPlan {
                head: None,
                tail: None
            },
            "范围内没有切换点"
        );
        assert_eq!(
            plan_trim(&cuts, 10.0, 0.0, 5.0),
            TrimPlan {
                head: None,
                tail: Some(8.5)
            }
        );
        assert_eq!(
            plan_trim(&[], 10.0, 5.0, 5.0),
            TrimPlan {
                head: None,
                tail: None
            }
        );
        // 只有一个切换点且两个范围都覆盖它：去掉离它近的一端
        assert_eq!(
            plan_trim(&[3.0], 10.0, 8.0, 8.0),
            TrimPlan {
                head: Some(3.0),
                tail: None
            }
        );
        assert_eq!(
            plan_trim(&[7.0], 10.0, 8.0, 8.0),
            TrimPlan {
                head: None,
                tail: Some(7.0)
            }
        );
        // 越界的切换点被忽略
        assert_eq!(
            plan_trim(&[0.0, 10.0], 10.0, 5.0, 5.0),
            TrimPlan {
                head: None,
                tail: None
            }
        );
    }

    #[test]
    fn trim_graph() {
        let format = sample_format();
        let plan = plan_trim(&[1.51, 8.49], 10.0, 3.0, 3.0);
        let pieces = keep_pieces(&[plan.kept(10.0)], &format);
        assert_eq!(pieces, [Piece::main(1.52, 8.48)]);
        let g = build_graph(&Timeline::cut(pieces), &format).unwrap();
        assert_eq!(
            g.filter,
            "[0:V:0]trim=start=1.52:end=8.48,setpts=PTS-STARTPTS[vout];\n\
             [0:a:0]atrim=start=1.52:end=8.48,asetpts=PTS-STARTPTS[aout]"
        );
        // 没有可去的头尾：整段
        let whole = keep_pieces(
            &[TrimPlan {
                head: None,
                tail: None,
            }
            .kept(10.0)],
            &format,
        );
        assert_eq!(
            build_graph(&Timeline::cut(whole), &format).unwrap().filter,
            "[0:V:0]setpts=PTS-STARTPTS[vout]"
        );
    }

    #[test]
    fn params() {
        let ok = SmartTrimParams::default();
        assert!(ok.validate().is_ok());
        assert_eq!(
            ok.detect(),
            SceneDetect {
                threshold: 0.3,
                min_gap: MIN_GAP
            }
        );
        for bad in [
            SmartTrimParams {
                max_head_seconds: 0.0,
                max_tail_seconds: 0.0,
                ..ok.clone()
            },
            SmartTrimParams {
                max_head_seconds: -1.0,
                ..ok.clone()
            },
            SmartTrimParams {
                scene_threshold: 2.0,
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"sceneThreshold": 0.3, "maxHeadSeconds": 10.0, "maxTailSeconds": 10.0})
        );
    }
}
