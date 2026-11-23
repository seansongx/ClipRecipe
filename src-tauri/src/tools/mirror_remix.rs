//! `tool.mirror-remix` 镜像重组：按场景切换点把视频分段，部分片段水平翻转，再按原顺序拼回一个视频。
//!
//! - 第 1 步场景检测（[`SceneDetect`]）；没有检测到切换点时改为每 `fallbackSeconds` 秒一段（日志里说明）；
//! - 选哪些片段翻转（[`choose_flips`]）：按比例（随机挑出 round(段数×比例) 段）、每隔 N 段翻转一段、
//!   或每段各自随机（50%），随机都来自 `ctx.rng`（种子可复现）；
//! - 第 2 步用共享引擎（[`crate::tools::common::cutlist`]）单遍编码：`split` + `trim` + `hflip` + `concat`。
//!   片段首尾相接、覆盖全片，音频不经过滤镜直接沿用源音轨，所以声音连续且与画面同步，时长与源视频相同。
//!
//! 分段的纯函数 [`scene_segments`] / [`merge_short`] 在 [`crate::tools::common::scenes`]（转场重组也用）。

use serde::{Deserialize, Serialize};

use crate::model::SeededRng;
use crate::pipeline::fmt_num;
#[cfg(test)]
use crate::tools::analysis::Interval;
use crate::tools::analysis::SceneDetect;
use crate::tools::common::cutlist::{keep_pieces, render, Piece, SourceFormat, Timeline};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.mirror-remix";

/// 选择翻转片段的方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlipMode {
    /// 随机挑出 round(段数 × 比例) 段
    #[default]
    Ratio,
    /// 每隔 N 段翻转一段（第 N、2N… 段）
    Nth,
    /// 每段各自随机（一半概率）
    Random,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorRemixParams {
    /// 场景切换阈值（0～1，越小越敏感）
    pub scene_threshold: f64,
    /// 最短片段（秒）：相邻切换点至少间隔这么久
    pub min_segment_seconds: f64,
    /// 没有检测到场景切换时，每段的时长（秒）
    pub fallback_seconds: f64,
    pub flip_mode: FlipMode,
    /// 按比例：翻转的片段占比（0～1）
    pub ratio: f64,
    /// 每隔 N 段：N
    pub nth: u32,
}

impl Default for MirrorRemixParams {
    fn default() -> Self {
        Self {
            scene_threshold: 0.3,
            min_segment_seconds: 1.0,
            fallback_seconds: 5.0,
            flip_mode: FlipMode::Ratio,
            ratio: 0.5,
            nth: 2,
        }
    }
}

// 场景分段已移到 `tools::common::scenes`，这里保留原名的再导出。
pub use crate::tools::common::scenes::{check_scene_params, merge_short, scene_segments};

impl MirrorRemixParams {
    fn validate(&self) -> Result<(), String> {
        check_scene_params(
            self.scene_threshold,
            self.min_segment_seconds,
            self.fallback_seconds,
        )?;
        if !(self.ratio.is_finite() && (0.0..=1.0).contains(&self.ratio)) {
            return Err("翻转比例必须在 0～1 之间".into());
        }
        if !(1..=1000).contains(&self.nth) {
            return Err("间隔段数必须在 1～1000 之间".into());
        }
        Ok(())
    }
}


/// 每段是否翻转。
pub fn choose_flips(
    count: usize,
    mode: FlipMode,
    ratio: f64,
    nth: u32,
    rng: &mut SeededRng,
) -> Vec<bool> {
    match mode {
        FlipMode::Ratio => {
            let wanted = ((count as f64) * ratio.clamp(0.0, 1.0)).round() as usize;
            // Fisher–Yates 打乱下标，前 wanted 个翻转
            let mut order: Vec<usize> = (0..count).collect();
            for i in (1..count).rev() {
                order.swap(i, rng.index(i + 1));
            }
            let mut flips = vec![false; count];
            for &i in order.iter().take(wanted) {
                flips[i] = true;
            }
            flips
        }
        FlipMode::Nth => {
            let nth = nth.max(1) as usize;
            (0..count).map(|i| (i + 1) % nth == 0).collect()
        }
        FlipMode::Random => (0..count).map(|_| rng.chance(0.5)).collect(),
    }
}

