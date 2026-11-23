//! 集成测试：`pipe.pip` 画中画，真实运行 ffmpeg 后用 ffprobe 与像素取样断言：
//! 前景位于预期的矩形、周围露出背景（自身虚化 / 外部图片 / 比视频短与长的外部视频），
//! 绿幕抠像、改变画布尺寸（横屏 → 竖屏）、时长与音频不变；覆盖横屏、竖屏、奇数尺寸、无音频的源。
//!
//! 主视频是“白底 + 黑色粗边框”（加少量噪点保证源码率），这样前景的边框在输出画面里很容易定位。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::pip::{PipBackground, PipLayout, PipRect};
use cliprecipe_lib::pipeline::modules::PipParams;
use common::*;

fn generate(paths: &FfmpegPaths, out: &Path, args: &[&str]) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 主视频：画面 `video`（lavfi 源）+ 可选的 1 kHz 音频。
fn clip(paths: &FfmpegPaths, out: &Path, video: &str, odd: bool, seconds: f64, audio: bool) {
    let sine = format!("sine=frequency=1000:duration={seconds}:sample_rate=44100");
    let mut args = vec!["-f", "lavfi", "-i", video];
    if audio {
        args.extend(["-f", "lavfi", "-i", &sine, "-c:a", "aac", "-shortest"]);
    }
    let pix_fmt = if odd { "yuv444p" } else { "yuv420p" };
    args.extend([
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        pix_fmt,
    ]);
    generate(paths, out, &args);
}

/// 白底、四周 20 像素黑边框的主视频。
fn framed_clip(paths: &FfmpegPaths, out: &Path, w: u32, h: u32, seconds: f64, audio: bool) {
    let video = format!(
        "color=c=white:s={w}x{h}:r=25:d={seconds},\
         drawbox=x=0:y=0:w=iw:h=ih:color=black:t=20,noise=alls=12:allf=t+u"
    );
    clip(paths, out, &video, w % 2 == 1 || h % 2 == 1, seconds, audio);
}

/// 纯色背景视频（30fps，与主视频帧率不同）。
fn color_video(paths: &FfmpegPaths, out: &Path, color: &str, seconds: f64) {
    generate(
        paths,
        out,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=160x90:r=30:d={seconds}"),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ],
    );
}

fn color_png(paths: &FfmpegPaths, out: &Path, color: &str) {
    generate(
        paths,
        out,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=64x48"),
            "-frames:v",
            "1",
        ],
    );
}

fn file(path: &Path) -> AssetSource {
    AssetSource::File {
        path: path.to_string_lossy().into_owned(),
    }
}

fn run(paths: &FfmpegPaths, params: PipParams, input: &Path, out: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.pip = PipParams {
        enabled: true,
        ..params
    };
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 5,
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
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成：{other:?}"),
    }
}

fn bright(c: [i32; 3]) -> bool {
    c.iter().all(|v| *v > 185)
}

fn dark(c: [i32; 3]) -> bool {
    c.iter().all(|v| *v < 70)
}

fn is_color(c: [i32; 3], expected: [i32; 3]) -> bool {
    c.iter().zip(expected).all(|(a, b)| (a - b).abs() <= 45)
}

const BLUE: [i32; 3] = [0, 0, 255];
const RED: [i32; 3] = [255, 0, 0];
const GREEN: [i32; 3] = [0, 255, 0];

fn size_of(paths: &FfmpegPaths, file: &Path) -> (u32, u32) {
    let video = probe(paths, file).video.unwrap();
    (video.width, video.height)
}

/// 时长、帧率、音频与源一致。
fn assert_timing_unchanged(paths: &FfmpegPaths, input: &Path, output: &Path) {
    let src = streams(paths, input);
    let out = streams(paths, output);
    assert_close(out.video, src.video, 0.05, "视频时长");
    assert_close(out.fps, 25.0, 0.01, "帧率");
    assert!(
        out.frames.abs_diff(src.frames) <= 1,
        "帧数 {} / {}",
        out.frames,
        src.frames
    );
    assert_eq!(out.audio.is_some(), src.audio.is_some(), "音频流");
    if let (Some(a), Some(b)) = (out.audio, src.audio) {
        assert_close(a, b, 0.03, "音频时长");
        assert_eq!(out.sample_rate, src.sample_rate);
    }
}

