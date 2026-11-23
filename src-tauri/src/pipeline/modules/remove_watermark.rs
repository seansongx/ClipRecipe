//! `pipe.remove-watermark` 去水印：把画面上的一处或几处矩形区域模糊掉（不是修复/填补）。
//!
//! 参数与取舍（FEATURES.md 未规定的部分）：
//! - 区域来源 `mode`：
//!   - `preset` 预设位置（可多选），按当前画面尺寸换算，不对应任何具体平台：
//!     - 四角标志 `topLeft` / `topRight` / `bottomLeft` / `bottomRight`：宽为短边的 30%、高为短边的 12%，
//!       距边缘短边的 2%（横屏、竖屏的角标大小一致）；
//!     - 底部字幕条 `bottomCaption`：宽为画面宽的 90%（水平居中），从画面高 76% 处起、高为画面高的 14%；
//!     - 顶部横幅 `topBanner`：整个宽度，高为短边的 10%。
//!   - `custom` 自定义 1～2 个矩形（`regions`），坐标为**源视频像素**（已按旋转转正，用 RegionPicker 框选）。
//!     本模块在固定顺序中排在裁剪、旋转、分辨率之前，看到的就是源画面；若画面尺寸与源不同则按比例换算。
//! - 区域规则：超出画面的部分被裁掉；左上角与宽高向下取偶数；宽或高为 0、完全在画面外、
//!   或裁剪后宽 / 高小于 [`MIN_REGION_PX`] 像素的区域视为无效（返回错误）。
//! - 模糊强度 `strength`（1～10，整数）：`boxblur` 亮度半径 = 区域短边的一半 × 强度 / 10（至少 1），
//!   色度半径取其一半（不超过 yuv420 色度平面允许的上限），各叠加 2 次（近似高斯）。10 几乎抹成一片纯色。
//! - 生效时段 `limitTime` + `start` / `end`（秒，`end > start`）：只在该时段内模糊。
//!   **时间指本模块所在位置的时间轴**：去头尾、抽帧、变速在本模块之前执行，所以这是**处理后视频**的时间
//!   （去掉片头后从 0 开始、变速后按新速度计；不含之后加上的片头视频）。
//!   前面的模块改变了时长时给出提示。
//!
//! 实现：`split` 出 1 + N 路，每个区域 `crop → format=yuv420p → boxblur`，再依次 `overlay` 回原位置，
//! 限定时段时 overlay 带 `enable='between(t,起,止)'`。不改变尺寸、帧率与时长。
//!
//! 预览（`ctx.is_preview()`）时再用 `drawbox` 沿每个区域的边缘（内侧）画一圈框线（[`PREVIEW_OUTLINE_COLOR`]，
//! 粗细见 [`preview_outline_px`]，时段与模糊相同），方便确认位置；正式输出不画。

use serde::{Deserialize, Serialize};

use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.remove-watermark";

/// 自定义区域的最大个数。
pub const MAX_REGIONS: usize = 2;
/// 区域裁剪到画面内后宽、高的最小像素数。
pub const MIN_REGION_PX: u32 = 8;
/// 时段参数允许的最大值（秒）。
pub const MAX_SECONDS: f64 = 86_400.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegionMode {
    /// 预设位置
    #[default]
    Preset,
    /// 自定义矩形（源像素）
    Custom,
}

/// 常见水印位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WatermarkPreset {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    BottomCaption,
    TopBanner,
}

/// 矩形区域（像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoveWatermarkParams {
    pub enabled: bool,
    pub mode: RegionMode,
    /// 预设位置（可多选）
    pub presets: Vec<WatermarkPreset>,
    /// 自定义区域（源像素，最多 2 个）
    pub regions: Vec<Region>,
    /// 模糊强度 1～10
    pub strength: u32,
    /// 只在 `start`～`end` 秒内模糊
    pub limit_time: bool,
    pub start: f64,
    pub end: f64,
}

