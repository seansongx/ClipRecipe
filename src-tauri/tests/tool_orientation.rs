//! 集成测试：`tool.orientation` 横竖互转（文件夹输入）——输出尺寸，视频按比例居中，空白处分别为纯色、
//! 图片、模糊画面；横转竖再转回横；奇数尺寸的源。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use cliprecipe_lib::pipeline::modules::{BitrateMode, BitrateParams};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn make_color(paths: &FfmpegPaths, out: &Path, color: &str, (w, h): (u32, u32), audio: bool) {
    make_lavfi(
        paths,
        out,
        &format!("color=c={color}:s={w}x{h}:r=30:d=2"),
        audio.then_some("sine=frequency=440:duration=2"),
        &[],
    );
}

/// 运行一次（文件夹输入），返回全部输出（按文件名排序）。
fn run(dir: &TempDir, folder: &Path, out: &str, params: Value) -> Vec<PathBuf> {
    let paths = ffmpeg_paths();
    let mut config = tool_config(&paths, &dir.join(out), &dir.join("tmp"));
    config.bitrate = BitrateParams {
        enabled: true,
        mode: BitrateMode::Fixed,
        ..BitrateParams::default()
    };
    let c = collector();
    let folders = BTreeMap::from([("source".to_owned(), folder.to_path_buf())]);
    let summary = start_tool(
        config,
        registered("tool.orientation", params),
        &[],
        &folders,
        &c,
    )
    .wait();
    assert_eq!(summary.failed + summary.cancelled, 0, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    all_files(&dir.join(out))
}

fn size(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let video = probe(paths, file).video.unwrap();
    (video.width, video.height)
}

fn near(actual: [i32; 3], expected: [i32; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 30)
}

#[test]
fn landscape_to_portrait_with_colour_fill() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("orientation-color");
    make_color(&paths, &dir.join("src/red.mp4"), "red", (320, 240), true);
    let outputs = run(
        &dir,
        &dir.join("src"),
        "out",
        json!({"target": "custom", "width": 180, "height": 320, "fill": "color", "color": "#00ff00"}),
    );
    assert_eq!(outputs.len(), 1);
    let out = &outputs[0];
    assert_eq!(out.file_name().unwrap(), "red.mp4");
    assert_eq!(size(&paths, out), (180, 320));
    // 320×240 放进 180×320 → 180×134（居中，上下各约 93 像素的绿边）
    assert!(
        near(avg_rgb(&paths, out, 1.0, (0, 10, 180, 70)), [0, 255, 0]),
        "上边绿色"
    );
    assert!(
        near(avg_rgb(&paths, out, 1.0, (0, 240, 180, 70)), [0, 255, 0]),
        "下边绿色"
    );
    assert!(
        near(avg_rgb(&paths, out, 1.0, (10, 110, 160, 100)), [255, 0, 0]),
        "中间是视频"
    );
    let s = streams(&paths, out);
    assert_eq!(s.frames, 60);
    assert_close(s.audio.unwrap(), 2.0, 0.1, "音频保留");
}

#[test]
fn portrait_to_landscape_with_image_fill_keeps_frame_rate() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("orientation-image");
    make_color(&paths, &dir.join("src/blue.mp4"), "blue", (180, 320), false);
    let image = dir.join("assets/yellow.png");
    make_lavfi(
        &paths,
        &image,
        "color=c=yellow:s=64x48:d=1",
        None,
        &[
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-pix_fmt",
            "rgb24",
            "-update",
            "1",
        ],
    );
    let outputs = run(
        &dir,
        &dir.join("src"),
        "out",
        json!({"fill": "image", "image": {"mode": "file", "path": image}}),
    );
    let out = &outputs[0];
    // 宽高互换：180×320 → 320×180；视频 100×180 居中，左右为黄色图片
    assert_eq!(size(&paths, out), (320, 180));
    assert!(
        near(avg_rgb(&paths, out, 1.0, (5, 20, 90, 140)), [255, 255, 0]),
        "左边图片"
    );
    assert!(
        near(avg_rgb(&paths, out, 1.0, (225, 20, 90, 140)), [255, 255, 0]),
        "右边图片"
    );
    assert!(
        near(avg_rgb(&paths, out, 1.0, (120, 20, 80, 140)), [0, 0, 255]),
        "中间是视频"
    );
    let s = streams(&paths, out);
    assert_eq!(s.frames, 60, "帧率沿用源视频（30fps）");
    assert_eq!(stream_count(&paths, out, "a"), 0);
}

#[test]
fn blur_fill_shows_blurred_video_content() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("orientation-blur");
    // 左半红、右半蓝
    make_lavfi(
        &paths,
        &dir.join("src/split.mp4"),
        "color=c=red:s=320x240:r=25:d=1,drawbox=x=160:y=0:w=160:h=240:c=blue:t=fill",
        None,
        &[],
    );
    let outputs = run(
        &dir,
        &dir.join("src"),
        "out",
        json!({"target": "custom", "width": 180, "height": 320, "blurStrength": 10}),
    );
    let out = &outputs[0];
    assert_eq!(size(&paths, out), (180, 320));
    // 上边空白：放大铺满后的画面左红右蓝（中缝附近被模糊混合）
    let left = avg_rgb(&paths, out, 0.5, (5, 20, 40, 40));
    let right = avg_rgb(&paths, out, 0.5, (135, 20, 40, 40));
    assert!(left[0] > left[2] + 80, "左上偏红：{left:?}");
    assert!(right[2] > right[0] + 80, "右上偏蓝：{right:?}");
    // 中缝处模糊：红蓝混合
    let seam = avg_rgb(&paths, out, 0.5, (86, 20, 8, 40));
    assert!(seam[0] > 60 && seam[2] > 60, "中缝模糊混合：{seam:?}");
}

#[test]
fn odd_source_round_trip() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("orientation-odd");
    make_odd_clip(&paths, &dir.join("src/odd.mp4"), 321, 241);
    let portrait = run(&dir, &dir.join("src"), "portrait", json!({}));
    assert_eq!(size(&paths, &portrait[0]), (240, 320), "321×241 → 240×320");
    let back = run(&dir, &dir.join("portrait"), "landscape", json!({}));
    assert_eq!(size(&paths, &back[0]), (320, 240), "再转回横屏");
    assert_close(
        streams(&paths, &back[0]).video,
        streams(&paths, &dir.join("src/odd.mp4")).video,
        0.1,
        "时长不变",
    );
}

#[test]
fn missing_image_fails_at_planning() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("orientation-bad");
    make_color(&paths, &dir.join("src/a.mp4"), "red", (160, 120), false);
    let folders = BTreeMap::from([("source".to_owned(), dir.join("src"))]);
    let tool = registered("tool.orientation", json!({"fill": "image"}));
    let settings = cliprecipe_lib::model::Settings::default();
    assert!(cliprecipe_lib::tools::runner::plan(tool.as_ref(), &[], &folders, &settings).is_err());
}
