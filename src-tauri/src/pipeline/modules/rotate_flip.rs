//! `pipe.rotate-flip` 旋转与翻转。
//!
//! 参数与取舍（FEATURES.md 未规定的部分）：
//! - `rotation`：顺时针 0° / 90° / 180° / 270°。90° 与 270° 用 `transpose`（宽高互换）；
//!   180° 等价于“水平 + 垂直翻转”，与用户勾选的翻转合并，互相抵消的翻转会被省略。
//! - `hflip` / `vflip`：作用于固定角度旋转之后的画面。
//! - `randomAngle`：在 `angle` 范围（单位：度，-180～180，保留 1 位小数，正值为顺时针）内
//!   为每个视频随机取一个角度做任意角度旋转，在固定旋转与翻转之后应用；取到 0 时不旋转。
//!   两种处理方式二选一（`fit`）：
//!   - `showAll`（完全显示）：画布扩大到恰好容纳旋转后的整个画面（向上取偶数），四角填黑；
//!   - `removeBlack`（去除黑边，默认）：画布尺寸不变，取旋转后画面内与原画面同比例、居中的
//!     最大内接矩形，再放大回原尺寸（相当于旋转并放大）；原尺寸为奇数时向下取偶数。
//! - 只做固定角度旋转 / 翻转时不改变像素尺寸（奇数尺寸保持奇数，由编码阶段处理）。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.rotate-flip";

/// 任意角度旋转允许的范围（度）。
pub const MAX_ANGLE: f64 = 180.0;

/// 固定角度旋转（顺时针）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Cw180,
    Cw270,
}

/// 任意角度旋转后如何处理四角的空白。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AngleFit {
    /// 完全显示：画布扩大
    ShowAll,
    /// 去除黑边：裁出最大内接矩形并放大回原尺寸
    #[default]
    RemoveBlack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RotateFlipParams {
    pub enabled: bool,
    pub rotation: Rotation,
    pub hflip: bool,
    pub vflip: bool,
    /// 是否启用随机任意角度旋转
    pub random_angle: bool,
    /// 任意角度（度，正值顺时针），取值保留 1 位小数
    pub angle: Range<f64>,
    pub fit: AngleFit,
}

impl Default for RotateFlipParams {
    fn default() -> Self {
        Self {
            enabled: false,
            rotation: Rotation::None,
            hflip: false,
            vflip: false,
            random_angle: false,
            angle: Range::new(-5.0, 5.0),
            fit: AngleFit::RemoveBlack,
        }
    }
}

/// 向上取偶数（先去掉浮点误差）。
fn even_up(x: f64) -> u32 {
    let n = (x - 1e-6).ceil().max(2.0) as u32;
    n + n % 2
}

/// 向下取偶数，至少为 2。
fn even_down(x: f64) -> u32 {
    let n = (x + 1e-6).floor().max(2.0) as u32;
    n - n % 2
}

impl RotateFlipParams {
    fn apply_angle(&self, ctx: &mut BuildContext) -> bool {
        let degrees = self.angle.sample_rounded(&mut ctx.rng, 1);
        if degrees == 0.0 {
            return false;
        }
        let (w, h) = (f64::from(ctx.stream.width), f64::from(ctx.stream.height));
        let radians = degrees.to_radians();
        let (c, s) = (radians.cos().abs(), radians.sin().abs());
        let angle = format!("a={}*PI/180", fmt_num(degrees));
        match self.fit {
            AngleFit::ShowAll => {
                let ow = even_up(w * c + h * s);
                let oh = even_up(w * s + h * c);
                ctx.graph
                    .video(format!("rotate={angle}:ow={ow}:oh={oh}:c=black"));
                ctx.stream.width = ow;
                ctx.stream.height = oh;
            }
            AngleFit::RemoveBlack => {
                // 与原画面同比例的矩形 (k·w, k·h) 反向旋转后仍需落在 w×h 内
                let k = (w / (w * c + h * s)).min(h / (w * s + h * c));
                let cw = even_down(w * k);
                let ch = even_down(h * k);
                // 反正要重新缩放：奇数尺寸顺便取偶数，便于 yuv420p 编码
                let (ow, oh) = (even_down(w), even_down(h));
                ctx.graph.video(format!(
                    "rotate={angle}:c=black,crop={cw}:{ch},scale={ow}:{oh},setsar=1"
                ));
                ctx.stream.width = ow;
                ctx.stream.height = oh;
            }
        }
        true
    }
}

