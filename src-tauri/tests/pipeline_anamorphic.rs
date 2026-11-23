//! 集成测试：变形（非方形像素）源。流水线在所有模块之前把它换算为方形像素，
//! 输出必须是方形像素、宽高比与源的显示比例一致；片头片尾素材是变形视频时同样按显示比例拼接。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::resolution::{FillMode, ResolutionMode, ScaleMode};
use cliprecipe_lib::pipeline::modules::{IntroOutroParams, ResolutionParams};
use common::*;

/// `width`x`height`、SAR `sar`（如 `2/1`）的测试视频，带音频。
fn make_anamorphic(
    paths: &FfmpegPaths,
    out: &Path,
    width: u32,
    height: u32,
    sar: &str,
    seconds: f64,
) {
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
        ])
        .arg(format!(
            "testsrc2=size={width}x{height}:rate=25:duration={seconds},setsar={sar}"
        ))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={seconds}"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-shortest"])
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(paths: &FfmpegPaths, recipe: Recipe, input: &Path, out: &Path) -> PathBuf {
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

/// 输出为方形像素且尺寸为 `size`。
fn assert_square(paths: &FfmpegPaths, file: &Path, size: (u32, u32)) {
    let v = probe(paths, file).video.unwrap();
    assert!(v.sar_ratio().is_none(), "应为方形像素，实际 SAR {}", v.sar);
    assert_eq!((v.width, v.height), size, "{}", file.display());
}

fn brightness(rgb: [i32; 3]) -> i32 {
    rgb.iter().sum::<i32>() / 3
}

#[test]
fn source_is_normalised_to_square_pixels() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("anamorphic-plain");
    let input = dir.join("src/wide.mp4");
    make_anamorphic(&paths, &input, 240, 240, "2/1", 1.0);
    let info = probe(&paths, &input).video.unwrap();
    assert_eq!(info.sar_ratio(), Some((2, 1)));
    assert_eq!(info.square_display_size(), (480, 240));

    let out = run(&paths, Recipe::disabled(), &input, &dir.join("out"));
    assert_square(&paths, &out, (480, 240));
}

#[test]
fn resolution_and_pip_use_the_display_aspect() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("anamorphic-resolution");
    let input = dir.join("src/wide.mp4");
    make_anamorphic(&paths, &input, 240, 240, "2/1", 1.0);

    // 显示比例 2:1 放进 640x640（等比、黑边）：画面 640x320，上下各 160 像素黑边
    let mut recipe = Recipe::disabled();
    recipe.modules.resolution = ResolutionParams {
        enabled: true,
        mode: ResolutionMode::Custom,
        width: 640,
        height: 640,
        scale_mode: ScaleMode::Fit,
        fill: FillMode::Black,
        ..ResolutionParams::default()
    };
    let out = run(&paths, recipe.clone(), &input, &dir.join("out-res"));
    assert_square(&paths, &out, (640, 640));
    assert!(
        brightness(avg_rgb(&paths, &out, 0.5, (0, 0, 640, 140))) < 8,
        "上方为黑边"
    );
    assert!(
        brightness(avg_rgb(&paths, &out, 0.5, (0, 180, 640, 280))) > 40,
        "中间为画面"
    );

    // 再加画中画（默认：自身虚化背景 + 居中 80%）
    recipe.modules.pip.enabled = true;
    let out = run(&paths, recipe, &input, &dir.join("out-pip"));
    assert_square(&paths, &out, (640, 640));
}

#[test]
fn anamorphic_intro_keeps_its_display_aspect() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("anamorphic-intro");
    let input = dir.join("src/main.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 320,
            height: 240,
            seconds: 1.0,
            ..Clip::default()
        },
    );
    // 片头 240x240、SAR 2:1（显示 2:1）：放进 320x240 应为 320x160，上下各 40 像素黑边
    let intro = dir.join("assets/intro.mp4");
    make_anamorphic(&paths, &intro, 240, 240, "2/1", 1.0);
    let mut recipe = Recipe::disabled();
    recipe.modules.intro_outro = IntroOutroParams {
        enabled: true,
        intro_enabled: true,
        intro: AssetSource::File {
            path: intro.to_string_lossy().into_owned(),
        },
        ..IntroOutroParams::default()
    };
    let out = run(&paths, recipe, &input, &dir.join("out"));
    assert_square(&paths, &out, (320, 240));
    assert_close(probe(&paths, &out).duration, 2.0, 0.15, "片头 + 正片");
    assert!(
        brightness(avg_rgb(&paths, &out, 0.5, (0, 0, 320, 30))) < 8,
        "片头上方为黑边"
    );
    assert!(
        brightness(avg_rgb(&paths, &out, 0.5, (0, 0, 30, 240))) > 20,
        "片头左右没有黑边"
    );
    // 正片是方形像素：不加黑边
    assert!(brightness(avg_rgb(&paths, &out, 1.5, (0, 0, 320, 30))) > 20);
}
