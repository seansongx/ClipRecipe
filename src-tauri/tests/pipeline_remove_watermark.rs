//! 集成测试：`pipe.remove-watermark` 去水印。用高对比度棋盘格视频真实运行，
//! 再按像素断言：区域内细节被抹平（亮度标准差大幅下降），区域外不受影响，时段外不模糊。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Recipe, Settings};
use cliprecipe_lib::pipeline::modules::remove_watermark::{Region, RegionMode, WatermarkPreset};
use cliprecipe_lib::pipeline::modules::RemoveWatermarkParams;
use common::*;

/// 4 像素方格的黑白棋盘格视频（处处都是高对比细节）。
fn make_checker(paths: &FfmpegPaths, out: &Path, (w, h): (u32, u32), seconds: f64, audio: bool) {
    let odd = w % 2 == 1 || h % 2 == 1;
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ])
    .arg(format!(
        "nullsrc=s={w}x{h}:r=25:d={seconds},format=yuv444p,\
             geq=lum='if(mod(floor(X/4)+floor(Y/4),2),235,16)':cb=128:cr=128"
    ));
    if audio {
        cmd.args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"));
    }
    cmd.args(["-c:v", "libx264", "-preset", "ultrafast", "-qp", "0"]);
    cmd.args(["-pix_fmt", if odd { "yuv444p" } else { "yuv420p" }]);
    if audio {
        cmd.args(["-c:a", "aac", "-shortest"]);
    }
    let output = cmd.arg(out).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 只启用本模块跑一个任务，返回输出文件路径。
fn run_job(
    paths: &FfmpegPaths,
    params: RemoveWatermarkParams,
    input: &Path,
    out_dir: &Path,
) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.remove_watermark = RemoveWatermarkParams {
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

/// 第 `time` 秒那一帧某区域亮度的标准差（细节多少）。
fn detail(paths: &FfmpegPaths, file: &Path, time: f64, (x, y, w, h): (u32, u32, u32, u32)) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &time.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("crop={w}:{h}:{x}:{y},format=gray"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let px = &output.stdout;
    assert_eq!(px.len(), (w * h) as usize);
    let mean = px.iter().map(|&v| f64::from(v)).sum::<f64>() / px.len() as f64;
    let var = px
        .iter()
        .map(|&v| (f64::from(v) - mean).powi(2))
        .sum::<f64>()
        / px.len() as f64;
    var.sqrt()
}

fn assert_blurred(paths: &FfmpegPaths, file: &Path, time: f64, area: (u32, u32, u32, u32)) {
    let d = detail(paths, file, time, area);
    assert!(d < 25.0, "{time}s {area:?} 应被模糊，标准差 {d}");
}

fn assert_sharp(paths: &FfmpegPaths, file: &Path, time: f64, area: (u32, u32, u32, u32)) {
    let d = detail(paths, file, time, area);
    assert!(d > 70.0, "{time}s {area:?} 不应被模糊，标准差 {d}");
}

fn check_output(paths: &FfmpegPaths, output: &Path, size: (u32, u32), seconds: f64, audio: bool) {
    let info = probe(paths, output);
    let video = info.video.as_ref().unwrap();
    assert_eq!((video.width, video.height), size);
    assert_close(info.duration, seconds, 0.15, "时长");
    assert_eq!(info.audio.len(), usize::from(audio));
}

#[test]
fn custom_region_is_blurred_and_outside_is_untouched() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rw-custom");
    let input = dir.join("in.mp4");
    make_checker(&paths, &input, (320, 240), 2.0, true);
    // 棋盘格本身处处高对比
    assert_sharp(&paths, &input, 1.0, (60, 60, 60, 20));

    let params = RemoveWatermarkParams {
        mode: RegionMode::Custom,
        regions: vec![
            Region {
                x: 40,
                y: 40,
                width: 100,
                height: 60,
            },
            Region {
                x: 200,
                y: 180,
                width: 400, // 超出画面的部分被裁掉
                height: 100,
            },
        ],
        strength: 8,
        ..RemoveWatermarkParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_output(&paths, &output, (320, 240), 2.0, true);
    assert_blurred(&paths, &output, 1.0, (50, 50, 80, 40));
    assert_blurred(&paths, &output, 1.0, (210, 190, 100, 40));
    // 区域之外
    assert_sharp(&paths, &output, 1.0, (160, 40, 40, 100));
    assert_sharp(&paths, &output, 1.0, (0, 120, 180, 50));
}

#[test]
fn time_window_limits_the_blur() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rw-time");
    let input = dir.join("in.mp4");
    make_checker(&paths, &input, (320, 240), 6.0, true);
    let params = RemoveWatermarkParams {
        mode: RegionMode::Custom,
        regions: vec![Region {
            x: 60,
            y: 60,
            width: 200,
            height: 120,
        }],
        limit_time: true,
        start: 2.0,
        end: 4.0,
        ..RemoveWatermarkParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_output(&paths, &output, (320, 240), 6.0, true);
    let area = (100, 90, 120, 60);
    assert_sharp(&paths, &output, 1.0, area);
    assert_blurred(&paths, &output, 3.0, area);
    assert_sharp(&paths, &output, 5.0, area);
}

#[test]
fn presets_on_portrait_clip_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rw-portrait");
    let input = dir.join("in.mp4");
    make_checker(&paths, &input, (240, 320), 1.5, false);
    let params = RemoveWatermarkParams {
        presets: vec![WatermarkPreset::BottomRight, WatermarkPreset::TopBanner],
        ..RemoveWatermarkParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_output(&paths, &output, (240, 320), 1.5, false);
    // 短边 240：右下角标 72x29（距边 5），顶部横幅高 24
    assert_blurred(&paths, &output, 0.5, (175, 291, 50, 20));
    assert_blurred(&paths, &output, 0.5, (20, 2, 200, 18));
    // 左下角与中间不受影响
    assert_sharp(&paths, &output, 0.5, (4, 286, 60, 28));
    assert_sharp(&paths, &output, 0.5, (40, 120, 160, 80));
}

#[test]
fn odd_sized_clip_with_region_on_the_edge() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("rw-odd");
    let input = dir.join("in.mp4");
    make_checker(&paths, &input, (321, 241), 1.0, false);
    let params = RemoveWatermarkParams {
        mode: RegionMode::Custom,
        regions: vec![Region {
            x: 251,
            y: 171,
            width: 200,
            height: 200,
        }],
        strength: 10,
        ..RemoveWatermarkParams::default()
    };
    let output = run_job(&paths, params, &input, &dir.join("out"));
    check_output(&paths, &output, (320, 240), 1.0, false);
    assert_blurred(&paths, &output, 0.5, (262, 182, 50, 50));
    assert_sharp(&paths, &output, 0.5, (20, 20, 200, 140));
}

