//! 集成测试：`tool.clip-merge` 截取合并——片段个数与长度、截取位置、按目标时长分成多个、截去超出长度、
//! 片头与中间视频、取消后不留输出与临时文件。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::Path;

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use cliprecipe_lib::jobs::{JobEvent, RunSummary, TaskState};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 前 2 秒红、后 2 秒蓝（带噪点，码率不至于太低），带正弦音频。
fn red_blue(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=red:s=320x240:r=25:d=2[a];color=c=blue:s=320x240:r=25:d=2[b];\
         [a][b]concat=n=2,noise=alls=20:allf=t+u,format=yuv420p",
        Some("sine=frequency=440:duration=4"),
        &[],
    );
}

fn folders(root: &Path) -> BTreeMap<String, std::path::PathBuf> {
    let mut map = BTreeMap::new();
    map.insert("source".to_owned(), root.to_path_buf());
    map
}

fn run(dir: &TempDir, root: &Path, params: Value) -> (RunSummary, Collector) {
    let paths = ffmpeg_paths();
    let c = collector();
    let summary = start_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.clip-merge", params),
        &[],
        &folders(root),
        &c,
    )
    .wait();
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    (summary, c)
}

fn is_red(rgb: [i32; 3]) -> bool {
    rgb[0] > 180 && rgb[2] < 70
}

fn is_blue(rgb: [i32; 3]) -> bool {
    rgb[2] > 180 && rgb[0] < 70
}

#[test]
fn one_clip_per_video_including_subfolders_at_the_chosen_position() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("clip-merge-pos");
    let root = dir.join("素材");
    red_blue(&paths, &root.join("1.mp4"));
    red_blue(&paths, &root.join("2.mp4"));
    red_blue(&paths, &root.join("sub/3.mp4"));
    // 比最短长度还短的视频被跳过
    make_clip(
        &paths,
        &root.join("short.mp4"),
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );

    let (summary, c) = run(
        &dir,
        &root,
        json!({"segmentSeconds": {"min": 1.5, "max": 1.5}, "position": "head"}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = done_outputs(&c, "tool.clip-merge#all");
    assert_eq!(out, [dir.join("out").join("素材.mp4")]);
    let s = streams(&paths, &out[0]);
    assert_close(s.video, 4.5, 0.06, "3 段 × 1.5 秒");
    assert_close(s.audio.unwrap(), 4.5, 0.06, "音频同样长");
    for t in [0.7, 2.2, 3.7] {
        assert!(
            is_red(avg_rgb(&paths, &out[0], t, (100, 80, 100, 80))),
            "开头截取应为红色（{t} 秒）"
        );
    }

    let dir2 = TempDir::new("clip-merge-tail");
    let (summary, c) = run(
        &dir2,
        &root,
        json!({"segmentSeconds": {"min": 1.5, "max": 1.5}, "position": "tail"}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "tool.clip-merge#all")[0];
    assert_close(video_duration(&paths, out), 4.5, 0.06, "结尾截取");
    for t in [0.7, 2.2, 3.7] {
        assert!(
            is_blue(avg_rgb(&paths, out, t, (100, 80, 100, 80))),
            "结尾截取应为蓝色（{t} 秒）"
        );
    }
}

#[test]
fn target_length_splits_outputs_and_trim_cuts_exactly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("clip-merge-target");
    let root = dir.join("src");
    for i in 1..=4 {
        red_blue(&paths, &root.join(format!("{i}.mp4")));
    }
    let base = json!({
        "segmentSeconds": {"min": 2, "max": 2},
        "position": "middle",
        "outputMode": "target",
        "targetSeconds": 3
    });
    let (summary, c) = run(&dir, &root, base.clone());
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "tool.clip-merge#all");
    let out = dir.join("out");
    assert_eq!(outputs, [out.join("src_01.mp4"), out.join("src_02.mp4")]);
    for f in &outputs {
        assert_close(
            video_duration(&paths, f),
            4.0,
            0.06,
            "每个输出累加到不少于目标",
        );
    }

    let dir2 = TempDir::new("clip-merge-trim");
    let mut trimmed = base;
    trimmed["trimExcess"] = json!(true);
    trimmed["introEnabled"] = json!(true);
    let (summary, c) = run(&dir2, &root, trimmed);
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "tool.clip-merge#all");
    assert_eq!(outputs.len(), 2);
    for f in &outputs {
        let s = streams(&paths, f);
        assert_close(s.video, 3.0, 0.045, "截去超出长度：视频");
        assert_close(s.audio.unwrap(), 3.0, 0.045, "截去超出长度：音频");
    }
    assert!(c.snapshot().iter().any(|e| matches!(e, JobEvent::Log { message, .. } if message.contains("片头") && message.contains("不可用"))));
}

