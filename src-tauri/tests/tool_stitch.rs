//! 集成测试：`tool.stitch` 画面拼接（文件夹输入）。素材是纯色（每格一种颜色）或帧计数视频，
//! 每个视频带不同频率的音调，用带通滤波测音量判断输出里是哪一路声音。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::pipeline::modules::{BitrateMode, BitrateParams};
use common::*;
use serde_json::{json, Value};
use tool_support::*;

const RED: [i32; 3] = [255, 0, 0];
const GREEN: [i32; 3] = [0, 255, 0];
const BLUE: [i32; 3] = [0, 0, 255];
const YELLOW: [i32; 3] = [255, 255, 0];

/// 纯色视频（160×120、25fps），带 `tone` Hz 的音调。
fn make_color(paths: &FfmpegPaths, out: &Path, color: &str, seconds: f64, tone: u32) {
    make_lavfi(
        paths,
        out,
        &format!("color=c={color}:s=160x120:r=25:d={seconds}"),
        Some(&format!(
            "sine=frequency={tone}:duration={seconds},volume=4"
        )),
        &[],
    );
}

/// 帧计数视频（160×120、25fps、2 秒）：第 N 帧亮度 Y = 16 + 4N。
fn make_counter(paths: &FfmpegPaths, out: &Path, tone: u32) {
    make_lavfi(
        paths,
        out,
        "color=c=black:s=160x120:r=25:d=2,format=yuv420p,geq=lum='16+4*N':cb=128:cr=128",
        Some(&format!("sine=frequency={tone}:duration=2,volume=4")),
        &[],
    );
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
    let folders = BTreeMap::from([("source".to_owned(), dir.join("src"))]);
    let summary = start_tool(config, registered("tool.stitch", params), &[], &folders, &c).wait();
    assert_eq!(summary.failed + summary.cancelled, 0, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    all_files(&dir.join("out"))
}

/// 某段时间内某个频率附近的音量（dB）。
fn tone_level(paths: &FfmpegPaths, file: &Path, start: f64, duration: f64, freq: u32) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-ss",
            &start.to_string(),
            "-t",
            &duration.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-vn", "-af"])
        .arg(format!(
            "bandpass=f={freq}:width_type=q:width=10,volumedetect"
        ))
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .map(|v| {
            v.trim()
                .trim_end_matches("dB")
                .trim()
                .parse()
                .unwrap_or(-200.0)
        })
        .unwrap_or_else(|| panic!("没有音量信息：{text}"))
}

/// 每一帧中区域 (x, y, w, h) 的 Y 平均值。
fn region_lumas(paths: &FfmpegPaths, file: &Path, (x, y, w, h): (u32, u32, u32, u32)) -> Vec<f64> {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args(["-vf"])
        .arg(format!("crop={w}:{h}:{x}:{y},extractplanes=y"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact((w * h) as usize)
        .map(|f| f.iter().map(|v| f64::from(*v)).sum::<f64>() / f.len() as f64)
        .collect()
}

fn near(actual: [i32; 3], expected: [i32; 3]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() <= 40)
}

/// 四宫格各格（320×240 画面）在 `t` 秒的颜色是否为 `colors`（左上、右上、左下、右下）。
fn quadrants(paths: &FfmpegPaths, file: &Path, t: f64) -> [[i32; 3]; 4] {
    [(0, 0), (160, 0), (0, 120), (160, 120)]
        .map(|(x, y)| avg_rgb(paths, file, t, (x + 20, y + 20, 120, 80)))
}

fn assert_quadrants(paths: &FfmpegPaths, file: &Path, t: f64, colors: [[i32; 3]; 4]) {
    let actual = quadrants(paths, file, t);
    assert!(
        actual.iter().zip(colors).all(|(a, e)| near(*a, e)),
        "{t} 秒：实际 {actual:?}，期望 {colors:?}"
    );
}

#[test]
fn grid_together_shows_each_source_and_mixes_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("stitch-grid");
    make_color(&paths, &dir.join("src/1.mp4"), "red", 1.0, 300);
    make_color(&paths, &dir.join("src/2.mp4"), "0x00ff00", 2.0, 700);
    make_color(&paths, &dir.join("src/3.mp4"), "blue", 1.0, 1600);
    make_color(&paths, &dir.join("src/10.mp4"), "yellow", 1.0, 3700);
    let outputs = run(&dir, json!({"layout": "grid4", "fps": 25}));
    assert_eq!(outputs.len(), 1);
    let out = &outputs[0];
    assert_eq!(
        out.file_name().unwrap(),
        "1_拼接.mp4",
        "按自然顺序，第一个是 1.mp4"
    );
    let video = probe(&paths, out).video.unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert_quadrants(&paths, out, 0.5, [RED, GREEN, BLUE, YELLOW]);
    assert_quadrants(&paths, out, 1.6, [RED, GREEN, BLUE, YELLOW]);
    let s = streams(&paths, out);
    assert_close(s.video, 2.0, 0.1, "总时长 = 最长的视频");
    assert_close(s.audio.unwrap(), 2.0, 0.1, "音频与画面等长");
    // 全部混合：第 1 秒四个音调都在
    for tone in [300, 700, 1600, 3700] {
        let level = tone_level(&paths, out, 0.2, 0.6, tone);
        assert!(level > -45.0, "{tone} Hz：{level} dB");
    }
}

