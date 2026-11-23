//! `tool.insert-frame` 插帧：在一个或多个位置把一张图片全屏显示指定的帧数。一进一出，重编码。
//!
//! - **图片** `image`：素材来源（单个文件，或目录中随机 / 依次取一张；png / jpg / webp / bmp / gif，gif 只取画面，
//!   不处理透明——透明部分按图片自身的颜色显示）。
//! - **位置** `positions`：与 `tool.freeze-play` 相同的写法（秒、分:秒、百分比，可写多个）；按帧对齐到该时间点
//!   （或之后）的第一帧；超出时长的位置忽略并提示。
//! - **帧数** `frames`（1～1000）：图片显示多少帧。
//! - **适配** `fit`：`fit` 等比缩放完整显示，空白处填 `background` 颜色；`fill` 等比放大铺满、居中裁去多余部分；
//!   `stretch` 拉伸到画面尺寸。
//! - **方式** `mode`：
//!   - `overlay` 覆盖（默认）：从该位置起的 `frames` 帧被图片盖住，时长不变，音频原样保留；
//!   - `insert` 插入：在该位置之前插入 `frames` 帧图片，原画面顺延，视频变长 `位置数 × 帧数`，
//!     音频在同一处插入同样长的静音（音画同步）。实现：`setpts` 让出空档（[`gap_video_filter`]），
//!     空档期间叠加图片；音频用 [`gap_audio_filter`]。
//!
//! 奇数尺寸裁成偶数。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::model::AssetSource;
use crate::pipeline::modules::text::escape_value;
use crate::tools::common::filter_job::{
    check_duration, color_hex, display_size, even_crop, gap_audio_filter, gap_video_filter,
    image_input_args, run_job, source_audio, AudioOut, FilterJob, Rate, IMAGE_EXTENSIONS,
};
use crate::tools::common::positions::{parse_positions, position_frames, windows_expr};
use crate::tools::freeze_anim::MAX_HOLDS;
use crate::tools::{Tool, ToolContext, ToolError};

pub const ID: &str = "tool.insert-frame";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InsertMode {
    /// 覆盖原画面，时长不变
    #[default]
    Overlay,
    /// 插入新帧，时长变长
    Insert,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageFit {
    /// 等比完整显示，空白处填背景色
    #[default]
    Fit,
    /// 等比铺满，裁去多余部分
    Fill,
    /// 拉伸
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InsertFrameParams {
    pub image: AssetSource,
    /// 位置（秒、分:秒、百分比，逗号 / 空格分隔）
    pub positions: String,
    /// 显示帧数
    pub frames: u32,
    pub fit: ImageFit,
    /// 等比完整显示时的背景色（#rrggbb）
    pub background: String,
    pub mode: InsertMode,
}

impl Default for InsertFrameParams {
    fn default() -> Self {
        Self {
            image: AssetSource::default(),
            positions: "50%".into(),
            frames: 1,
            fit: ImageFit::Fit,
            background: "#000000".into(),
            mode: InsertMode::Overlay,
        }
    }
}

/// 把图片缩放成 `w × h` 的全屏画面。
pub fn fit_filter(fit: ImageFit, (w, h): (u32, u32), background: &str) -> String {
    match fit {
        ImageFit::Fit => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={background}"
        ),
        ImageFit::Fill => {
            format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
        }
        ImageFit::Stretch => format!("scale={w}:{h}"),
    }
}

/// 生成命令；`image` 为已选定的图片路径。
pub fn build(
    params: &InsertFrameParams,
    info: &MediaInfo,
    input: &str,
    image: &str,
) -> Result<FilterJob, String> {
    if !(1..=1000).contains(&params.frames) {
        return Err("显示帧数必须在 1～1000 之间".into());
    }
    let background = color_hex(&params.background, "背景色")?;
    check_duration(info.duration)?;
    let (w, h) = display_size(info)?;
    let rate = Rate::of(info)?;
    let (times, mut warnings) = parse_positions(&params.positions, info.duration)?;
    let frames = position_frames(&times, &rate, info, &mut warnings);
    if frames.len() > MAX_HOLDS {
        return Err(format!("插入位置太多（最多 {MAX_HOLDS} 个）"));
    }
    let n = params.frames;
    let span = rate.seconds(f64::from(n));

    let mut job = FilterJob::new(input);
    job.warnings = warnings;
    let index = job.add_input(image_input_args(image), image);
    let picture = format!(
        "[{index}:v:0]{},setsar=1,format=yuv420p[img]",
        fit_filter(params.fit, (w & !1, h & !1), &background)
    );
    let mut audio_graph = None;
    let (base, windows) = match params.mode {
        InsertMode::Overlay => {
            let total = rate.total_frames(info);
            let windows = frames
                .iter()
                .map(|f| {
                    let end = (f + i64::from(n)).min(total);
                    (rate.seconds(*f as f64), rate.seconds(end as f64))
                })
                .collect::<Vec<_>>();
            job.audio = source_audio(info);
            job.duration = info.duration;
            (format!("[0:v:0]null{}[base]", even_crop(w, h)), windows)
        }
        InsertMode::Insert => {
            let after: Vec<i64> = frames.iter().map(|f| f - 1).collect();
            let windows = frames
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    let start = rate.seconds(*f as f64) + span * k as f64;
                    (start, start + span)
                })
                .collect::<Vec<_>>();
            job.duration = info.duration + span * frames.len() as f64;
            if let Some(audio) = info.audio.first() {
                let cuts: Vec<f64> = frames.iter().map(|f| rate.seconds(*f as f64)).collect();
                job.audio = AudioOut::Filtered {
                    channels: audio.channels,
                };
                audio_graph = Some(gap_audio_filter("[0:a:0]", &cuts, span, audio, "[a]"));
            }
            (
                format!(
                    "[0:v:0]{}{}[base]",
                    gap_video_filter(&after, n, &rate),
                    even_crop(w, h)
                ),
                windows,
            )
        }
    };
    let overlay = format!(
        "[base][img]overlay=0:0:shortest=1:enable={}[v]",
        escape_value(&windows_expr(&windows, &rate))
    );
    job.graph = [Some(base), Some(picture), Some(overlay), audio_graph]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(";");
    Ok(job)
}

