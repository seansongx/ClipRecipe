//! 集成测试：`tool.merge` 视频合并——无损（直接复制）、不兼容时重编码统一规格、转场、中间视频、每 N 个一组。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::jobs::{JobEvent, LogLevel, TaskSpec, TaskState};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn clip(
    paths: &FfmpegPaths,
    out: &Path,
    width: u32,
    height: u32,
    fps: u32,
    seconds: f64,
    audio: bool,
) {
    make_clip(
        paths,
        out,
        &Clip {
            width,
            height,
            fps,
            seconds,
            audio,
        },
    );
}

fn run_merge(
    dir: &TempDir,
    params: Value,
    videos: &[TaskSpec],
) -> (cliprecipe_lib::jobs::RunSummary, Collector) {
    let paths = ffmpeg_paths();
    run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.merge", params),
        videos,
    )
}

fn tasks(files: &[PathBuf]) -> Vec<TaskSpec> {
    files
        .iter()
        .enumerate()
        .map(|(i, f)| video_task(&format!("t{i}"), f))
        .collect()
}

/// 每个视频数据包的 MD5（`framemd5`，直接复制），用来确认无损合并没有重编码。
fn packet_hashes(paths: &FfmpegPaths, file: &Path) -> Vec<String> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-map", "0:v:0", "-c", "copy", "-f", "framemd5", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_owned()))
        .collect()
}

fn warnings(c: &Collector) -> Vec<String> {
    c.snapshot()
        .into_iter()
        .filter_map(|e| match e {
            JobEvent::Log {
                level: LogLevel::Warn,
                message,
                ..
            } => Some(message),
            _ => None,
        })
        .collect()
}

#[test]
fn lossless_merge_copies_packets_and_sums_durations() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("merge-lossless");
    let a = dir.join("src/a.mp4");
    let b = dir.join("src/b.mp4");
    clip(&paths, &a, 320, 240, 25, 2.0, true);
    clip(&paths, &b, 320, 240, 25, 3.0, true);
    let (summary, c) = run_merge(
        &dir,
        json!({"joinNames": true}),
        &tasks(&[a.clone(), b.clone()]),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "tool.merge#all");
    assert_eq!(outputs, [dir.join("out").join("a b.mp4")]);
    let out = &outputs[0];
    assert_close(video_duration(&paths, out), 5.0, 0.05, "合并后时长");
    assert_eq!(codecs(&paths, out), ["h264", "aac"]);
    // 每个源的第一个包（关键帧）会被 concat 分离器带上参数集，其余数据包逐个相同
    let (ha, hb) = (packet_hashes(&paths, &a), packet_hashes(&paths, &b));
    let merged = packet_hashes(&paths, out);
    assert_eq!(merged.len(), ha.len() + hb.len(), "数据包个数");
    assert_eq!(merged[1..ha.len()], ha[1..], "第一段数据包原样复制");
    assert_eq!(merged[ha.len() + 1..], hb[1..], "第二段数据包原样复制");
    assert!(warnings(&c).is_empty(), "{:?}", warnings(&c));
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
}

#[test]
fn mismatched_inputs_are_reencoded_to_the_first_video_in_sync() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("merge-mismatch");
    let a = dir.join("src/a.mp4");
    let b = dir.join("src/b.mp4");
    let c3 = dir.join("src/c.mp4");
    clip(&paths, &a, 320, 240, 25, 2.0, true);
    clip(&paths, &b, 640, 360, 30, 1.5, true);
    clip(&paths, &c3, 160, 120, 15, 1.0, false);
    let files = [a, b, c3];

    // 无损且不允许回退：失败并说明原因，不留输出
    let (summary, c) = run_merge(&dir, json!({"fallbackReencode": false}), &tasks(&files));
    assert_eq!(summary.failed, 1);
    assert!(
        matches!(
            c.final_state("tool.merge#all"),
            Some(JobEvent::TaskState { state: TaskState::Failed, error: Some(ref e), .. }) if e.contains("无法无损合并") && e.contains("分辨率不同")
        ),
        "{:#?}",
        c.snapshot()
    );
    assert!(all_files(&dir.join("out")).is_empty());

    // 默认：无损不兼容时自动改为重编码
    let (summary, c) = run_merge(&dir, json!({}), &tasks(&files));
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(warnings(&c).iter().any(|w| w.contains("已改为重编码")));
    let out = &done_outputs(&c, "tool.merge#all")[0];
    assert_eq!(out, &dir.join("out").join("a_merged.mp4"));
    let info = probe(&paths, out);
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height), (320, 240), "尺寸以第一个视频为准");
    assert_close(v.fps, 25.0, 0.01, "帧率以第一个视频为准");
    assert_eq!(info.audio.len(), 1);
    let s = streams(&paths, out);
    assert_close(s.video, 4.5, 0.08, "视频时长");
    assert_close(s.audio.unwrap(), 4.5, 0.08, "音频时长");
    assert_close(s.video, s.audio.unwrap(), 0.08, "音画同步");
    // 第二段（640x360 缩放到 320x240，比例不同，上下有黑边）
    let bar = avg_rgb(&paths, out, 2.7, (100, 0, 120, 4));
    assert!(bar.iter().all(|c| *c < 30), "上方应是黑边：{bar:?}");
    // 第三段没有音频：补静音
    assert!(
        max_volume(&paths, out, 3.7, 0.6) < -60.0,
        "无音频片段应为静音"
    );
    assert!(max_volume(&paths, out, 0.5, 1.0) > -30.0, "第一段有声音");
}

