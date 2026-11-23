//! 集成测试：`pipe.grid` 宫格分屏，真实运行 ffmpeg 后用 ffprobe / 像素取样断言。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::grid::GridDirection;
use cliprecipe_lib::pipeline::modules::GridParams;
use common::*;

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(paths: &FfmpegPaths, params: GridParams, input: &Path, out_dir: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.grid = params;
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

/// 区域内像素亮度的标准差（用来判断是否被虚化：越模糊越小）。
fn luma_detail(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &time.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("crop={w}:{h}:{x}:{y},format=gray"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let px: Vec<f64> = output.stdout.iter().map(|b| f64::from(*b)).collect();
    // 相邻像素差的平均值：锐利画面边缘多，虚化后变小
    let mut sum = 0.0;
    for row in px.chunks(w as usize) {
        for pair in row.windows(2) {
            sum += (pair[0] - pair[1]).abs();
        }
    }
    sum / px.len() as f64
}

fn size_of(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let video = probe(paths, file).video.unwrap();
    (video.width, video.height)
}

#[test]
fn three_grid_landscape_stacks_vertically_with_blurred_ends() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("grid-3");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = GridParams {
        enabled: true,
        count: 3,
        blur_ends: true,
        blur: Range::fixed(10.0),
        ..GridParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    // 320×240 竖排 3 格：缩放 320/720 → 每格 142×106，输出 142×318
    assert_eq!(size_of(&paths, &output), (142, 318));
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    assert_eq!(info.audio.len(), 1);
    let top = luma_detail(&paths, &output, 0.5, (0, 0, 142, 106));
    let middle = luma_detail(&paths, &output, 0.5, (0, 106, 142, 106));
    let bottom = luma_detail(&paths, &output, 0.5, (0, 212, 142, 106));
    assert!(
        middle > top * 1.5 && middle > bottom * 1.5,
        "中间清晰、两端虚化：{top} / {middle} / {bottom}"
    );
}

#[test]
fn four_grid_on_portrait_without_audio_repeats_picture() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("grid-4");
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
    let params = GridParams {
        enabled: true,
        count: 4,
        ..GridParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    assert_eq!(size_of(&paths, &output), (240, 320));
    assert!(info.audio.is_empty());
    let a = avg_rgb(&paths, &output, 0.5, (10, 10, 100, 140));
    let b = avg_rgb(&paths, &output, 0.5, (130, 170, 100, 140));
    assert!(
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 12),
        "四格应是同一画面：{a:?} / {b:?}"
    );
}

#[test]
fn forced_direction_and_odd_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("grid-odd");
    let input = dir.join("src/odd.mp4");
    make_odd_clip(&paths, &input, 321, 241);
    assert_eq!(size_of(&paths, &input), (321, 241));
    let two = GridParams {
        enabled: true,
        count: 2,
        ..GridParams::default()
    };
    let output = run_job(&paths, two, &input, &dir.join("out1"));
    // 竖排：缩放 321/482 → 每格 212×160，输出 212×320
    assert_eq!(size_of(&paths, &output), (212, 320));

    let nine = GridParams {
        enabled: true,
        count: 9,
        direction: GridDirection::Horizontal,
        blur_ends: true,
        ..GridParams::default()
    };
    let output = run_job(&paths, nine, &input, &dir.join("out2"));
    // 每格 106×80，输出 318×240
    assert_eq!(size_of(&paths, &output), (318, 240));
}
