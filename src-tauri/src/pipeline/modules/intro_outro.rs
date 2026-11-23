//! `pipe.intro-outro` 加头尾与边框：在正片前后拼接片头 / 片尾视频，并可给正片套一个相框。
//!
//! 参数与取值约定（FEATURES 未规定的部分在此说明）：
//! - 片头、片尾、相框各有开关；开启后素材来源为单个文件，或目录（随机 / 依次取）。
//!   取素材的随机顺序固定为：片头 → 片尾 → 相框。
//! - 片头片尾会被统一成正片当前的规格后用 `concat` 拼接：
//!   - 画面：等比缩放到正片宽高以内、黑边居中补齐（`scale…force_original_aspect_ratio=decrease` + `pad`），
//!     `setsar=1`，帧率转换为正片当前帧率（`fps`）；变形（非方形像素）素材先换算为方形像素，显示比例不变；
//!   - 声音（仅当正片有音频时）：转换为正片的采样率与声道布局（`aformat`），并补静音 / 截断到素材时长；
//!     素材没有音频时生成等长的静音（`anullsrc`）；正片没有音频时输出也不带音频。
//! - 片头片尾的时长需要读取素材信息：构建命令时用 `ctx.probe`（本次运行的 ffprobe）读取。
//! - 相框：一张中间透明的图片（目录模式取 png / webp），拉伸到正片大小盖在最上层。
//!   `frameMargin` 为画面与相框边缘的间距，按正片宽、高的百分比（范围值，0～40，保留一位小数）：
//!   正片先缩小到 (宽 − 2×左右间距) × (高 − 2×上下间距)，居中放在黑底上，再叠加相框。
//!   相框只作用于正片，不套在片头片尾上。
//! - 画面尺寸与帧率不变；`ctx.stream.duration` 加上片头片尾的时长。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil::VIDEO_EXTENSIONS;
use crate::model::{AssetSource, Range};
use crate::pipeline::{fmt_num, BuildContext, PipelineModule};

pub const ID: &str = "pipe.intro-outro";

/// 相框素材在目录模式下的扩展名（需要透明通道）。
pub const FRAME_EXTENSIONS: &[&str] = &["png", "webp"];

/// 相框间距上限（百分比）。
pub const MAX_MARGIN: f64 = 40.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct IntroOutroParams {
    pub enabled: bool,
    pub intro_enabled: bool,
    /// 片头视频
    pub intro: AssetSource,
    pub outro_enabled: bool,
    /// 片尾视频
    pub outro: AssetSource,
    pub frame_enabled: bool,
    /// 相框图片（中间透明）
    pub frame: AssetSource,
    /// 画面与相框边缘的间距（占宽高的百分比）
    pub frame_margin: Range<f64>,
}

impl Default for IntroOutroParams {
    fn default() -> Self {
        Self {
            enabled: false,
            intro_enabled: false,
            intro: AssetSource::default(),
            outro_enabled: false,
            outro: AssetSource::default(),
            frame_enabled: false,
            frame: AssetSource::default(),
            frame_margin: Range::fixed(5.0),
        }
    }
}

/// 一个要拼接的片段（片头或片尾）。
struct Clip {
    input: usize,
    duration: f64,
    has_audio: bool,
    /// 变形（非方形像素）素材：先换算为方形像素再缩放，显示比例才正确
    anamorphic: bool,
}

/// 正片的音频规格：采样率与声道布局。
struct AudioFormat {
    sample_rate: u32,
    layout: &'static str,
}

fn audio_format(ctx: &BuildContext) -> AudioFormat {
    let source = ctx.info.audio.first();
    let sample_rate = source
        .map(|a| a.sample_rate)
        .filter(|r| *r > 0)
        .unwrap_or(48_000);
    let layout = match source.map(|a| a.channels).unwrap_or(2) {
        1 => "mono",
        6 => "5.1",
        8 => "7.1",
        _ => "stereo",
    };
    AudioFormat {
        sample_rate,
        layout,
    }
}

/// 向下取偶数（yuv420 要求偶数尺寸 / 偏移）。
fn even(value: f64) -> u32 {
    let v = value.max(0.0).floor() as u32;
    v - v % 2
}

