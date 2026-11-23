//! 集成测试：`tool.gif` 生成 GIF——尺寸、帧率、帧数、截取一段；高质量模式（两步调色板）与快速模式
//! 输出不同，且颜色更接近原片。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, out: &str, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join(out), &dir.join("tmp")),
        registered("tool.gif", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(
        all_files(&dir.join("tmp")).is_empty(),
        "临时文件（调色板）已清理"
    );
    outputs[0].clone()
}

/// 实际解码出的帧数。
fn decoded_frames(paths: &FfmpegPaths, file: &Path) -> u64 {
    let output = ffmpeg::command(&paths.ffprobe)
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap()
}

/// 第一帧（可先经过滤镜）的 RGB 像素。
fn first_frame(paths: &FfmpegPaths, file: &Path, filter: &str) -> Vec<u8> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("{filter}format=rgb24"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(x.abs_diff(*y)))
        .sum::<f64>()
        / a.len() as f64
}

#[test]
fn fast_gif_has_expected_size_rate_and_frames() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("gif-fast");
    let input = dir.join("src/clip.mp4");
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=4",
        Some("sine=frequency=440:duration=4"),
        &[],
    );
    let out = run(
        &dir,
        "out",
        &input,
        json!({"fps": 10, "width": 160, "start": 1, "duration": 2}),
    );
    assert_eq!(out.file_name().unwrap(), "clip.gif");
    assert_eq!(codecs(&paths, &out), ["gif"]);
    let info = probe(&paths, &out);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (160, 120));
    assert_close(video.fps, 10.0, 0.01, "帧率");
    assert_eq!(decoded_frames(&paths, &out), 20, "2 秒 × 10fps");
    assert_close(info.duration, 2.0, 0.11, "时长");

    // 默认参数：整段、480 宽（源只有 320 宽时放大到 480）
    let whole = run(&dir, "whole", &input, json!({}));
    let video = probe(&paths, &whole).video.unwrap();
    assert_eq!((video.width, video.height), (480, 360));
    assert_eq!(decoded_frames(&paths, &whole), 40);
}

#[test]
fn high_quality_differs_and_is_closer_to_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("gif-hq");
    let input = dir.join("src/gradient.mp4");
    // 平滑的彩色渐变：固定调色板会出现明显色带
    make_lavfi(
        &paths,
        &input,
        "gradients=s=320x240:r=25:d=1:c0=0x203060:c1=0xe0a040:x0=0:y0=0:x1=320:y1=240:speed=0",
        None,
        &["-crf", "10"],
    );
    let params = |hq: bool| json!({"fps": 10, "width": 320, "highQuality": hq});
    let fast = run(&dir, "fast", &input, params(false));
    let hq = run(&dir, "hq", &input, params(true));
    assert_eq!(decoded_frames(&paths, &hq), 10);
    let source = first_frame(&paths, &input, "");
    let fast_px = first_frame(&paths, &fast, "");
    let hq_px = first_frame(&paths, &hq, "");
    assert_ne!(fast_px, hq_px, "两种模式的输出不同");
    let (fast_err, hq_err) = (
        mean_abs_diff(&source, &fast_px),
        mean_abs_diff(&source, &hq_px),
    );
    assert!(
        hq_err < fast_err * 0.7,
        "高质量误差 {hq_err:.2} 应明显小于快速误差 {fast_err:.2}"
    );
}

#[test]
fn start_beyond_end_fails_without_outputs() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("gif-bad");
    let input = dir.join("src/clip.mp4");
    make_lavfi(&paths, &input, "testsrc2=s=160x120:r=25:d=1", None, &[]);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.gif", json!({"start": 5})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
