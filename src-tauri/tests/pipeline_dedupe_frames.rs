//! 集成测试：`pipe.dedupe-frames` 重复帧移除，真实运行 ffmpeg 后用 ffprobe 检查帧数、时长与音画同步。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Recipe, Settings};
use cliprecipe_lib::pipeline::modules::DedupeFramesParams;
use common::*;

/// 生成每个画面重复 5 次的视频：5fps 内容按 25fps 输出（每秒 25 帧，只有 5 个不同画面）。
fn make_duplicated_clip(paths: &FfmpegPaths, out: &Path, seconds: f64, audio: bool) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ])
    .arg(format!(
        "testsrc2=size=320x240:rate=5:duration={seconds},fps=25"
    ));
    if audio {
        cmd.args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"));
    }
    cmd.args([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ]);
    if audio {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    let output = cmd.arg(out).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 只启用重复帧移除，处理一个文件；返回输出路径。
fn run(paths: &FfmpegPaths, input: &Path, out: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.dedupe_frames = DedupeFramesParams {
        enabled: true,
        ..DedupeFramesParams::default()
    };
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 3,
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

#[test]
fn duplicate_frames_are_removed_with_audio_in_sync() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("dedupe-dup");
    let input = dir.join("src/dup.mp4");
    make_duplicated_clip(&paths, &input, 2.0, true);
    let output = run(&paths, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    // 50 帧里只有 10 个不同画面
    assert!((10..=15).contains(&s.frames), "帧数 {}", s.frames);
    // 时间戳保持原样：总时长与音频不变；视频流最多比音频短末尾那组重复帧（0.16 秒）
    assert_close(s.format, 2.0, 0.05, "总时长");
    assert_close(s.audio.unwrap(), 2.0, 0.03, "音频时长");
    assert!(
        s.video > 2.0 - 0.2 - 0.01 && s.video <= 2.05,
        "视频时长 {}",
        s.video
    );
}

#[test]
fn clip_without_duplicates_or_audio_is_unchanged() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("dedupe-unique");
    let input = dir.join("src/mute.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    let output = run(&paths, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert!(s.audio.is_none());
    // testsrc2 每帧都在变化，几乎不应去掉帧
    assert!(s.frames >= 45, "帧数 {}", s.frames);
    assert_close(s.video, 2.0, 0.05, "视频时长");
}

#[test]
fn very_short_duplicated_clip_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("dedupe-short");
    let input = dir.join("src/short.mp4");
    // 0.4 秒 10 帧，只有 2 个不同画面
    make_duplicated_clip(&paths, &input, 0.4, false);
    let output = run(&paths, &input, &dir.join("out"));
    let s = streams(&paths, &output);
    assert!(s.audio.is_none());
    assert!((2..=4).contains(&s.frames), "帧数 {}", s.frames);
    assert!(s.format > 0.2 && s.format <= 0.45, "总时长 {}", s.format);
}
