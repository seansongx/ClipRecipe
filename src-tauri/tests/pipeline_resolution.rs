//! 集成测试：`pipe.resolution` 分辨率，真实运行 ffmpeg 后用 ffprobe / 像素取样断言。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::resolution::{FillMode, ResolutionMode, ScaleMode};
use cliprecipe_lib::pipeline::modules::ResolutionParams;
use common::*;

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(paths: &FfmpegPaths, params: ResolutionParams, input: &Path, out_dir: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.resolution = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out_dir.to_path_buf()),
            seed: 7,
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
    assert_eq!(
        summary,
        RunSummary {
            done: 1,
            failed: 0,
            cancelled: 0
        },
        "{:#?}",
        c.snapshot()
    );
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成，实际：{other:?}"),
    }
}

fn brightness(rgb: [i32; 3]) -> i32 {
    rgb.iter().sum::<i32>() / 3
}

fn size_of(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let video = probe(paths, file).video.unwrap();
    (video.width, video.height)
}

fn custom(width: u32, height: u32) -> ResolutionParams {
    ResolutionParams {
        enabled: true,
        mode: ResolutionMode::Custom,
        width,
        height,
        ..ResolutionParams::default()
    }
}

#[test]
fn standard_height_keeps_aspect() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("res-standard");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = ResolutionParams {
        enabled: true,
        standard_height: 360,
        ..ResolutionParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    assert_eq!(size_of(&paths, &output), (480, 360));
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    assert_eq!(info.audio.len(), 1);
}

#[test]
fn fit_with_blurred_background_fills_bars() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("res-blur");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let output = run_job(&paths, custom(360, 640), &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (360, 640));
    // 上方空白是虚化的画面，不是黑色
    let top = avg_rgb(&paths, &output, 0.5, (0, 0, 360, 150));
    assert!(brightness(top) > 30, "上方空白应为画面背景：{top:?}");
}

#[test]
fn fit_black_with_reflection() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("res-reflection");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = ResolutionParams {
        fill: FillMode::Black,
        reflection: true,
        ..custom(360, 640)
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (360, 640));
    // 画面 360×270 位于 y=184；下方 186 像素是倒影，从 50% 渐隐到 0
    let top = avg_rgb(&paths, &output, 0.5, (0, 0, 360, 150));
    let below = avg_rgb(&paths, &output, 0.5, (0, 456, 360, 24));
    let bottom = avg_rgb(&paths, &output, 0.5, (0, 630, 360, 10));
    assert!(brightness(top) < 20, "上方应为黑色：{top:?}");
    assert!(
        brightness(below) > brightness(bottom) + 10,
        "倒影应从上往下渐隐：{below:?} / {bottom:?}"
    );
    assert!(brightness(bottom) < 25, "倒影末端应接近黑色：{bottom:?}");
}

#[test]
fn stretch_on_portrait_clip_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("res-stretch");
    let input = dir.join("src/portrait.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 240,
            height: 320,
            audio: false,
            ..Clip::default()
        },
    );
    let params = ResolutionParams {
        scale_mode: ScaleMode::Stretch,
        ..custom(400, 300)
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    assert_eq!(size_of(&paths, &output), (400, 300));
    assert!(info.audio.is_empty());
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
}

#[test]
fn odd_source_becomes_even() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("res-odd");
    let input = dir.join("src/odd.mp4");
    make_odd_clip(&paths, &input, 321, 241);
    assert_eq!(size_of(&paths, &input), (321, 241));
    let params = ResolutionParams {
        enabled: true,
        standard_height: 240,
        ..ResolutionParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out1"));
    assert_eq!(size_of(&paths, &output), (320, 240));

    let blur = ResolutionParams {
        blur_strength: Range::new(5.0, 15.0),
        ..custom(301, 401)
    };
    let output = run_job(&paths, blur, &input, &dir.join("out2"));
    assert_eq!(size_of(&paths, &output), (300, 400));
}
