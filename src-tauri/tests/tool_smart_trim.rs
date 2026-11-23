//! 集成测试：`tool.smart-trim` 智能去头尾——在范围内的场景切换点处去掉片头、片尾；范围内没有切换点时
//! 那一端不动并在日志里说明；只处理一端。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, input: &Path, params: Value) -> (PathBuf, Vec<String>) {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.smart-trim", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    (outputs[0].clone(), log_messages(&c))
}

fn assert_av(paths: &FfmpegPaths, file: &Path, expected: f64) {
    let s = streams(paths, file);
    assert_close(s.video, expected, 0.05, "视频时长");
    assert_close(s.audio.expect("应有音频"), expected, 0.06, "音频时长");
}

/// 片头 1.6 秒红、正片 4 秒绿、片尾 1.6 秒蓝（都是整帧数）：切换点在 1.6 与 5.6 秒
const SCENES: [(&str, f64); 3] = [("red", 1.6), ("lime", 4.0), ("blue", 1.6)];

#[test]
fn head_and_tail_are_cut_at_scene_changes() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("smart-trim");
    let input = dir.join("src/show.mp4");
    colour_scenes(&paths, &input, &SCENES, true);
    let (output, logs) = run(
        &dir,
        &input,
        json!({"maxHeadSeconds": 3, "maxTailSeconds": 3}),
    );
    assert!(
        logs.iter().any(|m| m.contains("片头：去掉开头 1.6 秒")),
        "{logs:#?}"
    );
    assert!(
        logs.iter().any(|m| m.contains("片尾：去掉 5.6 秒之后")),
        "{logs:#?}"
    );
    assert_av(&paths, &output, 4.0);
    assert_colour(
        avg_rgb(&paths, &output, 0.0, CENTRE),
        [0, 255, 0],
        "第一帧已是正片",
    );
    assert_colour(
        avg_rgb(&paths, &output, 3.95, CENTRE),
        [0, 255, 0],
        "最后一帧仍是正片",
    );
}

#[test]
fn ends_without_a_cut_in_range_are_left_alone() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("smart-trim-none");
    let input = dir.join("show.mp4");
    colour_scenes(&paths, &input, &SCENES, true);
    let (output, logs) = run(
        &dir,
        &input,
        json!({"maxHeadSeconds": 1, "maxTailSeconds": 1}),
    );
    assert!(logs.iter().any(|m| m.contains("开头保持不变")), "{logs:#?}");
    assert!(logs.iter().any(|m| m.contains("结尾保持不变")), "{logs:#?}");
    assert_av(&paths, &output, 7.2);
    assert_colour(
        avg_rgb(&paths, &output, 0.5, CENTRE),
        [255, 0, 0],
        "片头还在",
    );

    // 只去片头
    let (head_only, logs) = run(
        &dir,
        &input,
        json!({"maxHeadSeconds": 3, "maxTailSeconds": 0}),
    );
    assert!(!logs.iter().any(|m| m.contains("片尾")), "{logs:#?}");
    assert_av(&paths, &head_only, 5.6);
    assert_colour(
        avg_rgb(&paths, &head_only, 5.0, CENTRE),
        [0, 0, 255],
        "片尾还在",
    );
}
