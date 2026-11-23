//! 集成测试：`pipe.speed` 变速，真实运行 ffmpeg 后用 ffprobe 检查时长、帧数与音画同步。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::speed::SpeedMode;
use cliprecipe_lib::pipeline::modules::SpeedParams;
use common::*;

/// 只启用变速，处理一个文件；返回输出路径与全部事件。
fn run(
    paths: &FfmpegPaths,
    params: SpeedParams,
    input: &Path,
    out: &Path,
) -> (PathBuf, Vec<JobEvent>) {
    let mut recipe = Recipe::disabled();
    recipe.modules.speed = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 23,
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
    let output = match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成：{other:?}"),
    };
    (output, c.snapshot())
}

fn clip(dir: &TempDir, name: &str, seconds: f64, audio: bool) -> PathBuf {
    let paths = ffmpeg_paths();
    let path = dir.join(&format!("src/{name}.mp4"));
    make_clip(
        &paths,
        &path,
        &Clip {
            seconds,
            audio,
            ..Clip::default()
        },
    );
    path
}

fn fixed(rate: f64) -> SpeedParams {
    SpeedParams {
        enabled: true,
        mode: SpeedMode::Fixed,
        rate: Range::fixed(rate),
        ..SpeedParams::default()
    }
}

#[test]
fn fixed_speed_up_keeps_av_in_sync_and_fps() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-fixed");
    let input = clip(&dir, "clip", 3.0, true);
    let (output, _) = run(&paths, fixed(1.5), &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.format, 2.0, 0.06, "总时长");
    assert_close(s.video, 2.0, 0.05, "视频时长");
    assert!((49..=51).contains(&s.frames), "帧数 {}", s.frames);
    assert_close(s.fps, 25.0, 0.01, "帧率");
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
}

#[test]
fn atempo_chain_for_large_rate() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-chain");
    let input = clip(&dir, "clip", 3.0, true);
    let (output, _) = run(&paths, fixed(3.0), &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 1.0, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
}

#[test]
fn slow_down_with_pitch_change_keeps_sample_rate() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-pitch");
    let input = clip(&dir, "clip", 2.0, true);
    let source_rate = probe(&paths, &input).audio[0].sample_rate;
    let params = SpeedParams {
        change_pitch: true,
        ..fixed(0.5)
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 4.0, 0.05, "视频时长");
    assert!((99..=101).contains(&s.frames), "帧数 {}", s.frames);
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
    assert_eq!(s.sample_rate, Some(source_rate));
}

#[test]
fn segmented_ladder_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-ladder");
    let input = clip(&dir, "mute", 4.0, false);
    let params = SpeedParams {
        enabled: true,
        mode: SpeedMode::Segmented,
        segment_count: Range::fixed(2),
        segment_rate: Range::new(0.5, 2.0),
        shuffle: false,
        ..SpeedParams::default()
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    // 2 秒 ÷ 0.5 + 2 秒 ÷ 2 = 5 秒
    assert_close(s.video, 5.0, 0.05, "视频时长");
    assert!((124..=126).contains(&s.frames), "帧数 {}", s.frames);
    assert!(s.audio.is_none());
}

#[test]
fn segmented_random_rates_keep_av_in_sync() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-random");
    let input = clip(&dir, "clip", 4.0, true);
    let params = SpeedParams {
        enabled: true,
        mode: SpeedMode::Segmented,
        segment_count: Range::new(3, 4),
        segment_rate: Range::new(0.8, 1.6),
        shuffle: true,
        ..SpeedParams::default()
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert!(
        s.video > 4.0 / 1.6 - 0.05 && s.video < 4.0 / 0.8 + 0.05,
        "视频时长 {}",
        s.video
    );
    assert_close(s.audio.unwrap(), s.video, 0.08, "音视频时长差");
    assert_close(s.fps, 25.0, 0.5, "帧率");
}

#[test]
fn min_duration_floor_lowers_rate() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-floor");
    let input = clip(&dir, "clip", 4.0, true);
    // ×4 会得到 1 秒；下限 2 秒 → 倍率降为 2
    let params = SpeedParams {
        min_duration: 2.0,
        ..fixed(4.0)
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 2.0, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.06, "音视频时长差");
}

#[test]
fn very_short_clip_below_floor_keeps_its_length() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("speed-short");
    let input = clip(&dir, "short", 0.8, true);
    let params = SpeedParams {
        min_duration: 3.0,
        ..fixed(2.0)
    };
    let (output, events) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 0.8, 0.04, "视频时长");
    assert_eq!(s.frames, 20);
    assert!(events.iter().any(|e| matches!(
        e,
        JobEvent::Log { message, .. } if message.contains("[pipe.speed]") && message.contains("最短时长")
    )));
}
