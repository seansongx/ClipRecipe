//! `pipe.fps` 帧率：在范围内取一个帧率，用 `fps` 滤镜转换。
//!
//! 这是“只追加简单滤镜”类模块的参考实现。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.fps";

/// 允许的帧率范围。
pub const MIN_FPS: f64 = 1.0;
pub const MAX_FPS: f64 = 240.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FpsParams {
    pub enabled: bool,
    /// 目标帧率，取值保留两位小数
    pub fps: Range<f64>,
}

impl Default for FpsParams {
    fn default() -> Self {
        // “重置参数后默认仅启用此项”
        Self {
            enabled: true,
            fps: Range::fixed(30.0),
        }
    }
}

impl PipelineModule for FpsParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let (lo, hi) = self.fps.ordered();
        if lo < MIN_FPS || hi > MAX_FPS {
            return Err(format!("帧率需在 {MIN_FPS}～{MAX_FPS} 之间"));
        }
        let fps = self.fps.sample_rounded(&mut ctx.rng, 2);
        ctx.graph.video(format!("fps={}", fmt_num(fps)));
        ctx.stream.fps = fps;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(fps: FpsParams) -> crate::model::Recipe {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.fps = fps;
        recipe
    }

    #[test]
    fn fixed_fps_adds_filter() {
        let built = build_with(&sample_info(), &recipe_with(FpsParams::default()), 1).unwrap();
        let fc = built.arg_after("-filter_complex").unwrap();
        assert_eq!(fc, "[0:v:0]fps=30[v0]");
    }

    #[test]
    fn ranged_fps_is_reproducible_per_seed() {
        let params = FpsParams {
            enabled: true,
            fps: Range::new(24.0, 30.0),
        };
        let recipe = recipe_with(params);
        let a = build_with(&sample_info(), &recipe, 42).unwrap();
        let b = build_with(&sample_info(), &recipe, 42).unwrap();
        assert_eq!(a.args, b.args);
        let fc = a.arg_after("-filter_complex").unwrap();
        let value: f64 = fc
            .trim_start_matches("[0:v:0]fps=")
            .trim_end_matches("[v0]")
            .parse()
            .unwrap();
        assert!((24.0..=30.0).contains(&value));
        // 固定参考值：种子 42 下的取值（算法或派生方式改变时会失败）
        assert_eq!(fc, "[0:v:0]fps=27.3[v0]");
    }

    #[test]
    fn out_of_range_is_rejected() {
        let params = FpsParams {
            enabled: true,
            fps: Range::new(0.0, 30.0),
        };
        let err = build_with(&sample_info(), &recipe_with(params), 1).unwrap_err();
        assert!(err.contains("帧率"), "{err}");
    }
}