impl IntroOutroParams {
    fn validate(&self) -> Result<(), String> {
        if self.frame_enabled {
            let (lo, hi) = self.frame_margin.ordered();
            if !lo.is_finite() || !hi.is_finite() || lo < 0.0 || hi > MAX_MARGIN {
                return Err(format!("相框间距需在 0～{MAX_MARGIN}% 之间"));
            }
        }
        for (on, source, name) in [
            (self.intro_enabled, &self.intro, "片头视频"),
            (self.outro_enabled, &self.outro, "片尾视频"),
            (self.frame_enabled, &self.frame, "相框图片"),
        ] {
            if on && source.is_empty() {
                return Err(format!("已开启{name}但没有选择文件"));
            }
        }
        Ok(())
    }

    fn build(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.validate()?;
        let intro = self.pick(ctx, self.intro_enabled, &self.intro, VIDEO_EXTENSIONS)?;
        let outro = self.pick(ctx, self.outro_enabled, &self.outro, VIDEO_EXTENSIONS)?;
        let frame = self.pick(ctx, self.frame_enabled, &self.frame, FRAME_EXTENSIONS)?;

        if let Some(frame) = frame {
            let margin = self.frame_margin.sample_rounded(&mut ctx.rng, 1);
            self.add_frame(ctx, &frame, margin)?;
        }
        let intro = intro.map(|p| Self::add_clip(ctx, &p, "片头")).transpose()?;
        let outro = outro.map(|p| Self::add_clip(ctx, &p, "片尾")).transpose()?;
        if intro.is_some() || outro.is_some() {
            Self::concat(ctx, intro, outro);
        }
        Ok(())
    }

    fn pick(
        &self,
        ctx: &mut BuildContext,
        on: bool,
        source: &AssetSource,
        extensions: &[&str],
    ) -> Result<Option<PathBuf>, String> {
        if !on {
            return Ok(None);
        }
        source
            .pick(extensions, ctx.task_index, &mut ctx.rng)
            .map(Some)
    }

    /// 正片缩小居中，盖上相框。
    fn add_frame(&self, ctx: &mut BuildContext, frame: &Path, margin: f64) -> Result<(), String> {
        let (w, h) = (ctx.stream.width, ctx.stream.height);
        let mx = even(f64::from(w) * margin / 100.0);
        let my = even(f64::from(h) * margin / 100.0);
        let input = ctx.add_input(Vec::new(), frame)?;
        let main = ctx.graph.take_video();
        let inner = if mx > 0 || my > 0 {
            let padded = ctx.graph.label("fp");
            ctx.graph.chain(
                &[&main],
                &format!(
                    "scale={}:{},setsar=1,pad={w}:{h}:{mx}:{my}:color=black",
                    w - 2 * mx,
                    h - 2 * my
                ),
                &[&padded],
            );
            padded
        } else {
            main
        };
        let image = ctx.graph.label("fi");
        ctx.graph.chain(
            &[&format!("{input}:v")],
            &format!("scale={w}:{h},format=rgba"),
            &[&image],
        );
        let out = ctx.graph.label("fo");
        ctx.graph.chain(&[&inner, &image], "overlay=0:0", &[&out]);
        ctx.graph.set_video(out);
        Ok(())
    }

    /// 加入片头 / 片尾输入并读取其信息。
    fn add_clip(ctx: &mut BuildContext, path: &Path, name: &str) -> Result<Clip, String> {
        let info = ctx
            .probe(path)
            .map_err(|e| format!("{name}视频无法读取：{e}"))?;
        if info.video.is_none() {
            return Err(format!("{name}文件中没有视频流：{}", path.display()));
        }
        if info.duration <= 0.0 {
            return Err(format!("{name}视频时长未知：{}", path.display()));
        }
        let input = ctx.add_input(Vec::new(), path)?;
        Ok(Clip {
            input,
            duration: info.duration,
            has_audio: !info.audio.is_empty(),
            anamorphic: info.video.as_ref().and_then(|v| v.sar_ratio()).is_some(),
        })
    }

