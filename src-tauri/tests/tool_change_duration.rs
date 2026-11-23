//! 集成测试：`tool.change-duration` 改变视频时长——只改写 mvhd：mvhd 记录的总时长等于目标（ffprobe 不读 mvhd，仍为真实时长），音视频数据包逐字节不变，
//! 能完整解码出原有帧数，源文件不变，范围值按种子可复现，不支持的容器干净地失败。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::tools::change_duration::read_mvhd;
use common::*;
use serde_json::json;
use tool_support::*;

fn packets_md5(paths: &FfmpegPaths, file: &Path, map: &str) -> String {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-map", map, "-c", "copy", "-f", "hash", "-hash", "md5", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 完整解码第一条视频流得到的帧数。
fn decoded_frames(paths: &FfmpegPaths, file: &Path) -> usize {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-xerror", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-f", "framecrc", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count()
}

fn container_duration(paths: &FfmpegPaths, file: &Path) -> f64 {
    probe(paths, file).duration
}

/// 输出文件 mvhd 记录的总时长（秒）。
fn header_duration(file: &Path) -> f64 {
    let mvhd = read_mvhd(file).unwrap();
    mvhd.duration as f64 / f64::from(mvhd.timescale)
}

/// 2 秒 25fps 带音频的视频，按扩展名封装；`faststart` 时 moov 在文件开头。
fn make_source(paths: &FfmpegPaths, out: &Path, faststart: bool) {
    let extra: &[&str] = if faststart {
        &["-movflags", "+faststart"]
    } else {
        &[]
    };
    make_lavfi(
        paths,
        out,
        "testsrc2=s=320x240:r=25:d=2",
        Some("sine=frequency=440:duration=2"),
        extra,
    );
}

fn run(
    paths: &FfmpegPaths,
    dir: &TempDir,
    out: &str,
    input: &Path,
    params: serde_json::Value,
) -> PathBuf {
    let out = dir.join(out);
    let (summary, c) = run_tool(
        tool_config(paths, &out, &dir.join("tmp")),
        registered("tool.change-duration", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    outputs.into_iter().next().unwrap()
}

fn assert_content_unchanged(paths: &FfmpegPaths, source: &Path, output: &Path) {
    assert_eq!(
        packets_md5(paths, output, "0:v:0"),
        packets_md5(paths, source, "0:v:0"),
        "视频数据包不变"
    );
    assert_eq!(
        packets_md5(paths, output, "0:a:0"),
        packets_md5(paths, source, "0:a:0"),
        "音频数据包不变"
    );
    assert_eq!(decoded_frames(paths, output), 50, "能完整解码出全部 50 帧");
    // FFmpeg 9.1 按采样表计算时长、不看 mvhd：ffprobe 仍报告真实时长（见 change_duration.rs 的说明）
    assert_eq!(
        container_duration(paths, output),
        container_duration(paths, source)
    );
    let (s, o) = (streams(paths, source), streams(paths, output));
    assert_eq!(
        (o.video, o.frames),
        (s.video, s.frames),
        "视频流时长与帧数不变"
    );
    assert_eq!(o.audio, s.audio, "音频流时长不变");
    assert_eq!(
        std::fs::metadata(output).unwrap().len(),
        std::fs::metadata(source).unwrap().len(),
        "文件大小不变（只改了几个字节）"
    );
}

#[test]
fn multiplier_on_mp4_with_moov_at_the_end() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("duration-mp4");
    let input = dir.join("src/clip.mp4");
    make_source(&paths, &input, false);
    let before = std::fs::read(&input).unwrap();
    let real = container_duration(&paths, &input);
    let output = run(
        &paths,
        &dir,
        "out",
        &input,
        json!({"mode": "multiplier", "multiplier": {"min": 3, "max": 3}}),
    );
    assert_eq!(output, dir.join("out/clip.mp4"));
    assert_close(
        header_duration(&output),
        (real * 3.0 * 1000.0).round() / 1000.0,
        0.002,
        "mvhd 时长 = 3 倍",
    );
    assert_content_unchanged(&paths, &input, &output);
    assert_eq!(std::fs::read(&input).unwrap(), before, "源文件不变");
    assert!(all_files(&dir.join("tmp")).is_empty());
}

#[test]
fn absolute_range_on_mov_with_moov_first_is_reproducible() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("duration-mov");
    let input = dir.join("src/clip.mov");
    make_source(&paths, &input, true);
    let params = json!({"mode": "absolute", "seconds": {"min": 100, "max": 200}});
    let first = run(&paths, &dir, "out1", &input, params.clone());
    let second = run(&paths, &dir, "out2", &input, params);
    let (a, b) = (header_duration(&first), header_duration(&second));
    assert_eq!(a, b, "同一种子、同一序号结果相同");
    assert!((100.0..=200.0).contains(&a), "{a}");
    assert_ne!(a, 100.0, "确实在范围内随机取值");
    assert_content_unchanged(&paths, &input, &first);

    let fixed = run(
        &paths,
        &dir,
        "out3",
        &input,
        json!({"mode": "absolute", "seconds": {"min": 0.5, "max": 0.5}}),
    );
    assert_close(header_duration(&fixed), 0.5, 0.002, "改短");
    assert_content_unchanged(&paths, &input, &fixed);
}

#[test]
fn unsupported_container_fails_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("duration-mkv");
    let input = dir.join("clip.mkv");
    make_source(&paths, &input, false);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.change-duration", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    let message = format!("{:?}", c.final_state("t1"));
    assert!(message.contains("只支持 MP4 / MOV"), "{message}");
    assert!(all_files(&out).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
