//! 集成测试：`pipe.add-watermark` 加水印，真实运行 ffmpeg 后用 ffprobe 与像素取样断言：
//! 水印出现在预期位置、大小与不透明度大致正确、别处不受影响；去背景色、滚动、分段依次显示、
//! 动态素材循环 / 倒放、子水印；输出时长始终等于视频时长。
//!
//! 主视频用纯灰色画面（便于区分水印像素），素材全部用 lavfi 现场生成。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, PickOrder, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::add_watermark::{Anchor, ScrollMode, SubWatermarkParams};
use cliprecipe_lib::pipeline::modules::AddWatermarkParams;
use common::*;

/// 用 ffmpeg 生成任意素材（参数直接传给 ffmpeg，输出路径放最后）。
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

/// 灰色（128）加少量随机噪点的主视频；奇数尺寸时用 yuv444p。
/// 噪点让源码率足够高：纯色画面的码率只有几 kbit/s，按源码率重新编码时开头几帧会糊成一片。
fn gray_clip(paths: &FfmpegPaths, out: &Path, width: u32, height: u32, seconds: f64, audio: bool) {
    let video =
        format!("color=c=0x808080:s={width}x{height}:r=25:d={seconds},noise=alls=12:allf=t+u");
    let sine = format!("sine=frequency=440:duration={seconds}");
    let pix_fmt = if width % 2 == 1 || height % 2 == 1 {
        "yuv444p"
    } else {
        "yuv420p"
    };
    let mut args = vec!["-f", "lavfi", "-i", &video];
    if audio {
        args.extend(["-f", "lavfi", "-i", &sine, "-c:a", "aac", "-shortest"]);
    }
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

/// 纯色图片（不带透明通道）。
fn solid_png(paths: &FfmpegPaths, out: &Path, color: &str, w: u32, h: u32) {
    generate(
        paths,
        out,
        &[
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s={w}x{h}"),
            "-frames:v",
            "1",
        ],
    );
}

/// 前半段蓝色、后半段黄色的动态素材（gif / mp4），总长 `seconds` 秒。
fn blue_yellow(paths: &FfmpegPaths, out: &Path, seconds: f64) {
    let half = seconds / 2.0;
    let blue = format!("color=c=blue:s=40x40:r=10:d={half}");
    let yellow = format!("color=c=yellow:s=40x40:r=10:d={half}");
    let mut args = vec![
        "-f",
        "lavfi",
        "-i",
        &blue,
        "-f",
        "lavfi",
        "-i",
        &yellow,
        "-filter_complex",
        "[0:v][1:v]concat=n=2:v=1:a=0",
    ];
    if out.extension().is_some_and(|e| e == "mp4") {
        args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    }
    generate(paths, out, &args);
}

/// 100×50 红色、四周 10 像素透明的 rgba 素材（png 或 qtrle mov）。
fn red_with_alpha_ring(paths: &FfmpegPaths, out: &Path) {
    let source = "color=c=red:s=100x50:r=10:d=1,format=rgba,\
                  geq=r=255:g=0:b=0:a='if(between(X,10,89)*between(Y,10,39),255,0)'";
    if out.extension().is_some_and(|e| e == "mov") {
        generate(
            paths,
            out,
            &[
                "-f", "lavfi", "-i", source, "-c:v", "qtrle", "-pix_fmt", "argb",
            ],
        );
    } else {
        generate(paths, out, &["-f", "lavfi", "-i", source, "-frames:v", "1"]);
    }
}

fn file(path: &Path) -> AssetSource {
    AssetSource::File {
        path: path.to_string_lossy().into_owned(),
    }
}

/// 只启用加水印处理一个文件，返回输出路径。
fn run(paths: &FfmpegPaths, params: AddWatermarkParams, input: &Path, out: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.add_watermark = AddWatermarkParams {
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
            seed: 11,
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

fn is_gray(c: [i32; 3]) -> bool {
    c.iter().all(|v| (100..=156).contains(v))
}

fn is_red(c: [i32; 3]) -> bool {
    c[0] > 200 && c[1] < 60 && c[2] < 60
}

fn is_blue(c: [i32; 3]) -> bool {
    c[0] < 60 && c[1] < 60 && c[2] > 200
}

fn is_yellow(c: [i32; 3]) -> bool {
    c[0] > 200 && c[1] > 200 && c[2] < 60
}

fn is_green(c: [i32; 3]) -> bool {
    c[0] < 60 && c[1] > 200 && c[2] < 60
}

/// 输出与源视频的时长一致（容器、视频、音频）。
fn assert_same_duration(paths: &FfmpegPaths, output: &Path, seconds: f64, audio: bool) {
    let s = streams(paths, output);
    assert_close(s.video, seconds, 0.05, "视频时长");
    assert_close(s.format, seconds, 0.07, "容器时长");
    let frames = (seconds * 25.0).round() as u64;
    assert!(s.frames.abs_diff(frames) <= 1, "帧数 {}", s.frames);
    assert_eq!(s.audio.is_some(), audio, "音频流");
    if let Some(a) = s.audio {
        assert_close(a, seconds, 0.07, "音频时长");
    }
}

#[test]
fn alpha_png_at_top_right_with_size_and_opacity() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-png");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 2.0, true);
    let logo = dir.join("assets/logo.png");
    red_with_alpha_ring(&paths, &logo);

    // 宽 25% → 80×40；边距 5% → x = 320-80-16 = 224，y = 12；透明边框缩为 8 像素
    let params = AddWatermarkParams {
        source: file(&logo),
        anchor: Anchor::TopRight,
        size: Range::fixed(25.0),
        margin_x: Range::fixed(5.0),
        margin_y: Range::fixed(5.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    let output = run(&paths, params.clone(), &main, &dir.join("out1"));
    let info = probe(&paths, &output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert_same_duration(&paths, &output, 2.0, true);

    for t in [0.2, 1.8] {
        let inside = avg_rgb(&paths, &output, t, (236, 24, 56, 16));
        assert!(is_red(inside), "水印中心应为红色：{inside:?}");
    }
    // 不透明区域的边界：x 232～296、y 20～44
    assert!(is_red(avg_rgb(&paths, &output, 1.0, (234, 21, 4, 4))));
    assert!(is_red(avg_rgb(&paths, &output, 1.0, (290, 38, 4, 4))));
    // 透明边框、水印之外都是原画面
    for region in [
        (225, 13, 4, 4),
        (300, 46, 4, 4),
        (200, 20, 20, 20),
        (240, 60, 40, 20),
        (0, 0, 60, 60),
        (0, 180, 60, 60),
        (260, 180, 60, 60),
        (130, 100, 60, 40),
    ] {
        let c = avg_rgb(&paths, &output, 1.0, region);
        assert!(is_gray(c), "{region:?} 应为原画面：{c:?}");
    }

    // 50% 不透明度：红色与灰色各半
    let half = AddWatermarkParams {
        opacity: Range::fixed(50.0),
        ..params
    };
    let output = run(&paths, half, &main, &dir.join("out2"));
    let c = avg_rgb(&paths, &output, 1.0, (236, 24, 56, 16));
    assert!(
        (165..=215).contains(&c[0]) && (40..=90).contains(&c[1]),
        "半透明：{c:?}"
    );
}

#[test]
fn colour_key_removes_background() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-key");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 1.0, true);
    // 白底红心 100×100（不带透明通道）
    let logo = dir.join("assets/white.png");
    generate(
        &paths,
        &logo,
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=white:s=100x100,drawbox=x=25:y=25:w=50:h=50:color=red:t=fill",
            "-frames:v",
            "1",
        ],
    );
    // 居中、宽 50% → 160×160 位于 (80, 40)，红心 (120, 80) 80×80
    let params = AddWatermarkParams {
        source: file(&logo),
        anchor: Anchor::Center,
        size: Range::fixed(50.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    let plain = run(&paths, params.clone(), &main, &dir.join("plain"));
    let white = avg_rgb(&paths, &plain, 0.5, (86, 46, 24, 24));
    assert!(
        white.iter().all(|v| *v > 230),
        "不去背景时为白色：{white:?}"
    );

    let keyed = AddWatermarkParams {
        key_enabled: true,
        key_color: "#ffffff".into(),
        key_similarity: 0.1,
        key_blend: 0.0,
        ..params
    };
    let output = run(&paths, keyed, &main, &dir.join("keyed"));
    let bg = avg_rgb(&paths, &output, 0.5, (86, 46, 24, 24));
    assert!(is_gray(bg), "白色背景应被去掉：{bg:?}");
    let center = avg_rgb(&paths, &output, 0.5, (140, 100, 40, 40));
    assert!(is_red(center), "红心保留：{center:?}");
    assert_same_duration(&paths, &output, 1.0, true);
}

#[test]
fn scrolling_moves_between_timestamps() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-scroll");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 2.0, false);
    let logo = dir.join("assets/red.png");
    solid_png(&paths, &logo, "red", 40, 40);
    // 宽 12.5% → 40×40，左中；从右向左每秒 50% × 320 = 160 像素：x = 320 - mod(160t, 360)
    let params = AddWatermarkParams {
        source: file(&logo),
        anchor: Anchor::Left,
        size: Range::fixed(12.5),
        opacity: Range::fixed(100.0),
        scroll: ScrollMode::Left,
        scroll_speed: Range::fixed(50.0),
        ..AddWatermarkParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    assert_same_duration(&paths, &output, 2.0, false);
    // t≈0.5：x≈240；t≈1.0：x≈160；y = 100
    let early_right = avg_rgb(&paths, &output, 0.5, (250, 110, 16, 20));
    let early_left = avg_rgb(&paths, &output, 0.5, (170, 110, 16, 20));
    let late_right = avg_rgb(&paths, &output, 1.0, (250, 110, 16, 20));
    let late_left = avg_rgb(&paths, &output, 1.0, (170, 110, 16, 20));
    assert!(is_red(early_right), "{early_right:?}");
    assert!(is_gray(early_left), "{early_left:?}");
    assert!(is_gray(late_right), "{late_right:?}");
    assert!(is_red(late_left), "{late_left:?}");
    // 纵向位置不变：上下都是原画面
    assert!(is_gray(avg_rgb(&paths, &output, 1.0, (160, 40, 40, 40))));
    assert!(is_gray(avg_rgb(&paths, &output, 1.0, (160, 160, 40, 40))));
}

#[test]
fn animated_gif_loops_and_reverses() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-gif");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 3.0, true);
    let gif = dir.join("assets/anim.gif");
    blue_yellow(&paths, &gif, 1.0);
    // 居中、宽 25% → 80×80 位于 (120, 80)
    let params = AddWatermarkParams {
        source: file(&gif),
        anchor: Anchor::Center,
        size: Range::fixed(25.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    let region = (140, 100, 40, 40);
    let output = run(&paths, params.clone(), &main, &dir.join("loop"));
    assert_same_duration(&paths, &output, 3.0, true);
    // 1 秒的素材循环播放：每秒前半蓝、后半黄
    for (t, blue) in [(0.2, true), (0.7, false), (2.2, true), (2.7, false)] {
        let c = avg_rgb(&paths, &output, t, region);
        assert!(if blue { is_blue(c) } else { is_yellow(c) }, "t={t}：{c:?}");
    }
    assert!(is_gray(avg_rgb(&paths, &output, 2.2, (0, 0, 80, 60))));

    let reversed = AddWatermarkParams {
        reverse: true,
        ..params
    };
    let output = run(&paths, reversed, &main, &dir.join("reverse"));
    assert_same_duration(&paths, &output, 3.0, true);
    for (t, blue) in [(0.2, false), (0.7, true), (2.2, false), (2.7, true)] {
        let c = avg_rgb(&paths, &output, t, region);
        assert!(
            if blue { is_blue(c) } else { is_yellow(c) },
            "倒放 t={t}：{c:?}"
        );
    }
}

#[test]
fn video_assets_shorter_and_longer_than_main() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-video");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 3.0, true);
    let short = dir.join("assets/short.mp4");
    blue_yellow(&paths, &short, 1.0);
    let long = dir.join("assets/long.mp4");
    blue_yellow(&paths, &long, 6.0);
    let params = |asset: &Path| AddWatermarkParams {
        source: file(asset),
        anchor: Anchor::BottomRight,
        size: Range::fixed(25.0),
        margin_x: Range::fixed(0.0),
        margin_y: Range::fixed(0.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    // 右下角 80×80：(240, 160)
    let region = (260, 180, 40, 40);
    let output = run(&paths, params(&short), &main, &dir.join("short"));
    assert_same_duration(&paths, &output, 3.0, true);
    assert!(is_blue(avg_rgb(&paths, &output, 2.2, region)));
    assert!(is_yellow(avg_rgb(&paths, &output, 2.7, region)));

    let output = run(&paths, params(&long), &main, &dir.join("long"));
    assert_same_duration(&paths, &output, 3.0, true);
    assert!(is_blue(avg_rgb(&paths, &output, 2.5, region)));
}

#[test]
fn alpha_mov_watermark_keeps_transparency() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-mov");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 2.0, true);
    let mov = dir.join("assets/alpha.mov");
    red_with_alpha_ring(&paths, &mov);
    // 左上角、宽 50% → 160×80 位于 (0, 0)，透明边框 16 像素
    let params = AddWatermarkParams {
        source: file(&mov),
        anchor: Anchor::TopLeft,
        size: Range::fixed(50.0),
        margin_x: Range::fixed(0.0),
        margin_y: Range::fixed(0.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    assert_same_duration(&paths, &output, 2.0, true);
    for t in [0.3, 1.7] {
        let c = avg_rgb(&paths, &output, t, (40, 30, 80, 20));
        assert!(is_red(c), "t={t}：{c:?}");
    }
    assert!(is_gray(avg_rgb(&paths, &output, 1.0, (2, 2, 10, 10))));
    assert!(is_gray(avg_rgb(&paths, &output, 1.0, (0, 100, 160, 60))));
}

#[test]
fn directory_turns_show_different_assets_per_segment() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-turns");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 3.0, true);
    solid_png(&paths, &dir.join("marks/a.png"), "red", 40, 40);
    solid_png(&paths, &dir.join("marks/b.png"), "0x00ff00", 40, 40);
    solid_png(&paths, &dir.join("marks/c.png"), "blue", 40, 40);
    std::fs::write(dir.join("marks/readme.txt"), b"not an image").unwrap();
    let params = AddWatermarkParams {
        source: AssetSource::Directory {
            path: dir.join("marks").to_string_lossy().into_owned(),
            recursive: false,
            order: PickOrder::Sequential,
        },
        turns: true,
        segment: 1.0,
        anchor: Anchor::Center,
        size: Range::fixed(25.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    assert_same_duration(&paths, &output, 3.0, true);
    let region = (140, 100, 40, 40);
    let first = avg_rgb(&paths, &output, 0.5, region);
    let second = avg_rgb(&paths, &output, 1.5, region);
    let third = avg_rgb(&paths, &output, 2.5, region);
    assert!(is_red(first), "{first:?}");
    assert!(is_green(second), "{second:?}");
    assert!(is_blue(third), "{third:?}");
    assert!(is_gray(avg_rgb(&paths, &output, 1.5, (0, 0, 100, 60))));
}

#[test]
fn sub_watermark_appears_with_main() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-sub");
    let main = dir.join("src/main.mp4");
    gray_clip(&paths, &main, 320, 240, 2.0, true);
    let red = dir.join("assets/red.png");
    solid_png(&paths, &red, "red", 40, 40);
    let gif = dir.join("assets/sub.gif");
    blue_yellow(&paths, &gif, 0.4);
    let params = AddWatermarkParams {
        source: file(&red),
        anchor: Anchor::TopRight,
        size: Range::fixed(20.0),
        margin_x: Range::fixed(0.0),
        margin_y: Range::fixed(0.0),
        opacity: Range::fixed(100.0),
        sub: SubWatermarkParams {
            enabled: true,
            source: file(&gif),
            anchor: Anchor::BottomLeft,
            size: Range::fixed(20.0),
            opacity: Range::fixed(100.0),
        },
        ..AddWatermarkParams::default()
    };
    let output = run(&paths, params, &main, &dir.join("out"));
    assert_same_duration(&paths, &output, 2.0, true);
    // 主水印右上 64×64，子水印左下 64×64（循环的蓝 / 黄动画）
    assert!(is_red(avg_rgb(&paths, &output, 1.0, (266, 10, 44, 44))));
    let sub = avg_rgb(&paths, &output, 1.75, (10, 186, 44, 44));
    assert!(is_blue(sub) || is_yellow(sub), "{sub:?}");
    for region in [(0, 0, 60, 60), (260, 180, 60, 60), (130, 100, 60, 40)] {
        assert!(is_gray(avg_rgb(&paths, &output, 1.0, region)), "{region:?}");
    }
}

#[test]
fn portrait_and_odd_sources_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("wm-shapes");
    let logo = dir.join("assets/red.png");
    solid_png(&paths, &logo, "red", 50, 25);
    let params = AddWatermarkParams {
        source: file(&logo),
        anchor: Anchor::BottomRight,
        size: Range::fixed(50.0),
        margin_x: Range::fixed(0.0),
        margin_y: Range::fixed(0.0),
        opacity: Range::fixed(100.0),
        ..AddWatermarkParams::default()
    };

    // 竖屏 240×320、无音频：水印 120×60 位于 (120, 260)
    let portrait = dir.join("src/portrait.mp4");
    gray_clip(&paths, &portrait, 240, 320, 1.5, false);
    let output = run(&paths, params.clone(), &portrait, &dir.join("p"));
    let video = probe(&paths, &output).video.unwrap();
    assert_eq!((video.width, video.height), (240, 320));
    assert_same_duration(&paths, &output, 1.5, false);
    assert!(is_red(avg_rgb(&paths, &output, 0.7, (140, 270, 80, 40))));
    assert!(is_gray(avg_rgb(&paths, &output, 0.7, (130, 180, 100, 60))));

    // 奇数尺寸 321×241：输出裁为 320×240；水印 160(.5)×80 贴右下
    let odd = dir.join("src/odd.mp4");
    gray_clip(&paths, &odd, 321, 241, 1.0, false);
    let output = run(&paths, params, &odd, &dir.join("o"));
    let video = probe(&paths, &output).video.unwrap();
    assert_eq!((video.width, video.height), (320, 240));
    assert_same_duration(&paths, &output, 1.0, false);
    assert!(is_red(avg_rgb(&paths, &output, 0.5, (180, 175, 120, 50))));
    assert!(is_gray(avg_rgb(&paths, &output, 0.5, (0, 0, 140, 140))));
}
