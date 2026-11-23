//! `tool.orientation` 横竖互转：把文件夹里的视频批量转成另一种画面方向（横屏 ↔ 竖屏）。
//! 视频按比例完整放进目标画面（不裁切、不变形），空白处用纯色、图片或视频自身的模糊画面填充。
//!
//! 参数：
//! - `target` 目标尺寸：`swap`（默认，宽高互换：1920×1080 → 1080×1920，竖屏同理；正方形保持不变）、
//!   `portrait`（1080×1920）、`landscape`（1920×1080）、`custom`（`width` × `height`，默认 1080×1920）。
//!   目标宽高为奇数时减 1 取偶（yuv420p 编码要求），范围 16～7680。
//! - `fill` 空白填充：`blur`（默认，视频自身放大铺满后高斯模糊，`blurStrength` 为模糊半径 sigma，像素，默认 20，1～200）、
//!   `color`（纯色 `color`，默认黑色）、`image`（图片 `image`，素材来源：单文件或目录随机 / 依次取；放大铺满并居中裁切）。
//! - 视频缩放后宽高取偶数（`force_divisible_by=2`），居中放置。
//!
//! 一个文件一个工作项；输出与源同名（按设置的目标格式），音频容器装得下时直接复制，否则转 AAC。
//! 图片填充时图片按源视频帧率循环（`-loop 1 -framerate`），画面随视频结束。
//!
//! 颜色、缩放方式、取偶等小函数在 [`crate::tools::common::frame`]（此处再导出，兼容旧的导入路径）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{path_arg, MediaInfo};
use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::AssetSource;
use crate::pipeline::fmt_num;
use crate::tools::context::Encoding;
use crate::tools::media::{container_accepts, StreamKind};
use crate::tools::{
    plan, FolderSlot, OutputSpec, PlanRequest, Tool, ToolContext, ToolError, ToolInputs, WorkItem,
};

pub const ID: &str = "tool.orientation";

/// 文件夹输入的键。
pub const SOURCE: &str = "source";

/// 填充图片的扩展名。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];

