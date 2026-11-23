//! `tool.camera-move` 运镜效果：从画面中裁取一个较小的窗口并平滑地往复移动（或缓慢推拉），
//! 再放大回源分辨率输出，模拟镜头运动。一进一出，重编码，时长与音频不变（音频容器装得下时直接复制）。
//!
//! - **窗口大小** `zoom`（1.05～4）：窗口宽高 = 源宽高 ÷ `zoom`（取偶数），即放大倍数。
//! - **运动方式** `direction`：
//!   - `horizontal` 左右平移、`vertical` 上下平移、`both` 斜向（左右与上下同时）平移：
//!     窗口从正中开始，按正弦曲线移动 —— 1/4 周期到一侧边缘，3/4 周期到另一侧，一个周期回到正中；
//!     `crop`（每帧计算位置）+ `scale` 回源尺寸；
//!   - `zoom` 缓慢推拉：画面居中，放大倍数从 1（全画面）按余弦曲线平滑变到 `zoom`（半个周期）再回到 1；`zoompan`。
//! - **周期**：`autoPeriod`（默认）时由时长决定 —— 每个周期约 8 秒，且时长内正好完成整数个周期
//!   （`周期数 = max(1, round(时长 / 8))`），结尾回到起点；否则用 `periodSeconds`（0.5～3600 秒）。
//! - 输出尺寸与源相同（奇数宽高减 1 取偶，与批量处理一致）。
//!
//! 预览：工具对话框目前没有“预览”入口（框架只提供参数表单），无法在处理前预览运镜效果。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::pipeline::modules::text::escape_value;
use crate::tools::common::filter_job::{
    check_duration, display_size, float_text, run_job, source_audio, FilterJob, Rate,
};
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.camera-move";

/// 自动周期时每个周期的目标长度（秒）。
pub const AUTO_PERIOD_SECONDS: f64 = 8.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraDirection {
    /// 左右平移
    #[default]
    Horizontal,
    /// 上下平移
    Vertical,
    /// 斜向平移
    Both,
    /// 缓慢推拉（放大缩小）
    Zoom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraMoveParams {
    pub direction: CameraDirection,
    /// 放大倍数（窗口 = 源尺寸 ÷ zoom）
    pub zoom: f64,
    /// 周期由时长自动决定
    pub auto_period: bool,
    /// 手动周期（秒）
    pub period_seconds: f64,
}

impl Default for CameraMoveParams {
    fn default() -> Self {
        Self {
            direction: CameraDirection::Horizontal,
            zoom: 1.25,
            auto_period: true,
            period_seconds: 6.0,
        }
    }
}

impl CameraMoveParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.zoom.is_finite() && (1.05..=4.0).contains(&self.zoom)) {
            return Err("放大倍数必须在 1.05～4 之间".into());
        }
        if !self.auto_period
            && !(self.period_seconds.is_finite() && (0.5..=3600.0).contains(&self.period_seconds))
        {
            return Err("运动周期必须在 0.5～3600 秒之间".into());
        }
        Ok(())
    }

    /// 一个往复周期的秒数。
    pub fn period(&self, duration: f64) -> f64 {
        if self.auto_period {
            let cycles = (duration / AUTO_PERIOD_SECONDS).round().max(1.0);
            duration / cycles
        } else {
            self.period_seconds
        }
    }
}

/// 运镜滤镜（输入为源画面，输出为 `out_w × out_h`）。
pub fn camera_filter(
    params: &CameraMoveParams,
    (w, h): (u32, u32),
    period: f64,
    rate: &Rate,
) -> String {
    let (ow, oh) = (w & !1, h & !1);
    let p = float_text(period);
    let z = float_text(params.zoom);
    if params.direction == CameraDirection::Zoom {
        let zoom = format!("1+({z}-1)*(1-cos(2*PI*it/{p}))/2");
        return format!(
            "zoompan=z={}:x={}:y={}:d=1:s={ow}x{oh}:fps={},setsar=1",
            escape_value(&zoom),
            escape_value("(iw-iw/zoom)/2"),
            escape_value("(ih-ih/zoom)/2"),
            rate.text
        );
    }
    let cw = ((f64::from(w) / params.zoom) as u32 & !1).max(2);
    let ch = ((f64::from(h) / params.zoom) as u32 & !1).max(2);
    let moving = |axis: &str, size: &str| format!("({axis}-{size})/2*(1+sin(2*PI*t/{p}))");
    let centred = |axis: &str, size: &str| format!("({axis}-{size})/2");
    let (x, y) = match params.direction {
        CameraDirection::Horizontal => (moving("iw", "ow"), centred("ih", "oh")),
        CameraDirection::Vertical => (centred("iw", "ow"), moving("ih", "oh")),
        _ => (moving("iw", "ow"), moving("ih", "oh")),
    };
    format!(
        "crop={cw}:{ch}:x={}:y={},scale={ow}:{oh},setsar=1",
        escape_value(&x),
        escape_value(&y)
    )
}