#[test]
fn sequential_freezes_waiting_cell_and_plays_current_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("stitch-sequential");
    make_counter(&paths, &dir.join("src/a.mp4"), 400);
    make_counter(&paths, &dir.join("src/b.mp4"), 2000);
    let outputs = run(
        &dir,
        json!({"playback": "sequential", "audio": "current", "fps": 25}),
    );
    let out = &outputs[0];
    let video = probe(&paths, out).video.unwrap();
    assert_eq!((video.width, video.height), (320, 120));
    let s = streams(&paths, out);
    assert_eq!(s.frames, 100, "总时长 = 2 + 2 秒");
    let left = region_lumas(&paths, out, (20, 20, 120, 80));
    let right = region_lumas(&paths, out, (180, 20, 120, 80));
    // 0～2 秒：左格播放（亮度逐帧增加），右格停在第一帧
    assert!(
        left[37] - left[12] > 80.0,
        "左格在播放：{} → {}",
        left[12],
        left[37]
    );
    assert!((right[37] - right[12]).abs() < 3.0, "右格停在第一帧");
    assert!(right[12] < 25.0, "右格是第一帧（最暗）：{}", right[12]);
    // 2～4 秒：右格播放，左格停在最后一帧
    assert!(right[87] - right[62] > 80.0, "右格在播放");
    assert!((left[87] - left[62]).abs() < 3.0, "左格停在最后一帧");
    assert!(left[62] > 200.0, "左格是最后一帧（最亮）：{}", left[62]);
    // 当前播放的音频：前 2 秒是 400 Hz，后 2 秒是 2000 Hz
    let (a1, b1) = (
        tone_level(&paths, out, 0.3, 1.4, 400),
        tone_level(&paths, out, 0.3, 1.4, 2000),
    );
    let (a2, b2) = (
        tone_level(&paths, out, 2.3, 1.4, 400),
        tone_level(&paths, out, 2.3, 1.4, 2000),
    );
    assert!(a1 > b1 + 20.0, "前 2 秒：400 Hz {a1} dB，2000 Hz {b1} dB");
    assert!(b2 > a2 + 20.0, "后 2 秒：400 Hz {a2} dB，2000 Hz {b2} dB");
    assert_close(s.audio.unwrap(), 4.0, 0.1, "音频时长");
}

