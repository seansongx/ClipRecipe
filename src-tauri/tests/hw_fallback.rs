//! 集成测试：硬件编码失败时的软件编码回退。
//!
//! 没有真实 GPU 也能跑：在本机挑一个**试编码失败**的硬件编码器（NVENC / QSV / AMF 中本机不支持的那个），
//! 假装探测结果说它可用，通过真实的调度器运行多模块配方 + 裂变 2 次。期望：第 1 轮硬件编码失败后
//! 改用软件编码重试成功，第 2 轮直接用软件编码；回退提示只出现一次；每一轮的输出都有效。

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::encoders::test_encoder;
use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths, HwVendor, VideoCodec, VideoEncoder};
use cliprecipe_lib::jobs::{self, JobEvent, LogLevel, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::bitrate::BitrateMode;
use cliprecipe_lib::pipeline::modules::crop::{CropMode, Sides};
use cliprecipe_lib::pipeline::modules::resolution::ResolutionMode;
use cliprecipe_lib::pipeline::modules::text::{TextItem, TextSource};
use cliprecipe_lib::pipeline::modules::trim::TrimMode;
use common::*;

/// 本机不可用的一个硬件编码器。
fn unsupported_encoder(paths: &FfmpegPaths, codec: VideoCodec) -> VideoEncoder {
    HwVendor::ALL
        .into_iter()
        .map(|vendor| VideoEncoder::hardware(codec, vendor))
        .find(|encoder| !test_encoder(&paths.ffmpeg, *encoder))
        .unwrap_or_else(|| {
            panic!("本机 NVENC / QSV / AMF 的 {codec:?} 编码器都可用，无法构造硬件编码失败的情形")
        })
}

fn recipe() -> Recipe {
    let mut r = Recipe::default(); // 含帧率（30）
    let m = &mut r.modules;
    m.trim.enabled = true;
    m.trim.mode = TrimMode::HeadTail;
    m.trim.head = Range::new(0.2, 0.4);
    m.trim.tail = Range::fixed(0.0);
    m.crop.enabled = true;
    m.crop.mode = CropMode::Margins;
    m.crop.margins = Sides::all(6);
    m.resolution.enabled = true;
    m.resolution.mode = ResolutionMode::Standard;
    m.resolution.standard_height = 240;
    m.text.enabled = true;
    m.text.items = vec![TextItem {
        enabled: true,
        source: TextSource::Custom,
        custom_text: "回退 Fallback".into(),
        ..TextItem::default()
    }];
    m.bitrate.enabled = true;
    m.bitrate.mode = BitrateMode::Fixed;
    m.bitrate.kbps = Range::fixed(600);
    r.fission = 2;
    r
}

fn run_with_broken_hardware(codec: VideoCodec, expected_codec: &str) {
    let paths = ffmpeg_paths();
    let encoder = unsupported_encoder(&paths, codec);
    let dir = TempDir::new(&format!("hw-fallback-{expected_codec}"));
    let input = dir.join("src/in.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 480,
            height: 360,
            fps: 25,
            seconds: 2.0,
            audio: true,
        },
    );
    let settings = Settings {
        gpu: true,
        hevc: codec == VideoCodec::Hevc,
        ..Settings::default()
    };
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps {
                hardware: vec![encoder],
            },
            recipe: recipe(),
            settings,
            output_dir: Some(dir.join("out")),
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
    let events = c.snapshot();
    assert_eq!(summary.done, 1, "{events:#?}");
    let outputs: Vec<PathBuf> = match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            outputs,
            ..
        }) => outputs.into_iter().map(PathBuf::from).collect(),
        other => panic!("任务应完成：{other:?}"),
    };
    assert_eq!(outputs.len(), 2);

    let logs: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Log {
                level: LogLevel::Warn | LogLevel::Info,
                message,
                ..
            } => Some(message.as_str()),
            _ => None,
        })
        .collect();
    let fallbacks: Vec<&&str> = logs
        .iter()
        .filter(|m| m.starts_with("硬件编码（"))
        .collect();
    assert_eq!(fallbacks.len(), 1, "回退提示只出现一次：{logs:#?}");
    assert!(fallbacks[0].contains(encoder.name()), "{fallbacks:?}");
    // 开始提示：第 1 轮先用硬件、再用软件；第 2 轮直接用软件
    let software = VideoEncoder::software(codec).name();
    let starts: Vec<&&str> = logs.iter().filter(|m| m.starts_with("开始：")).collect();
    assert_eq!(starts.len(), 3, "{starts:#?}");
    assert!(starts[0].contains("第 1/2 次裂变") && starts[0].contains(encoder.name()));
    assert!(starts[1].contains("第 1/2 次裂变") && starts[1].contains(software));
    assert!(starts[2].contains("第 2/2 次裂变") && starts[2].contains(software));

    for output in &outputs {
        let info = probe(&paths, output);
        let video = info.video.as_ref().expect("应有视频流");
        assert_eq!(video.codec, expected_codec, "{}", output.display());
        assert_eq!(video.height, 240);
        assert_eq!(info.audio.len(), 1);
        let s = streams(&paths, output);
        let audio = s.audio.expect("应有音频");
        assert!((audio - s.video).abs() <= 0.12, "音视频时长应一致：{s:?}");
        assert!(s.format > 1.4 && s.format < 1.9, "去头后时长：{}", s.format);
        assert!(s.frames > 30, "{s:?}");
    }
    // 没有留下半截文件
    let leftovers: Vec<String> = std::fs::read_dir(dir.join("out/in"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".part."))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn h264_hardware_failure_falls_back_to_software_for_every_round() {
    run_with_broken_hardware(VideoCodec::H264, "h264");
}

#[test]
fn hevc_hardware_failure_falls_back_to_software_for_every_round() {
    run_with_broken_hardware(VideoCodec::Hevc, "hevc");
}