pub fn build(
    params: &CameraMoveParams,
    info: &MediaInfo,
    input: &str,
) -> Result<FilterJob, String> {
    params.validate()?;
    check_duration(info.duration)?;
    let size = display_size(info)?;
    let rate = Rate::of(info)?;
    let period = params.period(info.duration);
    let mut job = FilterJob::new(input);
    job.graph = format!("[0:v:0]{}[v]", camera_filter(params, size, period, &rate));
    job.audio = source_audio(info);
    job.duration = info.duration;
    Ok(job)
}

impl Tool for CameraMoveParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let job = build(self, &info, &input)?;
        run_job(ctx, "运镜效果", &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    fn with(direction: CameraDirection) -> CameraMoveParams {
        CameraMoveParams {
            direction,
            ..CameraMoveParams::default()
        }
    }

    #[test]
    fn auto_period_fits_whole_cycles() {
        let p = CameraMoveParams::default();
        assert_eq!(p.period(10.0), 10.0);
        assert_eq!(p.period(3.0), 3.0);
        assert_eq!(p.period(60.0), 60.0 / 8.0);
        assert_eq!(p.period(24.0), 8.0);
        assert_eq!(p.period(28.0), 7.0, "3.5 → 4 个周期");
        let fixed = CameraMoveParams {
            auto_period: false,
            period_seconds: 4.0,
            ..p
        };
        assert_eq!(fixed.period(60.0), 4.0);
    }

    #[test]
    fn horizontal_pan_command() {
        let job = build(&with(CameraDirection::Horizontal), &sample_info(), "in.mp4").unwrap();
        // 1280x720 ÷ 1.25 = 1024x576；10 秒 → 周期 10 秒
        assert_eq!(
            job.graph,
            "[0:v:0]crop=1024:576:x=(iw-ow)/2*(1+sin(2*PI*t/10)):y=(ih-oh)/2,scale=1280:720,setsar=1[v]"
        );
        assert_eq!(job.duration, 10.0);
    }

    #[test]
    fn vertical_both_and_zoom() {
        let mut info = sample_info();
        info.video.as_mut().unwrap().width = 721;
        info.video.as_mut().unwrap().height = 1281;
        let fixed = |d| CameraMoveParams {
            auto_period: false,
            period_seconds: 4.0,
            zoom: 1.5,
            ..with(d)
        };
        let job = build(&fixed(CameraDirection::Vertical), &info, "in.mp4").unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]crop=480:854:x=(iw-ow)/2:y=(ih-oh)/2*(1+sin(2*PI*t/4)),scale=720:1280,setsar=1[v]"
        );
        let job = build(&fixed(CameraDirection::Both), &info, "in.mp4").unwrap();
        assert!(job
            .graph
            .contains("x=(iw-ow)/2*(1+sin(2*PI*t/4)):y=(ih-oh)/2*(1+sin(2*PI*t/4))"));
        let job = build(&fixed(CameraDirection::Zoom), &info, "in.mp4").unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]zoompan=z=1+(1.5-1)*(1-cos(2*PI*it/4))/2:x=(iw-iw/zoom)/2:y=(ih-ih/zoom)/2:\
             d=1:s=720x1280:fps=25,setsar=1[v]"
        );
    }

    #[test]
    fn invalid_params() {
        for p in [
            CameraMoveParams {
                zoom: 1.0,
                ..CameraMoveParams::default()
            },
            CameraMoveParams {
                zoom: f64::NAN,
                ..CameraMoveParams::default()
            },
            CameraMoveParams {
                auto_period: false,
                period_seconds: 0.1,
                ..CameraMoveParams::default()
            },
        ] {
            assert!(build(&p, &sample_info(), "in.mp4").is_err(), "{p:?}");
        }
        let mut no_video = sample_info();
        no_video.video = None;
        assert!(build(&CameraMoveParams::default(), &no_video, "in.mp4").is_err());
    }
}
