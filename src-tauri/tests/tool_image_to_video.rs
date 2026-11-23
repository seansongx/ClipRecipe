//! 集成测试：`tool.image-to-video` 图转视频（文件夹输入）——中文文件名、分辨率、帧率、时长、帧数、静音音轨、
//! 放置方式与背景色；各种入场动画让画面随时间变化。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use cliprecipe_lib::pipeline::modules::{BitrateMode, BitrateParams};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 用 lavfi 画一张图片（第一帧）。
fn make_image(paths: &FfmpegPaths, out: &Path, graph: &str) {
    let codec = if out.extension().is_some_and(|e| e == "png") {
        "png"
    } else {
        "mjpeg"
    };
    let mut extra = vec!["-frames:v", "1", "-update", "1", "-c:v", codec];
    extra.extend([
        "-pix_fmt",
        if codec == "png" { "rgb24" } else { "yuvj420p" },
    ]);
    make_lavfi(paths, out, &format!("{graph},format=rgb24"), None, &extra);
}

fn run(dir: &TempDir, params: Value) -> Vec<PathBuf> {
    let paths = ffmpeg_paths();
    let mut config = tool_config(&paths, &dir.join("out"), &dir.join("tmp"));
    config.bitrate = BitrateParams {
        enabled: true,
        mode: BitrateMode::Fixed,
        ..BitrateParams::default()
    };
    let c = collector();
    let folders = BTreeMap::from([("images".to_owned(), dir.join("src"))]);
    let summary = start_tool(
        config,
        registered("tool.image-to-video", params),
        &[],
        &folders,
        &c,
    )
    .wait();
    assert_eq!(summary.failed + summary.cancelled, 0, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    all_files(&dir.join("out"))
}

fn near(actual: [i32; 3], expected: [i32; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 30)
}

#[test]
fn chinese_names_size_rate_duration_and_background() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("image-to-video-basic");
    make_image(&paths, &dir.join("src/风景.png"), "color=c=red:s=200x100");
    make_image(
        &paths,
        &dir.join("src/人物 照片.jpg"),
        "color=c=0x00ff00:s=100x200",
    );
    let outputs = run(
        &dir,
        json!({"width": 320, "height": 240, "fps": 25, "duration": 2, "background": "#0000ff"}),
    );
    let names: Vec<String> = outputs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["人物 照片.mp4", "风景.mp4"]);
    for out in &outputs {
        let info = probe(&paths, out);
        let video = info.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_close(video.fps, 25.0, 0.01, "帧率");
        let s = streams(&paths, out);
        assert_eq!(s.frames, 50, "2 秒 × 25fps");
        assert_close(s.video, 2.0, 0.05, "视频时长");
        assert_close(s.audio.expect("静音音轨"), 2.0, 0.1, "音频时长");
    }
    // 风景 200×100 完整放入 320×240 → 320×160，上下各 40 像素蓝边
    let landscape = &outputs[1];
    assert!(near(
        avg_rgb(&paths, landscape, 1.0, (0, 5, 320, 30)),
        [0, 0, 255]
    ));
    assert!(near(
        avg_rgb(&paths, landscape, 1.0, (20, 60, 280, 120)),
        [255, 0, 0]
    ));
    // 人物 100×200 → 120×240，左右各 100 像素蓝边
    let portrait = &outputs[0];
    assert!(near(
        avg_rgb(&paths, portrait, 1.0, (5, 0, 90, 240)),
        [0, 0, 255]
    ));
    assert!(near(
        avg_rgb(&paths, portrait, 1.0, (110, 20, 100, 200)),
        [0, 255, 0]
    ));
}

#[test]
fn fill_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("image-to-video-fill");
    make_image(&paths, &dir.join("src/a.png"), "color=c=red:s=200x100");
    let outputs = run(
        &dir,
        json!({"width": 240, "height": 240, "fit": "fill", "silentAudio": false, "fps": 10, "duration": 1.5}),
    );
    let out = &outputs[0];
    assert_eq!(stream_count(&paths, out, "a"), 0);
    assert_eq!(streams(&paths, out).frames, 15);
    assert!(
        near(avg_rgb(&paths, out, 0.5, (0, 0, 240, 240)), [255, 0, 0]),
        "铺满没有背景"
    );
}

