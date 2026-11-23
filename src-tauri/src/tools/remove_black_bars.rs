//! `tool.remove-black-bars` 黑边去除：先用 `cropdetect` 检测黑边（[`BlackBarDetect`]），再裁掉。
//! 一进一出：第 1 步分析，第 2 步重编码（音频容器装得下时直接复制）。
//!
//! - **灵敏度**：`limit`（0～255，亮度不超过它的像素算黑色，越大越容易把暗部当成黑边）、
//!   `sampleFps`（每秒抽样几帧，0.1～30）、`minBar`（四边加起来少于这么多像素时视为没有黑边，0～10000）。
//!   检测结果的宽高取偶数，所以输出总是偶数尺寸。
//! - **没有检测到黑边**（`whenNone`）：`skip`（默认）不输出，在处理记录里提示；`copy` 原样复制一份（不重编码，
//!   容器按设置的目标格式，装不下时依次改用 mp4 / mkv；音频装不下时转 AAC）。
//! - **恢复尺寸**（`restore`）：`none`（默认）输出裁剪后的尺寸；`fill` 等比放大到铺满原尺寸后居中裁去多余部分
//!   （不变形、不再出现黑边）；`stretch` 直接拉伸到原尺寸（会变形）。原尺寸为奇数时减 1 取偶。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::tools::analysis::{BlackBarDetect, CropRect};
use crate::tools::context::Encoding;
use crate::tools::common::filter_job::{display_size, run_job, source_audio, FilterJob};
use crate::tools::media::{container_accepts, copy_container, StreamKind};
use crate::tools::{OutputSpec, Tool, ToolContext, ToolError};