    /// 把片头 / 片尾统一成正片规格，与正片按顺序拼接。
    fn concat(ctx: &mut BuildContext, intro: Option<Clip>, outro: Option<Clip>) {
        let (w, h) = (ctx.stream.width, ctx.stream.height);
        let fps = ctx.rate_text();
        let with_audio = ctx.graph.has_audio();
        let format = audio_format(ctx);
        let aformat = format!(
            "aformat=sample_rates={}:channel_layouts={}",
            format.sample_rate, format.layout
        );

        ctx.graph.video("setsar=1");
        ctx.graph.video("setpts=PTS-STARTPTS");
        ctx.graph.audio(aformat.clone());
        ctx.graph.audio("asetpts=PTS-STARTPTS");
        let main_video = ctx.graph.take_video();
        let main_audio = ctx.graph.take_audio();

        let mut segments: Vec<String> = Vec::new();
        let mut added = 0.0;
        let mut push_clip = |ctx: &mut BuildContext, clip: &Clip, segments: &mut Vec<String>| {
            let square = if clip.anamorphic {
                "scale=trunc(iw*sar/2)*2:ih,setsar=1,"
            } else {
                ""
            };
            let mut video = format!(
                "{square}scale={w}:{h}:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1"
            );
            if let Some(fps) = &fps {
                video.push_str(&format!(",fps={fps}"));
            }
            video.push_str(",setpts=PTS-STARTPTS");
            let v = ctx.graph.label("cv");
            ctx.graph
                .chain(&[&format!("{}:v:0", clip.input)], &video, &[&v]);
            segments.push(v);
            if with_audio {
                let d = fmt_num(clip.duration);
                // 数字静音（单声道 AAC 的死循环问题由 pipeline::aac_kbps 统一规避）
                let silence = format!(
                    "anullsrc=r={}:cl={}:d={d},{aformat}",
                    format.sample_rate, format.layout
                );
                let a = ctx.graph.label("ca");
                if clip.has_audio {
                    // 素材音频后接一段等长静音再截到素材时长：短了补静音，长了截断。
                    let own = ctx.graph.label("ca");
                    let pad = ctx.graph.label("ca");
                    ctx.graph.chain(
                        &[&format!("{}:a:0", clip.input)],
                        &format!("{aformat},asetpts=PTS-STARTPTS"),
                        &[&own],
                    );
                    ctx.graph.chain(&[], &silence, &[&pad]);
                    ctx.graph.chain(
                        &[&own, &pad],
                        &format!("concat=n=2:v=0:a=1,atrim=duration={d}"),
                        &[&a],
                    );
                } else {
                    ctx.graph.chain(&[], &silence, &[&a]);
                }
                segments.push(a);
            }
            added += clip.duration;
        };

        if let Some(clip) = &intro {
            push_clip(ctx, clip, &mut segments);
        }
        segments.push(main_video);
        if let Some(audio) = main_audio {
            segments.push(audio);
        }
        if let Some(clip) = &outro {
            push_clip(ctx, clip, &mut segments);
        }

        let count = 1 + usize::from(intro.is_some()) + usize::from(outro.is_some());
        let inputs: Vec<&str> = segments.iter().map(String::as_str).collect();
        let video_out = ctx.graph.label("io");
        if with_audio {
            let audio_out = ctx.graph.label("io");
            ctx.graph.chain(
                &inputs,
                &format!("concat=n={count}:v=1:a=1"),
                &[&video_out, &audio_out],
            );
            ctx.graph.set_audio(Some(audio_out));
        } else {
            ctx.graph
                .chain(&inputs, &format!("concat=n={count}:v=1:a=0"), &[&video_out]);
        }
        ctx.graph.set_video(video_out);
        ctx.stream.duration += added;
    }
}

