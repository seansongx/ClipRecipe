//! 集成测试：`tool.screenshots` 批量截图。素材是“帧计数”视频（第 N 帧亮度 Y = 16 + 2N），
//! 由截图的亮度还原出截的是第几帧；中文文件名画在图片上；jpg / png 两种格式、序号 / 时间两种命名。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 4 秒、25fps 的帧计数视频（64×64）。
fn make_counter(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=black:s=64x64:r=25:d=4,format=yuv420p,geq=lum='16+2*N':cb=128:cr=128",
        None,
        &["-crf", "0"],
    );
}

fn run(dir: &TempDir, input: &Path, out: &str, params: Value) -> Vec<PathBuf> {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join(out), &dir.join("tmp")),
        registered("tool.screenshots", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let mut outputs = done_outputs(&c, "t1");
    outputs.sort();
    assert_eq!(outputs, all_files(&dir.join(out)), "只有这些输出");
    assert!(all_files(&dir.join("tmp")).is_empty());
    outputs
}

/// 图片的全部像素（RGB）与宽高。图片不能用 `-ss`（见 common::avg_rgb 的说明）。
fn image_pixels(paths: &FfmpegPaths, file: &Path) -> (Vec<[u8; 3]>, (u32, u32)) {
    let info = probe(paths, file);
    let video = info.video.unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "format=rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pixels = output
        .stdout
        .chunks_exact(3)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    (pixels, (video.width, video.height))
}

/// 截图是源视频的第几帧：灰度值换回 Y（全范围 → 有限范围），再按 Y = 16 + 2N 还原。
fn frame_index(paths: &FfmpegPaths, file: &Path) -> i64 {
    let (pixels, _) = image_pixels(paths, file);
    let mean = pixels.iter().map(|p| f64::from(p[1])).sum::<f64>() / pixels.len() as f64;
    let y = mean * 219.0 / 255.0 + 16.0;
    ((y - 16.0) / 2.0).round() as i64
}

fn names(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn count_mode_takes_evenly_spaced_frames_into_stem_folder() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("screenshots-count");
    let input = dir.join("src/clip.mp4");
    make_counter(&paths, &input);
    // 5 张：0.4、1.2、2.0、2.8、3.6 秒 → 第 10、30、50、70、90 帧
    let outputs = run(
        &dir,
        &input,
        "out",
        json!({"mode": "count", "count": 5, "format": "png"}),
    );
    assert_eq!(
        names(&outputs),
        [
            "clip_01.png",
            "clip_02.png",
            "clip_03.png",
            "clip_04.png",
            "clip_05.png"
        ]
    );
    assert!(outputs
        .iter()
        .all(|p| p.parent().unwrap() == dir.join("out/clip")));
    let frames: Vec<i64> = outputs.iter().map(|p| frame_index(&paths, p)).collect();
    assert_eq!(frames, [10, 30, 50, 70, 90]);
    assert_eq!(codecs(&paths, &outputs[0]), ["png"]);
}

#[test]
fn interval_mode_with_time_names_and_jpg() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("screenshots-interval");
    let input = dir.join("src/clip.mp4");
    make_counter(&paths, &input);
    let outputs = run(
        &dir,
        &input,
        "out",
        json!({"mode": "interval", "intervalSeconds": 1.2, "naming": "time", "quality": 95}),
    );
    assert_eq!(
        names(&outputs),
        [
            "clip_00-00-00.000.jpg",
            "clip_00-00-01.200.jpg",
            "clip_00-00-02.400.jpg",
            "clip_00-00-03.600.jpg"
        ]
    );
    let frames: Vec<i64> = outputs.iter().map(|p| frame_index(&paths, p)).collect();
    assert_eq!(frames, [0, 30, 60, 90]);
    assert_eq!(codecs(&paths, &outputs[0]), ["mjpeg"]);
}

#[test]
fn timestamps_skip_late_ones_and_single_scales() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("screenshots-times");
    let input = dir.join("src/clip.mp4");
    make_counter(&paths, &input);
    let outputs = run(
        &dir,
        &input,
        "times",
        json!({"mode": "timestamps", "timestamps": "0:01，2.2\n9", "format": "png"}),
    );
    assert_eq!(
        names(&outputs),
        ["clip_01.png", "clip_02.png"],
        "9 秒超出时长被跳过"
    );
    let frames: Vec<i64> = outputs.iter().map(|p| frame_index(&paths, p)).collect();
    assert_eq!(frames, [25, 55]);

    let single = run(
        &dir,
        &input,
        "single",
        json!({"mode": "single", "position": 50, "format": "png"}),
    );
    assert_eq!(single.len(), 1);
    assert_eq!(frame_index(&paths, &single[0]), 50);
    assert_eq!(image_pixels(&paths, &single[0]).1, (64, 64));

    let scaled = run(
        &dir,
        &input,
        "scaled",
        json!({"mode": "single", "width": 32, "format": "jpg"}),
    );
    assert_eq!(image_pixels(&paths, &scaled[0]).1, (32, 32), "按宽度缩放");
}

/// 灰色画面上画出的文件名：返回白色像素的相对坐标图案。
fn drawn_name(dir: &TempDir, name: &str) -> Vec<(usize, usize)> {
    let paths = ffmpeg_paths();
    let input = dir.join(&format!("src/{name}.mp4"));
    make_lavfi(
        &paths,
        &input,
        "color=c=0x404040:s=320x240:r=25:d=2",
        None,
        &[],
    );
    let out_dir = format!("out-{name}");
    let outputs = run(
        dir,
        &input,
        &out_dir,
        json!({
            "mode": "single", "format": "png", "drawName": true,
            "nameSize": 12, "namePosition": "bottom", "nameOutline": 0
        }),
    );
    assert_eq!(names(&outputs), [format!("{name}_01.png")]);
    let (pixels, (w, _)) = image_pixels(&paths, &outputs[0]);
    let white: Vec<(usize, usize)> = pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| p.iter().all(|c| *c > 200))
        .map(|(i, _)| (i % w as usize, i / w as usize))
        .collect();
    assert!(white.len() > 100, "{name}：文字像素 {}", white.len());
    // 文字在下方：全部白色像素在图片下半部分
    assert!(white.iter().all(|(_, y)| *y > 160), "{name}：文字应在底部");
    let (x0, y0) = (
        white.iter().map(|p| p.0).min().unwrap(),
        white.iter().map(|p| p.1).min().unwrap(),
    );
    white.iter().map(|(x, y)| (x - x0, y - y0)).collect()
}

#[test]
fn chinese_file_name_is_drawn() {
    let dir = TempDir::new("screenshots-name");
    let first = drawn_name(&dir, "第一集");
    let second = drawn_name(&dir, "第二集");
    // 缺字时三个字都是同样的方框；真正渲染出字形时“一”和“二”的图案不同
    assert_ne!(first, second, "中文字形应不同");

    // 不画文件名时没有白色像素
    let paths = ffmpeg_paths();
    let input = dir.join("src/第一集.mp4");
    let plain = run(
        &dir,
        &input,
        "plain",
        json!({"mode": "single", "format": "png"}),
    );
    let (pixels, _) = image_pixels(&paths, &plain[0]);
    assert!(pixels.iter().all(|p| p.iter().any(|c| *c <= 200)));
}

#[test]
fn invalid_timestamps_fail_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("screenshots-bad");
    let input = dir.join("src/clip.mp4");
    make_counter(&paths, &input);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered(
            "tool.screenshots",
            json!({"mode": "timestamps", "timestamps": "10, 20"}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
