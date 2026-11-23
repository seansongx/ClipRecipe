//! 集成测试：`pipe.zoom` 动态缩放，真实运行 ffmpeg 后用 ffprobe / 像素取样断言。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::ZoomParams;
use common::*;

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(paths: &FfmpegPaths, params: ZoomParams, input: &Path, out_dir: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.zoom = params;
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out_dir.to_path_buf()),
            seed: 7,
        },
        vec![TaskSpec {
            id: "t".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(
        summary,
        RunSummary {
            done: 1,
            failed: 0,
            cancelled: 0
        },
        "{:#?}",
        c.snapshot()
    );
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成，实际：{other:?}"),
    }
}

/// 用一条 lavfi 视频描述生成测试视频（无音频）。
fn make_lavfi(paths: &FfmpegPaths, out: &Path, video: &str) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            video,
        ])
        .args(["-c:v", "libx264", "-preset", "ultrafast"])
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn brightness(rgb: [i32; 3]) -> i32 {
    rgb.iter().sum::<i32>() / 3
}

#[test]
fn zoom_in_and_out_keeps_size_and_enlarges_picture_mid_period() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("zoom-in");
    // 黑底，中心一个 80×80 的白块（x 120～200）；加噪点让源码率不至于太低（输出沿用源码率）
    let input = dir.join("src/box.mp4");
    make_lavfi(
        &paths,
        &input,
        "color=c=black:s=320x240:r=25:d=2,format=yuv420p,\
         drawbox=x=120:y=80:w=80:h=80:color=white:t=fill,noise=alls=12:allf=t",
    );
    let params = ZoomParams {
        enabled: true,
        scale: Range::new(1.0, 2.0),
        period: Range::fixed(2.0),
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert!((video.fps - 25.0).abs() < 0.01, "帧率 {}", video.fps);
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    // 白块右侧 x 222～232：开头（1 倍）是黑的，半个周期时（2 倍，白块延伸到 x=240）是白的
    let start = avg_rgb(&paths, &output, 0.0, (222, 110, 10, 20));
    let peak = avg_rgb(&paths, &output, 1.0, (222, 110, 10, 20));
    assert!(brightness(start) < 50, "开头应未放大：{start:?}");
    assert!(brightness(peak) > 180, "半个周期时应放大到 2 倍：{peak:?}");
}

#[test]
fn zoom_out_on_portrait_with_audio_shows_black_border() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("zoom-out");
    let input = dir.join("src/portrait.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 240,
            height: 320,
            ..Clip::default()
        },
    );
    let params = ZoomParams {
        enabled: true,
        scale: Range::new(0.5, 1.0),
        period: Range::new(1.0, 3.0),
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (240, 320));
    assert_eq!(info.audio.len(), 1);
    assert!((info.duration - 2.0).abs() < 0.2, "时长 {}", info.duration);
    // 开头缩小到 0.5 倍：四周是黑边
    let corner = avg_rgb(&paths, &output, 0.0, (0, 0, 40, 40));
    assert!(brightness(corner) < 20, "缩小时四周应为黑色：{corner:?}");
}

#[test]
fn odd_source_without_audio_becomes_even() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("zoom-odd");
    let input = dir.join("src/odd.mp4");
    // testsrc2 自身会把尺寸取偶数，所以先生成再缩放到奇数尺寸；yuv444p 允许奇数宽高
    make_lavfi(
        &paths,
        &input,
        "testsrc2=size=320x240:rate=25:duration=1,scale=321:241,format=yuv444p",
    );
    let src = probe(&paths, &input).video.unwrap();
    assert_eq!((src.width, src.height), (321, 241));
    let params = ZoomParams {
        enabled: true,
        ..ZoomParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert!(info.audio.is_empty());
    assert!((info.duration - 1.0).abs() < 0.2, "时长 {}", info.duration);
}
