//! 集成测试：`tool.reverse` 视频倒放。素材是“帧计数”视频：第 N 帧的亮度 Y = 16 + 2N，
//! 解码输出的 Y 平面即可还原每一帧原来的序号；音频只在第 1 秒有声音。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::jobs::JobEvent;
use cliprecipe_lib::pipeline::modules::{BitrateMode, BitrateParams};
use cliprecipe_lib::tools::ToolRunConfig;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 100 帧（25fps、4 秒）的帧计数视频，音频只在第 1 秒是 440 Hz。
fn make_counter(paths: &FfmpegPaths, out: &Path, audio: bool) {
    make_lavfi(
        paths,
        out,
        "color=c=black:s=64x64:r=25:d=4,format=yuv420p,geq=lum='16+2*N':cb=128:cr=128",
        audio.then_some("sine=frequency=440:duration=4,volume='if(lt(t,1),1,0)':eval=frame"),
        &[],
    );
}

/// 每一帧 Y 平面的平均值还原出的原始帧序号。
fn frame_indices(paths: &FfmpegPaths, file: &Path) -> Vec<i64> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-vf", "extractplanes=y", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(64 * 64)
        .map(|f| {
            let mean = f.iter().map(|v| f64::from(*v)).sum::<f64>() / f.len() as f64;
            ((mean - 16.0) / 2.0).round() as i64
        })
        .collect()
}

/// 某段时间内的平均音量（dB），没有声音时为 -200。
fn mean_volume(paths: &FfmpegPaths, file: &Path, start: f64, duration: f64) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-ss",
            &start.to_string(),
            "-t",
            &duration.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-vn", "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .map(|v| {
            v.trim()
                .trim_end_matches("dB")
                .trim()
                .parse()
                .unwrap_or(-200.0)
        })
        .unwrap_or_else(|| panic!("没有音量信息：{text}"))
}

fn config(dir: &TempDir) -> ToolRunConfig {
    let paths = ffmpeg_paths();
    let mut config = tool_config(&paths, &dir.join("out"), &dir.join("tmp"));
    // 纯色素材源码率太低，用固定码率保证亮度准确
    config.bitrate = BitrateParams {
        enabled: true,
        mode: BitrateMode::Fixed,
        ..BitrateParams::default()
    };
    config
}

fn run(dir: &TempDir, out: &str, input: &Path, params: Value) -> PathBuf {
    let mut config = config(dir);
    config.output_dir = Some(dir.join(out));
    let (summary, c) = run_tool(
        config,
        registered("tool.reverse", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

#[test]
fn chunked_reverse_equals_simple_reverse() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("reverse-chunks");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, true);
    assert_eq!(frame_indices(&paths, &input), (0..100).collect::<Vec<_>>());

    let simple = run(&dir, "simple", &input, json!({"chunkSeconds": 10}));
    // 1 秒一段 → 4 段，切点在第 25、50、75 帧之前
    let chunked = run(&dir, "chunked", &input, json!({"chunkSeconds": 1}));
    let expected: Vec<i64> = (0..100).rev().collect();
    assert_eq!(frame_indices(&paths, &simple), expected, "一次倒放");
    assert_eq!(frame_indices(&paths, &chunked), expected, "分段倒放");

    for file in [&simple, &chunked] {
        let s = streams(&paths, file);
        assert_eq!(s.frames, 100);
        assert_close(s.audio.unwrap(), 4.0, 0.1, "音频时长");
        // 原来第 1 秒的声音倒放到最后 1 秒
        let start = mean_volume(&paths, file, 0.0, 0.9);
        let end = mean_volume(&paths, file, 3.1, 0.85);
        assert!(
            end > -30.0 && start < -60.0,
            "{}：开头 {start} dB，结尾 {end} dB",
            file.display()
        );
    }
}

#[test]
fn remove_audio_and_odd_chunk_length() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("reverse-mute");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, true);
    // 0.7 秒一段（18 帧）：段长不整除总帧数
    let out = run(
        &dir,
        "out",
        &input,
        json!({"removeAudio": true, "chunkSeconds": 0.7}),
    );
    assert_eq!(stream_count(&paths, &out, "a"), 0, "去掉了音频");
    assert_eq!(
        frame_indices(&paths, &out),
        (0..100).rev().collect::<Vec<_>>()
    );
}

#[test]
fn cancel_while_reversing_chunks_leaves_nothing() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("reverse-cancel");
    let input = dir.join("src/long.mp4");
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=30",
        Some("sine=frequency=440:duration=30"),
        &[],
    );
    let c = collector();
    let handle = start_tool(
        config(&dir),
        registered("tool.reverse", json!({"chunkSeconds": 0.5})),
        &[video_task("t1", &input)],
        &Default::default(),
        &c,
    );
    c.wait_for(
        120,
        |e| matches!(e, JobEvent::Progress { round, .. } if *round >= 3),
    );
    handle.stop();
    let summary = handle.wait();
    assert_eq!(
        (summary.done, summary.cancelled),
        (0, 1),
        "{:#?}",
        c.snapshot()
    );
    assert!(all_files(&dir.join("out")).is_empty(), "没有输出");
    assert!(all_files(&dir.join("tmp")).is_empty(), "没有残留的分段文件");
}
