//! `tool.transition-remix` 转场重组：按场景切换点分段，相邻片段之间加转场（`xfade`，指定一种或每处随机），
//! 再拼成一个视频。
//!
//! - 第 1 步场景检测；切换点之间至少间隔 max(最短片段, 2×转场时长 + 0.2 秒)，没有切换点时每 `fallbackSeconds`
//!   秒一段（日志说明）；分段对齐帧网格后，仍短于“2×转场时长 + 1 帧”的片段并入相邻片段（[`transition_pieces`]）；
//! - 第 2 步用共享引擎（[`crate::tools::remove_remix`]）单遍编码：`split` + `trim` 取片段，视频逐个 `xfade`
//!   （offset = 已拼接长度 − 转场时长），音频 `acrossfade=n=段数:d=转场时长`。转场处前后两段重叠，
//!   视频和音频都按同样的时长重叠，所以输出时长 = 源时长 − (段数−1)×转场时长，音画始终同步。
//!   转场时后一段要提前缓冲约“转场时长”的帧，内存占用与片段数无关。
//! - 只剩一段时（视频太短）不加转场，按原样输出并在日志里说明。

use serde::{Deserialize, Serialize};

use crate::model::SeededRng;
use crate::pipeline::fmt_num;
use crate::tools::analysis::{Interval, SceneDetect};
use crate::tools::common::scenes::{check_scene_params, merge_short, scene_segments};
use crate::tools::common::cutlist::{render, Join, Piece, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.transition-remix";

/// 可选的转场（`xfade` 的 transition 名）；“随机”时从中挑选。前端 `TRANSITIONS` 与此一致。
pub const TRANSITIONS: &[&str] = &[
    "fade",
    "dissolve",
    "fadeblack",
    "fadewhite",
    "wipeleft",
    "wiperight",
    "wipeup",
    "wipedown",
    "slideleft",
    "slideright",
    "slideup",
    "slidedown",
    "smoothleft",
    "smoothright",
    "circleopen",
    "circleclose",
    "radial",
    "pixelize",
    "hblur",
    "zoomin",
];

/// 参数里表示“每处随机”的值。
pub const RANDOM: &str = "random";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TransitionRemixParams {
    /// 场景切换阈值（0～1，越小越敏感）
    pub scene_threshold: f64,
    /// 最短片段（秒）
    pub min_segment_seconds: f64,
    /// 没有检测到场景切换时，每段的时长（秒）
    pub fallback_seconds: f64,
    /// 转场：`random` 或 [`TRANSITIONS`] 中的一种
    pub transition: String,
    /// 转场时长（秒）
    pub transition_seconds: f64,
}

impl Default for TransitionRemixParams {
    fn default() -> Self {
        Self {
            scene_threshold: 0.3,
            min_segment_seconds: 2.0,
            fallback_seconds: 5.0,
            transition: RANDOM.to_owned(),
            transition_seconds: 1.0,
        }
    }
}

impl TransitionRemixParams {
    fn validate(&self) -> Result<(), String> {
        check_scene_params(
            self.scene_threshold,
            self.min_segment_seconds,
            self.fallback_seconds,
        )?;
        if self.transition != RANDOM && !TRANSITIONS.contains(&self.transition.as_str()) {
            return Err(format!("不支持的转场：{}", self.transition));
        }
        let d = self.transition_seconds;
        if !(d.is_finite() && (0.1..=5.0).contains(&d)) {
            return Err("转场时长必须在 0.1～5 秒之间".into());
        }
        if self.fallback_seconds < d * 2.0 + 0.2 {
            return Err("无场景切换时的分段时长必须大于转场时长的 2 倍".into());
        }
        Ok(())
    }

    /// 场景检测：切换点之间至少能放下两次转场。
    fn scene_detect(&self) -> SceneDetect {
        SceneDetect {
            threshold: self.scene_threshold,
            min_gap: self
                .min_segment_seconds
                .max(self.transition_seconds * 2.0 + 0.2),
        }
    }
}

/// 分段对齐帧网格后，把放不下两次转场的片段并入相邻片段。
pub fn transition_pieces(
    segments: &[Interval],
    format: &SourceFormat,
    transition_seconds: f64,
) -> Vec<Piece> {
    let snapped: Vec<Interval> = segments
        .iter()
        .map(|s| Interval::new(format.snap(s.start), format.snap(s.end)))
        .filter(|s| s.duration() > 0.0)
        .collect();
    let frame = if format.fps > 0.0 {
        1.0 / format.fps
    } else {
        0.04
    };
    merge_short(snapped, transition_seconds * 2.0 + frame)
        .into_iter()
        .map(|s| Piece::main(s.start, s.end))
        .collect()
}

/// 每个衔接处的转场。
pub fn choose_kinds(transition: &str, joints: usize, rng: &mut SeededRng) -> Vec<String> {
    (0..joints)
        .map(|_| {
            if transition == RANDOM {
                TRANSITIONS[rng.index(TRANSITIONS.len())].to_owned()
            } else {
                transition.to_owned()
            }
        })
        .collect()
}

impl Tool for TransitionRemixParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        self.validate()?;
        let info = ctx.info(0)?;
        let format = SourceFormat::from_info(&info)?;
        ctx.set_steps(&[1.0, 3.0]);
        let cuts = ctx.analyze_input(0, &self.scene_detect())?;
        let (segments, fell_back) = scene_segments(&cuts, format.duration, self.fallback_seconds);
        if fell_back {
            ctx.warn(format!(
                "没有检测到场景切换，改为每 {} 秒一段（可调低场景切换阈值）",
                fmt_num(self.fallback_seconds)
            ));
        } else {
            ctx.note(format!("检测到 {} 个场景切换点", cuts.len()));
        }
        let pieces = transition_pieces(&segments, &format, self.transition_seconds);
        if pieces.len() < 2 {
            ctx.warn("视频太短，片段不足两段，无法添加转场，按原样输出");
            return render(ctx, &Timeline::cut(pieces), &[], "转场重组");
        }
        let kinds = choose_kinds(&self.transition, pieces.len() - 1, &mut ctx.rng);
        ctx.note(format!(
            "共 {} 段，转场：{}",
            pieces.len(),
            kinds.join("、")
        ));
        let timeline = Timeline {
            pieces,
            join: Join::Crossfade {
                kinds,
                duration: self.transition_seconds,
            },
        };
        render(ctx, &timeline, &[], "转场重组")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::build_graph;
    use crate::tools::remove_remix::tests::sample_format;

    #[test]
    fn pieces_are_snapped_and_short_ones_merged() {
        let format = sample_format();
        let segments = [
            Interval::new(0.0, 3.01),
            Interval::new(3.01, 4.5),
            Interval::new(4.5, 7.0),
            Interval::new(7.0, 10.0),
        ];
        assert_eq!(
            transition_pieces(&segments, &format, 0.5),
            [
                Piece::main(0.0, 3.0),
                Piece::main(3.0, 4.52),
                Piece::main(4.52, 7.0),
                Piece::main(7.0, 10.0)
            ]
        );
        // 1 秒转场：中间片段至少 2.04 秒，1.52 秒的那段并入前一段
        assert_eq!(
            transition_pieces(&segments, &format, 1.0),
            [
                Piece::main(0.0, 4.52),
                Piece::main(4.52, 7.0),
                Piece::main(7.0, 10.0)
            ]
        );
    }

    #[test]
    fn kinds_fixed_or_seeded_random() {
        let mut rng = SeededRng::new(1);
        assert_eq!(
            choose_kinds("wipeleft", 2, &mut rng),
            ["wipeleft", "wipeleft"]
        );
        let a = choose_kinds(RANDOM, 12, &mut SeededRng::new(7));
        assert_eq!(a, choose_kinds(RANDOM, 12, &mut SeededRng::new(7)));
        assert!(a.iter().all(|k| TRANSITIONS.contains(&k.as_str())));
        assert!(a.iter().any(|k| k != &a[0]), "{a:?}");
        assert!(choose_kinds(RANDOM, 0, &mut rng).is_empty());
    }

    #[test]
    fn transition_graph_shortens_by_overlaps() {
        let format = sample_format();
        let segments = [
            Interval::new(0.0, 4.0),
            Interval::new(4.0, 7.0),
            Interval::new(7.0, 10.0),
        ];
        let pieces = transition_pieces(&segments, &format, 1.0);
        let kinds = choose_kinds("dissolve", 2, &mut SeededRng::new(0));
        let t = Timeline {
            pieces,
            join: Join::Crossfade {
                kinds,
                duration: 1.0,
            },
        };
        assert_eq!(t.duration(), 8.0, "10 秒 − 2 处 × 1 秒");
        let g = build_graph(&t, &format).unwrap();
        assert!(g.filter.contains(
            "[v0][v1]xfade=transition=dissolve:duration=1:offset=3[x1];\n\
             [x1][v2]xfade=transition=dissolve:duration=1:offset=5[vout];\n\
             [a0][a1][a2]acrossfade=n=3:d=1[aout]"
        ));
    }

    #[test]
    fn params_validation_and_scene_gap() {
        let ok = TransitionRemixParams::default();
        assert!(ok.validate().is_ok());
        assert_eq!(ok.scene_detect().min_gap, 2.2);
        let long = TransitionRemixParams {
            transition_seconds: 2.0,
            ..ok.clone()
        };
        assert_eq!(long.scene_detect().min_gap, 4.2);
        for bad in [
            TransitionRemixParams {
                transition: "nope".into(),
                ..ok.clone()
            },
            TransitionRemixParams {
                transition_seconds: 0.0,
                ..ok.clone()
            },
            TransitionRemixParams {
                transition_seconds: 3.0,
                ..ok.clone()
            },
            TransitionRemixParams {
                scene_threshold: -1.0,
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({
                "sceneThreshold": 0.3,
                "minSegmentSeconds": 2.0,
                "fallbackSeconds": 5.0,
                "transition": "random",
                "transitionSeconds": 1.0
            })
        );
    }
}
