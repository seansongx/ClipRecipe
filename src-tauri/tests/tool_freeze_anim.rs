//! 集成测试：`tool.freeze-anim` 定帧动画。
//!
//! 素材是“亮度计数器”：第 N 帧整幅画面的亮度为 16 + 3N（全部帧互不相同，最多 66 帧），所以输出每一帧的
//! 平均亮度就能还原出它来自源视频的第几帧，可以逐帧断言定帧效果。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 每一帧来自源视频的第几帧（由平均亮度还原）。
fn source_frames(paths: &FfmpegPaths, file: &Path) -> Vec<i64> {
    frame_lumas(paths, file, None)
        .into_iter()
        .map(counter_index)
        .collect()
}

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.freeze-anim", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

fn repeat(value: i64, times: usize) -> Vec<i64> {
    vec![value; times]
}

#[test]
fn stop_motion_shows_first_frame_of_each_interval() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-anim-stop");
    let input = dir.join("src/counter.mp4");
    make_luma_counter(&paths, &input, (160, 120), 15, 4.0, true);
    assert_eq!(source_frames(&paths, &input), (0..60).collect::<Vec<_>>());

    let out = run(
        &dir,
        &input,
        json!({"mode": "stopMotion", "spacing": "count", "count": 4}),
    );
    assert_eq!(out, dir.join("out").join("counter.mp4"));
    let expected: Vec<i64> = [0, 15, 30, 45]
        .iter()
        .flat_map(|f| repeat(*f, 15))
        .collect();
    assert_eq!(source_frames(&paths, &out), expected);
    let s = streams(&paths, &out);
    assert_eq!(s.frames, 60);
    assert_close(s.video, 4.0, 0.05, "视频时长不变");
    assert_close(s.audio.unwrap(), 4.0, 0.1, "音频原样保留");
    assert_eq!(codecs(&paths, &out), ["h264", "aac"]);
}

#[test]
fn stop_motion_by_interval_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-anim-interval");
    let input = dir.join("src/portrait.mp4");
    make_luma_counter(&paths, &input, (120, 200), 25, 2.0, false);
    let out = run(
        &dir,
        &input,
        json!({"spacing": "interval", "intervalSeconds": 0.5}),
    );
    let expected: Vec<i64> = [0, 13, 25, 38]
        .iter()
        .zip([13, 12, 13, 12])
        .flat_map(|(f, n)| repeat(*f, n))
        .collect();
    assert_eq!(source_frames(&paths, &out), expected);
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (120, 200));
    assert_eq!(stream_count(&paths, &out, "a"), 0);
}

#[test]
fn insert_holds_pause_with_silence() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-anim-holds");
    let input = dir.join("src/counter.mp4");
    make_luma_counter(&paths, &input, (160, 120), 15, 4.0, true);
    let out = run(
        &dir,
        &input,
        json!({"mode": "insertHolds", "spacing": "interval", "intervalSeconds": 1.5, "holdSeconds": 0.4}),
    );
    // 定帧点 0、1.5、3 秒 → 第 0、23、45 帧（15fps），各停 6 帧（0.4 秒）
    let mut expected = Vec::new();
    let mut next = 0;
    for f in [0, 23, 45] {
        expected.extend(next..=f);
        expected.extend(repeat(f, 6));
        next = f + 1;
    }
    expected.extend(next..60);
    let actual = source_frames(&paths, &out);
    assert_eq!(actual.len(), 78);
    assert_eq!(actual, expected);

    let s = streams(&paths, &out);
    assert_close(s.video, 5.2, 0.05, "视频变长 3 × 0.4 秒");
    assert_close(s.audio.unwrap(), 5.2, 0.1, "音频同样变长");
    // 停顿处（定帧那一帧之后）静音：[1/15, 1/15+0.4)、[24/15+0.4, +0.4)、[46/15+0.8, +0.4)
    let samples = audio_samples(&paths, &out);
    for a in [1.0 / 15.0, 24.0 / 15.0 + 0.4, 46.0 / 15.0 + 0.8] {
        let b = a + 0.4;
        assert!(rms(&samples, a + 0.05, b - 0.05) < 0.003, "{a}..{b} 应静音");
    }
    for (a, b) in [(0.55, 1.95), (2.45, 3.8), (4.35, 5.1)] {
        assert!(rms(&samples, a, b) > 0.05, "{a}..{b} 应有声音");
    }
}

#[test]
fn insert_holds_audio_continues_on_odd_size() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-anim-odd");
    let input = dir.join("src/odd.mp4");
    make_luma_counter(&paths, &input, (161, 121), 25, 2.0, true);
    let out = run(
        &dir,
        &input,
        json!({"mode": "insertHolds", "spacing": "count", "count": 2, "holdSeconds": 0.2, "holdAudio": "continue"}),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (160, 120));
    let mut expected: Vec<i64> = vec![0];
    expected.extend(repeat(0, 5));
    expected.extend(1..=25);
    expected.extend(repeat(25, 5));
    expected.extend(26..50);
    assert_eq!(source_frames(&paths, &out), expected);
    let s = streams(&paths, &out);
    assert_close(s.video, 2.4, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), 2.4, 0.1, "音频补静音到与视频等长");
    // 音频不停：开头 0.04～0.24 秒（画面停住时）仍有声音，结尾补的是静音
    let samples = audio_samples(&paths, &out);
    assert!(rms(&samples, 0.05, 0.2) > 0.05);
    assert!(rms(&samples, 2.1, 2.35) < 0.003);
}

#[test]
fn invalid_params_fail_without_leftovers() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-anim-bad");
    let input = dir.join("src/counter.mp4");
    make_luma_counter(&paths, &input, (160, 120), 25, 1.0, false);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.freeze-anim", json!({"spacing": "count", "count": 0})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
