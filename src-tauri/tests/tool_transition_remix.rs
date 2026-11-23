//! 集成测试：`tool.transition-remix` 转场重组——场景之间加转场，输出因重叠变短，转场中间的画面是两段的混合，
//! 音视频等长；随机转场；片段不足两段时原样输出。

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
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.transition-remix", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    (outputs[0].clone(), log_messages(&c))
}

fn assert_av_durations(paths: &FfmpegPaths, file: &Path, expected: f64) {
    let s = streams(paths, file);
    assert_close(s.video, expected, 0.05, "视频时长");
    let audio = s.audio.expect("应有音频");
    assert_close(audio, expected, 0.06, "音频时长");
    assert_close(audio, s.video, 0.06, "音视频等长");
}

#[test]
fn fade_between_scenes_blends_and_shortens_by_overlaps() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("transition-fade");
    let input = dir.join("src/scenes.mp4");
    colour_scenes(
        &paths,
        &input,
        &[("red", 2.0), ("lime", 2.0), ("blue", 2.0)],
        true,
    );
    let (output, logs) = run(
        &dir,
        &input,
        json!({"transition": "fade", "transitionSeconds": 0.5}),
    );
    assert!(
        logs.iter().any(|m| m.contains("共 3 段，转场：fade、fade")),
        "{logs:#?}"
    );
    // 6 秒 − 2 处 × 0.5 秒
    assert_av_durations(&paths, &output, 5.0);

    let pure = |t: f64| avg_rgb(&paths, &output, t, CENTRE);
    let red = pure(0.5);
    assert!(red[0] > 220 && red[1] < 40, "第一段红色：{red:?}");
    let lime = pure(2.5);
    assert!(lime[1] > 220 && lime[0] < 40, "第二段绿色：{lime:?}");
    let blue = pure(4.5);
    assert!(blue[2] > 220 && blue[0] < 40, "第三段蓝色：{blue:?}");
    // 第一个转场在输出的 1.5～2.0 秒：中间是红绿混合
    let mid = pure(1.76);
    assert!(
        (70..=190).contains(&mid[0]) && (70..=190).contains(&mid[1]) && mid[2] < 50,
        "转场中间应是红绿混合：{mid:?}"
    );
    // 第二个转场在 3.0～3.5 秒：绿蓝混合
    let mid2 = pure(3.24);
    assert!(
        (70..=190).contains(&mid2[1]) && (70..=190).contains(&mid2[2]) && mid2[0] < 50,
        "第二个转场应是绿蓝混合：{mid2:?}"
    );
}

#[test]
fn random_transitions_are_reproducible() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("transition-random");
    let input = dir.join("scenes.mp4");
    colour_scenes(
        &paths,
        &input,
        &[("red", 1.5), ("lime", 1.5), ("blue", 1.5), ("white", 1.5)],
        true,
    );
    let params = json!({"transitionSeconds": 0.5, "minSegmentSeconds": 1});
    let (first, logs) = run(&dir, &input, params.clone());
    let chosen = logs
        .iter()
        .find(|m| m.contains("转场："))
        .cloned()
        .unwrap_or_else(|| panic!("{logs:#?}"));
    assert!(chosen.contains("共 4 段"), "{chosen}");
    // 6 秒 − 3 处 × 0.5 秒
    assert_av_durations(&paths, &first, 4.5);
    std::fs::remove_file(&first).unwrap();
    let (_, again) = run(&dir, &input, params);
    assert!(again.contains(&chosen), "同一种子选出同样的转场：{chosen}");
}

#[test]
fn too_short_video_is_output_without_transition() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("transition-short");
    let input = dir.join("short.mp4");
    colour_scenes(&paths, &input, &[("red", 1.5)], true);
    let (output, logs) = run(&dir, &input, json!({}));
    assert!(
        logs.iter().any(|m| m.contains("没有检测到场景切换")),
        "{logs:#?}"
    );
    assert!(logs.iter().any(|m| m.contains("无法添加转场")), "{logs:#?}");
    assert_av_durations(&paths, &output, 1.5);
}
