//! 集成测试：非方形像素（变形，SAR ≠ 1）的源经过合并与片段列表类工具（拆分重组）后显示比例不变：
//! 拼接前统一换算成方形像素（框架的 `SQUARE_PIXELS` / `SourceFormat::anamorphic`），输出 SAR 为 1:1。
//!
//! 源：240×240、SAR 2:1 的红色画面（显示为 480×240）；另一段是 480×240 方形像素的蓝色画面。
//! 旧做法（直接 `setsar=1`）会把变形源当成 240×240 显示，画面被压扁，输出也变成 240×240。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::json;
use tool_support::*;

fn anamorphic_red(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=red:s=240x240:r=25:d=2,setsar=2,format=yuv420p",
        Some("sine=frequency=440:duration=2"),
        &[],
    );
    let info = probe(paths, out);
    let v = info.video.as_ref().unwrap();
    assert_eq!(v.sar, "2:1", "测试源应为变形源");
    assert_eq!(v.square_display_size(), (480, 240));
}

fn square_blue(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=blue:s=480x240:r=25:d=1,format=yuv420p",
        Some("sine=frequency=660:duration=1"),
        &[],
    );
}

/// 输出是 480×240、方形像素，画面两侧边缘在 `t` 时都是 `colour`（没有被压扁后留下的黑边）。
fn assert_full_width(paths: &FfmpegPaths, file: &Path, t: f64, colour: [i32; 3], what: &str) {
    let info = probe(paths, file);
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height), (480, 240), "{what}：输出尺寸");
    assert!(v.sar.is_empty() || v.sar == "1:1", "{what}：SAR {}", v.sar);
    assert_colour(avg_rgb(paths, file, t, (4, 100, 20, 40)), colour, what);
    assert_colour(avg_rgb(paths, file, t, (456, 100, 20, 40)), colour, what);
}

#[test]
fn merge_keeps_the_display_aspect_of_anamorphic_sources() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("anamorphic-merge");
    let red = dir.join("src/1-red.mp4");
    let blue = dir.join("src/2-blue.mp4");
    anamorphic_red(&paths, &red);
    square_blue(&paths, &blue);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.merge", json!({"mode": "reencode"})),
        &[video_task("a", &red), video_task("b", &blue)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = &done_outputs(&c, "tool.merge#all")[0];
    assert_full_width(&paths, output, 1.0, RED, "变形源部分");
    assert_full_width(&paths, output, 2.5, BLUE, "方形像素部分");

    // 无损合并要求 SAR 相同：自动改为重编码
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out2"), &dir.join("tmp")),
        registered("tool.merge", json!({})),
        &[video_task("a", &red), video_task("b", &blue)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
}

#[test]
fn cut_list_tool_keeps_the_display_aspect_of_anamorphic_sources() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("anamorphic-split-remix");
    let red = dir.join("src/red.mp4");
    let blue = dir.join("assets/blue.mp4");
    anamorphic_red(&paths, &red);
    square_blue(&paths, &blue);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered(
            "tool.split-remix",
            json!({
                "positionMode": "parts",
                "parts": 2,
                "insert": {"mode": "file", "path": blue.to_string_lossy()}
            }),
        ),
        &[video_task("t1", &red)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = &done_outputs(&c, "t1")[0];
    assert_full_width(&paths, output, 0.5, RED, "主视频（变形源）");
    assert_full_width(&paths, output, 1.5, BLUE, "插入的视频");
    assert_full_width(&paths, output, 2.5, RED, "主视频后半段");
}
