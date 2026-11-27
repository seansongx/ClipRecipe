//! `pipe.zoom` 动态缩放：画面以余弦曲线在两个缩放倍数之间平滑地往复放大、缩小，输出尺寸不变。
//!
//! 参数与取舍（docs/specs/pipeline.md 未规定的部分）：
//! - `scale`：往复的**区间**（不是随机取值）：从 `min` 倍开始，半个周期后到 `max` 倍，再回到 `min`；
//!   两端相等即固定缩放。允许 0.5～3 倍；小于 1 时画面缩小，四周补黑。
//! - `period`：一次“放大 + 缩小”的周期（秒），每个视频在范围内随机取值（保留 2 位小数），0.5～60 秒。
//! - 实现：`zoompan`（d=1，逐帧一进一出，帧率保持当前帧率），缩放中心为画面中心。
//!   最小倍数小于 1 时先用 `pad` 把画布按 1/min 扩大（补黑），再在扩大的画布上做 ≥1 的 zoompan。
//! - 区间为 1～1 时画面不会变化，只给出警告、不加滤镜。
//! - 输出尺寸与输入相同；输入宽高为奇数时向下取偶数（zoompan 本来就重新渲染每一帧）。

use serde::{Deserialize, Serialize};

use crate::model::Range;
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.zoom";

pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;
pub const MIN_PERIOD: f64 = 0.5;
pub const MAX_PERIOD: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ZoomParams {
    pub enabled: bool,
    /// 缩放倍数的往复区间
    pub scale: Range<f64>,
    /// 往复周期（秒），每个视频随机取值
    pub period: Range<f64>,
}

impl Default for ZoomParams {
    fn default() -> Self {
        Self {
            enabled: false,
            scale: Range::new(1.0, 1.2),
            period: Range::fixed(4.0),
        }
    }
}

/// 向上取偶数（先去掉浮点误差）。
fn even_up(x: f64) -> u32 {
    let n = (x - 1e-6).ceil().max(2.0) as u32;
    n + n % 2
}

