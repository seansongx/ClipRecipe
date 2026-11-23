//! 集成测试：`tool.remove-black-bars` 黑边去除——上下黑边、左右黑边、奇数尺寸、没有黑边时跳过或复制、恢复尺寸。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// `content` 尺寸的灰色噪点画面（没有黑色区域）放进 `frame` 尺寸的黑底里，左上角在 `at`。
fn make_boxed(
    paths: &FfmpegPaths,
    out: &Path,
    content: (u32, u32),
    frame: (u32, u32),
    at: (u32, u32),
    audio: bool,
) {
    let odd = frame.0 % 2 == 1 || frame.1 % 2 == 1;
    let pix = if odd { "yuv444p" } else { "yuv420p" };
    let graph = format!(
        "color=c=0x808080:s={}x{}:r=25:d=2,noise=alls=30:allf=t,pad={}:{}:{}:{}:black,format={pix}",
        content.0, content.1, frame.0, frame.1, at.0, at.1
    );
    make_lavfi(
        paths,
        out,
        &graph,
        audio.then_some("sine=frequency=440:duration=2"),
        &["-pix_fmt", pix],
    );
}

fn run(dir: &TempDir, input: &Path, params: Value) -> (Vec<PathBuf>, Collector) {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.remove-black-bars", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    (done_outputs(&c, "t1"), c)
}

fn size(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    probe(paths, file).video.unwrap().display_size()
}

/// 画面四条边上都没有黑色（每边 2 像素宽的条带平均亮度明显高于黑色）。
fn assert_no_bars(paths: &FfmpegPaths, file: &Path) {
    let (w, h) = size(paths, file);
    for (name, region) in [
        ("上", (0, 0, w, 2)),
        ("下", (0, h - 2, w, 2)),
        ("左", (0, 0, 2, h)),
        ("右", (w - 2, 0, 2, h)),
    ] {
        let [r, g, b] = avg_rgb(paths, file, 1.0, region);
        assert!(r + g + b > 120, "{name}边仍是黑色：{:?}", [r, g, b]);
    }
}

#[test]
fn letterbox_is_cropped() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-letterbox");
    let input = dir.join("src/letterbox.mp4");
    make_boxed(&paths, &input, (320, 180), (320, 240), (0, 30), true);
    let (outputs, _) = run(&dir, &input, json!({}));
    assert_eq!(outputs, [dir.join("out").join("letterbox.mp4")]);
    assert_eq!(size(&paths, &outputs[0]), (320, 180));
    assert_no_bars(&paths, &outputs[0]);
    let s = streams(&paths, &outputs[0]);
    assert_close(s.video, 2.0, 0.05, "时长不变");
    assert_close(s.audio.unwrap(), 2.0, 0.1, "音频保留");
}

#[test]
fn pillarbox_portrait_is_cropped() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-pillarbox");
    let input = dir.join("src/pillar.mp4");
    make_boxed(&paths, &input, (120, 320), (180, 320), (30, 0), false);
    let (outputs, _) = run(&dir, &input, json!({}));
    assert_eq!(size(&paths, &outputs[0]), (120, 320));
    assert_no_bars(&paths, &outputs[0]);
    assert_eq!(stream_count(&paths, &outputs[0], "a"), 0);
}

#[test]
fn odd_source_comes_out_even() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-odd");
    let input = dir.join("src/odd.mp4");
    // 奇数尺寸 + 奇数偏移：按精确边界裁剪
    make_boxed(&paths, &input, (240, 150), (241, 181), (0, 17), false);
    let (outputs, _) = run(&dir, &input, json!({}));
    assert_eq!(size(&paths, &outputs[0]), (240, 150));
    assert_no_bars(&paths, &outputs[0]);
}

/// 从第 y 行起两行的平均亮度（r+g+b；4:2:0 输出取不了 1 像素高的区域）。
fn row_level(paths: &FfmpegPaths, file: &Path, y: u32) -> i32 {
    let (w, _) = size(paths, file);
    let [r, g, b] = avg_rgb(paths, file, 1.0, (0, y, w, 2));
    r + g + b
}