pub use crate::tools::common::frame::{color_arg, even, fit_scale, target_size, Fit, SIZE_RANGE};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrientationTarget {
    /// 宽高互换
    #[default]
    Swap,
    /// 竖屏 1080×1920
    Portrait,
    /// 横屏 1920×1080
    Landscape,
    /// 自定义宽高
    Custom,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrientationFill {
    /// 视频自身的模糊画面
    #[default]
    Blur,
    /// 纯色
    Color,
    /// 图片
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OrientationParams {
    pub target: OrientationTarget,
    /// 自定义宽度（像素）
    pub width: u32,
    /// 自定义高度（像素）
    pub height: u32,
    pub fill: OrientationFill,
    /// 纯色填充的颜色（#rrggbb）
    pub color: String,
    /// 图片填充的图片
    pub image: AssetSource,
    /// 模糊半径（gblur 的 sigma，像素）
    pub blur_strength: f64,
}

impl Default for OrientationParams {
    fn default() -> Self {
        Self {
            target: OrientationTarget::Swap,
            width: 1080,
            height: 1920,
            fill: OrientationFill::Blur,
            color: "#000000".into(),
            image: AssetSource::default(),
            blur_strength: 20.0,
        }
    }
}

impl OrientationParams {
    fn validate(&self) -> Result<(), String> {
        match self.fill {
            OrientationFill::Color => {
                color_arg(&self.color, "填充颜色")?;
            }
            OrientationFill::Image if self.image.is_empty() => {
                return Err("请选择填充图片".into());
            }
            OrientationFill::Blur
                if !(self.blur_strength.is_finite()
                    && (1.0..=200.0).contains(&self.blur_strength)) =>
            {
                return Err("模糊强度必须在 1～200 之间".into());
            }
            _ => {}
        }
        if self.target == OrientationTarget::Custom {
            target_size(self.width, self.height)?;
        }
        Ok(())
    }

    /// 输出宽高（`source` 为源视频的显示宽高）。
    pub fn output_size(&self, source: (u32, u32)) -> Result<(u32, u32), String> {
        match self.target {
            OrientationTarget::Swap => {
                let (w, h) = source;
                if w < 2 || h < 2 {
                    return Err("无法读取视频分辨率".into());
                }
                Ok((even(h), even(w)))
            }
            OrientationTarget::Portrait => Ok((1080, 1920)),
            OrientationTarget::Landscape => Ok((1920, 1080)),
            OrientationTarget::Custom => target_size(self.width, self.height),
        }
    }
}

/// 一条转换命令（不含编码参数与输出）。
#[derive(Debug, Clone, PartialEq)]
pub struct OrientationJob {
    /// 输入参数（含 `-i`），第 0 个是源视频
    pub inputs: Vec<String>,
    /// `-filter_complex`，输出 `[v]`
    pub graph: String,
    pub size: (u32, u32),
}

/// 构建转换命令。`image`：图片填充时选中的图片路径。
pub fn build(
    params: &OrientationParams,
    info: &MediaInfo,
    input: &str,
    image: Option<&str>,
) -> Result<OrientationJob, String> {
    params.validate()?;
    let video = info.video.as_ref().ok_or("文件中没有视频流")?;
    let (w, h) = params.output_size(video.display_size())?;
    let mut inputs = vec!["-i".to_owned(), input.to_owned()];
    let fg = format!("{},setsar=1", fit_scale(Fit::Fit, w, h));
    let centre = "overlay=x=(W-w)/2:y=(H-h)/2";
    let graph = match params.fill {
        OrientationFill::Color => format!(
            "[0:v:0]{fg},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color={}[v]",
            color_arg(&params.color, "填充颜色")?
        ),
        OrientationFill::Blur => format!(
            "[0:v:0]split=2[bg][fg];[bg]{},gblur=sigma={},setsar=1[bgb];[fg]{fg}[fgs];\
             [bgb][fgs]{centre}[v]",
            fit_scale(Fit::Fill, w, h),
            fmt_num(params.blur_strength)
        ),
        OrientationFill::Image => {
            let image = image.ok_or("请选择填充图片")?;
            let rate = if video.fps > 0.0 && !video.fps_text.trim().is_empty() {
                video.fps_text.trim().to_owned()
            } else {
                "30".to_owned()
            };
            inputs.extend([
                "-loop".into(),
                "1".into(),
                "-framerate".into(),
                rate,
                "-i".into(),
                image.to_owned(),
            ]);
            format!(
                "[1:v]{},setsar=1,format=yuv420p[bg];[0:v:0]{fg}[fgs];[bg][fgs]{centre}:shortest=1[v]",
                fit_scale(Fit::Fill, w, h)
            )
        }
    };
    Ok(OrientationJob {
        inputs,
        graph,
        size: (w, h),
    })
}

/// 源音频的参数：容器装得下就复制，否则转 AAC；没有音频时 `-an`。
pub fn source_audio_args(info: &MediaInfo, container: &str) -> Vec<String> {
    match info.audio.first() {
        None => vec!["-an".into()],
        Some(audio) => {
            let mut args = vec!["-map".to_owned(), "0:a:0".to_owned()];
            if container_accepts(container, StreamKind::Audio, &audio.codec) == Some(true) {
                args.extend(["-c:a".into(), "copy".into()]);
            } else {
                args.extend(Encoding::audio_args(audio.channels.max(1)));
            }
            args
        }
    }
}

/// 完整参数（不含开头的通用参数）。
pub fn job_args(
    job: &OrientationJob,
    info: &MediaInfo,
    encoding: &Encoding,
    container: &str,
    output: &str,
) -> Vec<String> {
    let mut args = job.inputs.clone();
    args.extend([
        "-filter_complex".into(),
        job.graph.clone(),
        "-map".into(),
        "[v]".into(),
    ]);
    let audio = source_audio_args(info, container);
    args.extend(encoding.video_args(container));
    args.extend(audio);
    args.extend(["-sn".into(), "-dn".into()]);
    if matches!(container, "mp4" | "mov" | "m4v") {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.push(output.into());
    args
}

impl Tool for OrientationParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[FolderSlot {
            key: SOURCE,
            label: "视频文件夹",
        }];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let files = req.folder_files(SOURCE, VIDEO_EXTENSIONS)?;
        Ok(plan::per_file(ID, &files, req.folder(SOURCE)?))
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let info = ctx.info(0)?;
        let input = path_arg(ctx.input_path(0)?)?;
        let image = match self.fill {
            OrientationFill::Image => {
                let index = ctx.number().saturating_sub(1);
                Some(path_arg(&self.image.pick(
                    IMAGE_EXTENSIONS,
                    index,
                    &mut ctx.rng,
                )?)?)
            }
            _ => None,
        };
        let job = build(self, &info, &input, image.as_deref())?;
        if let Some(video) = &info.video {
            if video.display_size() == job.size {
                ctx.warn("源视频已是目标尺寸，画面不变");
            }
        }
        let container = ctx.video_extension(0)?;
        let output = path_arg(&ctx.output(&OutputSpec::file(&container))?)?;
        ctx.run_encode("横竖互转", info.duration, |enc| {
            Ok(job_args(&job, &info, enc, &container, &output))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};
    use crate::pipeline::testing::sample_info;

    fn with(fill: OrientationFill) -> OrientationParams {
        OrientationParams {
            fill,
            ..OrientationParams::default()
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(color_arg("#FFaa00", "x").unwrap(), "0xffaa00");
        assert_eq!(color_arg("#0f0", "x").unwrap(), "0x00ff00");
        assert!(color_arg("red", "x").is_err());
        assert_eq!(even(1081), 1080);
        assert_eq!(even(1), 2);
        assert_eq!(
            fit_scale(Fit::Fit, 1080, 1920),
            "scale=1080:1920:force_original_aspect_ratio=decrease:force_divisible_by=2"
        );
        assert_eq!(
            fit_scale(Fit::Fill, 100, 50),
            "scale=100:50:force_original_aspect_ratio=increase,crop=100:50"
        );
        assert_eq!(fit_scale(Fit::Stretch, 100, 50), "scale=100:50");
        assert_eq!(target_size(721, 1281).unwrap(), (720, 1280));
        assert!(target_size(8, 100).is_err());
    }

    #[test]
    fn output_sizes() {
        let p = OrientationParams::default();
        assert_eq!(p.output_size((1920, 1080)).unwrap(), (1080, 1920));
        assert_eq!(p.output_size((1081, 1921)).unwrap(), (1920, 1080));
        let fixed = |target| OrientationParams {
            target,
            width: 721,
            height: 405,
            ..OrientationParams::default()
        };
        assert_eq!(
            fixed(OrientationTarget::Portrait)
                .output_size((1920, 1080))
                .unwrap(),
            (1080, 1920)
        );
        assert_eq!(
            fixed(OrientationTarget::Landscape)
                .output_size((1080, 1920))
                .unwrap(),
            (1920, 1080)
        );
        assert_eq!(
            fixed(OrientationTarget::Custom)
                .output_size((1080, 1920))
                .unwrap(),
            (720, 404)
        );
    }

    #[test]
    fn blur_graph_swaps_landscape_to_portrait() {
        let job = build(&with(OrientationFill::Blur), &sample_info(), "in.mp4", None).unwrap();
        assert_eq!(job.size, (720, 1280));
        assert_eq!(job.inputs, ["-i", "in.mp4"]);
        assert_eq!(
            job.graph,
            "[0:v:0]split=2[bg][fg];\
             [bg]scale=720:1280:force_original_aspect_ratio=increase,crop=720:1280,gblur=sigma=20,setsar=1[bgb];\
             [fg]scale=720:1280:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1[fgs];\
             [bgb][fgs]overlay=x=(W-w)/2:y=(H-h)/2[v]"
        );
    }

    #[test]
    fn color_and_image_graphs() {
        let p = OrientationParams {
            color: "#123456".into(),
            ..with(OrientationFill::Color)
        };
        let job = build(&p, &sample_info(), "in.mp4", None).unwrap();
        assert_eq!(
            job.graph,
            "[0:v:0]scale=720:1280:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1,\
             pad=720:1280:(ow-iw)/2:(oh-ih)/2:color=0x123456[v]"
        );
        let p = OrientationParams {
            image: AssetSource::File {
                path: "bg.png".into(),
            },
            target: OrientationTarget::Landscape,
            ..with(OrientationFill::Image)
        };
        let job = build(&p, &sample_info(), "in.mp4", Some("bg.png")).unwrap();
        assert_eq!(
            job.inputs,
            [
                "-i",
                "in.mp4",
                "-loop",
                "1",
                "-framerate",
                "25/1",
                "-i",
                "bg.png"
            ]
        );
        assert_eq!(
            job.graph,
            "[1:v]scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080,setsar=1,format=yuv420p[bg];\
             [0:v:0]scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1[fgs];\
             [bg][fgs]overlay=x=(W-w)/2:y=(H-h)/2:shortest=1[v]"
        );
    }

    #[test]
    fn full_args_copy_compatible_audio() {
        let info = sample_info();
        let job = build(&with(OrientationFill::Color), &info, "in.mp4", None).unwrap();
        let enc = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(1000),
        };
        let args = job_args(&job, &info, &enc, "mp4", "out.mp4").join(" ");
        assert!(
            args.ends_with(
                "-map [v] -c:v libx264 -preset medium -pix_fmt yuv420p -b:v 1000k -maxrate 1500k \
                 -bufsize 2000k -map 0:a:0 -c:a copy -sn -dn -movflags +faststart out.mp4"
            ),
            "{args}"
        );
        let mut silent = info.clone();
        silent.audio.clear();
        assert_eq!(source_audio_args(&silent, "mp4"), ["-an"]);
    }

    #[test]
    fn invalid_params() {
        for p in [
            OrientationParams {
                color: "nope".into(),
                ..with(OrientationFill::Color)
            },
            with(OrientationFill::Image),
            OrientationParams {
                blur_strength: 0.0,
                ..with(OrientationFill::Blur)
            },
            OrientationParams {
                target: OrientationTarget::Custom,
                width: 10,
                ..OrientationParams::default()
            },
        ] {
            assert!(build(&p, &sample_info(), "in.mp4", None).is_err(), "{p:?}");
        }
        let mut no_video = sample_info();
        no_video.video = None;
        assert!(build(&OrientationParams::default(), &no_video, "in.mp4", None).is_err());
    }
}