#[test]
fn intro_and_middle_videos_are_added() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("clip-merge-extras");
    let root = dir.join("src");
    red_blue(&paths, &root.join("1.mp4"));
    red_blue(&paths, &root.join("2.mp4"));
    let green = dir.join("assets/green.mp4");
    make_lavfi(
        &paths,
        &green,
        "color=c=lime:s=640x480:r=30:d=0.6,noise=alls=20:allf=t+u,format=yuv420p",
        None,
        &[],
    );
    let asset = json!({"mode": "file", "path": green.to_string_lossy()});
    let (summary, c) = run(
        &dir,
        &root,
        json!({
            "segmentSeconds": {"min": 1.6, "max": 1.6},
            "position": "head",
            "introEnabled": true, "intro": asset,
            "middleEnabled": true, "middle": asset
        }),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "tool.clip-merge#all")[0];
    let s = streams(&paths, out);
    assert_close(
        s.video,
        4.4,
        0.05,
        "片头 0.6 + 片段 1.6 + 中间 0.6 + 片段 1.6",
    );
    assert_close(s.audio.unwrap(), 4.4, 0.05, "没有声音的片头补静音");
    let green_at = |t: f64| {
        let rgb = avg_rgb(&paths, out, t, (100, 80, 100, 80));
        rgb[1] > 180 && rgb[0] < 70
    };
    assert!(green_at(0.2), "片头");
    assert!(is_red(avg_rgb(&paths, out, 1.2, (100, 80, 100, 80))));
    assert!(green_at(2.5), "中间视频");
    assert!(is_red(avg_rgb(&paths, out, 3.5, (100, 80, 100, 80))));
}

#[test]
fn cancelling_mid_run_leaves_no_outputs_or_temp_files() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("clip-merge-cancel");
    let root = dir.join("src");
    for i in 1..=3 {
        make_lavfi(
            &paths,
            &root.join(format!("{i}.mp4")),
            "testsrc2=s=640x480:r=25:d=12",
            Some("sine=duration=12"),
            &[],
        );
    }
    let out = dir.join("out");
    let c = collector();
    let handle = start_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.clip-merge",
            json!({"segmentSeconds": {"min": 10, "max": 10}, "outputMode": "target", "targetSeconds": 10}),
        ),
        &[],
        &folders(&root),
        &c,
    );
    // 第一个输出完成、第二个开始编码后停止
    c.wait_for(
        180,
        |e| matches!(e, JobEvent::Progress { round, .. } if *round >= 2),
    );
    handle.stop();
    let summary = handle.wait();
    assert_eq!(
        (summary.done, summary.cancelled),
        (0, 1),
        "{:#?}",
        c.snapshot()
    );
    assert!(matches!(
        c.final_state("tool.clip-merge#all"),
        Some(JobEvent::TaskState {
            state: TaskState::Cancelled,
            ..
        })
    ));
    assert!(
        all_files(&out).is_empty(),
        "取消后不留输出（包括已完成的第一个）：{:?}",
        all_files(&out)
    );
    assert!(all_files(&dir.join("tmp")).is_empty(), "取消后不留临时文件");
}
