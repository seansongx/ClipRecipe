//! `pipe.resolution` 分辨率。
//!
//! 参数与取舍（docs/specs/pipeline.md 未规定的部分）：
//! - `standard`（标准档位）：统一**高度**为 `standardHeight`（取偶数），宽度按源比例换算并取最近的偶数；
//!   与当前尺寸相同时不加滤镜。
//! - `custom`（自定义宽高）：`width` × `height`，奇数会向下调整为偶数并给出警告；
//!   - `stretch`（拉伸）：直接缩放到目标尺寸；
//!   - `fit`（原比例）：画面等比缩小到目标尺寸内并居中（偏移取偶数），空白处：
//!     - `blur`：用画面自身作背景，`blurStrength` 为 `gblur` 的 sigma（0～100，0 表示不虚化）；
//!       `blurCenter` 为 true 时背景等比放大后取画面中心，false 时把整个画面拉伸铺满，null 时跟随设置 `set.blur-center`；
//!     - `black`：黑色填充；此时可开 `reflection`（倒影）：在画面下方的空白里放一份垂直翻转的画面，
//!       不透明度从 50% 线性渐隐到 0；只在上下有空白时生效（左右留空时给出警告）。
//! - 宽高允许 16～7680。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.resolution";

pub const MIN_SIZE: u32 = 16;
pub const MAX_SIZE: u32 = 7680;
pub const MAX_BLUR: f64 = 100.0;
/// 倒影起始不透明度
pub const REFLECTION_OPACITY: f64 = 0.5;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolutionMode {
    /// 标准档位：统一高度
    #[default]
    Standard,
    /// 自定义宽高
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScaleMode {
    Stretch,
    /// 保持原比例
    #[default]
    Fit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillMode {
    /// 画面自身作背景（可虚化）
    #[default]
    Blur,
    /// 黑色
    Black,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResolutionParams {
    pub enabled: bool,
    pub mode: ResolutionMode,
    /// 标准档位的统一高度
    pub standard_height: u32,
    /// 自定义宽高
    pub width: u32,
    pub height: u32,
    pub scale_mode: ScaleMode,
    pub fill: FillMode,
    /// 背景虚化强度（gblur sigma），取值保留 1 位小数
    pub blur_strength: Range<f64>,
    /// 背景取画面中心（true）还是拉伸整个画面（false）
    /// 为空（None）时跟随设置 `set.blur-center`（`Settings::blur_center`）；旧配方里的 true / false 仍然有效且优先
    pub blur_center: Option<bool>,
    /// 黑色填充时的倒影效果
    pub reflection: bool,
}

impl Default for ResolutionParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ResolutionMode::Standard,
            standard_height: 720,
            width: 1080,
            height: 1920,
            scale_mode: ScaleMode::Fit,
            fill: FillMode::Blur,
            blur_strength: Range::fixed(20.0),
            blur_center: None,
            reflection: false,
        }
    }
}

/// 最近的偶数，至少为 2。
fn even_round(x: f64) -> u32 {
    (((x / 2.0).round() * 2.0).max(2.0)) as u32
}

fn check_size(value: u32, what: &str) -> Result<(), String> {
    if (MIN_SIZE..=MAX_SIZE).contains(&value) {
        Ok(())
    } else {
        Err(format!("{what}需在 {MIN_SIZE}～{MAX_SIZE} 之间"))
    }
}

impl ResolutionParams {
    fn apply_standard(&self, ctx: &mut BuildContext) -> Result<(), String> {
        check_size(self.standard_height, "高度")?;
        let height = self.standard_height - self.standard_height % 2;
        if height != self.standard_height {
            ctx.warn(format!("高度需为偶数，已调整为 {height}"));
        }
        let (sw, sh) = (ctx.stream.width, ctx.stream.height);
        let width = even_round(f64::from(sw) * f64::from(height) / f64::from(sh.max(1)));
        if (width, height) != (sw, sh) {
            ctx.graph.video(format!("scale={width}:{height},setsar=1"));
            ctx.stream.width = width;
            ctx.stream.height = height;
        }
        Ok(())
    }

