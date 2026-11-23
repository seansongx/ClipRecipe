//! 集成测试：`tool.split` 视频分割——按时长 / 段数 / 场景（重编码）与无损分割，片段时长之和等于源视频。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::jobs::{JobEvent, TaskState};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn split(dir: &TempDir, input: &Path, params: Value) -> Vec<PathBuf> {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.split", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    // 进度单调不减，最后到 100
    let percents: Vec<f64> = c
        .snapshot()
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress { percent, .. } => Some(*percent),
            _ => None,
        })
        .collect();
    assert!(
        percents.windows(2).all(|w| w[1] >= w[0] - 1e-9),
        "{percents:?}"
    );
    assert_eq!(percents.last().copied(), Some(100.0));
    let outputs = done_outputs(&c, "t1");
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs
}

fn durations(outputs: &[PathBuf]) -> Vec<f64> {
    let paths = ffmpeg_paths();
    outputs.iter().map(|f| video_duration(&paths, f)).collect()
}

fn assert_pieces(outputs: &[PathBuf], expected: &[f64], tolerance: f64, total: f64) {
    let actual = durations(outputs);
    assert_eq!(actual.len(), expected.len(), "片段数：{actual:?}");
    for (a, e) in actual.iter().zip(expected) {
        assert_close(*a, *e, tolerance, "片段时长");
    }
    assert_close(actual.iter().sum(), total, 0.1, "片段时长之和");
}

#[test]
fn by_duration_reencodes_into_stem_folder() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-duration");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 5.0,
            ..Clip::default()
        },
    );
    let outputs = split(
        &dir,
        &input,
        json!({"mode": "duration", "segmentSeconds": 2}),
    );
    let folder = dir.join("out").join("clip");
    assert_eq!(
        outputs,
        [
            folder.join("clip_01.mp4"),
            folder.join("clip_02.mp4"),
            folder.join("clip_03.mp4")
        ]
    );
    assert_pieces(&outputs, &[2.0, 2.0, 1.0], 0.05, 5.0);
    let info = probe(&paths, &outputs[0]);
    assert_eq!(info.audio.len(), 1, "保留音频");
    assert_eq!(info.video.unwrap().codec, "h264");
}

#[test]
fn by_count_splits_evenly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-count");
    let input = dir.join("clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 6.0,
            audio: false,
            ..Clip::default()
        },
    );
    let outputs = split(&dir, &input, json!({"mode": "count", "count": 3}));
    assert_pieces(&outputs, &[2.0, 2.0, 2.0], 0.05, 6.0);
}

#[test]
fn by_scene_cuts_at_hard_scene_changes() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-scene");
    let input = dir.join("scenes.mp4");
    make_scene_clip(&paths, &input, 2.0);
    let outputs = split(&dir, &input, json!({"mode": "scene"}));
    assert_pieces(&outputs, &[2.0, 2.0, 2.0], 0.05, 6.0);
    // 每段的画面来自不同的源：第二段（smptebars）左上角是灰白色竖条
    let first = avg_rgb(&paths, &outputs[1], 0.5, (0, 0, 20, 20));
    assert!(first.iter().all(|c| *c > 150), "第二段应是彩条：{first:?}");
}

#[test]
fn lossless_split_lands_on_keyframes_and_sums_to_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-lossless");
    let input = dir.join("gop.mkv");
    // 每秒一个关键帧
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=5",
        Some("sine=duration=5"),
        &["-g", "25", "-keyint_min", "25", "-sc_threshold", "0"],
    );
    let outputs = split(
        &dir,
        &input,
        json!({"mode": "duration", "segmentSeconds": 2, "lossless": true}),
    );
    let folder = dir.join("out").join("gop");
    assert_eq!(
        outputs,
        [
            folder.join("gop_01.mp4"),
            folder.join("gop_02.mp4"),
            folder.join("gop_03.mp4")
        ]
    );
    assert_pieces(&outputs, &[2.0, 2.0, 1.0], 0.1, 5.0);
    for piece in &outputs {
        assert_eq!(codecs(&paths, piece), ["h264", "aac"]);
    }
}

#[test]
fn failed_split_leaves_nothing_behind() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-fail");
    let out = dir.join("out");
    let missing = dir.join("missing.mp4");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.split", json!({"mode": "count", "count": 2})),
        &[video_task("t1", &missing)],
    );
    assert_eq!(summary.failed, 1);
    assert!(matches!(
        c.final_state("t1"),
        Some(JobEvent::TaskState { state: TaskState::Failed, error: Some(ref e), .. }) if e.contains("不存在")
    ));
    assert!(all_files(&out).is_empty());
}