impl Default for RemoveWatermarkParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: RegionMode::Preset,
            presets: vec![WatermarkPreset::TopRight],
            regions: Vec::new(),
            strength: 6,
            limit_time: false,
            start: 0.0,
            end: 5.0,
        }
    }
}

fn even_floor(v: u32) -> u32 {
    v & !1
}

/// 预设位置在 `w x h` 画面上的矩形（未取偶）。
pub fn preset_rect(preset: WatermarkPreset, w: u32, h: u32) -> Region {
    let (fw, fh) = (f64::from(w), f64::from(h));
    let short = fw.min(fh);
    let px = |v: f64| v.round().max(0.0) as u32;
    let (cw, ch, m) = (px(short * 0.30), px(short * 0.12), px(short * 0.02));
    let corner = |right: bool, bottom: bool| Region {
        x: if right { w.saturating_sub(cw + m) } else { m },
        y: if bottom { h.saturating_sub(ch + m) } else { m },
        width: cw,
        height: ch,
    };
    match preset {
        WatermarkPreset::TopLeft => corner(false, false),
        WatermarkPreset::TopRight => corner(true, false),
        WatermarkPreset::BottomLeft => corner(false, true),
        WatermarkPreset::BottomRight => corner(true, true),
        WatermarkPreset::BottomCaption => Region {
            x: px(fw * 0.05),
            y: px(fh * 0.76),
            width: px(fw * 0.90),
            height: px(fh * 0.14),
        },
        WatermarkPreset::TopBanner => Region {
            x: 0,
            y: 0,
            width: w,
            height: px(short * 0.10),
        },
    }
}

/// 把区域裁剪到 `w x h` 画面内并取偶；无效时返回错误（`name` 用于提示）。
pub fn clamp_region(r: Region, w: u32, h: u32, name: &str) -> Result<Region, String> {
    if r.width == 0 || r.height == 0 {
        return Err(format!("{name}的宽或高为 0"));
    }
    if r.x >= w || r.y >= h {
        return Err(format!("{name}完全在画面（{w}x{h}）之外"));
    }
    let x = even_floor(r.x);
    let y = even_floor(r.y);
    let right = r.x.saturating_add(r.width).min(w);
    let bottom = r.y.saturating_add(r.height).min(h);
    let width = even_floor(right - x);
    let height = even_floor(bottom - y);
    if width < MIN_REGION_PX || height < MIN_REGION_PX {
        return Err(format!(
            "{name}太小（裁剪到画面内后为 {width}x{height}），宽、高至少需要 {MIN_REGION_PX} 像素"
        ));
    }
    Ok(Region {
        x,
        y,
        width,
        height,
    })
}

/// 源像素区域按比例换算到当前画面。
fn scale_region(r: Region, sx: f64, sy: f64) -> Region {
    if sx == 1.0 && sy == 1.0 {
        return r;
    }
    let s = |v: u32, k: f64| (f64::from(v) * k).round() as u32;
    Region {
        x: s(r.x, sx),
        y: s(r.y, sy),
        width: s(r.width, sx),
        height: s(r.height, sy),
    }
}

/// `boxblur` 参数：亮度半径与色度半径（yuv420p，色度平面为一半大小）。
/// boxblur 要求半径严格小于所在平面短边的一半。
pub fn blur_radii(region: Region, strength: u32) -> (u32, u32) {
    let short = region.width.min(region.height);
    let max_luma = short.saturating_sub(1) / 2;
    let max_chroma = (short / 2).saturating_sub(1) / 2;
    let luma = ((f64::from(short) / 2.0 * f64::from(strength) / 10.0).round() as u32)
        .clamp(1, max_luma.max(1));
    let chroma = (luma / 2).clamp(1, max_chroma.max(1));
    (luma, chroma)
}