/// 从第 x 列起两列的平均亮度。
fn column_level(paths: &FfmpegPaths, file: &Path, x: u32) -> i32 {
    let (_, h) = size(paths, file);
    let [r, g, b] = avg_rgb(paths, file, 1.0, (x, 0, 2, h));
    r + g + b
}

/// yuv444p 的黑底画面（偶数尺寸，内容可以在奇数偏移处）。
fn make_boxed_444(paths: &FfmpegPaths, out: &Path, content: (u32, u32), at: (u32, u32)) {
    let graph = format!(
        "color=c=0x808080:s={}x{}:r=25:d=2,noise=alls=30:allf=t,pad=320:240:{}:{}:black,format=yuv444p",
        content.0, content.1, at.0, at.1
    );
    make_lavfi(paths, out, &graph, None, &["-pix_fmt", "yuv444p"]);
}

#[test]
fn odd_offset_letterbox_is_cropped_tightly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-odd-offset");
    // 上黑边 31 行、画面 178 行：裁剪后首行与末行都是画面（cropdetect 的取整偏移会落到 32 行，
    // 少一行画面、多一行黑边）
    let input = dir.join("src/odd-offset.mp4");
    make_boxed_444(&paths, &input, (320, 178), (0, 31));
    let (outputs, _) = run(&dir, &input, json!({}));
    assert_eq!(size(&paths, &outputs[0]), (320, 178));
    assert!(row_level(&paths, &outputs[0], 0) > 300, "首行是画面");
    assert!(row_level(&paths, &outputs[0], 176) > 300, "末两行是画面");

    // 画面 177 行（奇数）：高度取偶时多带 1 行黑边，不裁掉画面
    let odd = dir.join("src/odd-height.mp4");
    make_boxed_444(&paths, &odd, (320, 177), (0, 31));
    let (outputs, _) = run(&dir, &odd, json!({}));
    assert_eq!(size(&paths, &outputs[0]), (320, 178));
    assert!(row_level(&paths, &outputs[0], 0) > 300, "首行是画面");
    assert!(
        row_level(&paths, &outputs[0], 175) > 300,
        "画面的最后两行保留"
    );

    // 左右奇数偏移
    let pillar = dir.join("src/odd-pillar.mp4");
    make_boxed_444(&paths, &pillar, (150, 240), (37, 0));
    let (outputs, _) = run(&dir, &pillar, json!({}));
    assert_eq!(size(&paths, &outputs[0]), (150, 240));
    assert!(column_level(&paths, &outputs[0], 0) > 300, "首列是画面");
    assert!(column_level(&paths, &outputs[0], 148) > 300, "末两列是画面");
}

#[test]
fn fill_restores_original_size_without_bars() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-fill");
    let input = dir.join("src/letterbox.mp4");
    make_boxed(&paths, &input, (320, 180), (320, 240), (0, 30), true);
    let (outputs, _) = run(&dir, &input, json!({"restore": "fill"}));
    assert_eq!(size(&paths, &outputs[0]), (320, 240));
    assert_no_bars(&paths, &outputs[0]);
    let (outputs, _) = run(&dir, &input, json!({"restore": "stretch"}));
    assert_eq!(size(&paths, &outputs[0]), (320, 240));
    assert_no_bars(&paths, &outputs[0]);
}

#[test]
fn no_bars_is_skipped_or_copied() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("black-bars-none");
    let input = dir.join("src/full.mp4");
    make_boxed(&paths, &input, (320, 240), (320, 240), (0, 0), true);

    let (outputs, c) = run(&dir, &input, json!({"whenNone": "skip"}));
    assert!(outputs.is_empty());
    assert!(all_files(&dir.join("out")).is_empty(), "跳过时不输出");
    let logged = c.snapshot().iter().any(|e| {
        matches!(e, cliprecipe_lib::jobs::JobEvent::Log { message, .. } if message.contains("没有检测到黑边"))
    });
    assert!(logged, "处理记录里有提示");

    let (outputs, _) = run(&dir, &input, json!({"whenNone": "copy"}));
    assert_eq!(outputs, [dir.join("out").join("full.mp4")]);
    assert_eq!(
        video_packets_md5(&paths, &outputs[0]),
        video_packets_md5(&paths, &input),
        "原样复制，没有重编码"
    );
    assert_eq!(stream_count(&paths, &outputs[0], "a"), 1);
}