#[test]
fn own_video_blurred_background_landscape() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-self");
    let input = dir.join("src/main.mp4");
    framed_clip(&paths, &input, 320, 240, 2.0, true);
    // 50%：前景 160×120 位于 (80, 60)，边框缩为 10 像素
    let params = PipParams {
        size: Range::fixed(50.0),
        blur: Range::fixed(5.0),
        ..PipParams::default()
    };
    let output = run(&paths, params, &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (320, 240));
    assert_timing_unchanged(&paths, &input, &output);
    for t in [0.3, 1.5] {
        let border = avg_rgb(&paths, &output, t, (82, 90, 6, 60));
        assert!(dark(border), "前景左边框 t={t}：{border:?}");
        let border = avg_rgb(&paths, &output, t, (100, 172, 120, 6));
        assert!(dark(border), "前景下边框 t={t}：{border:?}");
        let inner = avg_rgb(&paths, &output, t, (110, 90, 100, 60));
        assert!(bright(inner), "前景内部 t={t}：{inner:?}");
        // 前景周围是放大虚化的画面（原画面的白色内部）
        let left = avg_rgb(&paths, &output, t, (62, 90, 12, 60));
        assert!(bright(left), "前景左侧应为背景 t={t}：{left:?}");
        let above = avg_rgb(&paths, &output, t, (110, 36, 100, 16));
        assert!(bright(above), "前景上方应为背景 t={t}：{above:?}");
        // 背景的四角是虚化后的黑边
        let corner = avg_rgb(&paths, &output, t, (0, 0, 8, 8));
        assert!(dark(corner), "背景四角 t={t}：{corner:?}");
    }
}

#[test]
fn external_image_background_with_offset() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-image");
    let input = dir.join("src/main.mp4");
    framed_clip(&paths, &input, 320, 240, 2.0, true);
    let bg = dir.join("assets/blue.png");
    color_png(&paths, &bg, "blue");
    // 50% + 向右偏移 25%：前景 160×120 位于 (160, 60)
    let params = PipParams {
        background: PipBackground::Asset,
        asset: file(&bg),
        blur: Range::fixed(0.0),
        size: Range::fixed(50.0),
        offset_x: Range::fixed(25.0),
        ..PipParams::default()
    };
    let output = run(&paths, params, &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (320, 240));
    assert_timing_unchanged(&paths, &input, &output);
    for t in [0.2, 1.9] {
        for region in [(0, 0, 150, 240), (160, 0, 160, 56), (160, 184, 160, 56)] {
            let c = avg_rgb(&paths, &output, t, region);
            assert!(is_color(c, BLUE), "{region:?} t={t} 应为蓝色背景：{c:?}");
        }
        assert!(dark(avg_rgb(&paths, &output, t, (162, 80, 6, 80))));
        assert!(dark(avg_rgb(&paths, &output, t, (312, 80, 6, 80))));
        assert!(bright(avg_rgb(&paths, &output, t, (180, 80, 120, 80))));
    }
}

#[test]
fn external_videos_shorter_and_longer_on_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-video");
    let input = dir.join("src/portrait.mp4");
    framed_clip(&paths, &input, 240, 320, 3.0, false);
    let short = dir.join("assets/red.mp4");
    color_video(&paths, &short, "red", 1.0);
    let long = dir.join("assets/green.mp4");
    color_video(&paths, &long, "0x00ff00", 5.0);
    // 60%：框 144×192，前景 144×192 位于 (48, 64)，边框 12 像素
    let params = |asset: &Path| PipParams {
        background: PipBackground::Asset,
        asset: file(asset),
        blur: Range::fixed(3.0),
        size: Range::fixed(60.0),
        ..PipParams::default()
    };
    for (asset, color) in [(&short, RED), (&long, GREEN)] {
        let output = run(&paths, params(asset), &input, &dir.join("out"));
        assert_eq!(size_of(&paths, &output), (240, 320));
        assert_timing_unchanged(&paths, &input, &output);
        assert!(probe(&paths, &output).audio.is_empty());
        // 比视频短的背景循环到结尾
        for t in [0.5, 2.5, 2.9] {
            for region in [(0, 0, 40, 60), (200, 260, 40, 60), (60, 270, 120, 40)] {
                let c = avg_rgb(&paths, &output, t, region);
                assert!(is_color(c, color), "{region:?} t={t}：{c:?}");
            }
            assert!(dark(avg_rgb(&paths, &output, t, (50, 120, 6, 80))));
            assert!(bright(avg_rgb(&paths, &output, t, (80, 110, 80, 100))));
        }
        std::fs::remove_dir_all(dir.join("out")).unwrap();
    }
}

