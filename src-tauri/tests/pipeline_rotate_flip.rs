//! 集成测试：`pipe.rotate-flip` 旋转与翻转，真实运行 ffmpeg 后用 ffprobe / 像素取样断言。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::rotate_flip::{AngleFit, Rotation};
use cliprecipe_lib::pipeline::modules::RotateFlipParams;
use common::*;

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(paths: &FfmpegPaths, params: RotateFlipParams, input: &Path, out_dir: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.rotate_flip = params;
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

fn close(a: [i32; 3], b: [i32; 3], tolerance: i32) -> bool {
    a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= tolerance)
}

fn size_of(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let video = probe(paths, file).video.unwrap();
    (video.width, video.height)
}

#[test]
fn rotate_90_and_flip_transposes_picture() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rotate-90");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = RotateFlipParams {
        enabled: true,
        rotation: Rotation::Cw90,
        hflip: true,
        ..RotateFlipParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    assert_eq!(size_of(&paths, &output), (240, 320));
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    assert_eq!(info.audio.len(), 1);
    // 顺时针 90° 再水平翻转 = 转置：输出 (x, y) 处是源的 (y, x)
    let out = avg_rgb(&paths, &output, 0.5, (180, 20, 40, 40));
    let src = avg_rgb(&paths, &input, 0.5, (20, 180, 40, 40));
    assert!(close(out, src, 24), "输出 {out:?}，源 {src:?}");
}

#[test]
fn show_all_grows_canvas_with_black_corners() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rotate-show-all");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = RotateFlipParams {
        enabled: true,
        random_angle: true,
        angle: Range::fixed(30.0),
        fit: AngleFit::ShowAll,
        ..RotateFlipParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    // 320×240 旋转 30°：|320cos|+|240sin| = 397.1 → 398，|320sin|+|240cos| = 367.8 → 368
    assert_eq!(size_of(&paths, &output), (398, 368));
    let corner = avg_rgb(&paths, &output, 0.5, (0, 0, 12, 12));
    assert!(corner.iter().all(|c| *c < 30), "四角应为黑色：{corner:?}");
}

#[test]
fn remove_black_keeps_size_on_portrait_clip_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rotate-remove-black");
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
    let params = RotateFlipParams {
        enabled: true,
        random_angle: true,
        angle: Range::new(10.0, 20.0),
        fit: AngleFit::RemoveBlack,
        ..RotateFlipParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    assert_eq!(size_of(&paths, &output), (240, 320));
    assert!(info.audio.is_empty());
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    // 去除黑边后四角不应是纯黑
    let corner = avg_rgb(&paths, &output, 0.5, (0, 0, 6, 6));
    assert!(corner.iter().any(|c| *c > 40), "四角不应为黑色：{corner:?}");
}

#[test]
fn odd_dimensions_are_handled() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rotate-odd");
    let input = dir.join("src/odd.mp4");
    make_odd_clip(&paths, &input, 321, 241);
    assert_eq!(size_of(&paths, &input), (321, 241));
    // 去除黑边：重新缩放时取偶数
    let remove_black = RotateFlipParams {
        enabled: true,
        rotation: Rotation::Cw270,
        random_angle: true,
        angle: Range::fixed(5.0),
        ..RotateFlipParams::default()
    };
    let output = run_job(&paths, remove_black, &input, &dir.join("out1"));
    assert_eq!(size_of(&paths, &output), (240, 320));

    let show_all = RotateFlipParams {
        enabled: true,
        random_angle: true,
        angle: Range::fixed(-15.0),
        fit: AngleFit::ShowAll,
        ..RotateFlipParams::default()
    };
    let output = run_job(&paths, show_all, &input, &dir.join("out2"));
    // |321cos15|+|241sin15| = 372.4 → 374，|321sin15|+|241cos15| = 315.9 → 316
    assert_eq!(size_of(&paths, &output), (374, 316));
}