/// 某一时间段的最大音量（dB）。
fn max_volume(paths: &FfmpegPaths, file: &Path, start: f64, len: f64) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-ss",
            &start.to_string(),
            "-t",
            &len.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-map", "0:a:0", "-af", "volumedetect", "-f", "null", "-"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find_map(|l| l.split("max_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches("dB").trim().parse().ok())
        .unwrap_or(f64::NEG_INFINITY)
}

#[test]
fn transitions_overlap_clips_and_blend_mid_way() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("merge-xfade");
    let red = dir.join("src/red.mp4");
    let blue = dir.join("src/blue.mp4");
    color_clip(&paths, &red, "red", 2.0, true);
    color_clip(&paths, &blue, "blue", 2.0, true);
    let (summary, c) = run_merge(
        &dir,
        json!({"mode": "reencode", "transition": "fade", "transitionSeconds": 1}),
        &tasks(&[red, blue]),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "tool.merge#all")[0];
    let s = streams(&paths, out);
    assert_close(s.video, 3.0, 0.08, "转场重叠 1 秒");
    assert_close(s.audio.unwrap(), 3.0, 0.08, "音频交叉淡化同样重叠");
    let start = avg_rgb(&paths, out, 0.3, (100, 80, 100, 80));
    assert!(start[0] > 200 && start[2] < 50, "开头是红色：{start:?}");
    let mid = avg_rgb(&paths, out, 1.5, (100, 80, 100, 80));
    assert!(
        (90..170).contains(&mid[0]) && (90..170).contains(&mid[2]),
        "转场中间应是红蓝混合：{mid:?}"
    );
    let end = avg_rgb(&paths, out, 2.7, (100, 80, 100, 80));
    assert!(end[2] > 200 && end[0] < 50, "结尾是蓝色：{end:?}");
}

#[test]
fn middle_video_is_inserted_between_each_pair() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("merge-middle");
    let files: Vec<PathBuf> = ["a", "b", "c"]
        .iter()
        .map(|n| {
            let f = dir.join(&format!("src/{n}.mp4"));
            color_clip(&paths, &f, "red", 1.0, true);
            f
        })
        .collect();
    let middle_dir = dir.join("middle");
    color_clip(&paths, &middle_dir.join("green.mp4"), "lime", 0.6, true);
    let (summary, c) = run_merge(
        &dir,
        json!({
            "mode": "reencode",
            "middleEnabled": true,
            "middle": {"mode": "directory", "path": middle_dir.to_string_lossy(), "order": "sequential"}
        }),
        &tasks(&files),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "tool.merge#all")[0];
    assert_close(
        video_duration(&paths, out),
        4.2,
        0.08,
        "3 段 + 2 个中间视频",
    );
    let green = |t: f64| {
        let rgb = avg_rgb(&paths, out, t, (100, 80, 100, 80));
        rgb[1] > 200 && rgb[0] < 50
    };
    assert!(!green(0.5));
    assert!(green(1.3), "第一个中间视频");
    assert!(!green(2.1));
    assert!(green(2.9), "第二个中间视频");
    assert!(!green(3.8));
}

#[test]
fn every_n_groups_produce_one_output_per_group() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("merge-groups");
    let files: Vec<PathBuf> = (1..=5)
        .map(|i| {
            let f = dir.join(&format!("src/{i}.mp4"));
            clip(&paths, &f, 320, 240, 25, 1.0, true);
            f
        })
        .collect();
    let (summary, c) = run_merge(
        &dir,
        json!({"grouping": "everyN", "groupSize": 2}),
        &tasks(&files),
    );
    assert_eq!(summary.done, 3, "{:#?}", c.snapshot());
    let out = dir.join("out");
    assert_eq!(done_outputs(&c, "tool.merge#1"), [out.join("1_merged.mp4")]);
    assert_eq!(done_outputs(&c, "tool.merge#3"), [out.join("5_merged.mp4")]);
    assert_close(
        video_duration(&paths, &out.join("3_merged.mp4")),
        2.0,
        0.05,
        "第二组",
    );
    assert_close(
        video_duration(&paths, &out.join("5_merged.mp4")),
        1.0,
        0.05,
        "最后一组只有 1 个",
    );

    // 随机顺序：规划时按种子打乱后分组，仍是每组一个工作项（可并行）、3 个输出、总时长不变
    let dir2 = TempDir::new("merge-groups-shuffle");
    let (summary, c) = run_merge(
        &dir2,
        json!({"grouping": "everyN", "groupSize": 2, "shuffle": true}),
        &tasks(&files),
    );
    assert_eq!(summary.done, 3, "{:#?}", c.snapshot());
    let outputs: Vec<PathBuf> = (1..=3)
        .flat_map(|k| done_outputs(&c, &format!("tool.merge#{k}")))
        .collect();
    assert_eq!(outputs.len(), 3);
    let total: f64 = outputs.iter().map(|f| video_duration(&paths, f)).sum();
    assert_close(total, 5.0, 0.1, "总时长");
}
