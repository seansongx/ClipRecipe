//! 集成测试：`pipe.intro-outro` 加头尾与边框，真实运行 ffmpeg 后用 ffprobe 检查
//! 拼接后的尺寸、帧率、时长、音画同步，以及相框叠加后的像素。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, PickOrder, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::IntroOutroParams;
use common::*;

/// 用 lavfi 生成任意素材（参数直接传给 ffmpeg，输出路径放最后）。
fn generate(paths: &FfmpegPaths, out: &Path, args: &[&str]) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 只启用加头尾与边框，处理一个文件；返回输出路径。
fn run(paths: &FfmpegPaths, params: IntroOutroParams, input: &Path, out: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.intro_outro = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 9,
        },
        vec![TaskSpec {
            id: "t".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成：{other:?}"),
    }
}

fn file(path: &Path) -> AssetSource {
    AssetSource::File {
        path: path.to_string_lossy().into_owned(),
    }
}

#[test]
fn intro_and_outro_are_conformed_and_concatenated() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("intro-outro");
    let main = dir.join("src/main.mp4");
    make_clip(&paths, &main, &Clip::default()); // 320x240 25fps 2 秒，单声道 44.1kHz
                                                // 片头：尺寸、帧率都不同，且没有音频
    let intro = dir.join("assets/intro.mp4");
    make_clip(
        &paths,
        &intro,
        &Clip {
            width: 640,
            height: 360,
            fps: 30,
            seconds: 1.0,
            audio: false,
        },
    );
    // 片尾：竖屏、15fps、22.05kHz 立体声
    let outro = dir.join("assets/outro.mp4");
    generate(
        &paths,
        &outro,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=200x300:rate=15:duration=1.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:duration=1.5:sample_rate=22050",
            "-ac",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-shortest",
        ],
    );
    let main_audio = probe(&paths, &main).audio[0].clone();

    let params = IntroOutroParams {
        enabled: true,
        intro_enabled: true,
        intro: file(&intro),
        outro_enabled: true,
        outro: file(&outro),
        ..IntroOutroParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert_close(video.fps, 25.0, 0.01, "帧率");
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].sample_rate, main_audio.sample_rate);
    assert_eq!(info.audio[0].channels, main_audio.channels);

    let s = streams(&paths, &output);
    // 1 + 2 + 1.5 = 4.5 秒 ≈ 112 帧
    assert_close(s.format, 4.5, 0.06, "总时长");
    assert_close(s.video, 4.5, 0.06, "视频时长");
    assert!((110..=114).contains(&s.frames), "帧数 {}", s.frames);
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
}

#[test]
fn main_without_audio_and_intro_with_audio_outputs_no_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("intro-mute");
    let main = dir.join("src/mute.mp4");
    make_clip(
        &paths,
        &main,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    let intro = dir.join("assets/intro.mp4");
    make_clip(
        &paths,
        &intro,
        &Clip {
            width: 160,
            height: 120,
            fps: 50,
            seconds: 1.0,
            audio: true,
        },
    );
    let params = IntroOutroParams {
        enabled: true,
        intro_enabled: true,
        intro: file(&intro),
        ..IntroOutroParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    let info = probe(&paths, &output);
    assert!(info.audio.is_empty());
    let video = info.video.unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    let s = streams(&paths, &output);
    assert_close(s.video, 3.0, 0.05, "视频时长");
    assert!((74..=76).contains(&s.frames), "帧数 {}", s.frames);
}

/// 读取第 `t` 秒那一帧的 RGB 像素。
fn frame_rgb(paths: &FfmpegPaths, file: &Path, t: f64) -> Vec<u8> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &t.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}

fn pixel(rgb: &[u8], width: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let i = (y * width + x) * 3;
    (rgb[i], rgb[i + 1], rgb[i + 2])
}

#[test]
fn photo_frame_is_overlaid_with_margin() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("intro-frame");
    let main = dir.join("src/main.mp4");
    make_clip(&paths, &main, &Clip::default());
    // 320x240 的红色相框：四边 16 像素不透明，中间透明
    let frame = dir.join("assets/frame.png");
    generate(
        &paths,
        &frame,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240,format=rgba,\
             geq=r=255:g=0:b=0:a='if(between(X,16,303)*between(Y,16,223),0,255)'",
            "-frames:v",
            "1",
        ],
    );
    // 间距 10%：画面缩到 256x192，左右各 32、上下各 24 像素
    let params = IntroOutroParams {
        enabled: true,
        frame_enabled: true,
        frame: file(&frame),
        frame_margin: Range::fixed(10.0),
        ..IntroOutroParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    let s = streams(&paths, &output);
    assert_close(s.video, 2.0, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.05, "音视频时长差");

    let rgb = frame_rgb(&paths, &output, 1.0);
    assert_eq!(rgb.len(), 320 * 240 * 3);
    let (r, g, b) = pixel(&rgb, 320, 6, 6);
    assert!(
        r > 180 && g < 80 && b < 80,
        "相框边缘应为红色：{:?}",
        (r, g, b)
    );
    let (r, g, b) = pixel(&rgb, 320, 24, 120);
    assert!(
        r < 50 && g < 50 && b < 50,
        "间距处应为黑底：{:?}",
        (r, g, b)
    );
    let (r, g, b) = pixel(&rgb, 320, 160, 120);
    assert!(
        !(r > 180 && g < 80 && b < 80) && u32::from(r) + u32::from(g) + u32::from(b) > 60,
        "中间应为画面内容：{:?}",
        (r, g, b)
    );
}

#[test]
fn very_short_main_with_directory_intro() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("intro-short");
    let main = dir.join("src/short.mp4");
    make_clip(
        &paths,
        &main,
        &Clip {
            seconds: 0.4,
            ..Clip::default()
        },
    );
    make_clip(
        &paths,
        &dir.join("intros/a.mp4"),
        &Clip {
            width: 640,
            height: 480,
            fps: 24,
            seconds: 0.5,
            audio: false,
        },
    );
    let params = IntroOutroParams {
        enabled: true,
        intro_enabled: true,
        intro: AssetSource::Directory {
            path: dir.join("intros").to_string_lossy().into_owned(),
            recursive: false,
            order: PickOrder::Sequential,
        },
        ..IntroOutroParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    let s = streams(&paths, &output);
    assert_close(s.video, 0.9, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
}
