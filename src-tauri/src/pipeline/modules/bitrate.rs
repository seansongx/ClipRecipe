//! `pipe.bitrate` 码率：倍率模式（相对源视频码率）或固定值（kbit/s）。
//! 不启用时由流水线沿用源码率（见 `pipeline::build`）。
//!
//! 这是“只修改编码参数”类模块的参考实现。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{BuildContext, PipelineModule};

pub const ID: &str = "pipe.bitrate";

/// 码率下限（kbit/s），避免得到无法编码的极小值。
pub const MIN_KBPS: u64 = 50;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BitrateMode {
    /// 源码率 × 倍率（<1 压缩，>1 提升）
    #[default]
    Multiplier,
    /// 固定码率（kbit/s）
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BitrateParams {
    pub enabled: bool,
    pub mode: BitrateMode,
    /// 倍率模式下的倍率，取值保留两位小数
    pub multiplier: Range<f64>,
    /// 固定模式下的码率（kbit/s）
    pub kbps: Range<u64>,
}

impl Default for BitrateParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: BitrateMode::Multiplier,
            multiplier: Range::fixed(1.0),
            kbps: Range::fixed(3000),
        }
    }
}

impl PipelineModule for BitrateParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let kbps = match self.mode {
            BitrateMode::Multiplier => {
                let (lo, _) = self.multiplier.ordered();
                if lo <= 0.0 {
                    return Err("倍率必须大于 0".into());
                }
                let factor = self.multiplier.sample_rounded(&mut ctx.rng, 2);
                let Some(source) = ctx.info.video_bit_rate() else {
                    ctx.warn("源视频码率未知，倍率模式无法计算，改用恒定质量编码");
                    return Ok(());
                };
                (source as f64 * factor / 1000.0).round() as u64
            }
            BitrateMode::Fixed => {
                let (lo, _) = self.kbps.ordered();
                if lo == 0 {
                    return Err("码率必须大于 0".into());
                }
                self.kbps.sample(&mut ctx.rng)
            }
        };
        ctx.encode.video_kbps = Some(kbps.max(MIN_KBPS));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: BitrateParams) -> crate::model::Recipe {
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.bitrate = params;
        recipe
    }

    #[test]
    fn multiplier_scales_source_bitrate() {
        // sample_info 的视频码率为 4000 kbit/s
        let params = BitrateParams {
            enabled: true,
            multiplier: Range::fixed(0.5),
            ..BitrateParams::default()
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert_eq!(built.arg_after("-b:v"), Some("2000k"));
        assert_eq!(built.arg_after("-maxrate"), Some("3000k"));
    }

    #[test]
    fn fixed_mode_uses_sampled_kbps() {
        let params = BitrateParams {
            enabled: true,
            mode: BitrateMode::Fixed,
            kbps: Range::new(1000, 2000),
            ..BitrateParams::default()
        };
        let recipe = recipe_with(params);
        let a = build_with(&sample_info(), &recipe, 5).unwrap();
        let b = build_with(&sample_info(), &recipe, 5).unwrap();
        assert_eq!(a.args, b.args);
        let kbps: u64 = a
            .arg_after("-b:v")
            .unwrap()
            .trim_end_matches('k')
            .parse()
            .unwrap();
        assert!((1000..=2000).contains(&kbps));
    }

    #[test]
    fn unknown_source_bitrate_falls_back_to_quality_mode_with_warning() {
        let mut info = sample_info();
        info.bit_rate = None;
        info.size = 0;
        info.video.as_mut().unwrap().bit_rate = None;
        let params = BitrateParams {
            enabled: true,
            ..BitrateParams::default()
        };
        let built = build_with(&info, &recipe_with(params), 1).unwrap();
        assert_eq!(built.arg_after("-crf"), Some("23"));
        assert!(built.warnings.iter().any(|w| w.contains("码率未知")));
    }

    #[test]
    fn invalid_values_are_rejected() {
        let params = BitrateParams {
            enabled: true,
            multiplier: Range::new(0.0, 1.0),
            ..BitrateParams::default()
        };
        assert!(build_with(&sample_info(), &recipe_with(params), 1).is_err());
    }
}
