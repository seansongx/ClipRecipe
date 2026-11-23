//! 集成测试：`tool.remove-remix` 移除重组——在设定位置移除指定长度，输出时长等于保留部分之和，
//! 画面内容与音视频时长对得上；范围值长度；没有音频的源；全部移除时失败。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::jobs::{JobEvent, TaskState};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.remove-remix", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

const SCENES: [(&str, f64); 3] = [("red", 2.0), ("lime", 2.0), ("blue", 2.0)];

#[test]
fn removes_the_given_length_at_each_time() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remove-remix-times");
    let input = dir.join("src/scenes.mp4");
    colour_scenes(&paths, &input, &SCENES, true);
    // 移除 1～2 秒、4～5 秒：保留 [0,1] 红、[2,4] 绿、[5,6] 蓝
    let output = run(
        &dir,
        &input,
        json!({"positionMode": "times", "times": [4, 1], "length": {"min": 1, "max": 1}}),
    );
    let s = streams(&paths, &output);
    assert_close(s.video, 4.0, 0.05, "视频时长 = 保留部分之和");
    assert_eq!(s.frames, 100);
    let audio = s.audio.expect("保留音频");
    assert_close(audio, 4.0, 0.06, "音频时长");
    for (t, colour, what) in [
        (0.5, [255, 0, 0], "红"),
        (1.5, [0, 255, 0], "绿（原 2～3 秒）"),
        (2.9, [0, 255, 0], "绿（原 3.9 秒）"),
        (3.5, [0, 0, 255], "蓝（原 5.5 秒）"),
    ] {
        assert_colour(avg_rgb(&paths, &output, t, CENTRE), colour, what);
    }
}

#[test]
fn percent_positions_with_random_length_and_no_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remove-remix-percent");
    let input = dir.join("mute.mp4");
    colour_scenes(&paths, &input, &SCENES, false);
    let output = run(
        &dir,
        &input,
        json!({"positionMode": "percents", "percents": [50], "length": {"min": 0.5, "max": 1.5}}),
    );
    assert_eq!(stream_count(&paths, &output, "a"), 0, "源没有音频");
    let d = video_duration(&paths, &output);
    assert!((4.45..=5.55).contains(&d), "移除 0.5～1.5 秒：{d}");
    // 3 秒处之前保持红、绿
    assert_colour(
        avg_rgb(&paths, &output, 2.5, CENTRE),
        [0, 255, 0],
        "移除点之前",
    );
    // 同一种子同样的长度
    std::fs::remove_file(&output).unwrap();
    let again = run(
        &dir,
        &input,
        json!({"positionMode": "percents", "percents": [50], "length": {"min": 0.5, "max": 1.5}}),
    );
    assert_close(video_duration(&paths, &again), d, 0.001, "可复现");
}

#[test]
fn removing_everything_fails() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remove-remix-all");
    let input = dir.join("scenes.mp4");
    colour_scenes(&paths, &input, &SCENES, true);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.remove-remix",
            json!({"positionMode": "times", "times": [0], "length": {"min": 10, "max": 10}}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(matches!(
        c.final_state("t1"),
        Some(JobEvent::TaskState { state: TaskState::Failed, error: Some(ref e), .. }) if e.contains("没有剩余")
    ));
    assert!(all_files(&out).is_empty());
}