/// 预览（单模块预览）在区域边缘画黄色框线，只在生效时段内；正式输出没有框线。
#[test]
fn preview_outlines_regions_only_in_preview() {
    use cliprecipe_lib::ffmpeg::CancelToken;
    use cliprecipe_lib::pipeline::PreviewOutput;
    use cliprecipe_lib::preview::{render_preview, PreviewDir, PreviewSpec};

    let paths = ffmpeg_paths();
    let dir = TempDir::new("rw-preview-box");
    let input = dir.join("in.mp4");
    make_checker(&paths, &input, (640, 360), 2.0, false);
    let params = RemoveWatermarkParams {
        enabled: true,
        mode: RegionMode::Custom,
        regions: vec![Region {
            x: 100,
            y: 80,
            width: 200,
            height: 100,
        }],
        limit_time: true,
        start: 0.5,
        end: 1.5,
        ..RemoveWatermarkParams::default()
    };
    let mut recipe = Recipe::disabled();
    recipe.modules.remove_watermark = params.clone();
    let preview_dir = PreviewDir::under(dir.path(), 1);
    let frame = |time: f64| {
        let result = render_preview(
            &paths,
            &PreviewSpec {
                path: input.to_string_lossy().into_owned(),
                info: None,
                recipe: recipe.clone(),
                settings: Settings::default(),
                only_module: Some("pipe.remove-watermark".into()),
                output: PreviewOutput::Frame { time },
                seed: Some(1),
                number: 1,
                max_height: 360,
                subtitle: None,
            },
            &preview_dir,
            &CancelToken::new(),
            |_| {},
        )
        .unwrap();
        PathBuf::from(result.path)
    };
    // 框线粗 2（短边 360 / 200 → 至少 2），画在区域内侧：上边 y=80～81、左边 x=100～101
    let top_edge = (110, 80, 180, 2);
    let left_edge = (100, 90, 2, 80);
    let yellow = |rgb: [i32; 3]| rgb[0] > 170 && rgb[1] > 140 && rgb[2] < 100;
    let in_window = frame(1.0);
    for edge in [top_edge, left_edge] {
        let rgb = image_rgb(&paths, &in_window, edge);
        assert!(yellow(rgb), "预览应有黄色框线 {edge:?}：{rgb:?}");
    }
    // 生效时段外：既不模糊，也没有框线
    let outside = frame(1.8);
    let rgb = image_rgb(&paths, &outside, top_edge);
    assert!(!yellow(rgb), "时段外不应有框线：{rgb:?}");

    // 正式输出：只有模糊，没有框线
    let output = run_job(&paths, params, &input, &dir.join("out"));
    let rgb = avg_rgb(&paths, &output, 1.0, top_edge);
    assert!(
        !yellow(rgb) && (rgb[0] - rgb[2]).abs() < 30,
        "正式输出不应有框线：{rgb:?}"
    );
    assert_blurred(&paths, &output, 1.0, (120, 100, 160, 60));
}

/// 图片（预览单帧 jpg）某区域的平均颜色。不能用 `avg_rgb`：FFmpeg 9.1 对单张图片加 `-ss 0` 时不输出任何帧。
fn image_rgb(paths: &FfmpegPaths, image: &Path, (x, y, w, h): (u32, u32, u32, u32)) -> [i32; 3] {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(image)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "crop={w}:{h}:{x}:{y},scale=1:1:flags=area,format=rgb24"
        ))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        3,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    [0, 1, 2].map(|i| i32::from(output.stdout[i]))
}
