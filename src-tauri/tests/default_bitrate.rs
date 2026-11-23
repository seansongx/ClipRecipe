//! 集成测试：未启用“码率”（`pipe.bitrate`）时的默认码率——低码率的纯色源叠加了内容（进度条、水印）后
//! 不能糊掉。默认码率 = 源的每像素码率（换算到输出画面），但不低于质量下限（`encoders::default_video_kbps`）。
//!
//! 做法：同一个工具 / 配方分别用默认码率与固定高码率（6000k，作为参考）各跑一次，比较两者的 PSNR。
//! 沿用源码率（只有十几 kbit/s）时叠加的细节会糊成一片，PSNR 明显偏低。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::{AddWatermarkParams, BitrateMode};
use common::*;
use serde_json::json;
use tool_support::*;

/// 320×240 纯色、低码率（约 10 kbit/s）的源视频。
fn flat_low_bitrate(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=0x406080:s=320x240:r=25:d=3",
        None,
        &["-b:v", "10k", "-maxrate", "10k", "-bufsize", "20k"],
    );
    let kbps = probe(paths, out).video_bit_rate().unwrap_or(0) / 1000;
    assert!(kbps < 60, "测试源的码率应很低，实际 {kbps}k");
}

#[test]
fn tool_overlay_on_low_bitrate_source_keeps_quality() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("default-bitrate-tool");
    let input = dir.join("src/flat.mp4");
    flat_low_bitrate(&paths, &input);
    // 进度条：带章节文字与圆角，叠加的细节比纯色源复杂得多
    let params =
        json!({"height": 8.0, "rounded": true, "labelSize": 8.0, "chapters": "0,开始\n1.5,中段"});
    let run = |out: &str, config| {
        let (summary, c) = run_tool(
            config,
            registered("tool.progress-bar", params.clone()),
            &[video_task("t1", &input)],
        );
        assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
        let outputs = done_outputs(&c, "t1");
        assert_eq!(outputs.len(), 1, "{out}");
        (outputs[0].clone(), log_messages(&c))
    };
    let (default, commands) = run(
        "default",
        tool_config(&paths, &dir.join("out-default"), &dir.join("tmp")),
    );
    let (reference, _) = run(
        "reference",
        fixed_bitrate(
            tool_config(&paths, &dir.join("out-ref"), &dir.join("tmp")),
            6000,
        ),
    );
    let score = psnr(&paths, &default, &reference);
    assert!(score > 35.0, "默认码率的画质太差：PSNR {score:.1} dB");
    // 编码参数里的目标码率是质量下限（320×240×25×0.04 ≈ 77k → 最低 300k），而不是源的约 10k
    assert!(
        commands.iter().any(|c| c.contains("-b:v 300k")),
        "{commands:#?}"
    );
}

fn run_job(paths: &FfmpegPaths, recipe: Recipe, input: &Path, out: &Path) -> PathBuf {
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out.to_path_buf()),
            seed: 3,
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

#[test]
fn pipeline_watermark_on_low_bitrate_source_keeps_quality() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("default-bitrate-pipeline");
    let input = dir.join("src/flat.mp4");
    flat_low_bitrate(&paths, &input);
    // 细节丰富的水印（testsrc2 的一帧）
    let mark = dir.join("src/mark.png");
    let output = cliprecipe_lib::ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "testsrc2=s=160x120:r=1:d=1", "-frames:v", "1"])
        .arg(&mark)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut recipe = Recipe::disabled();
    recipe.modules.add_watermark = AddWatermarkParams {
        enabled: true,
        source: AssetSource::File {
            path: mark.to_string_lossy().into_owned(),
        },
        size: Range::fixed(50.0),
        ..AddWatermarkParams::default()
    };
    let default = run_job(&paths, recipe.clone(), &input, &dir.join("out-default"));
    let mut reference_recipe = recipe;
    reference_recipe.modules.bitrate.enabled = true;
    reference_recipe.modules.bitrate.mode = BitrateMode::Fixed;
    reference_recipe.modules.bitrate.kbps = Range::fixed(6000);
    let reference = run_job(&paths, reference_recipe, &input, &dir.join("out-ref"));
    let score = psnr(&paths, &default, &reference);
    assert!(score > 35.0, "默认码率的画质太差：PSNR {score:.1} dB");
}
