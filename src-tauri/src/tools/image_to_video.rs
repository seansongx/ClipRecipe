//! `tool.image-to-video` 图转视频：文件夹里的每张图片生成一个同名 `.mp4`（总是 mp4）。
//!
//! 参数：
//! - `width` × `height` 分辨率（默认 1080×1920，16～7680，奇数减 1 取偶）；
//! - `fit` 图片放进画面的方式：`fit`（默认，完整放入，空白处为背景色）、`fill`（铺满，居中裁切）、`stretch`（拉伸）；
//!   `background` 背景色（默认黑色；透明图片的透明部分也显示背景色）；
//! - `fps` 帧率（默认 30，1～120）、`duration` 时长（秒，默认 5，0.1～3600），帧数 = 时长 × 帧率（四舍五入）；
//! - `animation` 入场动画（默认 `none`），`animationSeconds` 入场时长（默认 1 秒，超过总时长时按总时长）：
//!   - `fadeIn` 淡入：从背景色淡入；
//!   - `zoomIn` 放大入场：画面从一半大小（居中，四周为背景色）放大到完整大小，先快后慢；
//!   - `slideIn` 滑入：画面从左侧滑入到位，先快后慢；
//!   - `kenBurns` 缓慢推拉：整个时长内从完整画面缓慢放大到 1.2 倍，同时从左向右平移（不使用入场时长）；
//! - `silentAudio` 加一条静音音轨（默认开启，立体声 48 kHz AAC；部分平台要求视频带音轨）。
//!
//! 一张图片一个工作项。图片用 image2 分离器读取（`-pattern_type none`，文件名里的 `%` 不会被当成序列）。
//! 码率：沿用“码率”设置的固定码率；未启用时用恒定质量（图片没有可参照的源码率）。

use serde::{Deserialize, Serialize};

use crate::ffmpeg::path_arg;
use crate::pipeline::fmt_num;
use crate::pipeline::modules::text::escape_value;
use crate::tools::common::frame::{color_arg, fit_scale, target_size, Fit};
use crate::tools::context::Encoding;
use crate::tools::{
    plan, EncodeBasis, FolderSlot, OutputSpec, PlanRequest, Tool, ToolContext, ToolError,
    ToolInputs, WorkItem,
};

pub const ID: &str = "tool.image-to-video";

/// 文件夹输入的键。
pub const IMAGES: &str = "images";

/// 处理的图片扩展名。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];