impl Tool for InsertFrameParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        if self.image.is_empty() {
            return Err("请选择要插入的图片".into());
        }
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let index = ctx.number().saturating_sub(1);
        let image = path_arg(&self.image.pick(IMAGE_EXTENSIONS, index, &mut ctx.rng)?)?;
        let job = build(self, &info, &input, &image)?;
        run_job(ctx, "插帧", &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    fn params(mode: InsertMode) -> InsertFrameParams {
        InsertFrameParams {
            positions: "2, 50%".into(),
            frames: 3,
            mode,
            ..InsertFrameParams::default()
        }
    }

    #[test]
    fn fit_modes() {
        assert_eq!(
            fit_filter(ImageFit::Fit, (1280, 720), "0x102030"),
            "scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=0x102030"
        );
        assert_eq!(
            fit_filter(ImageFit::Fill, (720, 1280), "0x000000"),
            "scale=720:1280:force_original_aspect_ratio=increase,crop=720:1280"
        );
        assert_eq!(
            fit_filter(ImageFit::Stretch, (100, 50), "0x000000"),
            "scale=100:50"
        );
    }

    #[test]
    fn overlay_mode_keeps_duration() {
        let job = build(
            &params(InsertMode::Overlay),
            &sample_info(),
            "in.mp4",
            "a.png",
        )
        .unwrap();
        assert_eq!(job.inputs, ["-i", "in.mp4", "-loop", "1", "-i", "a.png"]);
        // 第 50～52 帧、第 125～127 帧
        assert_eq!(
            job.graph,
            "[0:v:0]null[base];\
             [1:v:0]scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:\
             color=0x000000,setsar=1,format=yuv420p[img];\
             [base][img]overlay=0:0:shortest=1:enable=gt(t\\,1.98)*lt(t\\,2.1)+gt(t\\,4.98)*lt(t\\,5.1)[v]"
        );
        assert_eq!(job.duration, 10.0);
        assert_eq!(job.audio, AudioOut::Source { channels: 2 });
    }

    #[test]
    fn insert_mode_adds_frames_and_silence() {
        let mut info = sample_info();
        info.video.as_mut().unwrap().height = 721;
        let p = InsertFrameParams {
            positions: "0, 2".into(),
            fit: ImageFit::Stretch,
            ..params(InsertMode::Insert)
        };
        let job = build(&p, &info, "in.mp4", "a.gif").unwrap();
        assert_eq!(job.inputs[2..4], ["-stream_loop", "-1"]);
        assert_eq!(
            job.graph,
            "[0:v:0]fps=25,setpts=PTS+(gt(N\\,-1)+gt(N\\,49))*3*1/25/TB,fps=25:start_time=0,\
             crop=1280:720:0:0[base];\
             [1:v:0]scale=1280:720,setsar=1,format=yuv420p[img];\
             [base][img]overlay=0:0:shortest=1:enable=gt(t\\,-0.02)*lt(t\\,0.1)+gt(t\\,2.1)*lt(t\\,2.22)[v];\
             [0:a:0]asplit=2[ga0][ga1];\
             [ga0]atrim=start=0:end=2,asetpts=PTS-STARTPTS[gs0];\
             [ga1]atrim=start=2,asetpts=PTS-STARTPTS[gs1];\
             anullsrc=r=48000:cl=stereo,atrim=duration=0.12[gz0];\
             anullsrc=r=48000:cl=stereo,atrim=duration=0.12[gz1];\
             [gz0][gs0][gz1][gs1]concat=n=4:v=0:a=1[a]"
        );
        assert!((job.duration - 10.24).abs() < 1e-9);
    }

    #[test]
    fn invalid_params() {
        for p in [
            InsertFrameParams {
                frames: 0,
                ..InsertFrameParams::default()
            },
            InsertFrameParams {
                background: "black".into(),
                ..InsertFrameParams::default()
            },
            InsertFrameParams {
                positions: "200%".into(),
                ..InsertFrameParams::default()
            },
        ] {
            assert!(
                build(&p, &sample_info(), "in.mp4", "a.png").is_err(),
                "{p:?}"
            );
        }
    }
}
