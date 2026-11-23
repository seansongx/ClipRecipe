//! 集成测试：`tool.remove-still` 静止帧移除——定格的片段连同对应音频被移除，输出里再也检测不到静止画面，
//! 音视频等长；没有静止画面时原样输出并提示。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{CancelToken, FfmpegPaths};
use cliprecipe_lib::jobs::JobEvent;
use cliprecipe_lib::tools::analysis::{run_analysis, FreezeDetect};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, input: &Path, params: Value) -> (PathBuf, Vec<String>) {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.remove-still", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    let logs = c
        .snapshot()
        .into_iter()
        .filter_map(|e| match e {
            JobEvent::Log { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    (outputs[0].clone(), logs)
}

fn freezes(paths: &FfmpegPaths, file: &Path) -> Vec<(f64, f64)> {
    let info = probe(paths, file);
    run_analysis(
        &paths.ffmpeg,
        file,
        &info,
        &FreezeDetect::default(),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap()
    .iter()
    .map(|i| (i.start, i.end))
    .collect()
}

#[test]
fn frozen_stretch_and_its_audio_are_removed() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remove-still");
    let input = dir.join("src/freeze.mp4");
    // 0～2 秒运动，2～5 秒定格（复制最后一帧），5～7 秒运动；音频全程
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=2,tpad=stop_mode=clone:stop_duration=3[a];\
         testsrc2=s=320x240:r=25:d=2[b];[a][b]concat",
        Some("sine=frequency=440:duration=7"),
        &[],
    );
    assert_eq!(freezes(&paths, &input).len(), 1, "测试素材有一段静止");
    let (output, logs) = run(&dir, &input, json!({}));
    assert!(
        logs.iter().any(|m| m.contains("检测到 1 段静止画面")),
        "{logs:#?}"
    );
    let s = streams(&paths, &output);
    // 定格约 3 秒（有损编码后前几帧仍有细微变化，检测到的静止从 2 秒后稍晚开始）
    let detected = freezes(&paths, &input);
    let frozen = detected[0].1 - detected[0].0;
    assert!((2.7..=3.05).contains(&frozen), "{detected:?}");
    assert_close(s.video, 7.0 - frozen, 0.05, "去掉静止画面");
    assert_close(s.audio.expect("保留音频"), s.video, 0.06, "音视频等长");
    assert!(
        freezes(&paths, &output).is_empty(),
        "输出里不应再有静止画面"
    );
}

#[test]
fn nothing_frozen_keeps_the_whole_video() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remove-still-none");
    let input = dir.join("moving.mp4");
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=3",
        Some("sine=duration=3"),
        &[],
    );
    let (output, logs) = run(&dir, &input, json!({"minDuration": 1.5}));
    assert!(
        logs.iter().any(|m| m.contains("没有检测到静止画面")),
        "{logs:#?}"
    );
    let s = streams(&paths, &output);
    assert_close(s.video, 3.0, 0.05, "原样输出");
    assert_close(s.audio.unwrap(), 3.0, 0.06, "音频");
}