pub const ID: &str = "tool.remove-black-bars";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhenNoBars {
    /// 不输出，只记录
    #[default]
    Skip,
    /// 原样复制一份
    Copy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreSize {
    /// 输出裁剪后的尺寸
    #[default]
    None,
    /// 等比铺满原尺寸（居中裁去多余部分）
    Fill,
    /// 拉伸到原尺寸
    Stretch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoveBlackBarsParams {
    /// 黑色阈值（0～255）
    pub limit: u32,
    /// 每秒抽样帧数
    pub sample_fps: f64,
    /// 最小黑边（四边合计像素）
    pub min_bar: u32,
    pub when_none: WhenNoBars,
    pub restore: RestoreSize,
}

impl Default for RemoveBlackBarsParams {
    fn default() -> Self {
        let detect = BlackBarDetect::default();
        Self {
            limit: detect.limit,
            sample_fps: detect.sample_fps,
            min_bar: detect.min_bar,
            when_none: WhenNoBars::Skip,
            restore: RestoreSize::None,
        }
    }
}

impl RemoveBlackBarsParams {
    /// 检测参数（宽高取偶数）。
    pub fn detect(&self) -> Result<BlackBarDetect, String> {
        if self.limit > 255 {
            return Err("黑色阈值必须在 0～255 之间".into());
        }
        if !(self.sample_fps.is_finite() && (0.1..=30.0).contains(&self.sample_fps)) {
            return Err("抽样帧率必须在 0.1～30 之间".into());
        }
        if self.min_bar > 10_000 {
            return Err("最小黑边不能超过 10000 像素".into());
        }
        Ok(BlackBarDetect {
            limit: self.limit,
            round: 2,
            sample_fps: self.sample_fps,
            min_bar: self.min_bar,
        })
    }
}

/// 裁掉黑边的命令。
pub fn crop_job(
    params: &RemoveBlackBarsParams,
    info: &MediaInfo,
    input: &str,
    rect: CropRect,
) -> Result<FilterJob, String> {
    let (w, h) = display_size(info)?;
    let (ow, oh) = (w & !1, h & !1);
    let restore = match params.restore {
        RestoreSize::None => String::new(),
        RestoreSize::Fill => {
            format!(",scale={ow}:{oh}:force_original_aspect_ratio=increase,crop={ow}:{oh}")
        }
        RestoreSize::Stretch => format!(",scale={ow}:{oh}"),
    };
    let mut job = FilterJob::new(input);
    job.graph = format!("[0:v:0]{}{restore},setsar=1[v]", rect.filter());
    job.audio = source_audio(info);
    job.duration = info.duration;
    Ok(job)
}

/// 没有黑边时原样复制的命令（不含开头的通用参数）。
pub fn copy_args(info: &MediaInfo, input: &str, output: &str, container: &str) -> Vec<String> {
    let mut args = vec![
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:V:0".into(),
        "-c:v".into(),
        "copy".into(),
    ];
    if let Some(audio) = info.audio.first() {
        args.extend(["-map".into(), "0:a:0".into()]);
        if container_accepts(container, StreamKind::Audio, &audio.codec) == Some(false) {
            args.extend(Encoding::audio_args(audio.channels));
        } else {
            args.extend(["-c:a".into(), "copy".into()]);
        }
    }
    args.extend(["-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

impl Tool for RemoveBlackBarsParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let detect = self.detect()?;
        let info = ctx.info(0)?;
        let input_path = ctx.input_path(0)?.to_path_buf();
        let input = path_arg(&input_path)?;
        display_size(&info)?;
        ctx.set_steps(&[1.0, 3.0]);
        let Some(rect) = ctx.analyze_input(0, &detect)? else {
            return match self.when_none {
                WhenNoBars::Skip => {
                    ctx.warn("没有检测到黑边，已跳过（不输出）");
                    ctx.skip_step();
                    Ok(())
                }
                WhenNoBars::Copy => {
                    ctx.note("没有检测到黑边，原样复制");
                    let codec = info
                        .video
                        .as_ref()
                        .map(|v| v.codec.clone())
                        .unwrap_or_default();
                    let container = copy_container(ctx.settings(), &input_path, &codec);
                    let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
                    ctx.run_ffmpeg(
                        "复制",
                        copy_args(&info, &input, &output, &container),
                        info.duration,
                    )
                }
            };
        };
        let (w, h) = display_size(&info)?;
        ctx.note(format!(
            "检测到黑边：{w}×{h} → {}×{}（左 {}、上 {}）",
            rect.width, rect.height, rect.x, rect.y
        ));
        let job = crop_job(self, &info, &input, rect)?;
        run_job(ctx, "去除黑边", &job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::testing::sample_info;

    const RECT: CropRect = CropRect {
        x: 0,
        y: 90,
        width: 1280,
        height: 540,
    };

    #[test]
    fn detection_params() {
        let p = RemoveBlackBarsParams {
            limit: 40,
            sample_fps: 1.0,
            min_bar: 8,
            ..RemoveBlackBarsParams::default()
        };
        assert_eq!(
            p.detect().unwrap(),
            BlackBarDetect {
                limit: 40,
                round: 2,
                sample_fps: 1.0,
                min_bar: 8
            }
        );
        for bad in [
            RemoveBlackBarsParams {
                limit: 256,
                ..RemoveBlackBarsParams::default()
            },
            RemoveBlackBarsParams {
                sample_fps: 0.0,
                ..RemoveBlackBarsParams::default()
            },
            RemoveBlackBarsParams {
                min_bar: 20_000,
                ..RemoveBlackBarsParams::default()
            },
        ] {
            assert!(bad.detect().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn crop_with_each_restore_mode() {
        let graph = |restore| {
            let p = RemoveBlackBarsParams {
                restore,
                ..RemoveBlackBarsParams::default()
            };
            crop_job(&p, &sample_info(), "in.mp4", RECT).unwrap().graph
        };
        assert_eq!(
            graph(RestoreSize::None),
            "[0:v:0]crop=1280:540:0:90,setsar=1[v]"
        );
        assert_eq!(
            graph(RestoreSize::Fill),
            "[0:v:0]crop=1280:540:0:90,scale=1280:720:force_original_aspect_ratio=increase,\
             crop=1280:720,setsar=1[v]"
        );
        assert_eq!(
            graph(RestoreSize::Stretch),
            "[0:v:0]crop=1280:540:0:90,scale=1280:720,setsar=1[v]"
        );
    }

    #[test]
    fn copy_command() {
        assert_eq!(
            copy_args(&sample_info(), "in.mp4", "out.mp4", "mp4").join(" "),
            "-i in.mp4 -map 0:V:0 -c:v copy -map 0:a:0 -c:a copy -sn -dn -movflags +faststart out.mp4"
        );
        let mut info = sample_info();
        info.audio[0].codec = "pcm_s16le".into();
        let args = copy_args(&info, "in.mov", "out.mp4", "mp4").join(" ");
        assert!(args.contains("-c:a aac -b:a 192k"), "{args}");
        info.audio.clear();
        assert_eq!(
            copy_args(&info, "in.mkv", "out.mkv", "mkv").join(" "),
            "-i in.mkv -map 0:V:0 -c:v copy -sn -dn out.mkv"
        );
    }
}