    fn apply_custom(&self, ctx: &mut BuildContext) -> Result<(), String> {
        check_size(self.width, "宽度")?;
        check_size(self.height, "高度")?;
        let (lo, hi) = self.blur_strength.ordered();
        if self.scale_mode == ScaleMode::Fit
            && self.fill == FillMode::Blur
            && (lo < 0.0 || hi > MAX_BLUR)
        {
            return Err(format!("虚化强度需在 0～{MAX_BLUR} 之间"));
        }
        let w = self.width - self.width % 2;
        let h = self.height - self.height % 2;
        if (w, h) != (self.width, self.height) {
            ctx.warn(format!("宽高需为偶数，已调整为 {w}x{h}"));
        }

        let (sw, sh) = (
            f64::from(ctx.stream.width),
            f64::from(ctx.stream.height.max(1)),
        );
        // 等比缩放后的画面尺寸
        let (fw, fh) = if sw * f64::from(h) >= f64::from(w) * sh {
            (w, even_round(sh * f64::from(w) / sw).min(h))
        } else {
            (even_round(sw * f64::from(h) / sh).min(w), h)
        };
        ctx.stream.width = w;
        ctx.stream.height = h;

        if self.scale_mode == ScaleMode::Stretch || (fw, fh) == (w, h) {
            ctx.graph.video(format!("scale={w}:{h},setsar=1"));
            return Ok(());
        }
        // 偏移取偶数，保证 yuv420p 下 pad 与 overlay 的位置一致
        let x = (w - fw) / 2 / 2 * 2;
        let y = (h - fh) / 2 / 2 * 2;
        match self.fill {
            FillMode::Blur => {
                let sigma = self.blur_strength.sample_rounded(&mut ctx.rng, 1);
                let mut bg = if self.blur_center.unwrap_or(ctx.settings.blur_center) {
                    format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
                } else {
                    format!("scale={w}:{h}")
                };
                bg.push_str(",setsar=1");
                if sigma > 0.0 {
                    bg.push_str(&format!(",gblur=sigma={}", fmt_num(sigma)));
                }
                let input = ctx.graph.take_video();
                let [a, b, c, d, out] = ["rs", "rs", "rs", "rs", "rs"].map(|p| ctx.graph.label(p));
                ctx.graph.chain(&[&input], "split=2", &[&a, &b]);
                ctx.graph.chain(&[&a], &bg, &[&c]);
                ctx.graph
                    .chain(&[&b], &format!("scale={fw}:{fh},setsar=1"), &[&d]);
                ctx.graph
                    .chain(&[&c, &d], &format!("overlay={x}:{y}"), &[&out]);
                ctx.graph.set_video(out);
            }
            FillMode::Black => {
                let below = h - y - fh;
                let reflect_h = below.min(fh) / 2 * 2;
                if self.reflection && reflect_h == 0 {
                    ctx.warn("画面上下没有空白，倒影效果未生效");
                }
                if !self.reflection || reflect_h == 0 {
                    ctx.graph.video(format!(
                        "scale={fw}:{fh},setsar=1,pad={w}:{h}:{x}:{y}:black"
                    ));
                    return Ok(());
                }
                let input = ctx.graph.take_video();
                let [a, b, c, d, out] = ["rs", "rs", "rs", "rs", "rs"].map(|p| ctx.graph.label(p));
                ctx.graph.chain(
                    &[&input],
                    &format!("scale={fw}:{fh},setsar=1,split=2"),
                    &[&a, &b],
                );
                ctx.graph
                    .chain(&[&a], &format!("pad={w}:{h}:{x}:{y}:black"), &[&c]);
                ctx.graph.chain(
                    &[&b],
                    &format!(
                        "vflip,crop={fw}:{reflect_h}:0:0,format=yuva420p,\
                         geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='{}*(1-Y/H)'",
                        fmt_num(255.0 * REFLECTION_OPACITY)
                    ),
                    &[&d],
                );
                ctx.graph.chain(
                    &[&c, &d],
                    &format!("overlay={x}:{}:format=auto", y + fh),
                    &[&out],
                );
                ctx.graph.set_video(out);
            }
        }
        Ok(())
    }
}

impl PipelineModule for ResolutionParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        match self.mode {
            ResolutionMode::Standard => self.apply_standard(ctx),
            ResolutionMode::Custom => self.apply_custom(ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: ResolutionParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.resolution = params;
        recipe
    }

    fn filter(params: ResolutionParams, seed: u64) -> String {
        let built = build_with(&sample_info(), &recipe_with(params), seed).unwrap();
        built.arg_after("-filter_complex").unwrap_or("").to_owned()
    }

    fn stream_after(params: &ResolutionParams) -> (u32, u32) {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(1, ID);
        params.apply(&mut ctx).unwrap();
        (ctx.stream.width, ctx.stream.height)
    }

    fn custom(w: u32, h: u32) -> ResolutionParams {
        ResolutionParams {
            enabled: true,
            mode: ResolutionMode::Custom,
            width: w,
            height: h,
            ..ResolutionParams::default()
        }
    }