/// 缓慢推拉的最终放大倍数。
pub const KEN_BURNS_ZOOM: f64 = 1.2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Animation {
    #[default]
    None,
    FadeIn,
    ZoomIn,
    SlideIn,
    KenBurns,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageToVideoParams {
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
    /// 背景色（#rrggbb）
    pub background: String,
    pub fps: f64,
    /// 时长（秒）
    pub duration: f64,
    pub animation: Animation,
    /// 入场动画时长（秒）
    pub animation_seconds: f64,
    /// 加静音音轨
    pub silent_audio: bool,
}

impl Default for ImageToVideoParams {
    fn default() -> Self {
        Self {
            width: 1080,
            height: 1920,
            fit: Fit::Fit,
            background: "#000000".into(),
            fps: 30.0,
            duration: 5.0,
            animation: Animation::None,
            animation_seconds: 1.0,
            silent_audio: true,
        }
    }
}

impl ImageToVideoParams {
    fn validate(&self) -> Result<(), String> {
        target_size(self.width, self.height)?;
        color_arg(&self.background, "背景色")?;
        if !(self.fps.is_finite() && (1.0..=120.0).contains(&self.fps)) {
            return Err("帧率必须在 1～120 之间".into());
        }
        if !(self.duration.is_finite() && (0.1..=3600.0).contains(&self.duration)) {
            return Err("时长必须在 0.1～3600 秒之间".into());
        }
        if !(self.animation_seconds.is_finite() && self.animation_seconds >= 0.1) {
            return Err("入场动画时长至少 0.1 秒".into());
        }
        Ok(())
    }

    /// 输出帧数。
    pub fn frames(&self) -> u64 {
        (self.duration * self.fps).round().max(1.0) as u64
    }
}

/// 一张图片的转换命令（不含编码参数与输出路径）。
#[derive(Debug, Clone, PartialEq)]
pub struct ImageJob {
    pub inputs: Vec<String>,
    /// `-filter_complex`，输出 `[v]`
    pub graph: String,
    pub size: (u32, u32),
    pub frames: u64,
    pub duration: f64,
    pub audio: bool,
}

/// 构建转换命令。
pub fn build(params: &ImageToVideoParams, image: &str) -> Result<ImageJob, String> {
    params.validate()?;
    let (w, h) = target_size(params.width, params.height)?;
    let bg = color_arg(&params.background, "背景色")?;
    let fps = fmt_num(params.fps);
    let d = params.duration;
    let dur = fmt_num(d);
    let a = fmt_num(params.animation_seconds.min(d));
    let mut inputs: Vec<String> = [
        "-f",
        "image2",
        "-pattern_type",
        "none",
        "-loop",
        "1",
        "-framerate",
        &fps,
        "-t",
        &dur,
        "-i",
        image,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if params.silent_audio {
        inputs.extend(
            [
                "-f",
                "lavfi",
                "-t",
                &dur,
                "-i",
                "anullsrc=r=48000:cl=stereo",
            ]
            .iter()
            .map(|s| (*s).to_owned()),
        );
    }

    let x = if params.animation == Animation::SlideIn {
        escape_value(&format!("(W-w)/2-W*pow(1-min(t/{a},1),2)"))
    } else {
        "(W-w)/2".to_owned()
    };
    let mut graph = format!(
        "color=c={bg}:s={w}x{h}:r={fps}:d={dur}[bg];[0:v]{},setsar=1[img];\
         [bg][img]overlay=x={x}:y=(H-h)/2:shortest=1",
        fit_scale(params.fit, w, h)
    );
    match params.animation {
        Animation::None | Animation::SlideIn => {}
        Animation::FadeIn => graph.push_str(&format!(",fade=t=in:st=0:d={a}:color={bg}")),
        Animation::ZoomIn => {
            let p = format!("min(it/{a},1)");
            graph.push_str(&format!(
                ",pad={}:{}:{}:{}:color={bg},zoompan=z={}:x={}:y={}:d=1:s={w}x{h}:fps={fps}",
                w * 2,
                h * 2,
                w / 2,
                h / 2,
                escape_value(&format!("1+{p}*(2-{p})")),
                escape_value("(iw-iw/zoom)/2"),
                escape_value("(ih-ih/zoom)/2"),
            ));
        }
        Animation::KenBurns => {
            let p = format!("min(it/{dur},1)");
            graph.push_str(&format!(
                ",zoompan=z={}:x={}:y={}:d=1:s={w}x{h}:fps={fps}",
                escape_value(&format!("1+{}*{p}", fmt_num(KEN_BURNS_ZOOM - 1.0))),
                escape_value(&format!("(iw-iw/zoom)*{p}")),
                escape_value("(ih-ih/zoom)/2"),
            ));
        }
    }
    graph.push_str(",setsar=1,format=yuv420p[v]");
    Ok(ImageJob {
        inputs,
        graph,
        size: (w, h),
        frames: params.frames(),
        duration: d,
        audio: params.silent_audio,
    })
}

/// 完整参数（不含开头的通用参数）。
pub fn job_args(job: &ImageJob, encoding: &Encoding, output: &str) -> Vec<String> {
    let mut args = job.inputs.clone();
    args.extend([
        "-filter_complex".into(),
        job.graph.clone(),
        "-map".into(),
        "[v]".into(),
    ]);
    if job.audio {
        args.extend(["-map".into(), "1:a:0".into()]);
    }
    args.extend(encoding.video_args("mp4"));
    if job.audio {
        args.extend(Encoding::audio_args(2));
    } else {
        args.push("-an".into());
    }
    args.extend([
        "-frames:v".into(),
        job.frames.to_string(),
        "-t".into(),
        fmt_num(job.duration),
        "-movflags".into(),
        "+faststart".into(),
        output.into(),
    ]);
    args
}

impl Tool for ImageToVideoParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn inputs(&self) -> ToolInputs {
        const FOLDERS: &[FolderSlot] = &[FolderSlot {
            key: IMAGES,
            label: "图片文件夹",
        }];
        ToolInputs::folders(FOLDERS)
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        self.validate()?;
        let files = req.folder_files(IMAGES, IMAGE_EXTENSIONS)?;
        Ok(plan::per_file(ID, &files, req.folder(IMAGES)?))
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let path = ctx.input_path(0)?.to_path_buf();
        if !path.is_file() {
            return Err(format!("图片不存在：{}", path.display()).into());
        }
        let job = build(self, &path_arg(&path)?)?;
        let output = path_arg(&ctx.output(&OutputSpec::file("mp4"))?)?;
        // 码率依据：只有输出画面（图片没有可参照的码率；“码率”未启用时用恒定质量）
        let basis = EncodeBasis::output_only(job.size.0, job.size.1, self.fps);
        ctx.run_encode_from("图转视频", job.duration, &basis, |enc| {
            Ok(job_args(&job, enc, &output))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{VideoCodec, VideoEncoder};

    fn with(animation: Animation) -> ImageToVideoParams {
        ImageToVideoParams {
            width: 641,
            height: 360,
            fps: 25.0,
            duration: 2.0,
            animation,
            animation_seconds: 0.5,
            ..ImageToVideoParams::default()
        }
    }

    const BASE: &str = "color=c=0x000000:s=640x360:r=25:d=2[bg];\
        [0:v]scale=640:360:force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1[img];\
        [bg][img]overlay=x=(W-w)/2:y=(H-h)/2:shortest=1";

    #[test]
    fn plain_job_and_args() {
        let job = build(&with(Animation::None), "a.png").unwrap();
        assert_eq!(job.size, (640, 360));
        assert_eq!(job.frames, 50);
        assert_eq!(
            job.inputs.join(" "),
            "-f image2 -pattern_type none -loop 1 -framerate 25 -t 2 -i a.png \
             -f lavfi -t 2 -i anullsrc=r=48000:cl=stereo"
        );
        assert_eq!(job.graph, format!("{BASE},setsar=1,format=yuv420p[v]"));
        let enc = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: None,
        };
        let args = job_args(&job, &enc, "a.mp4").join(" ");
        assert!(
            args.ends_with(
                "-map [v] -map 1:a:0 -c:v libx264 -preset medium -pix_fmt yuv420p -crf 23 \
                 -c:a aac -b:a 192k -frames:v 50 -t 2 -movflags +faststart a.mp4"
            ),
            "{args}"
        );
    }

    #[test]
    fn animations() {
        let fade = build(&with(Animation::FadeIn), "a.png").unwrap();
        assert_eq!(
            fade.graph,
            format!("{BASE},fade=t=in:st=0:d=0.5:color=0x000000,setsar=1,format=yuv420p[v]")
        );
        let zoom = build(&with(Animation::ZoomIn), "a.png").unwrap();
        assert_eq!(
            zoom.graph,
            format!(
                "{BASE},pad=1280:720:320:180:color=0x000000,\
                 zoompan=z=1+min(it/0.5\\,1)*(2-min(it/0.5\\,1)):x=(iw-iw/zoom)/2:y=(ih-ih/zoom)/2:\
                 d=1:s=640x360:fps=25,setsar=1,format=yuv420p[v]"
            )
        );
        let slide = build(&with(Animation::SlideIn), "a.png").unwrap();
        assert!(
            slide
                .graph
                .contains("overlay=x=(W-w)/2-W*pow(1-min(t/0.5\\,1)\\,2):y=(H-h)/2"),
            "{}",
            slide.graph
        );
        let ken = build(&with(Animation::KenBurns), "a.png").unwrap();
        assert!(
            ken.graph.contains(
                "zoompan=z=1+0.2*min(it/2\\,1):x=(iw-iw/zoom)*min(it/2\\,1):y=(ih-ih/zoom)/2:d=1:s=640x360:fps=25"
            ),
            "{}",
            ken.graph
        );
    }

    #[test]
    fn fill_stretch_and_no_audio() {
        let p = ImageToVideoParams {
            fit: Fit::Fill,
            silent_audio: false,
            background: "#fff".into(),
            ..with(Animation::None)
        };
        let job = build(&p, "a.png").unwrap();
        assert!(job
            .graph
            .starts_with("color=c=0xffffff:s=640x360:r=25:d=2[bg];[0:v]scale=640:360:force_original_aspect_ratio=increase,crop=640:360,setsar=1[img]"));
        assert!(!job.inputs.contains(&"lavfi".to_owned()));
        let enc = Encoding {
            encoder: VideoEncoder::software(VideoCodec::H264),
            video_kbps: Some(800),
        };
        let args = job_args(&job, &enc, "a.mp4").join(" ");
        assert!(args.contains("-bufsize 1600k -an -frames:v 50"), "{args}");
        let stretch = ImageToVideoParams {
            fit: Fit::Stretch,
            ..with(Animation::None)
        };
        assert!(build(&stretch, "a.png")
            .unwrap()
            .graph
            .contains("[0:v]scale=640:360,setsar=1[img]"));
    }

    #[test]
    fn invalid_params() {
        for p in [
            ImageToVideoParams {
                width: 8,
                ..ImageToVideoParams::default()
            },
            ImageToVideoParams {
                fps: 0.0,
                ..ImageToVideoParams::default()
            },
            ImageToVideoParams {
                duration: 0.0,
                ..ImageToVideoParams::default()
            },
            ImageToVideoParams {
                background: "x".into(),
                ..ImageToVideoParams::default()
            },
            ImageToVideoParams {
                animation_seconds: 0.0,
                ..ImageToVideoParams::default()
            },
        ] {
            assert!(build(&p, "a.png").is_err(), "{p:?}");
        }
        assert_eq!(ImageToVideoParams::default().frames(), 150);
    }
}