/// 片段（已对齐帧网格）按 `flips` 标记翻转。
pub fn mirror_pieces(pieces: Vec<Piece>, flips: &[bool]) -> Vec<Piece> {
    pieces
        .into_iter()
        .zip(flips)
        .map(|(piece, flip)| match piece {
            Piece::Main {
                start, end, speed, ..
            } => Piece::Main {
                start,
                end,
                speed,
                mirror: *flip,
            },
            other => other,
        })
        .collect()
}

/// 日志里列出片段编号（从 1 起），如 `1、3、5`。
pub fn list_numbers(flags: &[bool]) -> String {
    flags
        .iter()
        .enumerate()
        .filter(|(_, f)| **f)
        .map(|(i, _)| (i + 1).to_string())
        .collect::<Vec<_>>()
        .join("、")
}

impl Tool for MirrorRemixParams {
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
        let cuts = ctx.analyze_input(
            0,
            &SceneDetect {
                threshold: self.scene_threshold,
                min_gap: self.min_segment_seconds,
            },
        )?;
        let (segments, fell_back) = scene_segments(&cuts, format.duration, self.fallback_seconds);
        if fell_back {
            ctx.warn(format!(
                "没有检测到场景切换，改为每 {} 秒一段（可调低场景切换阈值）",
                fmt_num(self.fallback_seconds)
            ));
        } else {
            ctx.note(format!("检测到 {} 个场景切换点", cuts.len()));
        }
        let pieces = keep_pieces(&segments, &format);
        let flips = choose_flips(
            pieces.len(),
            self.flip_mode,
            self.ratio,
            self.nth,
            &mut ctx.rng,
        );
        if flips.iter().any(|f| *f) {
            ctx.note(format!(
                "共 {} 段，翻转第 {} 段",
                pieces.len(),
                list_numbers(&flips)
            ));
        } else {
            ctx.warn(format!("共 {} 段，按当前设置没有片段被翻转", pieces.len()));
        }
        let pieces = mirror_pieces(pieces, &flips);
        render(ctx, &Timeline::cut(pieces), &[], "镜像重组")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::remove_remix::tests::sample_format;
    use crate::tools::common::cutlist::{build_graph, AudioOut};

    #[test]
    fn segments_from_scene_cuts_or_fixed_length() {
        assert_eq!(
            scene_segments(&[2.0, 6.0], 10.0, 5.0),
            (
                vec![
                    Interval::new(0.0, 2.0),
                    Interval::new(2.0, 6.0),
                    Interval::new(6.0, 10.0)
                ],
                false
            )
        );
        // 没有切换点：每 4 秒一段，末段 2.5 秒（不超过一半时并入上一段）
        assert_eq!(
            scene_segments(&[], 10.5, 4.0),
            (
                vec![
                    Interval::new(0.0, 4.0),
                    Interval::new(4.0, 8.0),
                    Interval::new(8.0, 10.5)
                ],
                true
            )
        );
        assert_eq!(
            scene_segments(&[], 9.0, 4.0).0,
            [Interval::new(0.0, 4.0), Interval::new(4.0, 9.0)]
        );
        assert_eq!(
            scene_segments(&[], 3.0, 5.0).0,
            [Interval::new(0.0, 3.0)],
            "比一段还短"
        );
        // 切点在最开头 / 最末尾时被忽略
        assert_eq!(
            scene_segments(&[0.0, 10.0], 10.0, 5.0).0,
            [Interval::new(0.0, 10.0)]
        );
    }

    #[test]
    fn short_segments_merge_into_neighbours() {
        let segs = vec![
            Interval::new(0.0, 0.5),
            Interval::new(0.5, 3.0),
            Interval::new(3.0, 3.4),
            Interval::new(3.4, 7.0),
            Interval::new(7.0, 10.0),
        ];
        assert_eq!(
            merge_short(segs, 1.0),
            [
                Interval::new(0.0, 3.4),
                Interval::new(3.4, 7.0),
                Interval::new(7.0, 10.0)
            ]
        );
        assert_eq!(
            merge_short(vec![Interval::new(0.0, 0.5)], 1.0),
            [Interval::new(0.0, 0.5)]
        );
    }