impl RemoveWatermarkParams {
    /// 在 `w x h` 的当前画面上计算全部区域。`source` 为源视频显示尺寸（自定义区域的坐标系）。
    pub fn compute_regions(
        &self,
        w: u32,
        h: u32,
        source: (u32, u32),
    ) -> Result<Vec<Region>, String> {
        if w == 0 || h == 0 {
            return Err("无法获取视频画面尺寸".into());
        }
        match self.mode {
            RegionMode::Preset => {
                if self.presets.is_empty() {
                    return Err("请至少选择一个预设位置".into());
                }
                let mut seen = Vec::new();
                for preset in &self.presets {
                    if !seen.contains(preset) {
                        seen.push(*preset);
                    }
                }
                seen.iter()
                    .map(|p| clamp_region(preset_rect(*p, w, h), w, h, "预设区域"))
                    .collect()
            }
            RegionMode::Custom => {
                if self.regions.is_empty() {
                    return Err("请先在画面上框选水印区域".into());
                }
                if self.regions.len() > MAX_REGIONS {
                    return Err(format!("自定义区域最多 {MAX_REGIONS} 个"));
                }
                let (sw, sh) = source;
                let sx = if sw > 0 {
                    f64::from(w) / f64::from(sw)
                } else {
                    1.0
                };
                let sy = if sh > 0 {
                    f64::from(h) / f64::from(sh)
                } else {
                    1.0
                };
                self.regions
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        clamp_region(scale_region(*r, sx, sy), w, h, &format!("区域 {}", i + 1))
                    })
                    .collect()
            }
        }
    }

    fn time_window(&self) -> Result<Option<(f64, f64)>, String> {
        if !self.limit_time {
            return Ok(None);
        }
        let valid = |v: f64| v.is_finite() && (0.0..=MAX_SECONDS).contains(&v);
        if !valid(self.start) || !valid(self.end) {
            return Err(format!("生效时段需在 0～{} 秒之间", fmt_num(MAX_SECONDS)));
        }
        if self.end <= self.start {
            return Err("生效时段的结束时间必须晚于开始时间".into());
        }
        Ok(Some((self.start, self.end)))
    }
}

impl PipelineModule for RemoveWatermarkParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        if !(1..=10).contains(&self.strength) {
            return Err("模糊强度需在 1～10 之间".into());
        }
        let window = self.time_window()?;
        let source = ctx.source_size();
        let regions = self.compute_regions(ctx.stream.width, ctx.stream.height, source)?;

        if let Some((start, _)) = window {
            if ctx.info.duration > 0.0 && (ctx.stream.duration - ctx.info.duration).abs() > 0.01 {
                ctx.warn(
                    "前面的模块改变了视频时长，生效时段按处理后（去头尾 / 变速之后）的时间计算",
                );
            }
            if ctx.stream.duration > 0.0 && start >= ctx.stream.duration {
                ctx.warn("生效时段的开始时间超过视频时长，模糊不会出现");
            }
        }
        let enable = window
            .map(|(a, b)| format!(":enable=between(t\\,{}\\,{})", fmt_num(a), fmt_num(b)))
            .unwrap_or_default();

        let input = ctx.graph.take_video();
        let base = ctx.graph.label("rw");
        let crops: Vec<String> = regions.iter().map(|_| ctx.graph.label("rw")).collect();
        let mut outs = vec![base.as_str()];
        outs.extend(crops.iter().map(String::as_str));
        ctx.graph
            .chain(&[&input], &format!("split={}", regions.len() + 1), &outs);
        let mut current = base.clone();
        for (region, crop) in regions.iter().zip(&crops) {
            let (luma, chroma) = blur_radii(*region, self.strength);
            let blurred = ctx.graph.label("rw");
            ctx.graph.chain(
                &[crop],
                &format!(
                    "crop={}:{}:{}:{},format=yuv420p,boxblur=lr={luma}:lp=2:cr={chroma}:cp=2",
                    region.width, region.height, region.x, region.y
                ),
                &[&blurred],
            );
            let out = ctx.graph.label("rw");
            ctx.graph.chain(
                &[&current, &blurred],
                &format!("overlay=x={}:y={}{enable}", region.x, region.y),
                &[&out],
            );
            current = out;
        }
        ctx.graph.set_video(current);
        if ctx.is_preview() {
            // 只在预览里用框线标出区域（与模糊同一时段），正式输出绝不添加
            let thickness = preview_outline_px(ctx.stream.width, ctx.stream.height);
            let boxes: Vec<String> = regions
                .iter()
                .map(|r| {
                    format!(
                        "drawbox=x={}:y={}:w={}:h={}:color={PREVIEW_OUTLINE_COLOR}:t={thickness}{enable}",
                        r.x, r.y, r.width, r.height
                    )
                })
                .collect();
            ctx.graph.video(boxes.join(","));
        }
        Ok(())
    }
}