impl PipelineModule for ZoomParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        let (lo, hi) = self.scale.ordered();
        if lo < MIN_SCALE || hi > MAX_SCALE {
            return Err(format!("缩放倍数需在 {MIN_SCALE}～{MAX_SCALE} 之间"));
        }
        let (plo, phi) = self.period.ordered();
        if plo < MIN_PERIOD || phi > MAX_PERIOD {
            return Err(format!("周期需在 {MIN_PERIOD}～{MAX_PERIOD} 秒之间"));
        }
        let period = self.period.sample_rounded(&mut ctx.rng, 2);
        if lo == 1.0 && hi == 1.0 {
            ctx.warn("缩放区间为 1～1，画面不会变化");
            return Ok(());
        }
        if ctx.stream.fps <= 0.0 {
            ctx.warn("源视频帧率未知，按 25 fps 计算缩放周期");
            ctx.stream.fps = 25.0;
        }
        // zoompan 按帧序号重排时间：前面若输出了可变帧率，先恢复恒定帧率
        ctx.ensure_cfr();

        let (w, h) = (ctx.stream.width, ctx.stream.height);
        // zoompan 只能放大（≥1），缩小时先把画布按 1/lo 扩大
        let base = lo.min(1.0);
        if base < 1.0 {
            let pw = even_up(f64::from(w) / base);
            let ph = even_up(f64::from(h) / base);
            ctx.graph.video(format!(
                "pad={pw}:{ph}:{}:{}:black",
                (pw - w) / 2,
                (ph - h) / 2
            ));
        }
        let mid = (lo + hi) / 2.0 / base;
        let amp = (hi - lo) / 2.0 / base;
        let zoom = if amp == 0.0 {
            fmt_num(mid)
        } else {
            let frames = ctx.stream.fps * period;
            // 预览定位时帧序号从定位起点数起：补上起点之前的帧数，缩放相位与从头解码一致
            let index = match ctx.preview_seek() {
                Some(seek) => format!("(in+{seek}*{})", fmt_num(ctx.stream.fps)),
                None => "in".to_owned(),
            };
            format!(
                "{}-{}*cos(2*PI*{index}/{})",
                fmt_num(mid),
                fmt_num(amp),
                fmt_num(frames)
            )
        };
        // zoompan 本来就重新渲染：奇数尺寸顺便取偶数，便于 yuv420p 编码
        let (ow, oh) = ((w - w % 2).max(2), (h - h % 2).max(2));
        ctx.graph.video(format!(
            "zoompan=z={zoom}:x=iw/2-iw/zoom/2:y=ih/2-ih/zoom/2:d=1:s={ow}x{oh}:fps={}",
            ctx.rate_text().unwrap_or_else(|| "25".into())
        ));
        // zoompan 输出的时间戳从 0 开始：预览定位时加回起点
        if let Some(seek) = ctx.preview_seek() {
            ctx.graph.video(format!("setpts=PTS+{seek}/TB"));
        }
        ctx.stream.width = ow;
        ctx.stream.height = oh;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Recipe, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    fn recipe_with(params: ZoomParams) -> Recipe {
        let mut recipe = Recipe::disabled();
        recipe.modules.zoom = params;
        recipe
    }

    fn filter(params: ZoomParams, seed: u64) -> String {
        let built = build_with(&sample_info(), &recipe_with(params), seed).unwrap();
        built.arg_after("-filter_complex").unwrap_or("").to_owned()
    }

    fn on() -> ZoomParams {
        ZoomParams {
            enabled: true,
            ..ZoomParams::default()
        }
    }

    #[test]
    fn default_zooms_between_1_and_1_2() {
        assert_eq!(
            filter(on(), 1),
            "[0:v:0]zoompan=z=1.1-0.1*cos(2*PI*in/100):x=iw/2-iw/zoom/2:y=ih/2-ih/zoom/2:d=1:s=1280x720:fps=25[v0]"
        );
        // 尺寸、帧率、时长都不变
        let info = sample_info();
        let settings = Settings::default();
        let mut ctx = BuildContext::new(&info, &settings, 0);
        ctx.rng = SeededRng::derive(1, ID);
        let before = ctx.stream.clone();
        on().apply(&mut ctx).unwrap();
        assert_eq!(ctx.stream, before);
    }

    #[test]
    fn zoom_out_pads_canvas_first() {
        let params = ZoomParams {
            scale: Range::new(0.8, 1.2),
            period: Range::fixed(2.0),
            ..on()
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]pad=1600:900:160:90:black,\
             zoompan=z=1.25-0.25*cos(2*PI*in/50):x=iw/2-iw/zoom/2:y=ih/2-ih/zoom/2:d=1:s=1280x720:fps=25[v0]"
        );
    }

    #[test]
    fn fixed_scale_is_constant_zoom() {
        let params = ZoomParams {
            scale: Range::fixed(1.5),
            ..on()
        };
        assert_eq!(
            filter(params, 1),
            "[0:v:0]zoompan=z=1.5:x=iw/2-iw/zoom/2:y=ih/2-ih/zoom/2:d=1:s=1280x720:fps=25[v0]"
        );
    }

    #[test]
    fn period_range_is_reproducible_and_bounded() {
        let params = ZoomParams {
            period: Range::new(2.0, 6.0),
            ..on()
        };
        assert_eq!(filter(params.clone(), 42), filter(params.clone(), 42));
        for seed in 0..20 {
            let fc = filter(params.clone(), seed);
            let frames: f64 = fc
                .split("in/")
                .nth(1)
                .unwrap()
                .split(')')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            // 25 fps × 2～6 秒
            assert!((50.0..=150.0).contains(&frames), "{fc}");
        }
    }

    #[test]
    fn identity_warns_and_invalid_rejected() {
        let params = ZoomParams {
            scale: Range::fixed(1.0),
            ..on()
        };
        let built = build_with(&sample_info(), &recipe_with(params), 1).unwrap();
        assert_eq!(built.arg_after("-filter_complex"), None);
        assert_eq!(built.warnings.len(), 1);

        for bad in [
            ZoomParams {
                scale: Range::new(0.1, 1.0),
                ..on()
            },
            ZoomParams {
                scale: Range::new(1.0, 5.0),
                ..on()
            },
            ZoomParams {
                period: Range::new(0.0, 1.0),
                ..on()
            },
        ] {
            assert!(build_with(&sample_info(), &recipe_with(bad), 1).is_err());
        }
    }
}