/// 白色图片在黑色背景上，各种动画在开头与结尾的画面。
fn animated(animation: &str) -> (PathBuf, TempDir) {
    let paths = ffmpeg_paths();
    let dir = TempDir::new(&format!("image-to-video-{animation}"));
    make_image(&paths, &dir.join("src/a.png"), "color=c=white:s=320x240");
    let outputs = run(
        &dir,
        json!({
            "width": 320, "height": 240, "fps": 25, "duration": 2,
            "animation": animation, "animationSeconds": 1
        }),
    );
    (outputs[0].clone(), dir)
}

fn brightness(rgb: [i32; 3]) -> i32 {
    (rgb[0] + rgb[1] + rgb[2]) / 3
}

#[test]
fn fade_in_starts_dark() {
    let paths = ffmpeg_paths();
    let (out, _dir) = animated("fadeIn");
    let start = brightness(avg_rgb(&paths, &out, 0.0, (0, 0, 320, 240)));
    let middle = brightness(avg_rgb(&paths, &out, 0.5, (0, 0, 320, 240)));
    let end = brightness(avg_rgb(&paths, &out, 1.5, (0, 0, 320, 240)));
    assert!(
        start < 30 && middle > 80 && middle < 180 && end > 230,
        "{start} {middle} {end}"
    );
    assert_eq!(streams(&paths, &out).frames, 50);
}

#[test]
fn zoom_in_grows_from_the_centre() {
    let paths = ffmpeg_paths();
    let (out, _dir) = animated("zoomIn");
    let corner = |t| brightness(avg_rgb(&paths, &out, t, (0, 0, 60, 40)));
    let centre = |t| brightness(avg_rgb(&paths, &out, t, (130, 100, 60, 40)));
    assert!(
        corner(0.0) < 30 && centre(0.0) > 230,
        "开头是一半大小：角 {} 中 {}",
        corner(0.0),
        centre(0.0)
    );
    assert!(corner(1.5) > 230, "结束时铺满：{}", corner(1.5));
    assert_eq!(streams(&paths, &out).frames, 50);
}

#[test]
fn slide_in_enters_from_the_left() {
    let paths = ffmpeg_paths();
    let (out, _dir) = animated("slideIn");
    let left = |t| brightness(avg_rgb(&paths, &out, t, (0, 0, 80, 240)));
    let right = |t| brightness(avg_rgb(&paths, &out, t, (240, 0, 80, 240)));
    assert!(left(0.0) < 30 && right(0.0) < 30, "开头画面在左侧外面");
    assert!(
        left(0.4) > 230 && right(0.4) < 30,
        "滑入中：左 {} 右 {}",
        left(0.4),
        right(0.4)
    );
    assert!(left(1.5) > 230 && right(1.5) > 230, "到位");
}

#[test]
fn ken_burns_moves_the_picture() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("image-to-video-ken-burns");
    make_image(&paths, &dir.join("src/a.png"), "testsrc2=s=320x240");
    let outputs = run(
        &dir,
        json!({"width": 320, "height": 240, "fps": 25, "duration": 2, "animation": "kenBurns"}),
    );
    let out = &outputs[0];
    let still = run_still(&paths, &dir);
    let region = (40, 40, 80, 60);
    let diff = |a: [i32; 3], b: [i32; 3]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<i32>();
    let start = avg_rgb(&paths, out, 0.0, region);
    let end = avg_rgb(&paths, out, 1.9, region);
    assert!(
        diff(start, avg_rgb(&paths, &still, 0.0, region)) < 20,
        "开头与原图一致"
    );
    assert!(diff(start, end) > 30, "结尾画面已移动：{start:?} → {end:?}");
}

/// 同一张图不加动画的结果（对照）。
fn run_still(paths: &FfmpegPaths, dir: &TempDir) -> PathBuf {
    let mut config = tool_config(paths, &dir.join("still"), &dir.join("tmp"));
    config.bitrate = BitrateParams {
        enabled: true,
        mode: BitrateMode::Fixed,
        ..BitrateParams::default()
    };
    let c = collector();
    let folders = BTreeMap::from([("images".to_owned(), dir.join("src"))]);
    let tool = registered(
        "tool.image-to-video",
        json!({"width": 320, "height": 240, "fps": 25, "duration": 1}),
    );
    let summary = start_tool(config, tool, &[], &folders, &c).wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    all_files(&dir.join("still"))[0].clone()
}
