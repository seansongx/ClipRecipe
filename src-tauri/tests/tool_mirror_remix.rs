//! 集成测试：`tool.mirror-remix` 镜像重组——按场景切换点分段、指定片段水平翻转，时长与源相同，
//! 音频连续且与画面同步；没有切换点时按固定时长分段；没有音频的源；中途取消不留文件。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::CancelToken;
use cliprecipe_lib::jobs::{JobEvent, TaskState};
use cliprecipe_lib::tools::analysis::{run_analysis, SilenceDetect};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

const LEFT: Rect = (20, 100, 40, 40);
const RIGHT: Rect = (260, 100, 40, 40);

fn run_mirror(dir: &TempDir, input: &Path, params: Value) -> (PathBuf, Collector) {
    let paths = ffmpeg_paths();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.mirror-remix", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    (outputs[0].clone(), c)
}

#[test]
fn every_second_scene_is_mirrored_and_audio_stays_continuous() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mirror-nth");
    let input = dir.join("src/scenes.mp4");
    split_colour_scenes(
        &paths,
        &input,
        &[("red", "blue"), ("lime", "yellow"), ("white", "black")],
        2.0,
        true,
    );
    let (output, c) = run_mirror(&dir, &input, json!({"flipMode": "nth", "nth": 2}));
    assert_eq!(output, dir.join("out").join("scenes.mp4"));
    assert!(
        log_messages(&c).iter().any(|m| m.contains("翻转第 2 段")),
        "{:#?}",
        log_messages(&c)
    );

    // 时长与源相同，音视频等长
    let s = streams(&paths, &output);
    assert_close(s.video, 6.0, 0.05, "视频时长");
    assert_close(s.audio.expect("保留音频"), 6.0, 0.06, "音频时长");
    assert_eq!(s.frames, 150, "帧数不变");

    // 第 1、3 段不翻转，第 2 段左右互换
    assert_colour(avg_rgb(&paths, &output, 1.0, LEFT), RED, "第 1 段左");
    assert_colour(avg_rgb(&paths, &output, 1.0, RIGHT), BLUE, "第 1 段右");
    assert_colour(
        avg_rgb(&paths, &output, 3.0, LEFT),
        YELLOW,
        "第 2 段左（翻转）",
    );
    assert_colour(
        avg_rgb(&paths, &output, 3.0, RIGHT),
        LIME,
        "第 2 段右（翻转）",
    );
    assert_colour(avg_rgb(&paths, &output, 5.0, LEFT), WHITE, "第 3 段左");
    assert_colour(avg_rgb(&paths, &output, 5.0, RIGHT), BLACK, "第 3 段右");
    // 翻转从切换点开始、到下一个切换点结束（帧级）
    assert_colour(avg_rgb(&paths, &output, 2.0, LEFT), YELLOW, "切换后第一帧");
    assert_colour(
        avg_rgb(&paths, &output, 3.95, LEFT),
        YELLOW,
        "下一切换前最后一帧（3.96 秒）",
    );
    assert_colour(
        avg_rgb(&paths, &output, 4.0, LEFT),
        WHITE,
        "下一切换后第一帧",
    );

    // 音频连续：整段 440 Hz 正弦，没有任何静音间隙
    let info = probe(&paths, &output);
    let gaps = run_analysis(
        &paths.ffmpeg,
        &output,
        &info,
        &SilenceDetect {
            noise_db: -40.0,
            min_duration: 0.02,
        },
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(gaps.is_empty(), "音频不应有间隙：{gaps:?}");
}

#[test]
fn ratio_mode_flips_half_of_the_scenes_reproducibly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mirror-ratio");
    let input = dir.join("scenes.mp4");
    let scenes = [
        ("red", "blue"),
        ("lime", "yellow"),
        ("white", "black"),
        ("blue", "red"),
    ];
    split_colour_scenes(&paths, &input, &scenes, 1.5, true);
    let flipped_at = |output: &Path| -> Vec<bool> {
        (0..4)
            .map(|i| {
                let t = 0.75 + 1.5 * i as f64;
                let left = avg_rgb(&paths, output, t, LEFT);
                let expected_left = match scenes[i].0 {
                    "red" => RED,
                    "lime" => LIME,
                    "white" => WHITE,
                    _ => BLUE,
                };
                left.iter()
                    .zip(expected_left)
                    .any(|(a, e)| (a - e).abs() > 40)
            })
            .collect()
    };
    let (first, _) = run_mirror(&dir, &input, json!({"flipMode": "ratio", "ratio": 0.5}));
    let a = flipped_at(&first);
    assert_eq!(a.iter().filter(|f| **f).count(), 2, "4 段翻转 2 段：{a:?}");
    std::fs::remove_file(&first).unwrap();
    // 同一种子结果相同
    let (second, _) = run_mirror(&dir, &input, json!({"flipMode": "ratio", "ratio": 0.5}));
    assert_eq!(flipped_at(&second), a);
    assert_close(video_duration(&paths, &second), 6.0, 0.05, "时长不变");
}

#[test]
fn without_scene_cuts_falls_back_to_fixed_segments_and_works_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mirror-fallback");
    let input = dir.join("plain.mp4");
    // 左红右蓝、没有任何场景切换、没有音频
    split_colour_scenes(&paths, &input, &[("red", "blue")], 4.0, false);
    let (output, c) = run_mirror(
        &dir,
        &input,
        json!({"flipMode": "nth", "nth": 2, "fallbackSeconds": 1}),
    );
    assert!(
        log_messages(&c)
            .iter()
            .any(|m| m.contains("没有检测到场景切换")),
        "{:#?}",
        log_messages(&c)
    );
    assert_eq!(stream_count(&paths, &output, "a"), 0);
    assert_close(video_duration(&paths, &output), 4.0, 0.05, "时长不变");
    for (t, flipped) in [(0.5, false), (1.5, true), (2.5, false), (3.5, true)] {
        let (left, right) = if flipped { (BLUE, RED) } else { (RED, BLUE) };
        assert_colour(
            avg_rgb(&paths, &output, t, LEFT),
            left,
            &format!("{t} 秒左"),
        );
        assert_colour(
            avg_rgb(&paths, &output, t, RIGHT),
            right,
            &format!("{t} 秒右"),
        );
    }
}

#[test]
fn cancelling_mid_encode_leaves_no_files() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mirror-cancel");
    let input = dir.join("long.mp4");
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=640x480:r=25:d=40",
        Some("sine=duration=40"),
        &[],
    );
    let out = dir.join("out");
    let c = collector();
    let handle = start_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.mirror-remix", json!({"fallbackSeconds": 2})),
        &[video_task("t1", &input)],
        &BTreeMap::new(),
        &c,
    );
    // 等到第 2 步（编码）开始
    c.wait_for(
        120,
        |e| matches!(e, JobEvent::Progress { round, .. } if *round >= 2),
    );
    handle.stop();
    let summary = handle.wait();
    assert_eq!(summary.cancelled, 1, "{:#?}", c.snapshot());
    assert!(matches!(
        c.final_state("t1"),
        Some(JobEvent::TaskState {
            state: TaskState::Cancelled,
            ..
        })
    ));
    assert!(
        all_files(&out).is_empty(),
        "取消后不留输出：{:?}",
        all_files(&out)
    );
    assert!(all_files(&dir.join("tmp")).is_empty(), "取消后不留临时文件");
}