    #[test]
    fn flips_by_mode_are_seeded() {
        let mut rng = SeededRng::new(1);
        assert_eq!(
            choose_flips(5, FlipMode::Nth, 0.0, 2, &mut rng),
            [false, true, false, true, false]
        );
        assert_eq!(
            choose_flips(3, FlipMode::Nth, 0.0, 1, &mut rng),
            [true, true, true]
        );
        let a = choose_flips(10, FlipMode::Ratio, 0.3, 0, &mut SeededRng::new(42));
        assert_eq!(a.iter().filter(|f| **f).count(), 3);
        assert_eq!(
            a,
            choose_flips(10, FlipMode::Ratio, 0.3, 0, &mut SeededRng::new(42))
        );
        assert_ne!(
            a,
            choose_flips(10, FlipMode::Ratio, 0.3, 0, &mut SeededRng::new(43)),
            "不同种子挑出不同的片段"
        );
        assert_eq!(
            choose_flips(4, FlipMode::Ratio, 1.0, 0, &mut rng),
            [true; 4]
        );
        assert_eq!(
            choose_flips(4, FlipMode::Ratio, 0.0, 0, &mut rng),
            [false; 4]
        );
        let r = choose_flips(64, FlipMode::Random, 0.0, 0, &mut SeededRng::new(3));
        assert_eq!(
            r,
            choose_flips(64, FlipMode::Random, 0.0, 0, &mut SeededRng::new(3))
        );
        let flipped = r.iter().filter(|f| **f).count();
        assert!((16..=48).contains(&flipped), "{flipped}");
        assert!(choose_flips(0, FlipMode::Ratio, 0.5, 2, &mut rng).is_empty());
    }

    #[test]
    fn mirror_graph_flips_selected_pieces_and_keeps_audio() {
        let format = sample_format();
        let (segments, _) = scene_segments(&[2.03, 6.01], format.duration, 5.0);
        let pieces = keep_pieces(&segments, &format);
        let flips = choose_flips(pieces.len(), FlipMode::Nth, 0.5, 2, &mut SeededRng::new(0));
        let t = Timeline::cut(mirror_pieces(pieces, &flips));
        assert_eq!(t.duration(), 10.0, "时长与源视频相同");
        let g = build_graph(&t, &format).unwrap();
        assert_eq!(g.audio, AudioOut::Source);
        assert_eq!(
            g.filter,
            "[0:V:0]split=3[s0][s1][s2];\n\
             [s0]trim=end=2.04,setpts=PTS-STARTPTS[v0];\n\
             [s1]trim=start=2.04:end=6,setpts=PTS-STARTPTS,hflip[v1];\n\
             [s2]trim=start=6,setpts=PTS-STARTPTS[v2];\n\
             [v0][v1][v2]concat=n=3:v=1:a=0[vout]"
        );
        assert_eq!(list_numbers(&flips), "2");
    }

    #[test]
    fn invalid_params_are_rejected() {
        let ok = MirrorRemixParams::default();
        assert!(ok.validate().is_ok());
        for bad in [
            MirrorRemixParams {
                scene_threshold: 1.5,
                ..ok.clone()
            },
            MirrorRemixParams {
                ratio: -0.1,
                ..ok.clone()
            },
            MirrorRemixParams {
                nth: 0,
                ..ok.clone()
            },
            MirrorRemixParams {
                fallback_seconds: 0.1,
                ..ok.clone()
            },
            MirrorRemixParams {
                min_segment_seconds: -1.0,
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({
                "sceneThreshold": 0.3,
                "minSegmentSeconds": 1.0,
                "fallbackSeconds": 5.0,
                "flipMode": "ratio",
                "ratio": 0.5,
                "nth": 2
            })
        );
    }
}
