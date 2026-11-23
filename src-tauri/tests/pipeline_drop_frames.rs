//! 集成测试：`pipe.drop-frames` 抽帧，真实运行 ffmpeg 后用 ffprobe 检查时长、帧数与音画同步。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::DropFramesParams;
use common::*;

/// 只启用抽帧，处理一个文件；返回输出路径。
fn run(paths: &FfmpegPaths, params: DropFramesParams, input: &Path, out: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.drop_frames = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 5,
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

fn params(interval: u32, sync_audio: bool) -> DropFramesParams {
    DropFramesParams {
        enabled: true,
        interval: Range::fixed(interval),
        sync_audio,
    }
}

#[test]
fn drops_frames_and_speeds_up_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("drop-sync");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 4.0,
            ..Clip::default()
        },
    );
    // 100 帧每 5 帧丢 1 帧 → 80 帧 = 3.2 秒
    let output = run(&paths, params(5, true), &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_eq!(s.frames, 80);
    assert_close(s.video, 3.2, 0.02, "视频时长");
    assert_close(s.format, 3.2, 0.05, "总时长");
    assert_close(s.audio.unwrap(), s.video, 0.05, "音视频时长差");
}

#[test]
fn without_audio_sync_timeline_is_kept() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("drop-nosync");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default()); // 2 秒 50 帧
    let output = run(&paths, params(5, false), &input, &dir.join("out"));
    let s = streams(&paths, &output);
    // 帧被去掉，但剩下的帧保留原时间戳：时长、音频不变
    assert_eq!(s.frames, 40);
    assert_close(s.format, 2.0, 0.05, "总时长");
    assert_close(s.video, 2.0, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), 2.0, 0.03, "音频时长");
}

#[test]
fn clip_without_audio_and_ranged_interval() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("drop-mute");
    let input = dir.join("src/mute.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 2.0,
            fps: 30,
            audio: false,
            ..Clip::default()
        },
    );
    let p = DropFramesParams {
        enabled: true,
        interval: Range::new(3, 6),
        sync_audio: true,
    };
    let output = run(&paths, p, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert!(s.audio.is_none());
    // 60 帧，N ∈ [3, 6] → 剩 40～50 帧，时长 = 帧数 / 30
    assert!((40..=50).contains(&s.frames), "帧数 {}", s.frames);
    assert_close(s.video, s.frames as f64 / 30.0, 0.02, "视频时长");
}

#[test]
fn very_short_clip() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("drop-short");
    let input = dir.join("src/short.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 0.4,
            ..Clip::default()
        },
    );
    // 10 帧每 2 帧丢 1 帧 → 5 帧
    let output = run(&paths, params(2, true), &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_eq!(s.frames, 5);
    assert_close(s.video, 0.2, 0.02, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.05, "音视频时长差");
}
