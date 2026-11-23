//! `pipe.crop` 裁剪：四种方式，最终都归结为一个 `crop=宽:高:x:y` 滤镜。
//!
//! - **指定四边**（`margins`）：左/上/右/下各裁掉多少像素，单位是**源视频像素**（已考虑旋转后的显示方向）。
//!   画面框选得到的矩形 `(x, y, w, h)` 直接对应 `left = x`、`top = y`、
//!   `right = 源宽 - x - w`、`bottom = 源高 - y - h`。
//!   若之前的模块改变了画面尺寸，四边按比例换算到当前画面。
//! - **按百分比**（`percent`）：四边各裁掉画面宽/高的百分之几（每边 0～45%）。
//! - **四边随机**（`randomPercent`）：四边各自在范围内随机取百分比（每边 0～45%），
//!   取值保留两位小数，依次取左、上、右、下。
//! - **居中 + 比例**（`aspect`）：按 `aspectWidth:aspectHeight` 取画面中央最大的区域。
//!
//! 规则：
//! - 结果的宽、高与左上角坐标都向下取偶数（yuv420p 编码要求），因此奇数尺寸的源视频
//!   即使四边都为 0 也会裁成偶数尺寸；结果永远不会超出画面。
//! - 四边之和不能达到或超过画面尺寸（返回错误）；裁剪后宽、高都必须不小于
//!   [`MIN_KEEP_PX`] 像素且不小于原画面的 [`MIN_KEEP_RATIO`]，否则视为“剩余画面太小”。
//! - 结果与原画面完全相同时不添加滤镜。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{BuildContext, PipelineModule};

pub const ID: &str = "pipe.crop";

/// 裁剪后宽、高的最小像素数。
pub const MIN_KEEP_PX: u32 = 16;
/// 裁剪后宽、高至少保留原画面的比例。
pub const MIN_KEEP_RATIO: f64 = 0.1;
/// 百分比 / 随机模式下每一边最多裁掉的百分比。
pub const MAX_SIDE_PERCENT: f64 = 45.0;
/// 比例模式下宽、高比值各自的上限。
pub const MAX_ASPECT_TERM: u32 = 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CropMode {
    /// 指定四边（源像素）
    #[default]
    Margins,
    /// 按百分比
    Percent,
    /// 四边随机（百分比范围）
    Random,
    /// 居中 + 比例
    Aspect,
}

/// 四条边的值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sides<T> {
    pub left: T,
    pub top: T,
    pub right: T,
    pub bottom: T,
}

impl<T: Copy> Sides<T> {
    pub const fn all(value: T) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    fn values(&self) -> [T; 4] {
        [self.left, self.top, self.right, self.bottom]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CropParams {
    pub enabled: bool,
    pub mode: CropMode,
    /// 指定四边：各边裁掉的源像素数
    pub margins: Sides<u32>,
    /// 按百分比：各边裁掉的百分比（0～45）
    pub percent: Sides<f64>,
    /// 四边随机：各边裁掉的百分比范围（0～45）
    pub random_percent: Sides<Range<f64>>,
    /// 居中 + 比例：宽高比，如 4:3、9:16
    pub aspect_width: u32,
    pub aspect_height: u32,
}

impl Default for CropParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: CropMode::Margins,
            margins: Sides::all(0),
            percent: Sides::all(5.0),
            random_percent: Sides::all(Range::new(0.0, 5.0)),
            aspect_width: 4,
            aspect_height: 3,
        }
    }
}

/// 一个裁剪矩形（像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

fn even_floor(v: u32) -> u32 {
    v & !1
}

/// 由四边（像素，可为小数）算出裁剪矩形，并检查是否越界、剩余画面是否太小。
fn rect_from_margins(frame_w: u32, frame_h: u32, sides: [f64; 4]) -> Result<CropRect, String> {
    let [l, t, r, b] = sides.map(|v| v.round().max(0.0) as u64);
    let (fw, fh) = (u64::from(frame_w), u64::from(frame_h));
    if l + r >= fw || t + b >= fh {
        return Err(format!(
            "裁剪区域超出画面：画面为 {frame_w}x{frame_h}，左右共裁 {} 像素、上下共裁 {} 像素",
            l + r,
            t + b
        ));
    }
    let x = even_floor(l as u32);
    let y = even_floor(t as u32);
    // 左上角向下取偶后，宽高相应多出的部分也算进去，再整体向下取偶，保证不越界。
    let width = even_floor((fw - r - u64::from(x)) as u32);
    let height = even_floor((fh - b - u64::from(y)) as u32);
    check_remaining(frame_w, frame_h, width, height)?;
    Ok(CropRect {
        x,
        y,
        width,
        height,
    })
}

