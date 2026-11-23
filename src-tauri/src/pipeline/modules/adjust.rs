//! `pipe.adjust` 画面调整：调色、锐化降噪、LUT 与多种画面效果。每一项单独开关。
//!
//! 参数与取舍（FEATURES.md 未规定的部分）。范围值每个视频随机取值：
//! - 亮度 `brightness`（-1～1）、对比度 `contrast`（0～3）、饱和度 `saturation`（0～3）：合并为一个 `eq`，保留 2 位小数；
//! - 锐化 `sharpen`：`unsharp` 亮度强度（0～5，2 位小数）；降噪 `denoise`：`hqdn3d` 空间强度（0～30，1 位小数）；
//! - LUT `lut`：`.cube/.3dl/.dat/.m3d/.csp` 文件（单文件或目录随机/依次取），`lut3d`；
//! - 入场 `fadeIn`：从黑场淡入，时长（秒，0～60）；渐出 `fadeOut`：结尾淡出到黑场，时长（秒），需要已知时长；
//! - 光晕 `vignette`：`vignette` 镜头角（度，1～89，越大越暗，FFmpeg 默认 36°）；
//! - 黑白 `blackWhite`：`hue=s=0`；
//! - 边框 `border`：沿画面四周画实心边框（宽度 1～500 像素，整数），颜色 `#rrggbb`，或每个视频随机颜色；
//! - 开幕 `opening`：两块黑幕从中间向左右拉开，时长（秒，0～60）；
//! - 马赛克 `mosaic`：整幅画面 `pixelize`，方块边长（像素，2～256，整数）；
//! - 蒙板倒置 `maskInvert`：把画面上下倒置后以给定不透明度（0～1，2 位小数）叠加在原画面上（`blend`）。
//!
//! 应用顺序：LUT → eq → 降噪 → 锐化 → 黑白 → 蒙板倒置 → 马赛克 → 光晕 → 边框 → 入场 → 渐出 → 开幕。
//! 不改变尺寸、帧率与时长。

use serde::{Deserialize, Serialize};

use crate::model::{AssetSource, Range};
use crate::pipeline::{escape_filter_path, fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.adjust";

/// LUT 文件扩展名（目录模式下的过滤条件）。
pub const LUT_EXTENSIONS: [&str; 5] = ["cube", "3dl", "dat", "m3d", "csp"];

/// 一个可单独开关的范围值参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tweak<T> {
    pub enabled: bool,
    pub value: Range<T>,
}

impl<T: Default> Default for Tweak<T> {
    fn default() -> Self {
        Self {
            enabled: false,
            value: Range::default(),
        }
    }
}

