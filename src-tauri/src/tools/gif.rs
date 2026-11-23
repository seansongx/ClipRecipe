//! `tool.gif` 生成 GIF：队列中每个视频转成一个同名 `.gif`（无限循环）。
//!
//! 参数：
//! - `fps` 帧率（默认 10，1～50；GIF 的帧间隔以 1/100 秒为单位，50 以上的帧率浏览器会降速播放）；
//! - `width` 宽度（默认 480 像素，16～4096，高度按比例；0 表示保持源宽度），lanczos 缩放；
//! - `start` 开始时间（秒，默认 0）、`duration` 时长（秒，默认 0 = 到结尾）；
//! - `highQuality` 高质量模式（默认关闭）：
//!   - 关闭（快速）：一步完成，统一用固定的 256 色调色板（`format=bgr8`，RGB 3-3-2，无抖动），渐变会有色带；
//!   - 开启：两步——先 `palettegen=stats_mode=diff` 为这段画面生成最优调色板（临时 png），再
//!     `paletteuse=dither=sierra2_4a:diff_mode=rectangle` 用它量化并抖动。颜色接近原片，但更慢、文件更大。
//!
//! 一进一出，不用视频编码器（GIF 编码），因此不受“码率”设置影响。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::path_arg;
use crate::pipeline::fmt_num;
use crate::tools::media::secs;
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.gif";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GifParams {
    /// 帧率
    pub fps: f64,
    /// 宽度（像素），0 表示保持源宽度
    pub width: u32,
    /// 开始时间（秒）
    pub start: f64,
    /// 时长（秒），0 表示到结尾
    pub duration: f64,
    /// 高质量（两步调色板）
    pub high_quality: bool,
}

impl Default for GifParams {
    fn default() -> Self {
        Self {
            fps: 10.0,
            width: 480,
            start: 0.0,
            duration: 0.0,
            high_quality: false,
        }
    }
}

/// 一个视频的转换计划。
#[derive(Debug, Clone, PartialEq)]
pub struct GifPlan {
    /// 输入前的 `-ss` / `-t`
    pub seek: Vec<String>,
    /// 缩放与帧率滤镜（不含调色板部分）
    pub base: String,
    /// 输出时长（秒）
    pub duration: f64,
}

impl GifParams {
    fn validate(&self) -> Result<(), String> {
        if !(self.fps.is_finite() && (1.0..=50.0).contains(&self.fps)) {
            return Err("GIF 帧率必须在 1～50 之间".into());
        }
        if self.width != 0 && !(16..=4096).contains(&self.width) {
            return Err("GIF 宽度必须在 16～4096 像素之间（0 表示保持源宽度）".into());
        }
        if !(self.start.is_finite() && self.start >= 0.0) {
            return Err("开始时间不能为负数".into());
        }
        if !(self.duration.is_finite() && self.duration >= 0.0) {
            return Err("时长不能为负数".into());
        }
        Ok(())
    }

    /// 由源视频时长得到转换计划。
    pub fn plan(&self, source_duration: f64) -> Result<GifPlan, String> {
        self.validate()?;
        if !(source_duration.is_finite() && source_duration > 0.0) {
            return Err("无法读取视频时长".into());
        }
        if self.start >= source_duration {
            return Err(format!(
                "开始时间（{} 秒）超过了视频时长（{} 秒）",
                fmt_num(self.start),
                fmt_num(source_duration)
            ));
        }
        let left = source_duration - self.start;
        let duration = if self.duration > 0.0 {
            self.duration.min(left)
        } else {
            left
        };
        let mut seek = Vec::new();
        if self.start > 0.0 {
            seek.extend(["-ss".to_owned(), secs(self.start)]);
        }
        if self.duration > 0.0 {
            seek.extend(["-t".to_owned(), secs(duration)]);
        }
        let mut base = format!("fps={}", fmt_num(self.fps));
        if self.width > 0 {
            base.push_str(&format!(",scale={}:-1:flags=lanczos", self.width));
        }
        Ok(GifPlan {
            seek,
            base,
            duration,
        })
    }
}

/// 快速模式：一步，固定调色板。
pub fn fast_args(plan: &GifPlan, input: &str, output: &str) -> Vec<String> {
    let mut args = plan.seek.clone();
    args.extend([
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-vf".into(),
        format!("{},format=bgr8", plan.base),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-loop".into(),
        "0".into(),
        output.into(),
    ]);
    args
}

