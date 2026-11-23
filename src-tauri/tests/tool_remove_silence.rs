//! 集成测试：`tool.remove-silence` 静音移除——移除模式输出里不再有静音、时长 = 有声部分；保留边距；
//! 加速模式静音按倍率变短；音视频始终等长；没有音频的源明确失败。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{CancelToken, FfmpegPaths};
use cliprecipe_lib::jobs::{JobEvent, TaskState};
use cliprecipe_lib::tools::analysis::{run_analysis, Interval, SilenceDetect};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.remove-silence", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

fn silences(paths: &FfmpegPaths, file: &Path, min_duration: f64) -> Vec<Interval> {
    let info = probe(paths, file);
    run_analysis(
        &paths.ffmpeg,
        file,
        &info,
        &SilenceDetect {
            noise_db: -30.0,
            min_duration,
        },
        &CancelToken::new(),
        |_| {},
    )
    .unwrap()
}

/// 6 秒：0～2 秒有声、2～4 秒静音、4～6 秒有声。
fn speech_with_gap(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "testsrc2=s=320x240:r=25:d=6",
        Some("aevalsrc='if(between(t,2,4),0,0.5*sin(440*2*PI*t))':d=6:s=48000"),
        &[],
    );
}

fn assert_av(paths: &FfmpegPaths, file: &Path, expected: f64) {
    let s = streams(paths, file);
    assert_close(s.video, expected, 0.06, "视频时长");
    let audio = s.audio.expect("应有音频");
    assert_close(audio, expected, 0.06, "音频时长");
    assert_close(audio, s.video, 0.06, "音视频等长");
}

#[test]
fn silence_is_removed() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("silence-remove");
    let input = dir.join("src/talk.mp4");
    speech_with_gap(&paths, &input);
    assert_eq!(silences(&paths, &input, 0.5).len(), 1, "测试素材有一段静音");
    let output = run(&dir, &input, json!({"paddingSeconds": 0}));
    assert_av(&paths, &output, 4.0);
    let left = silences(&paths, &output, 0.2);
    assert!(left.is_empty(), "输出里不应再有静音：{left:?}");
}

#[test]
fn padding_keeps_a_little_silence_around_speech() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("silence-padding");
    let input = dir.join("talk.mp4");
    speech_with_gap(&paths, &input);
    let output = run(&dir, &input, json!({"paddingSeconds": 0.25}));
    // 2 秒静音两端各留 0.25 秒：只去掉 1.5 秒
    assert_av(&paths, &output, 4.5);
    let left = silences(&paths, &output, 0.2);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_close(left[0].duration(), 0.5, 0.08, "保留的静音");
    assert_close(left[0].start, 2.0, 0.08, "保留的静音位置");
}

#[test]
fn silence_is_sped_up() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("silence-speed");
    let input = dir.join("talk.mp4");
    speech_with_gap(&paths, &input);
    let output = run(
        &dir,
        &input,
        json!({"mode": "speed", "speed": 4, "paddingSeconds": 0}),
    );
    // 2 秒静音 4 倍速 → 0.5 秒
    assert_av(&paths, &output, 4.5);
    let s = streams(&paths, &output);
    assert_close(s.fps, 25.0, 0.01, "加速后帧率回到源帧率");
    let left = silences(&paths, &output, 0.2);
    assert_eq!(left.len(), 1, "{left:?}");
    assert_close(left[0].duration(), 0.5, 0.08, "加速后的静音");
    assert!(silences(&paths, &output, 0.8).is_empty(), "长静音已被压短");
}

#[test]
fn source_without_audio_fails_clearly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("silence-mute");
    let input = dir.join("mute.mp4");
    make_lavfi(&paths, &input, "testsrc2=s=160x120:r=25:d=1", None, &[]);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.remove-silence", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(matches!(
        c.final_state("t1"),
        Some(JobEvent::TaskState { state: TaskState::Failed, error: Some(ref e), .. }) if e.contains("没有音轨")
    ));
    assert!(all_files(&out).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