fn check_remaining(frame_w: u32, frame_h: u32, width: u32, height: u32) -> Result<(), String> {
    let min_w = MIN_KEEP_PX.max((f64::from(frame_w) * MIN_KEEP_RATIO).ceil() as u32);
    let min_h = MIN_KEEP_PX.max((f64::from(frame_h) * MIN_KEEP_RATIO).ceil() as u32);
    if width < min_w || height < min_h {
        return Err(format!(
            "裁剪后剩余画面太小（{width}x{height}），宽、高都至少要保留 {MIN_KEEP_PX} 像素且不少于原画面的 {}%",
            (MIN_KEEP_RATIO * 100.0) as u32
        ));
    }
    Ok(())
}

/// 居中取最大的 `aw:ah` 区域。
fn rect_for_aspect(frame_w: u32, frame_h: u32, aw: u32, ah: u32) -> Result<CropRect, String> {
    if aw == 0 || ah == 0 || aw > MAX_ASPECT_TERM || ah > MAX_ASPECT_TERM {
        return Err(format!("画面比例的宽、高需在 1～{MAX_ASPECT_TERM} 之间"));
    }
    let (fw, fh) = (u64::from(frame_w), u64::from(frame_h));
    let (aw, ah) = (u64::from(aw), u64::from(ah));
    // 画面比目标更宽：高度占满，宽度按比例；否则宽度占满。
    let (w, h) = if fw * ah > fh * aw {
        (fh * aw / ah, fh)
    } else {
        (fw, fw * ah / aw)
    };
    let width = even_floor(w as u32);
    let height = even_floor(h as u32);
    check_remaining(frame_w, frame_h, width, height)?;
    Ok(CropRect {
        x: even_floor((frame_w - width) / 2),
        y: even_floor((frame_h - height) / 2),
        width,
        height,
    })
}

fn check_percent(value: f64) -> Result<(), String> {
    if !(0.0..=MAX_SIDE_PERCENT).contains(&value) {
        return Err(format!("每边裁剪百分比需在 0～{MAX_SIDE_PERCENT}% 之间"));
    }
    Ok(())
}

fn percent_to_px(frame_w: u32, frame_h: u32, p: [f64; 4]) -> [f64; 4] {
    let (w, h) = (f64::from(frame_w), f64::from(frame_h));
    [
        p[0] * w / 100.0,
        p[1] * h / 100.0,
        p[2] * w / 100.0,
        p[3] * h / 100.0,
    ]
}

impl CropParams {
    /// 在 `frame_w x frame_h` 的当前画面上计算裁剪矩形。
    /// `source` 是源视频的显示尺寸，用于把“源像素”四边换算到当前画面。
    pub fn compute_rect(
        &self,
        frame_w: u32,
        frame_h: u32,
        source: (u32, u32),
        rng: &mut crate::model::SeededRng,
    ) -> Result<CropRect, String> {
        if frame_w == 0 || frame_h == 0 {
            return Err("无法获取视频画面尺寸".into());
        }
        match self.mode {
            CropMode::Margins => {
                let (sw, sh) = source;
                let sx = if sw > 0 {
                    f64::from(frame_w) / f64::from(sw)
                } else {
                    1.0
                };
                let sy = if sh > 0 {
                    f64::from(frame_h) / f64::from(sh)
                } else {
                    1.0
                };
                let m = self.margins.values().map(f64::from);
                rect_from_margins(
                    frame_w,
                    frame_h,
                    [m[0] * sx, m[1] * sy, m[2] * sx, m[3] * sy],
                )
            }
            CropMode::Percent => {
                let p = self.percent.values();
                for v in p {
                    check_percent(v)?;
                }
                rect_from_margins(frame_w, frame_h, percent_to_px(frame_w, frame_h, p))
            }
            CropMode::Random => {
                for range in self.random_percent.values() {
                    let (lo, hi) = range.ordered();
                    check_percent(lo)?;
                    check_percent(hi)?;
                }
                let p = self
                    .random_percent
                    .values()
                    .map(|range| range.sample_rounded(rng, 2));
                rect_from_margins(frame_w, frame_h, percent_to_px(frame_w, frame_h, p))
            }
            CropMode::Aspect => {
                rect_for_aspect(frame_w, frame_h, self.aspect_width, self.aspect_height)
            }
        }
    }
}