    #[test]
    fn standard_unifies_height() {
        let params = ResolutionParams {
            enabled: true,
            standard_height: 480,
            ..ResolutionParams::default()
        };
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]scale=854:480,setsar=1[v0]"
        );
        assert_eq!(stream_after(&params), (854, 480));
        // 已经是目标尺寸：不加滤镜
        let same = ResolutionParams {
            standard_height: 720,
            ..params
        };
        assert_eq!(filter(same, 1), "");
    }

    #[test]
    fn custom_stretch() {
        let params = ResolutionParams {
            scale_mode: ScaleMode::Stretch,
            ..custom(640, 640)
        };
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]scale=640:640,setsar=1[v0]"
        );
        assert_eq!(stream_after(&params), (640, 640));
    }

    #[test]
    fn fit_with_blurred_center_background() {
        let params = custom(1080, 1920);
        assert_eq!(
            filter(params.clone(), 1),
            "[0:v:0]split=2[rs0][rs1];\
             [rs0]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,setsar=1,gblur=sigma=20[rs2];\
             [rs1]scale=1080:608,setsar=1[rs3];\
             [rs2][rs3]overlay=0:656[rs4]"
        );
        assert_eq!(stream_after(&params), (1080, 1920));
    }

    #[test]
    fn fit_with_stretched_unblurred_background() {
        let params = ResolutionParams {
            blur_center: Some(false),
            blur_strength: Range::fixed(0.0),
            ..custom(1080, 1920)
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]split=2[rs0][rs1];[rs0]scale=1080:1920,setsar=1[rs2];\
             [rs1]scale=1080:608,setsar=1[rs3];[rs2][rs3]overlay=0:656[rs4]"
        );
    }

    #[test]
    fn blur_center_follows_settings_unless_set_in_recipe() {
        use crate::pipeline::testing::{build_full, sample_info};
        let build = |blur_center: Option<bool>, global: bool| {
            let mut recipe = crate::model::Recipe::disabled();
            recipe.modules.resolution = ResolutionParams {
                blur_center,
                ..custom(1080, 1920)
            };
            let settings = crate::model::Settings {
                blur_center: global,
                ..crate::model::Settings::default()
            };
            let built = build_full(
                &sample_info(),
                &recipe,
                &settings,
                &crate::ffmpeg::EncoderCaps::software_only(),
                1,
            )
            .unwrap();
            built
                .arg_after("-filter_complex")
                .unwrap()
                .contains("force_original_aspect_ratio=increase")
        };
        assert!(build(None, true), "跟随设置：取中间");
        assert!(!build(None, false), "跟随设置：整幅拉伸");
        assert!(build(Some(true), false), "配方里的值优先");
        assert!(!build(Some(false), true), "配方里的值优先");
    }

    #[test]
    fn fit_black_and_reflection() {
        let black = ResolutionParams {
            fill: FillMode::Black,
            ..custom(1080, 1920)
        };
        assert_eq!(
            filter(black.clone(), 1),
            "[0:v:0]scale=1080:608,setsar=1,pad=1080:1920:0:656:black[v0]"
        );
        let reflection = ResolutionParams {
            reflection: true,
            ..black
        };
        assert_eq!(
            filter(reflection, 1),
            "[0:v:0]scale=1080:608,setsar=1,split=2[rs0][rs1];\
             [rs0]pad=1080:1920:0:656:black[rs2];\
             [rs1]vflip,crop=1080:608:0:0,format=yuva420p,\
             geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='127.5*(1-Y/H)'[rs3];\
             [rs2][rs3]overlay=0:1264:format=auto[rs4]"
        );
    }

    #[test]
    fn reflection_without_vertical_space_warns() {
        // 目标画布比源更宽：只有左右留空
        let params = ResolutionParams {
            fill: FillMode::Black,
            reflection: true,
            ..custom(2000, 720)
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]scale=1280:720,setsar=1,pad=2000:720:360:0:black[v0]")
        );
        assert!(built.warnings.iter().any(|w| w.contains("倒影")));
    }

    #[test]
    fn blur_range_is_reproducible_and_bounded() {
        let params = ResolutionParams {
            blur_strength: Range::new(5.0, 40.0),
            ..custom(1080, 1920)
        };
        assert_eq!(filter(params.clone(), 9), filter(params.clone(), 9));
        for seed in 0..20 {
            let fc = filter(params.clone(), seed);
            let sigma: f64 = fc
                .split("gblur=sigma=")
                .nth(1)
                .unwrap()
                .split('[')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!((5.0..=40.0).contains(&sigma), "{fc}");
        }
    }

    #[test]
    fn odd_sizes_are_made_even_and_invalid_rejected() {
        let params = ResolutionParams {
            scale_mode: ScaleMode::Stretch,
            ..custom(641, 481)
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]scale=640:480,setsar=1[v0]")
        );
        assert!(built.warnings.iter().any(|w| w.contains("偶数")));

        assert!(build_with(&sample_info(), &recipe_with(custom(8, 720)), 1).is_err());
        let huge = ResolutionParams {
            standard_height: 10000,
            ..ResolutionParams {
                enabled: true,
                ..ResolutionParams::default()
            }
        };
        assert!(build_with(&sample_info(), &recipe_with(huge), 1).is_err());
        let bad_blur = ResolutionParams {
            blur_strength: Range::new(-1.0, 10.0),
            ..custom(1080, 1920)
        };
        let err = build_with(&sample_info(), &recipe_with(bad_blur), 1).unwrap_err();
        assert!(err.contains("虚化"), "{err}");
    }
}