/// 预览中区域框线的颜色。
pub const PREVIEW_OUTLINE_COLOR: &str = "0xffd000";

/// 预览中区域框线的粗细：短边的 1/200，至少 2 像素（预览会把画面缩到 480 高，粗一点才看得清）。
pub fn preview_outline_px(w: u32, h: u32) -> u32 {
    (w.min(h) / 200).max(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, Settings};
    use crate::pipeline::testing::{self, build_with, sample_info};

    fn recipe_with(params: RemoveWatermarkParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.remove_watermark = RemoveWatermarkParams {
            enabled: true,
            ..params
        };
        recipe
    }

    fn filter(params: RemoveWatermarkParams) -> Result<String, String> {
        build_with(&sample_info(), &recipe_with(params), 42)
            .map(|b| b.arg_after("-filter_complex").unwrap_or("").to_owned())
    }

    fn custom(regions: Vec<Region>) -> RemoveWatermarkParams {
        RemoveWatermarkParams {
            mode: RegionMode::Custom,
            regions,
            ..RemoveWatermarkParams::default()
        }
    }

    const fn rect(x: u32, y: u32, width: u32, height: u32) -> Region {
        Region {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn default_preset_blurs_top_right_corner() {
        // 1280x720：短边 720 → 216x86，边距 14
        assert_eq!(
            filter(RemoveWatermarkParams::default()).unwrap(),
            "[0:v:0]split=2[rw0][rw1];\
             [rw1]crop=216:86:1050:14,format=yuv420p,boxblur=lr=26:lp=2:cr=13:cp=2[rw2];\
             [rw0][rw2]overlay=x=1050:y=14[rw3]"
        );
    }

    #[test]
    fn preview_outlines_regions_but_real_output_does_not() {
        let params = RemoveWatermarkParams {
            presets: vec![WatermarkPreset::TopRight, WatermarkPreset::TopLeft],
            limit_time: true,
            start: 1.0,
            end: 3.0,
            ..RemoveWatermarkParams::default()
        };
        let recipe = recipe_with(params);
        // 正式输出：没有框线
        let real = build_with(&sample_info(), &recipe, 42).unwrap();
        let fc = real.arg_after("-filter_complex").unwrap();
        assert!(!fc.contains("drawbox"), "{fc}");
        // 预览：模糊之后、缩小之前，每个区域一个框线，时段与模糊相同；1280x720 → 粗 3
        let preview = testing::build_preview_with(&sample_info(), &recipe, 42).unwrap();
        let fc = preview.arg_after("-filter_complex").unwrap();
        assert!(
            fc.ends_with(
                "[rw6]drawbox=x=1050:y=14:w=216:h=86:color=0xffd000:t=3:enable=between(t\\,1\\,3),\
                 drawbox=x=14:y=14:w=216:h=86:color=0xffd000:t=3:enable=between(t\\,1\\,3),\
                 scale=-2:360[v7]"
            ),
            "{fc}"
        );
        assert_eq!(preview_outline_px(321, 241), 2);
        assert_eq!(preview_outline_px(3840, 2160), 10);
    }

    #[test]
    fn two_custom_regions_with_time_window() {
        let params = RemoveWatermarkParams {
            limit_time: true,
            start: 1.5,
            end: 4.0,
            strength: 10,
            ..custom(vec![rect(11, 21, 101, 51), rect(1000, 600, 400, 300)])
        };
        // 区域 1：x/y 取偶 10,20；右边界 112 → 宽 102，下边界 72 → 高 52
        // 区域 2：超出画面的部分裁掉 → 280x120
        assert_eq!(
            filter(params).unwrap(),
            "[0:v:0]split=3[rw0][rw1][rw2];\
             [rw1]crop=102:52:10:20,format=yuv420p,boxblur=lr=25:lp=2:cr=12:cp=2[rw3];\
             [rw0][rw3]overlay=x=10:y=20:enable=between(t\\,1.5\\,4)[rw4];\
             [rw2]crop=280:120:1000:600,format=yuv420p,boxblur=lr=59:lp=2:cr=29:cp=2[rw5];\
             [rw4][rw5]overlay=x=1000:y=600:enable=between(t\\,1.5\\,4)[rw6]"
        );
    }

    #[test]
    fn presets_scale_with_frame_and_orientation() {
        for (w, h) in [(1280, 720), (720, 1280), (321, 241), (1920, 1080)] {
            for preset in [
                WatermarkPreset::TopLeft,
                WatermarkPreset::TopRight,
                WatermarkPreset::BottomLeft,
                WatermarkPreset::BottomRight,
                WatermarkPreset::BottomCaption,
                WatermarkPreset::TopBanner,
            ] {
                let r = clamp_region(preset_rect(preset, w, h), w, h, "x").unwrap();
                assert!(
                    r.x + r.width <= w && r.y + r.height <= h,
                    "{preset:?} {r:?}"
                );
                assert_eq!([r.x % 2, r.y % 2, r.width % 2, r.height % 2], [0; 4]);
            }
        }
        // 竖屏的角标大小与横屏一致（按短边计算）
        let land = preset_rect(WatermarkPreset::BottomLeft, 1280, 720);
        let port = preset_rect(WatermarkPreset::BottomLeft, 720, 1280);
        assert_eq!((land.width, land.height), (port.width, port.height));
        assert_eq!(port, rect(14, 1180, 216, 86));
        assert_eq!(
            preset_rect(WatermarkPreset::BottomCaption, 1280, 720),
            rect(64, 547, 1152, 101)
        );
    }

    #[test]
    fn duplicate_presets_are_merged() {
        let params = RemoveWatermarkParams {
            presets: vec![WatermarkPreset::TopBanner, WatermarkPreset::TopBanner],
            ..RemoveWatermarkParams::default()
        };
        let fc = filter(params).unwrap();
        assert!(
            fc.starts_with("[0:v:0]split=2[rw0][rw1];[rw1]crop=1280:72:0:0,"),
            "{fc}"
        );
    }

    #[test]
    fn custom_regions_are_in_source_pixels() {
        let params = custom(vec![rect(100, 100, 200, 100)]);
        let regions = params.compute_regions(640, 360, (1280, 720)).unwrap();
        assert_eq!(regions, [rect(50, 50, 100, 50)]);
    }

    #[test]
    fn blur_radius_respects_plane_limits() {
        assert_eq!(blur_radii(rect(0, 0, 8, 8), 10), (3, 1));
        assert_eq!(blur_radii(rect(0, 0, 8, 8), 1), (1, 1));
        assert_eq!(blur_radii(rect(0, 0, 1000, 100), 6), (30, 15));
        assert_eq!(blur_radii(rect(0, 0, 70, 70), 10), (34, 17));
        for strength in 1..=10 {
            for size in [8, 10, 12, 50, 70, 98, 400] {
                let (l, c) = blur_radii(rect(0, 0, size, size + 6), strength);
                assert!(l >= 1 && 2 * l < size, "{size} {strength}");
                // yuv420p 色度平面为一半
                assert!(c >= 1 && 2 * c < size / 2, "{size} {strength}");
            }
        }
    }

    #[test]
    fn invalid_params_are_rejected() {
        let cases: Vec<(RemoveWatermarkParams, &str)> = vec![
            (
                RemoveWatermarkParams {
                    presets: vec![],
                    ..RemoveWatermarkParams::default()
                },
                "预设",
            ),
            (custom(vec![]), "框选"),
            (
                custom(vec![
                    rect(0, 0, 10, 10),
                    rect(20, 20, 10, 10),
                    rect(40, 40, 10, 10),
                ]),
                "最多",
            ),
            (custom(vec![rect(10, 10, 0, 50)]), "为 0"),
            (custom(vec![rect(1280, 10, 50, 50)]), "画面"),
            (custom(vec![rect(1276, 10, 50, 50)]), "太小"),
            (custom(vec![rect(10, 10, 7, 50)]), "太小"),
            (
                RemoveWatermarkParams {
                    strength: 0,
                    ..RemoveWatermarkParams::default()
                },
                "强度",
            ),
            (
                RemoveWatermarkParams {
                    strength: 11,
                    ..RemoveWatermarkParams::default()
                },
                "强度",
            ),
            (
                RemoveWatermarkParams {
                    limit_time: true,
                    start: 5.0,
                    end: 5.0,
                    ..RemoveWatermarkParams::default()
                },
                "晚于",
            ),
            (
                RemoveWatermarkParams {
                    limit_time: true,
                    start: -1.0,
                    end: 5.0,
                    ..RemoveWatermarkParams::default()
                },
                "时段",
            ),
            (
                RemoveWatermarkParams {
                    limit_time: true,
                    start: 0.0,
                    end: f64::NAN,
                    ..RemoveWatermarkParams::default()
                },
                "时段",
            ),
        ];
        for (params, needle) in cases {
            let err = filter(params.clone()).unwrap_err();
            assert!(err.contains(needle), "{params:?} → {err}");
        }
    }

    #[test]
    fn time_window_warnings() {
        let params = RemoveWatermarkParams {
            limit_time: true,
            start: 20.0,
            end: 30.0,
            ..RemoveWatermarkParams::default()
        };
        let built = build_with(&sample_info(), &recipe_with(params.clone()), 1).unwrap();
        assert!(built.warnings.iter().any(|w| w.contains("超过视频时长")));

        // 前面去头尾改变了时长：提示时间轴
        let mut recipe = recipe_with(RemoveWatermarkParams {
            start: 1.0,
            end: 2.0,
            ..params
        });
        recipe.modules.trim.enabled = true;
        recipe.modules.trim.head = crate::model::Range::fixed(2.0);
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert!(
            built.warnings.iter().any(|w| w.contains("处理后")),
            "{:?}",
            built.warnings
        );
    }

    #[test]
    fn stream_is_unchanged_and_odd_frames_work() {
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        let before = ctx.stream.clone();
        RemoveWatermarkParams {
            enabled: true,
            ..RemoveWatermarkParams::default()
        }
        .apply(&mut ctx)
        .unwrap();
        assert_eq!(ctx.stream, before);

        let mut odd = sample_info();
        let v = odd.video.as_mut().unwrap();
        v.width = 321;
        v.height = 241;
        let built = build_with(
            &odd,
            &recipe_with(custom(vec![rect(300, 200, 100, 100)])),
            1,
        )
        .unwrap();
        // 区域被裁到画面内：右边界 321 → 宽 20，下边界 241 → 高 40；最后整体裁成偶数尺寸
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[0:v:0]split=2[rw0][rw1];\
                 [rw1]crop=20:40:300:200,format=yuv420p,boxblur=lr=6:lp=2:cr=3:cp=2[rw2];\
                 [rw0][rw2]overlay=x=300:y=200[rw3];[rw3]crop=320:240:0:0[v4]"
            )
        );
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_value(RemoveWatermarkParams::default()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": false,
                "mode": "preset",
                "presets": ["topRight"],
                "regions": [],
                "strength": 6,
                "limitTime": false,
                "start": 0.0,
                "end": 5.0
            })
        );
        let partial: RemoveWatermarkParams = serde_json::from_str(
            r#"{"enabled":true,"mode":"custom","regions":[{"x":1,"y":2,"width":30,"height":40}]}"#,
        )
        .unwrap();
        assert_eq!(partial.regions, [rect(1, 2, 30, 40)]);
        assert_eq!(partial.strength, 6);
    }
}