#[test]
fn chroma_key_shows_background_through_green_screen() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-key");
    let input = dir.join("src/green.mp4");
    // 绿幕上一个白色主体 (110, 80) 100×80
    clip(
        &paths,
        &input,
        "color=c=0x00ff00:s=320x240:r=25:d=2,\
         drawbox=x=110:y=80:w=100:h=80:color=white:t=fill,noise=alls=6:allf=t+u",
        false,
        2.0,
        true,
    );
    let bg = dir.join("assets/blue.png");
    color_png(&paths, &bg, "blue");
    let params = PipParams {
        background: PipBackground::Asset,
        asset: file(&bg),
        blur: Range::fixed(0.0),
        size: Range::fixed(100.0),
        ..PipParams::default()
    };
    // 不抠像：前景铺满，整幅是绿色
    let plain = run(&paths, params.clone(), &input, &dir.join("plain"));
    assert!(is_color(
        avg_rgb(&paths, &plain, 1.0, (10, 10, 60, 60)),
        GREEN
    ));

    let keyed = PipParams {
        key_enabled: true,
        key_color: "#00ff00".into(),
        key_similarity: 0.2,
        key_blend: 0.05,
        ..params
    };
    let output = run(&paths, keyed, &input, &dir.join("keyed"));
    assert_timing_unchanged(&paths, &input, &output);
    for t in [0.2, 1.5] {
        for region in [(10, 10, 60, 60), (250, 170, 60, 60), (120, 10, 80, 50)] {
            let c = avg_rgb(&paths, &output, t, region);
            assert!(is_color(c, BLUE), "绿幕处应露出蓝色背景 {region:?}：{c:?}");
        }
        let subject = avg_rgb(&paths, &output, t, (130, 95, 60, 50));
        assert!(bright(subject), "主体保留：{subject:?}");
    }
}

#[test]
fn landscape_to_portrait_canvas() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-canvas");
    let input = dir.join("src/main.mp4");
    framed_clip(&paths, &input, 320, 240, 2.0, true);
    // 画布 240×432、前景 100%：240×180 位于 (0, 126)，边框缩为 15 像素
    let params = PipParams {
        canvas_enabled: true,
        canvas_width: 240,
        canvas_height: 432,
        size: Range::fixed(100.0),
        blur: Range::fixed(5.0),
        ..PipParams::default()
    };
    let output = run(&paths, params, &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (240, 432));
    assert_timing_unchanged(&paths, &input, &output);
    let t = 1.0;
    assert!(
        dark(avg_rgb(&paths, &output, t, (60, 128, 120, 10))),
        "前景上边框"
    );
    assert!(
        dark(avg_rgb(&paths, &output, t, (60, 294, 120, 10))),
        "前景下边框"
    );
    assert!(
        bright(avg_rgb(&paths, &output, t, (40, 170, 160, 90))),
        "前景内部"
    );
    // 上下空白处是放大取中、虚化后的画面
    assert!(
        bright(avg_rgb(&paths, &output, t, (60, 60, 120, 40))),
        "上方背景"
    );
    assert!(
        bright(avg_rgb(&paths, &output, t, (60, 330, 120, 40))),
        "下方背景"
    );
}

#[test]
fn custom_rect_on_odd_source_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("pip-rect");
    let input = dir.join("src/odd.mp4");
    framed_clip(&paths, &input, 321, 241, 1.0, false);
    assert_eq!(size_of(&paths, &input), (321, 241));
    let bg = dir.join("assets/blue.png");
    color_png(&paths, &bg, "blue");
    // 321×241 放进 (20, 20) 160×120：158×120，x = 21
    let params = PipParams {
        background: PipBackground::Asset,
        asset: file(&bg),
        blur: Range::fixed(0.0),
        layout: PipLayout::Rect,
        rect: PipRect {
            x: 20,
            y: 20,
            width: 160,
            height: 120,
        },
        ..PipParams::default()
    };
    let output = run(&paths, params, &input, &dir.join("out"));
    assert_eq!(size_of(&paths, &output), (320, 240));
    let s = streams(&paths, &output);
    assert_close(s.video, 1.0, 0.05, "视频时长");
    assert!(s.audio.is_none());
    for region in [
        (0, 0, 320, 16),
        (186, 0, 134, 240),
        (0, 144, 320, 96),
        (0, 0, 17, 240),
    ] {
        let c = avg_rgb(&paths, &output, 0.5, region);
        assert!(is_color(c, BLUE), "{region:?} 应为背景：{c:?}");
    }
    assert!(
        dark(avg_rgb(&paths, &output, 0.5, (23, 40, 5, 80))),
        "前景左边框"
    );
    assert!(
        dark(avg_rgb(&paths, &output, 0.5, (172, 40, 5, 80))),
        "前景右边框"
    );
    assert!(
        bright(avg_rgb(&paths, &output, 0.5, (40, 40, 120, 80))),
        "前景内部"
    );
}