/// 高质量第 1 步：生成调色板图片。
pub fn palette_args(plan: &GifPlan, input: &str, palette: &str) -> Vec<String> {
    let mut args = plan.seek.clone();
    args.extend([
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-vf".into(),
        format!("{},palettegen=stats_mode=diff", plan.base),
        "-frames:v".into(),
        "1".into(),
        "-update".into(),
        "1".into(),
        palette.into(),
    ]);
    args
}

/// 高质量第 2 步：用调色板量化并抖动。
pub fn paletteuse_args(plan: &GifPlan, input: &str, palette: &str, output: &str) -> Vec<String> {
    let mut args = plan.seek.clone();
    args.extend([
        "-i".into(),
        input.into(),
        "-i".into(),
        palette.into(),
        "-filter_complex".into(),
        format!(
            "[0:V:0]{}[x];[x][1:v]paletteuse=dither=sierra2_4a:diff_mode=rectangle[v]",
            plan.base
        ),
        "-map".into(),
        "[v]".into(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-loop".into(),
        "0".into(),
        output.into(),
    ]);
    args
}

impl Tool for GifParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        if info.video.is_none() {
            return Err("文件中没有视频流".into());
        }
        let plan = self.plan(info.duration)?;
        let input = path_arg(ctx.input_path(0)?)?;
        if !self.high_quality {
            let output = path_arg(&ctx.output(&OutputSpec::file("gif"))?)?;
            return ctx.run_ffmpeg("生成 GIF", fast_args(&plan, &input, &output), plan.duration);
        }
        ctx.set_steps(&[1.0, 2.0]);
        let palette = path_arg(&ctx.temp_file("palette.png")?)?;
        ctx.run_ffmpeg(
            "生成调色板",
            palette_args(&plan, &input, &palette),
            plan.duration,
        )?;
        let output = path_arg(&ctx.output(&OutputSpec::file("gif"))?)?;
        ctx.run_ffmpeg(
            "生成 GIF（高质量）",
            paletteuse_args(&plan, &input, &palette, &output),
            plan.duration,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_defaults_and_clipping() {
        let plan = GifParams::default().plan(10.0).unwrap();
        assert_eq!(
            plan,
            GifPlan {
                seek: vec![],
                base: "fps=10,scale=480:-1:flags=lanczos".into(),
                duration: 10.0
            }
        );
        let part = GifParams {
            start: 2.5,
            duration: 20.0,
            width: 0,
            fps: 12.5,
            ..GifParams::default()
        }
        .plan(10.0)
        .unwrap();
        assert_eq!(part.seek, ["-ss", "2.5", "-t", "7.5"]);
        assert_eq!(part.base, "fps=12.5");
        assert_eq!(part.duration, 7.5);
    }

    #[test]
    fn invalid_params() {
        for p in [
            GifParams {
                fps: 0.5,
                ..GifParams::default()
            },
            GifParams {
                fps: 60.0,
                ..GifParams::default()
            },
            GifParams {
                width: 8,
                ..GifParams::default()
            },
            GifParams {
                start: -1.0,
                ..GifParams::default()
            },
            GifParams {
                duration: f64::NAN,
                ..GifParams::default()
            },
            GifParams {
                start: 10.0,
                ..GifParams::default()
            },
        ] {
            assert!(p.plan(10.0).is_err(), "{p:?}");
        }
        assert!(GifParams::default().plan(0.0).is_err());
    }

    #[test]
    fn command_lines() {
        let plan = GifParams {
            start: 1.0,
            duration: 2.0,
            ..GifParams::default()
        }
        .plan(10.0)
        .unwrap();
        assert_eq!(
            fast_args(&plan, "in.mp4", "o.gif").join(" "),
            "-ss 1 -t 2 -i in.mp4 -map 0:V:0 -vf fps=10,scale=480:-1:flags=lanczos,format=bgr8 \
             -an -sn -dn -loop 0 o.gif"
        );
        assert_eq!(
            palette_args(&plan, "in.mp4", "p.png").join(" "),
            "-ss 1 -t 2 -i in.mp4 -map 0:V:0 -vf fps=10,scale=480:-1:flags=lanczos,palettegen=stats_mode=diff \
             -frames:v 1 -update 1 p.png"
        );
        assert_eq!(
            paletteuse_args(&plan, "in.mp4", "p.png", "o.gif").join(" "),
            "-ss 1 -t 2 -i in.mp4 -i p.png -filter_complex \
             [0:V:0]fps=10,scale=480:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=sierra2_4a:diff_mode=rectangle[v] \
             -map [v] -an -sn -dn -loop 0 o.gif"
        );
    }
}