impl PipelineModule for CropParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let source = ctx.source_size();
        let (fw, fh) = (ctx.stream.width, ctx.stream.height);
        let rect = self.compute_rect(fw, fh, source, &mut ctx.rng)?;
        if rect.x == 0 && rect.y == 0 && rect.width == fw && rect.height == fh {
            return Ok(());
        }
        ctx.graph.video(format!(
            "crop={}:{}:{}:{}",
            rect.width, rect.height, rect.x, rect.y
        ));
        ctx.stream.width = rect.width;
        ctx.stream.height = rect.height;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, SeededRng};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: CropParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.crop = CropParams {
            enabled: true,
            ..params
        };
        recipe
    }

    fn filter(params: CropParams) -> Result<Option<String>, String> {
        build_with(&sample_info(), &recipe_with(params), 42)
            .map(|b| b.arg_after("-filter_complex").map(str::to_owned))
    }

    fn sized(w: u32, h: u32) -> crate::ffmpeg::MediaInfo {
        let mut info = sample_info();
        let v = info.video.as_mut().unwrap();
        v.width = w;
        v.height = h;
        info
    }

    #[test]
    fn defaults_are_a_no_op() {
        assert_eq!(filter(CropParams::default()).unwrap(), None);
    }

    #[test]
    fn margins_map_to_crop_filter() {
        let params = CropParams {
            margins: Sides {
                left: 100,
                top: 20,
                right: 80,
                bottom: 60,
            },
            ..CropParams::default()
        };
        // 1280 - 100 - 80 = 1100；720 - 20 - 60 = 640
        assert_eq!(
            filter(params).unwrap().as_deref(),
            Some("[0:v:0]crop=1100:640:100:20[v0]")
        );
    }

    #[test]
    fn odd_margins_and_odd_frames_round_to_even() {
        let params = CropParams {
            margins: Sides {
                left: 11,
                top: 5,
                right: 10,
                bottom: 0,
            },
            ..CropParams::default()
        };
        // x 11→10，y 5→4；宽 1280-10-10=1260，高 720-0-4=716
        assert_eq!(
            filter(params).unwrap().as_deref(),
            Some("[0:v:0]crop=1260:716:10:4[v0]")
        );
        // 奇数尺寸源视频：四边为 0 也要裁成偶数
        let built = build_with(&sized(321, 241), &recipe_with(CropParams::default()), 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]crop=320:240:0:0[v0]")
        );
    }

    #[test]
    fn percent_mode() {
        let params = CropParams {
            mode: CropMode::Percent,
            percent: Sides {
                left: 10.0,
                top: 0.0,
                right: 10.0,
                bottom: 25.0,
            },
            ..CropParams::default()
        };
        // 左右各 128，下 180
        assert_eq!(
            filter(params).unwrap().as_deref(),
            Some("[0:v:0]crop=1024:540:128:0[v0]")
        );
    }

    #[test]
    fn aspect_mode_takes_largest_centred_area() {
        let cases = [
            ((4, 3), "crop=960:720:160:0"),
            ((1, 1), "crop=720:720:280:0"),
            ((9, 16), "crop=404:720:438:0"),
            ((16, 9), ""),
            ((21, 9), "crop=1280:548:0:86"),
        ];
        for ((aw, ah), expected) in cases {
            let params = CropParams {
                mode: CropMode::Aspect,
                aspect_width: aw,
                aspect_height: ah,
                ..CropParams::default()
            };
            let fc = filter(params).unwrap();
            if expected.is_empty() {
                assert_eq!(fc, None, "{aw}:{ah}");
            } else {
                assert_eq!(fc, Some(format!("[0:v:0]{expected}[v0]")), "{aw}:{ah}");
            }
        }
        // 竖屏源取 1:1
        let params = recipe_with(CropParams {
            mode: CropMode::Aspect,
            aspect_width: 1,
            aspect_height: 1,
            ..CropParams::default()
        });
        let built = build_with(&sized(720, 1280), &params, 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]crop=720:720:0:280[v0]")
        );
    }

    #[test]
    fn random_mode_is_reproducible_and_bounded() {
        let params = CropParams {
            mode: CropMode::Random,
            random_percent: Sides::all(Range::new(0.0, 20.0)),
            ..CropParams::default()
        };
        let a = filter(params.clone()).unwrap();
        let b = filter(params.clone()).unwrap();
        assert_eq!(a, b);
        // 固定参考值：种子 42 下的取值（算法或派生方式改变时会失败）
        assert_eq!(a.as_deref(), Some("[0:v:0]crop=1116:586:136:72[v0]"));

        let source = (1280, 720);
        for seed in 0..200 {
            let mut rng = SeededRng::new(seed);
            let r = params.compute_rect(1280, 720, source, &mut rng).unwrap();
            assert!(r.x + r.width <= 1280 && r.y + r.height <= 720, "{r:?}");
            // 每边最多 20%：宽至少 60%，高至少 60%（再减去取偶的 2 像素）
            assert!(r.width >= 768 - 2 && r.height >= 432 - 2, "{r:?}");
            assert!(r.x <= 256 && r.y <= 144, "{r:?}");
            assert_eq!([r.x % 2, r.y % 2, r.width % 2, r.height % 2], [0; 4]);
        }
    }

    #[test]
    fn margins_scale_when_frame_differs_from_source() {
        let params = CropParams {
            margins: Sides {
                left: 100,
                top: 100,
                right: 100,
                bottom: 100,
            },
            ..CropParams::default()
        };
        let mut rng = SeededRng::new(0);
        // 源 1280x720，当前画面已被缩小一半
        let r = params
            .compute_rect(640, 360, (1280, 720), &mut rng)
            .unwrap();
        assert_eq!(
            r,
            CropRect {
                x: 50,
                y: 50,
                width: 540,
                height: 260
            }
        );
    }

    #[test]
    fn invalid_crops_are_rejected() {
        let too_wide = CropParams {
            margins: Sides {
                left: 700,
                top: 0,
                right: 700,
                bottom: 0,
            },
            ..CropParams::default()
        };
        let err = filter(too_wide).unwrap_err();
        assert!(err.contains("超出画面"), "{err}");

        let too_small = CropParams {
            margins: Sides {
                left: 0,
                top: 340,
                right: 0,
                bottom: 340,
            },
            ..CropParams::default()
        };
        let err = filter(too_small).unwrap_err();
        assert!(err.contains("太小"), "{err}");

        let bad_percent = CropParams {
            mode: CropMode::Percent,
            percent: Sides::all(50.0),
            ..CropParams::default()
        };
        assert!(filter(bad_percent).unwrap_err().contains("百分比"));

        let bad_random = CropParams {
            mode: CropMode::Random,
            random_percent: Sides::all(Range::new(-1.0, 10.0)),
            ..CropParams::default()
        };
        assert!(filter(bad_random).is_err());

        let bad_aspect = CropParams {
            mode: CropMode::Aspect,
            aspect_width: 0,
            ..CropParams::default()
        };
        assert!(filter(bad_aspect).unwrap_err().contains("比例"));

        let extreme_aspect = CropParams {
            mode: CropMode::Aspect,
            aspect_width: 100,
            aspect_height: 1,
            ..CropParams::default()
        };
        assert!(filter(extreme_aspect).unwrap_err().contains("太小"));
    }

    #[test]
    fn crop_runs_before_later_video_filters() {
        let mut recipe = recipe_with(CropParams {
            mode: CropMode::Aspect,
            aspect_width: 1,
            aspect_height: 1,
            ..CropParams::default()
        });
        recipe.modules.fps.enabled = true;
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[0:v:0]crop=720:720:280:0,fps=30[v0]")
        );
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_value(CropParams::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": false,
                "mode": "margins",
                "margins": {"left": 0, "top": 0, "right": 0, "bottom": 0},
                "percent": {"left": 5.0, "top": 5.0, "right": 5.0, "bottom": 5.0},
                "randomPercent": {
                    "left": {"min": 0.0, "max": 5.0},
                    "top": {"min": 0.0, "max": 5.0},
                    "right": {"min": 0.0, "max": 5.0},
                    "bottom": {"min": 0.0, "max": 5.0}
                },
                "aspectWidth": 4,
                "aspectHeight": 3
            })
        );
        let partial: CropParams =
            serde_json::from_str(r#"{"enabled":true,"mode":"aspect"}"#).unwrap();
        assert_eq!(partial.aspect_width, 4);
        assert_eq!(partial.mode, CropMode::Aspect);
    }
}