impl<T> Tweak<T> {
    const fn off(value: Range<T>) -> Self {
        Self {
            enabled: false,
            value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LutParams {
    pub enabled: bool,
    pub source: AssetSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BorderParams {
    pub enabled: bool,
    /// 边框宽度（像素）
    pub width: Range<u32>,
    /// `#rrggbb`
    pub color: String,
    /// 每个视频随机颜色（忽略 `color`）
    pub random_color: bool,
}

impl Default for BorderParams {
    fn default() -> Self {
        Self {
            enabled: false,
            width: Range::fixed(20),
            color: "#ffffff".into(),
            random_color: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdjustParams {
    pub enabled: bool,
    pub brightness: Tweak<f64>,
    pub contrast: Tweak<f64>,
    pub saturation: Tweak<f64>,
    pub sharpen: Tweak<f64>,
    pub denoise: Tweak<f64>,
    pub lut: LutParams,
    /// 入场（淡入）时长，秒
    pub fade_in: Tweak<f64>,
    /// 光晕镜头角，度
    pub vignette: Tweak<f64>,
    pub black_white: bool,
    /// 渐出（淡出）时长，秒
    pub fade_out: Tweak<f64>,
    pub border: BorderParams,
    /// 开幕动画时长，秒
    pub opening: Tweak<f64>,
    /// 马赛克方块边长，像素
    pub mosaic: Tweak<u32>,
    /// 蒙板倒置的不透明度
    pub mask_invert: Tweak<f64>,
}

impl Default for AdjustParams {
    fn default() -> Self {
        Self {
            enabled: false,
            brightness: Tweak::off(Range::new(0.0, 0.05)),
            contrast: Tweak::off(Range::new(1.0, 1.1)),
            saturation: Tweak::off(Range::new(1.0, 1.2)),
            sharpen: Tweak::off(Range::new(0.5, 1.0)),
            denoise: Tweak::off(Range::new(2.0, 4.0)),
            lut: LutParams::default(),
            fade_in: Tweak::off(Range::fixed(1.0)),
            vignette: Tweak::off(Range::fixed(30.0)),
            black_white: false,
            fade_out: Tweak::off(Range::fixed(1.0)),
            border: BorderParams::default(),
            opening: Tweak::off(Range::fixed(1.5)),
            mosaic: Tweak::off(Range::fixed(10)),
            mask_invert: Tweak::off(Range::fixed(0.3)),
        }
    }
}

/// 校验并取值；未启用时返回 None。`low_open` 为 true 时下限不含 `lo`。
fn sample(
    tweak: &Tweak<f64>,
    ctx: &mut BuildContext,
    bounds: (f64, f64, bool),
    decimals: u32,
    name: &str,
) -> Result<Option<f64>, String> {
    if !tweak.enabled {
        return Ok(None);
    }
    let (lo, hi) = tweak.value.ordered();
    let (min, max, low_open) = bounds;
    if lo < min || (low_open && lo <= min) || hi > max {
        return Err(format!(
            "{name}需在 {}～{} 之间",
            fmt_num(min),
            fmt_num(max)
        ));
    }
    Ok(Some(tweak.value.sample_rounded(&mut ctx.rng, decimals)))
}

fn sample_int(
    tweak: &Tweak<u32>,
    ctx: &mut BuildContext,
    (min, max): (u32, u32),
    name: &str,
) -> Result<Option<u32>, String> {
    if !tweak.enabled {
        return Ok(None);
    }
    let (lo, hi) = tweak.value.ordered();
    if lo < min || hi > max {
        return Err(format!("{name}需在 {min}～{max} 之间"));
    }
    Ok(Some(tweak.value.sample(&mut ctx.rng)))
}

/// 解析 `#rrggbb` / `rrggbb`。
fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

impl AdjustParams {
    fn apply_mask_invert(ctx: &mut BuildContext, opacity: f64) {
        let input = ctx.graph.take_video();
        let [a, b, c, out] = ["ad", "ad", "ad", "ad"].map(|p| ctx.graph.label(p));
        ctx.graph.chain(&[&input], "split=2", &[&a, &b]);
        ctx.graph.chain(&[&b], "vflip", &[&c]);
        ctx.graph.chain(
            &[&c, &a],
            &format!("blend=all_mode=normal:all_opacity={}", fmt_num(opacity)),
            &[&out],
        );
        ctx.graph.set_video(out);
    }

    fn apply_opening(ctx: &mut BuildContext, seconds: f64) {
        let half = ctx.stream.width.div_ceil(2);
        let d = fmt_num(seconds);
        let input = ctx.graph.take_video();
        let [left, right, mid, out] = ["ad", "ad", "ad", "ad"].map(|p| ctx.graph.label(p));
        ctx.graph.chain(
            &[],
            &format!(
                "color=c=black:s={half}x{}:r={}:d={d},split=2",
                ctx.stream.height,
                ctx.rate_text().unwrap_or_else(|| "25".into())
            ),
            &[&left, &right],
        );
        ctx.graph.chain(
            &[&input, &left],
            &format!("overlay=x=-w*t/{d}:y=0:eof_action=pass"),
            &[&mid],
        );
        ctx.graph.chain(
            &[&mid, &right],
            &format!("overlay=x=W-w+w*t/{d}:y=0:eof_action=pass"),
            &[&out],
        );
        ctx.graph.set_video(out);
    }
}

impl PipelineModule for AdjustParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        // 先校验并取值（固定顺序，保证同一种子结果相同），再写滤镜
        let brightness = sample(&self.brightness, ctx, (-1.0, 1.0, false), 2, "亮度")?;
        let contrast = sample(&self.contrast, ctx, (0.0, 3.0, false), 2, "对比度")?;
        let saturation = sample(&self.saturation, ctx, (0.0, 3.0, false), 2, "饱和度")?;
        let sharpen = sample(&self.sharpen, ctx, (0.0, 5.0, false), 2, "锐化强度")?;
        let denoise = sample(&self.denoise, ctx, (0.0, 30.0, false), 1, "降噪强度")?;
        let fade_in = sample(&self.fade_in, ctx, (0.0, 60.0, true), 2, "入场时长（秒）")?;
        let vignette = sample(&self.vignette, ctx, (1.0, 89.0, false), 1, "光晕角度")?;
        let fade_out = sample(&self.fade_out, ctx, (0.0, 60.0, true), 2, "渐出时长（秒）")?;
        let opening = sample(&self.opening, ctx, (0.0, 60.0, true), 2, "开幕时长（秒）")?;
        let mosaic = sample_int(&self.mosaic, ctx, (2, 256), "马赛克大小")?;
        let mask = sample(&self.mask_invert, ctx, (0.0, 1.0, true), 2, "蒙板不透明度")?;
        let border = if self.border.enabled {
            let (lo, hi) = self.border.width.ordered();
            if lo < 1 || hi > 500 {
                return Err("边框宽度需在 1～500 像素之间".into());
            }
            let width = self.border.width.sample(&mut ctx.rng);
            let color = if self.border.random_color {
                (ctx.rng.next_u64() & 0xff_ffff) as u32
            } else {
                parse_color(&self.border.color)
                    .ok_or_else(|| format!("边框颜色格式不正确：{}", self.border.color))?
            };
            Some((width, color))
        } else {
            None
        };
        let lut = if self.lut.enabled {
            if self.lut.source.is_empty() {
                return Err("请选择 LUT 文件".into());
            }
            let path = self
                .lut
                .source
                .pick(&LUT_EXTENSIONS, ctx.task_index, &mut ctx.rng)?;
            let path = path
                .to_str()
                .ok_or_else(|| format!("路径包含无法识别的字符：{}", path.display()))?
                .to_owned();
            Some(path)
        } else {
            None
        };

        // 一进一出的滤镜；遇到需要多输入的效果时先把已有的写进滤镜图
        let mut simple: Vec<String> = Vec::new();
        if let Some(path) = lut {
            simple.push(format!("lut3d=file={}", escape_filter_path(&path)));
        }
        let eq: Vec<String> = [
            ("brightness", brightness),
            ("contrast", contrast),
            ("saturation", saturation),
        ]
        .iter()
        .filter_map(|(k, v)| v.map(|v| format!("{k}={}", fmt_num(v))))
        .collect();
        if !eq.is_empty() {
            simple.push(format!("eq={}", eq.join(":")));
        }
        if let Some(v) = denoise {
            simple.push(format!("hqdn3d={}", fmt_num(v)));
        }
        if let Some(v) = sharpen {
            simple.push(format!("unsharp=lx=5:ly=5:la={}", fmt_num(v)));
        }
        if self.black_white {
            simple.push("hue=s=0".into());
        }
        let mut applied = !simple.is_empty();
        for filter in simple.drain(..) {
            ctx.graph.video(filter);
        }
        if let Some(opacity) = mask {
            Self::apply_mask_invert(ctx, opacity);
            applied = true;
        }

        if let Some(size) = mosaic {
            simple.push(format!("pixelize=w={size}:h={size}"));
        }
        if let Some(deg) = vignette {
            simple.push(format!("vignette=angle={}*PI/180", fmt_num(deg)));
        }
        if let Some((width, color)) = border {
            simple.push(format!(
                "drawbox=x=0:y=0:w=iw:h=ih:color=0x{color:06x}:t={width}"
            ));
        }
        if let Some(d) = fade_in {
            simple.push(format!("fade=t=in:st=0:d={}", fmt_num(d)));
        }
        if let Some(d) = fade_out {
            if ctx.stream.duration > 0.0 {
                let start = (ctx.stream.duration - d).max(0.0);
                simple.push(format!("fade=t=out:st={}:d={}", fmt_num(start), fmt_num(d)));
            } else {
                ctx.warn("视频时长未知，无法添加渐出效果");
            }
        }
        applied |= !simple.is_empty();
        for filter in simple {
            ctx.graph.video(filter);
        }
        if let Some(d) = opening {
            Self::apply_opening(ctx, d);
            applied = true;
        }
        if !applied {
            ctx.warn("没有启用任何调整项");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PickOrder, Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: AdjustParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.adjust = params;
        recipe
    }

    fn filter(params: AdjustParams, seed: u64) -> String {
        let built = build_with(&sample_info(), &recipe_with(params), seed).unwrap();
        built.arg_after("-filter_complex").unwrap_or("").to_owned()
    }

    fn on() -> AdjustParams {
        AdjustParams {
            enabled: true,
            ..AdjustParams::default()
        }
    }

    fn fixed(value: f64) -> Tweak<f64> {
        Tweak {
            enabled: true,
            value: Range::fixed(value),
        }
    }

    #[test]
    fn color_and_detail_filters() {
        let params = AdjustParams {
            brightness: fixed(0.1),
            saturation: fixed(1.5),
            sharpen: fixed(0.8),
            denoise: fixed(3.0),
            black_white: true,
            ..on()
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]eq=brightness=0.1:saturation=1.5,hqdn3d=3,unsharp=lx=5:ly=5:la=0.8,hue=s=0[v0]"
        );
    }

    #[test]
    fn effects_in_order() {
        let params = AdjustParams {
            contrast: fixed(1.2),
            fade_in: fixed(1.0),
            fade_out: fixed(2.0),
            vignette: fixed(30.0),
            mosaic: Tweak {
                enabled: true,
                value: Range::fixed(12),
            },
            border: BorderParams {
                enabled: true,
                width: Range::fixed(8),
                color: "#FF8000".into(),
                random_color: false,
            },
            ..on()
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]eq=contrast=1.2,pixelize=w=12:h=12,vignette=angle=30*PI/180,\
             drawbox=x=0:y=0:w=iw:h=ih:color=0xff8000:t=8,fade=t=in:st=0:d=1,fade=t=out:st=8:d=2[v0]"
        );
    }

    #[test]
    fn mask_invert_and_opening_use_complex_chains() {
        let params = AdjustParams {
            mask_invert: fixed(0.3),
            opening: fixed(1.5),
            ..on()
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]split=2[ad0][ad1];[ad1]vflip[ad2];\
             [ad2][ad0]blend=all_mode=normal:all_opacity=0.3[ad3];\
             color=c=black:s=640x720:r=25:d=1.5,split=2[ad4][ad5];\
             [ad3][ad4]overlay=x=-w*t/1.5:y=0:eof_action=pass[ad6];\
             [ad6][ad5]overlay=x=W-w+w*t/1.5:y=0:eof_action=pass[ad7]"
        );
        // 尺寸、帧率、时长不变
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(1, ID);
        let before = ctx.stream.clone();
        AdjustParams {
            mask_invert: fixed(0.3),
            opening: fixed(1.5),
            ..on()
        }
        .apply(&mut ctx)
        .unwrap();
        assert_eq!(ctx.stream, before);
    }

    #[test]
    fn ranges_are_reproducible_and_bounded() {
        let params = AdjustParams {
            brightness: Tweak {
                enabled: true,
                value: Range::new(-0.2, 0.2),
            },
            contrast: Tweak {
                enabled: true,
                value: Range::new(0.8, 1.2),
            },
            border: BorderParams {
                enabled: true,
                random_color: true,
                width: Range::new(4, 10),
                ..BorderParams::default()
            },
            ..on()
        };
        assert_eq!(filter(params.clone(), 42), filter(params.clone(), 42));
        let mut colors = std::collections::HashSet::new();
        for seed in 0..20 {
            let fc = filter(params.clone(), seed);
            let eq = fc
                .trim_start_matches("[0:v:0]eq=")
                .split(',')
                .next()
                .unwrap();
            let mut parts = eq.split(':');
            let b: f64 = parts.next().unwrap()["brightness=".len()..]
                .parse()
                .unwrap();
            let c: f64 = parts.next().unwrap()["contrast=".len()..].parse().unwrap();
            assert!((-0.2..=0.2).contains(&b), "{fc}");
            assert!((0.8..=1.2).contains(&c), "{fc}");
            let t: u32 = fc
                .split(":t=")
                .nth(1)
                .unwrap()
                .trim_end_matches("[v0]")
                .parse()
                .unwrap();
            assert!((4..=10).contains(&t), "{fc}");
            colors.insert(fc.split("color=").nth(1).unwrap()[..8].to_owned());
        }
        assert!(colors.len() > 1, "随机颜色应随种子变化");
    }

    #[test]
    fn lut_path_is_escaped_and_picked() {
        assert_eq!(
            escape_filter_path(r"C:\LUTs\a b,c's.cube"),
            r"C\\:/LUTs/a b\,c\\\'s.cube"
        );
        let dir = std::env::temp_dir().join(format!("cr-adjust-lut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("look.cube"), "LUT_3D_SIZE 2\n").unwrap();
        std::fs::write(dir.join("readme.txt"), "x").unwrap();
        let params = AdjustParams {
            lut: LutParams {
                enabled: true,
                source: AssetSource::Directory {
                    path: dir.to_string_lossy().into_owned(),
                    recursive: false,
                    order: PickOrder::Random,
                },
            },
            ..on()
        };
        let fc = filter(params, 1);
        let expected = escape_filter_path(&dir.join("look.cube").to_string_lossy());
        assert_eq!(fc, format!("[0:v:0]lut3d=file={expected}[v0]"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_params_are_rejected() {
        let cases = [
            AdjustParams {
                brightness: fixed(1.5),
                ..on()
            },
            AdjustParams {
                contrast: fixed(-1.0),
                ..on()
            },
            AdjustParams {
                fade_in: fixed(0.0),
                ..on()
            },
            AdjustParams {
                mosaic: Tweak {
                    enabled: true,
                    value: Range::fixed(1),
                },
                ..on()
            },
            AdjustParams {
                border: BorderParams {
                    enabled: true,
                    color: "red".into(),
                    ..BorderParams::default()
                },
                ..on()
            },
            AdjustParams {
                lut: LutParams {
                    enabled: true,
                    source: AssetSource::default(),
                },
                ..on()
            },
        ];
        for params in cases {
            assert!(
                build_with(&sample_info(), &recipe_with(params.clone()), 1).is_err(),
                "{params:?}"
            );
        }
    }

    #[test]
    fn nothing_enabled_warns_and_unknown_duration_skips_fade_out() {
        let built = build_with(&sample_info(), &recipe_with(on()), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.warnings.len(), 1);

        let mut info = sample_info();
        info.duration = 0.0;
        let params = AdjustParams {
            fade_out: fixed(1.0),
            ..on()
        };
        let built = build_with(&info, &recipe_with(params), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert!(built.warnings.iter().any(|w| w.contains("渐出")));
    }
}
