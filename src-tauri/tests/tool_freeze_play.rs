//! 集成测试：`tool.freeze-play` 定格播放——在指定位置暂停、暂停期间显示图标并静音。
//!
//! 素材是“亮度计数器”（第 N 帧亮度为 16 + 3N），由输出各帧角落的亮度还原出它来自源视频的第几帧。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 25fps 的亮度计数器。
fn make_counter(paths: &FfmpegPaths, out: &Path, size: (u32, u32), seconds: f64, audio: bool) {
    make_luma_counter(paths, out, size, 25, seconds, audio);
}

/// 每一帧中区域的平均亮度 → 源视频帧序号。
fn frame_ids(paths: &FfmpegPaths, file: &Path, region: Rect) -> Vec<i64> {
    frame_lumas(paths, file, Some(region))
        .into_iter()
        .map(counter_index)
        .collect()
}

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.freeze-play", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

/// 在第 `frames` 帧处各停 `hold` 帧后的帧序列（源视频共 `total` 帧）。
fn held_sequence(frames: &[i64], hold: usize, total: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut next = 0;
    for f in frames {
        out.extend(next..=*f);
        out.extend(vec![*f; hold]);
        next = f + 1;
    }
    out.extend(next..total);
    out
}

#[test]
fn pauses_with_builtin_icon_and_silence() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-play-builtin");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), 2.0, true);
    let out = run(&dir, &input, json!({"positions": "1", "pauseSeconds": 0.4}));

    // 角落（图标之外）：第 25 帧停住 10 帧
    let ids = frame_ids(&paths, &out, (0, 0, 16, 16));
    assert_eq!(ids, held_sequence(&[25], 10, 50));
    // 内置图标的左竖条（x 75..79, y 54..66）只在暂停的 10 帧里出现（白色）
    let bar = frame_lumas(&paths, &out, Some((76, 57, 2, 6)));
    for (i, l) in bar.iter().enumerate() {
        let paused = (26..36).contains(&i);
        assert_eq!(*l > 180, paused, "第 {i} 帧图标：亮度 {l}");
    }

    let s = streams(&paths, &out);
    assert_eq!(s.frames, 60);
    assert_close(s.video, 2.4, 0.05, "视频变长 0.4 秒");
    assert_close(s.audio.unwrap(), 2.4, 0.1, "音频同样变长");
    let samples = audio_samples(&paths, &out);
    assert!(rms(&samples, 1.09, 1.39) < 0.003, "暂停期间静音");
    assert!(rms(&samples, 0.2, 1.0) > 0.05);
    assert!(rms(&samples, 1.5, 2.3) > 0.05);
}

#[test]
fn several_positions_without_icon() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-play-multi");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), 2.0, true);
    let out = run(
        &dir,
        &input,
        json!({"positions": "25%, 1.5", "pauseSeconds": 0.2, "showIcon": false}),
    );
    // 0.5 秒 = 第 13 帧（取该时间点之后的第一帧，0.5 × 25 = 12.5）、1.5 秒 = 第 38 帧（37.5 → 38）
    let ids = frame_ids(&paths, &out, (72, 52, 16, 16));
    assert_eq!(ids, held_sequence(&[13, 38], 5, 50), "中央没有图标");
    let s = streams(&paths, &out);
    assert_close(s.video, 2.4, 0.05, "视频时长");
    assert_close(s.audio.unwrap(), 2.4, 0.1, "音频时长");
}

#[test]
fn gif_icon_on_odd_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-play-gif");
    let input = dir.join("src/portrait.mp4");
    make_counter(&paths, &input, (121, 161), 2.0, false);
    let icon = dir.join("assets/pause.gif");
    make_lavfi(
        &paths,
        &icon,
        "color=c=red:s=40x40:r=10:d=0.5",
        None,
        &["-c:v", "gif", "-loop", "0"],
    );
    let out = run(
        &dir,
        &input,
        json!({"positions": "50%", "pauseSeconds": 0.4, "icon": {"mode": "file", "path": icon}, "iconSize": 50}),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (120, 160));
    assert_eq!(stream_count(&paths, &out, "a"), 0);
    let s = streams(&paths, &out);
    assert_eq!(s.frames, 60);
    // 图标 60×60 居中：暂停期间中央是红色，其余时间是灰色计数器
    let paused = avg_rgb(&paths, &out, 1.2, (55, 75, 10, 10));
    assert!(
        paused[0] > 180 && paused[1] < 80,
        "暂停时中央为红色：{paused:?}"
    );
    for t in [0.5, 1.0, 2.0] {
        let c = avg_rgb(&paths, &out, t, (55, 75, 10, 10));
        assert!((c[0] - c[1]).abs() < 20, "{t} 秒没有图标：{c:?}");
    }
}

#[test]
fn positions_outside_video_fail() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("freeze-play-bad");
    let input = dir.join("src/counter.mp4");
    make_counter(&paths, &input, (160, 120), 1.0, false);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.freeze-play", json!({"positions": "5, 120%"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
