//! 集成测试：`pipe.adjust` 画面调整，真实运行 ffmpeg 后用 ffprobe / 像素取样断言。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::adjust::{BorderParams, LutParams, Tweak};
use cliprecipe_lib::pipeline::modules::AdjustParams;
use common::*;

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(paths: &FfmpegPaths, params: AdjustParams, input: &Path, out_dir: &Path) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.adjust = params;
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

fn brightness(rgb: [i32; 3]) -> i32 {
    rgb.iter().sum::<i32>() / 3
}

fn on(value: Range<f64>) -> Tweak<f64> {
    Tweak {
        enabled: true,
        value,
    }
}

fn enabled() -> AdjustParams {
    AdjustParams {
        enabled: true,
        ..AdjustParams::default()
    }
}

fn check_geometry(
    paths: &FfmpegPaths,
    output: &Path,
    size: (u32, u32),
    seconds: f64,
    audio: usize,
) {
    let info = probe(paths, output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), size);
    assert!(
        (info.duration - seconds).abs() < 0.2,
        "时长 {}",
        info.duration
    );
    assert_eq!(info.audio.len(), audio);
}

#[test]
fn color_detail_and_red_border() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("adjust-color");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = AdjustParams {
        brightness: on(Range::new(0.0, 0.1)),
        contrast: on(Range::new(1.0, 1.2)),
        saturation: on(Range::new(1.0, 1.3)),
        sharpen: on(Range::fixed(1.0)),
        denoise: on(Range::fixed(3.0)),
        vignette: on(Range::fixed(30.0)),
        mosaic: Tweak {
            enabled: true,
            value: Range::fixed(8),
        },
        border: BorderParams {
            enabled: true,
            width: Range::fixed(12),
            color: "#ff0000".into(),
            random_color: false,
        },
        ..enabled()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_geometry(&paths, &output, (320, 240), 2.0, 1);
    let edge = avg_rgb(&paths, &output, 0.5, (2, 100, 6, 40));
    assert!(
        edge[0] > 180 && edge[1] < 70 && edge[2] < 70,
        "边框应为红色：{edge:?}"
    );
}

#[test]
fn black_white_mask_invert_and_opening() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("adjust-effects");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let params = AdjustParams {
        black_white: true,
        mask_invert: on(Range::new(0.2, 0.4)),
        opening: on(Range::fixed(1.0)),
        ..enabled()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_geometry(&paths, &output, (320, 240), 2.0, 1);
    // 开幕：开头两块黑幕合拢，1 秒后完全拉开
    let closed = avg_rgb(&paths, &output, 0.0, (100, 60, 120, 120));
    assert!(brightness(closed) < 20, "开头应被黑幕遮住：{closed:?}");
    let open = avg_rgb(&paths, &output, 1.5, (100, 60, 120, 120));
    assert!(brightness(open) > 30, "黑幕应已拉开：{open:?}");
    // 黑白
    let spread = open.iter().max().unwrap() - open.iter().min().unwrap();
    assert!(spread <= 8, "应为黑白画面：{open:?}");
}

#[test]
fn fade_in_and_out_on_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("adjust-fade");
    let input = dir.join("src/portrait.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 240,
            height: 320,
            audio: false,
            ..Clip::default()
        },
    );
    let params = AdjustParams {
        fade_in: on(Range::fixed(0.8)),
        fade_out: on(Range::fixed(0.8)),
        ..enabled()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_geometry(&paths, &output, (240, 320), 2.0, 0);
    let whole = (0, 0, 240, 320);
    let first = avg_rgb(&paths, &output, 0.0, whole);
    let middle = avg_rgb(&paths, &output, 1.0, whole);
    let last = avg_rgb(&paths, &output, 1.96, whole);
    assert!(brightness(first) < 15, "第一帧应为黑场：{first:?}");
    assert!(brightness(middle) > 40, "中间应正常：{middle:?}");
    assert!(brightness(last) < 25, "最后应接近黑场：{last:?}");
}

#[test]
fn lut_file_with_special_characters_in_path() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("adjust-lut");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    // 把所有颜色映射成纯红的 2×2×2 LUT；目录名含空格、逗号、单引号、方括号，检验滤镜转义
    let lut_dir = dir.join("luts, it's [x]");
    std::fs::create_dir_all(&lut_dir).unwrap();
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for _ in 0..8 {
        cube.push_str("1 0 0\n");
    }
    std::fs::write(lut_dir.join("red.cube"), cube).unwrap();
    std::fs::write(lut_dir.join("notes.txt"), "不是 LUT").unwrap();
    let params = AdjustParams {
        lut: LutParams {
            enabled: true,
            source: AssetSource::Directory {
                path: lut_dir.to_string_lossy().into_owned(),
                recursive: false,
                order: Default::default(),
            },
        },
        ..enabled()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_geometry(&paths, &output, (320, 240), 1.0, 1);
    let px = avg_rgb(&paths, &output, 0.5, (40, 40, 240, 160));
    assert!(
        px[0] > 200 && px[1] < 50 && px[2] < 50,
        "LUT 应把画面变成红色：{px:?}"
    );
}