impl PipelineModule for RotateFlipParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        if self.random_angle {
            let (lo, hi) = self.angle.ordered();
            if lo < -MAX_ANGLE || hi > MAX_ANGLE {
                return Err(format!("旋转角度需在 -{MAX_ANGLE}°～{MAX_ANGLE}° 之间"));
            }
        }

        let mut hflip = self.hflip;
        let mut vflip = self.vflip;
        let mut changed = false;
        match self.rotation {
            Rotation::None => {}
            Rotation::Cw90 | Rotation::Cw270 => {
                let dir = if self.rotation == Rotation::Cw90 {
                    "clock"
                } else {
                    "cclock"
                };
                ctx.graph.video(format!("transpose={dir}"));
                std::mem::swap(&mut ctx.stream.width, &mut ctx.stream.height);
                changed = true;
            }
            Rotation::Cw180 => {
                hflip = !hflip;
                vflip = !vflip;
            }
        }
        if hflip {
            ctx.graph.video("hflip");
            changed = true;
        }
        if vflip {
            ctx.graph.video("vflip");
            changed = true;
        }
        if self.random_angle && self.apply_angle(ctx) {
            changed = true;
        }
        if !changed {
            ctx.warn("没有需要应用的旋转或翻转");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: RotateFlipParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.rotate_flip = params;
        recipe
    }

    fn filter(params: RotateFlipParams, seed: u64) -> String {
        let built = build_with(&sample_info(), &recipe_with(params), seed).unwrap();
        built.arg_after("-filter_complex").unwrap_or("").to_owned()
    }

    /// 直接调用 apply，返回 (宽, 高)。
    fn stream_after(params: &RotateFlipParams, seed: u64) -> (u32, u32) {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(seed, ID);
        params.apply(&mut ctx).unwrap();
        (ctx.stream.width, ctx.stream.height)
    }

    fn on() -> RotateFlipParams {
        RotateFlipParams {
            enabled: true,
            ..RotateFlipParams::default()
        }
    }

    #[test]
    fn rotate_90_then_flip_swaps_size() {
        let params = RotateFlipParams {
            rotation: Rotation::Cw90,
            hflip: true,
            ..on()
        };
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]transpose=clock,hflip[v0]"
        );
        assert_eq!(stream_after(&params, 1), (720, 1280));

        let params = RotateFlipParams {
            rotation: Rotation::Cw270,
            ..on()
        };
        assert_eq!(filter(params.clone(), 1), "[0:v:0]transpose=cclock[v0]");
        assert_eq!(stream_after(&params, 1), (720, 1280));
    }

    #[test]
    fn rotate_180_merges_with_flips() {
        let both = RotateFlipParams {
            rotation: Rotation::Cw180,
            ..on()
        };
        assert_eq!(filter(both.clone(), 1), "[0:v:0]hflip,vflip[v0]");
        assert_eq!(stream_after(&both, 1), (1280, 720));
        let cancel = RotateFlipParams {
            hflip: true,
            ..both
        };
        assert_eq!(filter(cancel, 1), "[0:v:0]vflip[v0]");
    }

    #[test]
    fn show_all_grows_canvas() {
        let params = RotateFlipParams {
            random_angle: true,
            angle: Range::fixed(30.0),
            fit: AngleFit::ShowAll,
            ..on()
        };
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]rotate=a=30*PI/180:ow=1470:oh=1264:c=black[v0]"
        );
        assert_eq!(stream_after(&params, 1), (1470, 1264));
        // 90° 时不能因为浮点误差多出像素
        let right = RotateFlipParams {
            angle: Range::fixed(-90.0),
            ..params
        };
        assert_eq!(stream_after(&right, 1), (720, 1280));
    }

    #[test]
    fn remove_black_crops_and_keeps_size() {
        let params = RotateFlipParams {
            random_angle: true,
            angle: Range::fixed(30.0),
            fit: AngleFit::RemoveBlack,
            ..on()
        };
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]rotate=a=30*PI/180:c=black,crop=728:410,scale=1280:720,setsar=1[v0]"
        );
        assert_eq!(stream_after(&params, 1), (1280, 720));
    }

    #[test]
    fn random_angle_is_reproducible_and_bounded() {
        let params = RotateFlipParams {
            random_angle: true,
            angle: Range::new(-10.0, 10.0),
            fit: AngleFit::ShowAll,
            ..on()
        };
        assert_eq!(filter(params.clone(), 42), filter(params.clone(), 42));
        let mut seen = std::collections::HashSet::new();
        for seed in 0..30 {
            let fc = filter(params.clone(), seed);
            if fc.is_empty() {
                // 取到 0°：不旋转
                continue;
            }
            let deg: f64 = fc
                .trim_start_matches("[0:v:0]rotate=a=")
                .split('*')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!((-10.0..=10.0).contains(&deg), "{fc}");
            seen.insert(fc);
        }
        assert!(seen.len() > 1, "不同种子应得到不同角度");
        // 固定参考值（算法或派生方式改变时会失败）
        assert_eq!(
            filter(params, 42),
            "[0:v:0]rotate=a=-8.7*PI/180:ow=1376:oh=906:c=black[v0]"
        );
    }

    #[test]
    fn invalid_angle_is_rejected_and_noop_warns() {
        let params = RotateFlipParams {
            random_angle: true,
            angle: Range::new(-200.0, 0.0),
            ..on()
        };
        let err = build_with(&sample_info(), &recipe_with(params), 1).unwrap_err();
        assert!(err.contains("旋转角度"), "{err}");

        let built = build_with(&sample_info(), &recipe_with(on()), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.warnings.len(), 1);
    }
}