impl PipelineModule for IntroOutroParams {
    fn id(&self) -> &'static str {
        ID
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn apply(&self, ctx: &mut BuildContext) -> Result<(), String> {
        self.build(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::{AudioStream, MediaInfo, VideoStream};
    use crate::model::{PickOrder, SeededRng, Settings};
    use crate::pipeline::testing::{build_with, sample_info};

    /// 单元测试用的临时素材目录（文件内容无关紧要，读取信息由假 probe 提供）。
    struct Assets(PathBuf);

    impl Assets {
        fn new(name: &str, files: &[&str]) -> Self {
            let dir =
                std::env::temp_dir().join(format!("cr-intro-outro-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for f in files {
                std::fs::write(dir.join(f), b"x").unwrap();
            }
            Self(dir)
        }

        fn file(&self, name: &str) -> AssetSource {
            AssetSource::File {
                path: self.0.join(name).to_string_lossy().into_owned(),
            }
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }
    }

    impl Drop for Assets {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 假 probe：文件名含 `silent` 的没有音频；时长 = 文件名里 `_` 之后的数字（默认 2 秒）。
    fn fake_probe(path: &Path) -> Result<MediaInfo, String> {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        if name.contains("broken") {
            return Err("无法读取媒体信息：Invalid data".into());
        }
        let duration = name
            .rsplit('_')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2.0);
        Ok(MediaInfo {
            duration,
            video: Some(VideoStream {
                width: 640,
                height: 360,
                fps: 30.0,
                ..VideoStream::default()
            }),
            audio: if name.contains("silent") {
                Vec::new()
            } else {
                vec![AudioStream {
                    sample_rate: 44_100,
                    channels: 1,
                    ..AudioStream::default()
                }]
            },
            ..MediaInfo::default()
        })
    }

    struct Built {
        filter: Option<String>,
        video_map: String,
        audio_map: Option<String>,
        inputs: Vec<String>,
        duration: f64,
    }

    fn run(params: &IntroOutroParams, info: &MediaInfo, seed: u64) -> Result<Built, String> {
        let settings = Settings::default();
        let mut ctx = BuildContext::new(info, &settings, 0);
        ctx.rng = SeededRng::derive(seed, ID);
        ctx.set_probe(&fake_probe);
        params.apply(&mut ctx)?;
        let rendered = ctx.graph.clone().render()?;
        Ok(Built {
            filter: rendered.filter_complex,
            video_map: rendered.video_map,
            audio_map: rendered.audio_map,
            inputs: ctx.extra_inputs.iter().map(|i| i.path.clone()).collect(),
            duration: ctx.stream.duration,
        })
    }

    #[test]
    fn intro_without_audio_and_outro_with_audio() {
        let assets = Assets::new("both", &["intro_silent_1.5.mp4", "outro_3.mp4"]);
        let params = IntroOutroParams {
            enabled: true,
            intro_enabled: true,
            intro: assets.file("intro_silent_1.5.mp4"),
            outro_enabled: true,
            outro: assets.file("outro_3.mp4"),
            ..IntroOutroParams::default()
        };
        let built = run(&params, &sample_info(), 1).unwrap();
        assert_eq!(
            built.inputs,
            [
                assets.path("intro_silent_1.5.mp4"),
                assets.path("outro_3.mp4")
            ]
        );
        assert_eq!(
            built.filter.as_deref(),
            Some(
                "[0:v:0]setsar=1,setpts=PTS-STARTPTS[v0];\
                 [0:a:0]aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS[a1];\
                 [1:v:0]scale=1280:720:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=25,setpts=PTS-STARTPTS[cv2];\
                 anullsrc=r=48000:cl=stereo:d=1.5,\
                 aformat=sample_rates=48000:channel_layouts=stereo[ca3];\
                 [2:v:0]scale=1280:720:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=25,setpts=PTS-STARTPTS[cv4];\
                 [2:a:0]aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS[ca6];\
                 anullsrc=r=48000:cl=stereo:d=3,\
                 aformat=sample_rates=48000:channel_layouts=stereo[ca7];\
                 [ca6][ca7]concat=n=2:v=0:a=1,atrim=duration=3[ca5];\
                 [cv2][ca3][v0][a1][cv4][ca5]concat=n=3:v=1:a=1[io8][io9]"
            )
        );
        assert_eq!(built.video_map, "[io8]");
        assert_eq!(built.audio_map.as_deref(), Some("[io9]"));
        assert!((built.duration - 14.5).abs() < 1e-9);
    }

    #[test]
    fn main_without_audio_ignores_clip_audio() {
        let assets = Assets::new("noaudio", &["outro_2.mp4"]);
        let mut info = sample_info();
        info.audio.clear();
        let params = IntroOutroParams {
            enabled: true,
            outro_enabled: true,
            outro: assets.file("outro_2.mp4"),
            ..IntroOutroParams::default()
        };
        let built = run(&params, &info, 1).unwrap();
        assert_eq!(
            built.filter.as_deref(),
            Some(
                "[0:v:0]setsar=1,setpts=PTS-STARTPTS[v0];\
                 [1:v:0]scale=1280:720:force_original_aspect_ratio=decrease:force_divisible_by=2,\
                 pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps=25,setpts=PTS-STARTPTS[cv1];\
                 [v0][cv1]concat=n=2:v=1:a=0[io2]"
            )
        );
        assert_eq!(built.audio_map, None);
        assert_eq!(built.duration, 12.0);
    }

    #[test]
    fn frame_overlay_with_margin() {
        let assets = Assets::new("frame", &["frame.png"]);
        let params = IntroOutroParams {
            enabled: true,
            frame_enabled: true,
            frame: assets.file("frame.png"),
            frame_margin: Range::fixed(5.0),
            ..IntroOutroParams::default()
        };
        let mut recipe = crate::model::Recipe::disabled();
        recipe.modules.intro_outro = params.clone();
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        // 1280×5% = 64，720×5% = 36
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some(
                "[0:v:0]scale=1152:648,setsar=1,pad=1280:720:64:36:color=black[fp0];\
                 [1:v]scale=1280:720,format=rgba[fi1];[fp0][fi1]overlay=0:0[fo2]"
            )
        );
        assert!(built.args.contains(&assets.path("frame.png")));
        assert_eq!(built.expected_duration, 10.0);
        assert_eq!(built.arg_after("-c:a"), Some("copy"));

        let no_margin = IntroOutroParams {
            frame_margin: Range::fixed(0.0),
            ..params
        };
        recipe.modules.intro_outro = no_margin;
        let built = build_with(&sample_info(), &recipe, 1).unwrap();
        assert_eq!(
            built.arg_after("-filter_complex"),
            Some("[1:v]scale=1280:720,format=rgba[fi0];[0:v:0][fi0]overlay=0:0[fo1]")
        );
    }

    #[test]
    fn directory_pick_and_margin_are_reproducible() {
        let assets = Assets::new(
            "dir",
            &[
                "a_1.mp4", "b_2.mp4", "c_3.mp4", "f1.png", "f2.webp", "note.txt",
            ],
        );
        let dir = AssetSource::Directory {
            path: assets.0.to_string_lossy().into_owned(),
            recursive: false,
            order: PickOrder::Random,
        };
        let params = IntroOutroParams {
            enabled: true,
            intro_enabled: true,
            intro: dir.clone(),
            frame_enabled: true,
            frame: dir,
            frame_margin: Range::new(2.0, 10.0),
            ..IntroOutroParams::default()
        };
        let a = run(&params, &sample_info(), 42).unwrap();
        let b = run(&params, &sample_info(), 42).unwrap();
        assert_eq!(a.filter, b.filter);
        assert_eq!(a.inputs, b.inputs);
        // 相框先加入输入，片头在后
        assert!(a.inputs[1].ends_with(".mp4"));
        assert!(a.inputs[0].ends_with(".png") || a.inputs[0].ends_with(".webp"));
        for seed in 0..30 {
            let built = run(&params, &sample_info(), seed).unwrap();
            let fc = built.filter.unwrap();
            let pad = fc.split("pad=1280:720:").nth(1).unwrap();
            let mx: f64 = pad.split(':').next().unwrap().parse().unwrap();
            assert!((24.0..=128.0).contains(&mx), "{mx}");
        }
    }

    #[test]
    fn invalid_params_are_rejected() {
        let assets = Assets::new("invalid", &["broken.mp4", "f.png"]);
        let cases = [
            IntroOutroParams {
                enabled: true,
                intro_enabled: true,
                ..IntroOutroParams::default()
            },
            IntroOutroParams {
                enabled: true,
                intro_enabled: true,
                intro: assets.file("missing.mp4"),
                ..IntroOutroParams::default()
            },
            IntroOutroParams {
                enabled: true,
                outro_enabled: true,
                outro: assets.file("broken.mp4"),
                ..IntroOutroParams::default()
            },
            IntroOutroParams {
                enabled: true,
                frame_enabled: true,
                frame: assets.file("f.png"),
                frame_margin: Range::fixed(45.0),
                ..IntroOutroParams::default()
            },
        ];
        for params in cases {
            assert!(run(&params, &sample_info(), 1).is_err(), "{params:?}");
        }
    }

    #[test]
    fn disabled_parts_do_nothing() {
        let params = IntroOutroParams {
            enabled: true,
            ..IntroOutroParams::default()
        };
        let built = run(&params, &sample_info(), 1).unwrap();
        assert_eq!(built.filter, None);
        assert!(built.inputs.is_empty());
    }
}
