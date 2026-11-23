//! 集成测试：`pipe.crop` 真实运行 ffmpeg，用 ffprobe 断言输出尺寸。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::crop::{CropMode, CropParams, Sides};
use common::*;

const SEED: u64 = 7;

/// 只启用裁剪模块跑一次，返回输出文件路径。
fn run_crop(paths: &FfmpegPaths, dir: &TempDir, input: &Path, params: CropParams) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.crop = CropParams {
        enabled: true,
        ..params
    };
    let out_dir = dir.join("out");
    let _ = std::fs::remove_dir_all(&out_dir);
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out_dir),
            seed: SEED,
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

fn output_size(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let info = probe(paths, file);
    let v = info.video.expect("输出应有视频流");
    (v.width, v.height)
}

fn margins(left: u32, top: u32, right: u32, bottom: u32) -> CropParams {
    CropParams {
        mode: CropMode::Margins,
        margins: Sides {
            left,
            top,
            right,
            bottom,
        },
        ..CropParams::default()
    }
}

fn aspect(w: u32, h: u32) -> CropParams {
    CropParams {
        mode: CropMode::Aspect,
        aspect_width: w,
        aspect_height: h,
        ..CropParams::default()
    }
}

#[test]
fn margins_percent_and_aspect_on_landscape() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("crop-landscape");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 320,
            height: 240,
            seconds: 1.0,
            ..Clip::default()
        },
    );

    let out = run_crop(&paths, &dir, &input, margins(10, 20, 30, 40));
    assert_eq!(output_size(&paths, &out), (280, 180));
    // 音频原样保留
    assert_eq!(probe(&paths, &out).audio.len(), 1);

    // 奇数边距：左上角与宽高都向下取偶
    let out = run_crop(&paths, &dir, &input, margins(11, 7, 0, 0));
    assert_eq!(output_size(&paths, &out), (310, 234));

    let percent = CropParams {
        mode: CropMode::Percent,
        percent: Sides::all(10.0),
        ..CropParams::default()
    };
    let out = run_crop(&paths, &dir, &input, percent);
    assert_eq!(output_size(&paths, &out), (256, 192));

    let out = run_crop(&paths, &dir, &input, aspect(1, 1));
    assert_eq!(output_size(&paths, &out), (240, 240));
    // 240 * 9 / 16 = 135 → 134
    let out = run_crop(&paths, &dir, &input, aspect(9, 16));
    assert_eq!(output_size(&paths, &out), (134, 240));
    let out = run_crop(&paths, &dir, &input, aspect(21, 9));
    assert_eq!(output_size(&paths, &out), (320, 136));
}

#[test]
fn random_mode_is_reproducible_and_bounded() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("crop-random");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 320,
            height: 240,
            seconds: 1.0,
            audio: false,
            ..Clip::default()
        },
    );
    let params = CropParams {
        mode: CropMode::Random,
        random_percent: Sides {
            left: Range::new(0.0, 20.0),
            top: Range::new(5.0, 15.0),
            right: Range::new(0.0, 20.0),
            bottom: Range::new(10.0, 30.0),
        },
        ..CropParams::default()
    };
    let out = run_crop(&paths, &dir, &input, params.clone());
    let (w, h) = output_size(&paths, &out);
    // 同一种子再跑一次，结果相同
    let again = run_crop(&paths, &dir, &input, params);
    assert_eq!(output_size(&paths, &again), (w, h));
    assert!(w % 2 == 0 && h % 2 == 0);
    assert!((192..=320).contains(&w), "宽 {w}");
    assert!((132..=204).contains(&h), "高 {h}");
}

#[test]
fn portrait_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("crop-portrait");
    let input = dir.join("src/portrait.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 240,
            height: 320,
            seconds: 1.0,
            ..Clip::default()
        },
    );
    let out = run_crop(&paths, &dir, &input, aspect(1, 1));
    assert_eq!(output_size(&paths, &out), (240, 240));
    let out = run_crop(&paths, &dir, &input, aspect(16, 9));
    assert_eq!(output_size(&paths, &out), (240, 134));
    let out = run_crop(&paths, &dir, &input, margins(0, 40, 0, 40));
    assert_eq!(output_size(&paths, &out), (240, 240));
}

#[test]
fn odd_sized_source_becomes_even() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("crop-odd");
    let input = dir.join("src/odd.mkv");
    make_odd_clip(&paths, &input, 321, 241);
    assert_eq!(output_size(&paths, &input), (321, 241));

    // 四边为 0 也要裁成偶数，否则 yuv420p 无法编码
    let out = run_crop(&paths, &dir, &input, margins(0, 0, 0, 0));
    assert_eq!(output_size(&paths, &out), (320, 240));
    let out = run_crop(&paths, &dir, &input, margins(3, 3, 3, 3));
    assert_eq!(output_size(&paths, &out), (316, 236));
    let out = run_crop(&paths, &dir, &input, aspect(4, 3));
    assert_eq!(output_size(&paths, &out), (320, 240));
}

#[test]
fn oversized_crop_fails_the_task() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("crop-invalid");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    let mut recipe = Recipe::disabled();
    recipe.modules.crop = CropParams {
        enabled: true,
        ..margins(200, 0, 200, 0)
    };
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(dir.join("out")),
            seed: SEED,
        },
        vec![TaskSpec {
            id: "bad".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    match c.final_state("bad") {
        Some(JobEvent::TaskState {
            error: Some(error), ..
        }) => assert!(error.contains("超出画面"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert!(dir.file_names("out").is_empty());
}
