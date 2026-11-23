//! 集成测试：`tool.insert-frame` 插帧——覆盖与插入两种方式，逐帧确认图片恰好出现 N 帧、位置正确。
//!
//! 素材是灰色的“亮度计数器”（第 N 帧亮度为 16 + 3N），插入的图片是纯蓝色：输出每一帧的平均颜色
//! 要么是蓝色（图片），要么是灰色（可还原出源视频的第几帧）。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 一帧的内容：图片，或源视频的第几帧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Image,
    Source(i64),
}
use Shown::*;

/// 2 秒、25fps 的亮度计数器。
fn make_counter(paths: &FfmpegPaths, out: &Path, size: (u32, u32), audio: bool) {
    make_luma_counter(paths, out, size, 25, 2.0, audio);
}

fn make_image(paths: &FfmpegPaths, out: &Path, size: (u32, u32)) {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let status = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("color=c=blue:s={}x{}", size.0, size.1))
        .args(["-frames:v", "1"])
        .arg(out)
        .status()
        .unwrap();
    assert!(status.success());
}

/// 每一帧中区域的平均颜色 → 图片或源帧序号。
fn shown(paths: &FfmpegPaths, file: &Path, region: Rect) -> Vec<Shown> {
    frame_colours(paths, file, Some(region))
        .into_iter()
        .map(|[r, g, b]| {
            if i32::from(b) > i32::from(r) + 100 {
                Image
            } else {
                Source(counter_index(g))
            }
        })
        .collect()
}

fn sources(range: std::ops::Range<i64>) -> Vec<Shown> {
    range.map(Source).collect()
}

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.insert-frame", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

#[test]
fn overlay_covers_exactly_n_frames() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("insert-frame-overlay");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), true);
    let image = dir.join("assets/blue.png");
    make_image(&paths, &image, (40, 30));
    let out = run(
        &dir,
        &input,
        json!({"image": {"mode": "file", "path": image}, "positions": "1", "frames": 3}),
    );
    let mut expected = sources(0..25);
    expected.extend([Image; 3]);
    expected.extend(sources(28..50));
    assert_eq!(shown(&paths, &out, (0, 0, 160, 120)), expected);
    let s = streams(&paths, &out);
    assert_close(s.video, 2.0, 0.05, "覆盖方式时长不变");
    assert_close(s.audio.unwrap(), 2.0, 0.1, "音频保留");
}

#[test]
fn insert_adds_n_frames_and_silence() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("insert-frame-insert");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), true);
    let image = dir.join("assets/blue.png");
    make_image(&paths, &image, (40, 30));
    let out = run(
        &dir,
        &input,
        json!({"image": {"mode": "file", "path": image}, "positions": "1", "frames": 3, "mode": "insert"}),
    );
    let mut expected = sources(0..25);
    expected.extend([Image; 3]);
    expected.extend(sources(25..50));
    assert_eq!(shown(&paths, &out, (0, 0, 160, 120)), expected);
    let s = streams(&paths, &out);
    assert_eq!(s.frames, 53);
    assert_close(s.video, 2.12, 0.05, "视频变长 3 帧");
    assert_close(s.audio.unwrap(), 2.12, 0.1, "音频同样变长");
    let samples = audio_samples(&paths, &out);
    assert!(rms(&samples, 1.02, 1.1) < 0.003, "插入的帧期间静音");
    assert!(rms(&samples, 0.3, 0.95) > 0.05);
    assert!(rms(&samples, 1.2, 2.0) > 0.05);
}

#[test]
fn fit_pads_with_background() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("insert-frame-fit");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), false);
    // 竖图 30×60 完整显示：缩放为 60×120 居中，两侧填白色背景
    let image = dir.join("assets/tall.png");
    make_image(&paths, &image, (30, 60));
    let out = run(
        &dir,
        &input,
        json!({"image": {"mode": "file", "path": image}, "positions": "50%", "frames": 2, "background": "#ffffff"}),
    );
    let centre = shown(&paths, &out, (60, 0, 40, 120));
    assert_eq!(centre[25..27], [Image, Image]);
    assert_eq!(centre[27], Source(27));
    let side = avg_rgb(&paths, &out, 1.02, (0, 0, 20, 120));
    assert!(side.iter().all(|c| *c > 230), "两侧为白色背景：{side:?}");
}

#[test]
fn several_positions_on_odd_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("insert-frame-odd");
    let input = dir.join("src/portrait.mp4");
    make_counter(&paths, &input, (121, 161), false);
    let image = dir.join("assets/blue.png");
    make_image(&paths, &image, (40, 30));
    let out = run(
        &dir,
        &input,
        json!({
            "image": {"mode": "directory", "path": dir.join("assets"), "recursive": false, "order": "random"},
            "positions": "0 1.5",
            "frames": 2,
            "fit": "fill",
            "mode": "insert"
        }),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (120, 160));
    assert_eq!(stream_count(&paths, &out, "a"), 0);
    let mut expected = vec![Image; 2];
    expected.extend(sources(0..38));
    expected.extend([Image; 2]);
    expected.extend(sources(38..50));
    assert_eq!(shown(&paths, &out, (0, 0, 120, 160)), expected);
}

#[test]
fn missing_image_fails_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("insert-frame-missing");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), false);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.insert-frame", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
