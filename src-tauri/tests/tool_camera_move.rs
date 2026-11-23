//! 集成测试：`tool.camera-move` 运镜效果。
//!
//! 素材是静止的亮度渐变（左黑右亮，或上黑下亮），所以输出画面某处的亮度直接说明窗口此刻落在源画面的哪里。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 静止渐变：`axis` 为 "X" 时从左到右、"Y" 时从上到下，亮度 16 → 216（有限范围）。
fn make_gradient(paths: &FfmpegPaths, out: &Path, (w, h): (u32, u32), axis: &str, audio: bool) {
    let pix = if w % 2 == 1 || h % 2 == 1 {
        "yuv444p"
    } else {
        "yuv420p"
    };
    let size = if axis == "X" { "W" } else { "H" };
    let graph = format!(
        "color=c=black:s={w}x{h}:r=25:d=2,format={pix},geq=lum='16+{axis}*200/{size}':cb=128:cr=128"
    );
    make_lavfi(
        paths,
        out,
        &graph,
        audio.then_some("sine=frequency=440:duration=2"),
        &["-qp", "0", "-pix_fmt", pix],
    );
}

/// 源画面坐标 `pos`（沿渐变方向，`len` 为该方向长度）处的灰度（全范围）。
fn gradient_gray(pos: f64, len: f64) -> f64 {
    (pos * 200.0 / len) * 255.0 / 219.0
}

fn gray(paths: &FfmpegPaths, file: &Path, t: f64, region: (u32, u32, u32, u32)) -> f64 {
    let [r, g, b] = avg_rgb(paths, file, t, region);
    f64::from(r + g + b) / 3.0
}

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.camera-move", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

#[test]
fn horizontal_pan_moves_a_zoomed_window() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("camera-move-pan");
    let input = dir.join("src/gradient.mp4");
    make_gradient(&paths, &input, (320, 240), "X", true);
    let out = run(
        &dir,
        &input,
        json!({"direction": "horizontal", "zoom": 2, "autoPeriod": false, "periodSeconds": 2}),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (320, 240));
    let s = streams(&paths, &out);
    assert_eq!(s.frames, 50);
    assert_close(s.audio.unwrap(), 2.0, 0.1, "音频保留");

    // 窗口 160×120：开头在正中（源 x 80..240），1/4 周期到右边缘（160..320），3/4 周期到左边缘（0..160）
    let left = |t| gray(&paths, &out, t, (0, 100, 4, 40));
    let right = |t| gray(&paths, &out, t, (316, 100, 4, 40));
    assert_close(
        left(0.0),
        gradient_gray(81.0, 320.0),
        8.0,
        "开头左缘（已放大，不是源画面左缘）",
    );
    assert_close(right(0.0), gradient_gray(239.0, 320.0), 8.0, "开头右缘");
    assert_close(left(0.5), gradient_gray(161.0, 320.0), 8.0, "1/4 周期左缘");
    assert_close(left(1.5), gradient_gray(1.0, 320.0), 8.0, "3/4 周期左缘");
    // 两个时刻看到的画面不同
    assert!(left(0.5) - left(1.5) > 100.0);
    // 上下方向不动
    let top = gray(&paths, &out, 0.5, (100, 0, 40, 4));
    let bottom = gray(&paths, &out, 0.5, (100, 236, 40, 4));
    assert_close(top, bottom, 4.0, "水平移动时上下一致");
}

#[test]
fn vertical_pan_on_portrait() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("camera-move-vertical");
    let input = dir.join("src/portrait.mp4");
    make_gradient(&paths, &input, (180, 320), "Y", true);
    let out = run(&dir, &input, json!({"direction": "vertical", "zoom": 2}));
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (180, 320));
    // 自动周期：2 秒视频一个周期 2 秒；窗口 90×160，1/4 周期到底部（源 y 160..320）
    let top = |t| gray(&paths, &out, t, (70, 0, 40, 4));
    assert_close(top(0.0), gradient_gray(81.0, 320.0), 8.0, "开头上缘");
    assert_close(top(0.5), gradient_gray(161.0, 320.0), 8.0, "1/4 周期上缘");
    assert_close(top(1.5), gradient_gray(1.0, 320.0), 8.0, "3/4 周期上缘");
}

#[test]
fn slow_zoom_on_odd_size_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("camera-move-zoom");
    let input = dir.join("src/odd.mp4");
    make_gradient(&paths, &input, (161, 121), "X", false);
    let out = run(
        &dir,
        &input,
        json!({"direction": "zoom", "zoom": 2, "autoPeriod": false, "periodSeconds": 2}),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (160, 120));
    assert_eq!(stream_count(&paths, &out, "a"), 0);
    assert_eq!(streams(&paths, &out).frames, 50);
    // 开头放大倍数 1（全画面），半个周期放大到 2 倍（只看到中间一半）
    let left = |t| gray(&paths, &out, t, (0, 40, 3, 40));
    assert!(left(0.0) < 12.0, "开头是全画面：{}", left(0.0));
    assert_close(
        left(1.0),
        gradient_gray(41.0, 161.0),
        10.0,
        "最大放大时的左缘",
    );
    assert!(left(1.0) - left(0.0) > 40.0);
}