#[test]
fn fill_turns_enlarges_each_video_in_turn() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("stitch-fill-turns");
    make_color(&paths, &dir.join("src/a.mp4"), "red", 1.0, 300);
    make_color(&paths, &dir.join("src/b.mp4"), "0x00ff00", 1.0, 700);
    make_color(&paths, &dir.join("src/c.mp4"), "blue", 1.0, 1600);
    make_color(&paths, &dir.join("src/d.mp4"), "yellow", 1.0, 3700);
    let outputs = run(
        &dir,
        json!({"layout": "grid4", "playback": "fillTurns", "gridSeconds": 0.5, "audio": "first", "fps": 25}),
    );
    let out = &outputs[0];
    // 时间轴：宫格 0～0.5，红 0.5～1.5，宫格 1.5～2，绿 2～3，宫格 3～3.5，蓝 3.5～4.5，宫格 4.5～5，黄 5～6，宫格 6～6.5
    let grid = [RED, GREEN, BLUE, YELLOW];
    assert_quadrants(&paths, out, 0.2, grid);
    assert_quadrants(&paths, out, 1.0, [RED; 4]);
    assert_quadrants(&paths, out, 1.75, grid);
    assert_quadrants(&paths, out, 2.5, [GREEN; 4]);
    assert_quadrants(&paths, out, 3.25, grid);
    assert_quadrants(&paths, out, 4.0, [BLUE; 4]);
    assert_quadrants(&paths, out, 5.5, [YELLOW; 4]);
    assert_quadrants(&paths, out, 6.3, grid);
    let s = streams(&paths, out);
    assert_close(s.video, 6.5, 0.1, "总时长");
    assert_close(s.audio.unwrap(), 6.5, 0.1, "音频补齐到总时长");
    // 只用第一个视频的音频（从头开始）
    assert!(tone_level(&paths, out, 0.2, 0.6, 300) > -35.0);
    // 其它视频播放（放大）时听不到它们的声音
    for (tone, start) in [(700, 2.1), (1600, 3.6), (3700, 5.1)] {
        let level = tone_level(&paths, out, start, 0.8, tone);
        assert!(level < -60.0, "{start} 秒起 {tone} Hz 不应出现：{level} dB");
    }
}

#[test]
fn incomplete_last_group_fills_background() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("stitch-groups");
    make_color(&paths, &dir.join("src/a.mp4"), "red", 1.0, 300);
    make_color(&paths, &dir.join("src/b.mp4"), "0x00ff00", 1.0, 700);
    make_color(&paths, &dir.join("src/c.mp4"), "blue", 1.0, 1600);
    let outputs = run(
        &dir,
        json!({"layout": "stacked", "background": "#ff00ff", "cellFit": "fit", "fps": 25}),
    );
    let names: Vec<String> = outputs
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["a_拼接.mp4", "c_拼接.mp4"]);
    for out in &outputs {
        let video = probe(&paths, out).video.unwrap();
        assert_eq!((video.width, video.height), (160, 240));
    }
    assert!(near(
        avg_rgb(&paths, &outputs[0], 0.5, (20, 20, 120, 80)),
        RED
    ));
    assert!(near(
        avg_rgb(&paths, &outputs[0], 0.5, (20, 140, 120, 80)),
        GREEN
    ));
    assert!(near(
        avg_rgb(&paths, &outputs[1], 0.5, (20, 20, 120, 80)),
        BLUE
    ));
    assert!(
        near(
            avg_rgb(&paths, &outputs[1], 0.5, (20, 140, 120, 80)),
            [255, 0, 255]
        ),
        "空位是背景色"
    );
}

#[test]
fn invalid_combination_is_rejected_when_planning() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("stitch-bad");
    make_color(&paths, &dir.join("src/a.mp4"), "red", 1.0, 300);
    make_color(&paths, &dir.join("src/b.mp4"), "0x00ff00", 1.0, 700);
    let folders = BTreeMap::from([("source".to_owned(), dir.join("src"))]);
    let settings = cliprecipe_lib::model::Settings::default();
    for params in [
        json!({"playback": "fillTurns"}),
        json!({"audio": "current"}),
    ] {
        let tool = registered("tool.stitch", params);
        assert!(
            cliprecipe_lib::tools::runner::plan(tool.as_ref(), &[], &folders, &settings).is_err()
        );
    }
    let one = TempDir::new("stitch-one");
    make_color(&paths, &one.join("src/a.mp4"), "red", 1.0, 300);
    let folders = BTreeMap::from([("source".to_owned(), one.join("src"))]);
    let tool = registered("tool.stitch", json!({}));
    assert!(
        cliprecipe_lib::tools::runner::plan(tool.as_ref(), &[], &folders, &settings).is_err(),
        "只有 1 个视频"
    );
}
