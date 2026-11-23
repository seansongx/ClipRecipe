//! 集成测试：`tool.split-remix` 拆分重组——在拆分点插入其它视频（尺寸、帧率、音频规格不同，统一成主视频规格；
//! 没有音频的插入片段补静音），目录依次取，未选择插入视频时失败。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{CancelToken, FfmpegPaths};
use cliprecipe_lib::jobs::{JobEvent, TaskState};
use cliprecipe_lib::tools::analysis::{run_analysis, SilenceDetect};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.split-remix", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

/// 主视频：320x240、25fps 纯色，带 440 Hz 立体声音频。
fn main_clip(paths: &FfmpegPaths, out: &Path, colour: &str, secs: f64) {
    make_lavfi(
        paths,
        out,
        &format!("color=c={colour}:s=320x240:r=25:d={secs},format=yuv420p"),
        Some(&format!(
            "sine=frequency=440:duration={secs},aformat=channel_layouts=stereo"
        )),
        &[],
    );
}

fn colour_at(paths: &FfmpegPaths, file: &Path, t: f64) -> [i32; 3] {
    avg_rgb(paths, file, t, CENTRE)
}

#[test]
fn inserted_clip_is_conformed_and_appears_at_the_split_point() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-remix-file");
    let input = dir.join("src/main.mp4");
    main_clip(&paths, &input, "red", 4.0);
    // 插入：160x120、30fps、没有音频
    let insert = dir.join("assets/blue.mp4");
    make_lavfi(
        &paths,
        &insert,
        "color=c=blue:s=160x120:r=30:d=1,format=yuv420p",
        None,
        &[],
    );
    let output = run(
        &dir,
        &input,
        json!({
            "positionMode": "parts",
            "parts": 2,
            "insert": {"mode": "file", "path": insert.to_string_lossy()}
        }),
    );
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240), "统一成主视频尺寸");
    let s = streams(&paths, &output);
    assert_close(s.video, 5.0, 0.05, "4 秒主视频 + 1 秒插入");
    assert_close(s.fps, 25.0, 0.01, "统一成主视频帧率");
    assert_close(s.audio.expect("保留音频"), 5.0, 0.06, "音频时长");

    assert_colour(colour_at(&paths, &output, 1.0), RED, "拆分点之前");
    assert_colour(colour_at(&paths, &output, 2.5), BLUE, "插入的视频");
    assert_colour(colour_at(&paths, &output, 4.0), RED, "拆分点之后");
    // 插入片段缩放后居中，上下留黑边（160x120 → 320x240 正好铺满；这里检查左上角也是蓝色）
    assert_colour(
        avg_rgb(&paths, &output, 2.5, (0, 0, 20, 20)),
        BLUE,
        "等比放大铺满",
    );

    // 插入片段没有音频：补的是静音，位置正好在 2～3 秒
    let silences = run_analysis(
        &paths.ffmpeg,
        &output,
        &info,
        &SilenceDetect {
            noise_db: -40.0,
            min_duration: 0.3,
        },
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert_eq!(silences.len(), 1, "{silences:?}");
    assert_close(silences[0].start, 2.0, 0.06, "静音开始");
    assert_close(silences[0].end, 3.0, 0.06, "静音结束");
}

#[test]
fn directory_inserts_are_taken_in_order_at_each_time() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-remix-dir");
    let input = dir.join("main.mp4");
    main_clip(&paths, &input, "red", 4.0);
    let assets = dir.join("assets");
    // 第一个带单声道 22.05 kHz 音频、竖屏；第二个没有音频
    make_lavfi(
        &paths,
        &assets.join("a_lime.mp4"),
        "color=c=lime:s=120x160:r=30:d=1,format=yuv420p",
        Some("sine=frequency=880:sample_rate=22050:duration=1"),
        &[],
    );
    make_lavfi(
        &paths,
        &assets.join("b_yellow.mkv"),
        "color=c=yellow:s=320x240:r=25:d=1,format=yuv420p",
        None,
        &[],
    );
    std::fs::write(assets.join("note.txt"), b"not a video").unwrap();
    let output = run(
        &dir,
        &input,
        json!({
            "positionMode": "times",
            "times": [3, 1, 99],
            "insert": {
                "mode": "directory",
                "path": assets.to_string_lossy(),
                "recursive": false,
                "order": "sequential"
            }
        }),
    );
    let s = streams(&paths, &output);
    assert_close(s.video, 6.0, 0.05, "4 秒 + 两段各 1 秒");
    assert_close(s.audio.expect("保留音频"), 6.0, 0.06, "音频时长");
    assert_eq!(s.sample_rate, Some(44100), "统一成主视频采样率");
    for (t, colour, what) in [
        (0.5, RED, "开头"),
        (1.5, LIME, "1 秒处插入第一个"),
        (3.0, RED, "中间"),
        (4.5, YELLOW, "3 秒处插入第二个"),
        (5.5, RED, "结尾"),
    ] {
        assert_colour(colour_at(&paths, &output, t), colour, what);
    }
    // 竖屏插入片段：左右补黑边
    let edge = avg_rgb(&paths, &output, 1.5, (0, 100, 20, 20));
    assert!(edge.iter().all(|c| *c < 40), "左右应有黑边：{edge:?}");
}

#[test]
fn missing_insert_fails_and_leaves_nothing() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("split-remix-missing");
    let input = dir.join("main.mp4");
    main_clip(&paths, &input, "red", 2.0);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.split-remix", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(matches!(
        c.final_state("t1"),
        Some(JobEvent::TaskState { state: TaskState::Failed, error: Some(ref e), .. }) if e.contains("请选择要插入的视频")
    ));
    assert!(all_files(&out).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
