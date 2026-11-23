//! 集成测试：`pipe.trim` 去头尾，真实运行 ffmpeg 后用 ffprobe 检查时长、帧数与音画同步。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::trim::TrimMode;
use cliprecipe_lib::pipeline::modules::TrimParams;
use common::*;

/// 只启用去头尾，处理一个文件；返回输出路径与全部事件。
fn run(
    paths: &FfmpegPaths,
    params: TrimParams,
    input: &Path,
    out: &Path,
) -> (PathBuf, Vec<JobEvent>) {
    let mut recipe = Recipe::disabled();
    recipe.modules.trim = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 11,
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

#[test]
fn head_and_tail_are_cut_frame_accurately_in_sync() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("trim-head-tail");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 4.0,
            ..Clip::default()
        },
    );
    let params = TrimParams {
        enabled: true,
        mode: TrimMode::HeadTail,
        head: Range::fixed(0.6),
        tail: Range::fixed(1.0),
        ..TrimParams::default()
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    // 4 − 0.6 − 1.0 = 2.4 秒 = 60 帧（25fps）
    assert_close(s.format, 2.4, 0.05, "总时长");
    assert_close(s.video, 2.4, 0.04, "视频时长");
    assert!((59..=61).contains(&s.frames), "帧数 {}", s.frames);
    assert_close(s.audio.unwrap(), s.video, 0.05, "音视频时长差");
    assert_eq!(dir.file_names("out"), ["clip.mp4"]);
}

#[test]
fn segment_of_clip_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("trim-segment");
    let input = dir.join("src/mute.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 4.0,
            fps: 30,
            audio: false,
            ..Clip::default()
        },
    );
    let params = TrimParams {
        enabled: true,
        mode: TrimMode::Segment,
        start: Range::fixed(1.0),
        length: Range::fixed(1.5),
        ..TrimParams::default()
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 1.5, 0.04, "视频时长");
    assert!((44..=46).contains(&s.frames), "帧数 {}", s.frames);
    assert!(s.audio.is_none());
}

#[test]
fn min_duration_floor_reduces_the_cut() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("trim-floor");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default()); // 2 秒
                                                 // 去 1 + 1 秒会剩 0；下限 1.5 秒 → 各去 0.25 秒
    let params = TrimParams {
        enabled: true,
        head: Range::fixed(1.0),
        tail: Range::fixed(1.0),
        min_duration: 1.5,
        ..TrimParams::default()
    };
    let (output, _) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 1.5, 0.04, "视频时长");
    assert_close(s.audio.unwrap(), s.video, 0.05, "音视频时长差");
}

#[test]
fn very_short_clip_below_floor_is_left_untouched_with_warning() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("trim-short");
    let input = dir.join("src/short.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 0.8,
            ..Clip::default()
        },
    );
    let params = TrimParams {
        enabled: true,
        head: Range::fixed(0.2),
        tail: Range::fixed(0.2),
        min_duration: 3.0,
        ..TrimParams::default()
    };
    let (output, events) = run(&paths, params, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert_close(s.video, 0.8, 0.04, "视频时长");
    assert_eq!(s.frames, 20);
    assert!(
        events.iter().any(|e| matches!(
            e,
            JobEvent::Log { message, .. } if message.contains("[pipe.trim]") && message.contains("最短时长")
        )),
        "{events:#?}"
    );
}
