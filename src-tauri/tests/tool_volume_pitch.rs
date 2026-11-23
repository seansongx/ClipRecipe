//! 集成测试：`tool.volume-pitch` 音量音调——增益、变调（时长不变）、两遍 loudnorm 统一响度，视频直接复制。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use common::*;
use serde_json::json;
use tool_support::*;

/// 3 秒测试视频：testsrc2 + 指定音量的正弦波。
fn make_tone(paths: &FfmpegPaths, out: &Path, freq: u32, volume: f64) {
    make_lavfi(
        paths,
        out,
        "testsrc2=s=320x240:r=25:d=3",
        Some(&format!(
            "sine=frequency={freq}:duration=3:sample_rate=44100,volume={volume}"
        )),
        &[],
    );
}

/// 第一条音轨的一段 ffmpeg 日志（`-af 滤镜 -f null -`）。
fn audio_log(paths: &FfmpegPaths, file: &Path, filter: &str) -> String {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-af", filter, "-f", "null", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// volumedetect 的平均音量（dB）。
fn mean_volume(paths: &FfmpegPaths, file: &Path) -> f64 {
    let log = audio_log(paths, file, "volumedetect");
    log.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches("dB").trim().parse().ok())
        .unwrap_or_else(|| panic!("读不到平均音量：{log}"))
}

/// ebur128 的整体响度（LUFS）。
fn integrated_loudness(paths: &FfmpegPaths, file: &Path) -> f64 {
    let log = audio_log(paths, file, "ebur128=framelog=quiet");
    log.lines()
        .rev()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("I:")
                .map(|v| v.trim().trim_end_matches("LUFS").trim().to_owned())
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("读不到整体响度：{log}"))
}

/// 用过零次数估算第一条音轨的主频（Hz）。
fn dominant_frequency(paths: &FfmpegPaths, file: &Path) -> f64 {
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file)
        .args([
            "-map", "0:a:0", "-ss", "0.5", "-t", "2", "-ac", "1", "-ar", "44100", "-f", "s16le",
            "-",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let samples: Vec<i16> = output
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    let rising = samples.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
    rising as f64 / (samples.len() as f64 / 44_100.0)
}

#[test]
fn gain_in_db_raises_the_level_and_copies_video() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("volume-gain");
    let input = dir.join("src/tone.mp4");
    make_tone(&paths, &input, 440, 0.2);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.volume-pitch",
            json!({"volumeMode": "gain", "gainUnit": "db", "gainDb": 6}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [out.join("tone.mp4")]);
    let output = &outputs[0];
    assert_close(
        mean_volume(&paths, output) - mean_volume(&paths, &input),
        6.0,
        0.3,
        "平均音量增加",
    );
    assert_eq!(
        video_packets_md5(&paths, output),
        video_packets_md5(&paths, &input),
        "视频直接复制"
    );
    assert_eq!(codecs(&paths, output), ["h264", "aac"]);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
}

#[test]
fn percent_gain_halves_the_amplitude() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("volume-percent");
    let input = dir.join("tone.mp4");
    make_tone(&paths, &input, 440, 0.4);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.volume-pitch",
            json!({"volumeMode": "gain", "gainUnit": "percent", "gainPercent": 50}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = &done_outputs(&c, "t1")[0];
    // 50% 振幅 = -6.02 dB
    assert_close(
        mean_volume(&paths, output) - mean_volume(&paths, &input),
        -6.02,
        0.3,
        "平均音量减少",
    );
}

#[test]
fn pitch_shift_moves_the_tone_and_keeps_duration() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("volume-pitch");
    let input = dir.join("tone.mp4");
    make_tone(&paths, &input, 440, 0.5);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.volume-pitch",
            json!({"volumeMode": "keep", "pitchSemitones": 12}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = &done_outputs(&c, "t1")[0];
    let before = dominant_frequency(&paths, &input);
    let after = dominant_frequency(&paths, output);
    assert_close(before, 440.0, 5.0, "原始频率");
    assert_close(after / before, 2.0, 0.03, "升高一个八度：频率翻倍");
    assert_close(
        stream_duration(&paths, output, "a:0"),
        stream_duration(&paths, &input, "a:0"),
        0.06,
        "音频时长不变",
    );
    assert_eq!(
        video_packets_md5(&paths, output),
        video_packets_md5(&paths, &input)
    );

    // 降低 7 个半音：频率 × 2^(-7/12) ≈ 0.6674
    let out2 = dir.join("out2");
    let (summary, c) = run_tool(
        tool_config(&paths, &out2, &dir.join("tmp")),
        registered(
            "tool.volume-pitch",
            json!({"volumeMode": "keep", "pitchSemitones": -7}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let lowered = &done_outputs(&c, "t1")[0];
    assert_close(
        dominant_frequency(&paths, lowered) / before,
        2f64.powf(-7.0 / 12.0),
        0.02,
        "降低 7 个半音",
    );
}

#[test]
fn loudness_normalization_brings_a_batch_to_the_target() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("volume-loudnorm");
    let quiet = dir.join("src/quiet.mp4");
    let loud = dir.join("src/loud.mp4");
    make_tone(&paths, &quiet, 440, 0.03);
    make_tone(&paths, &loud, 660, 0.9);
    let (q0, l0) = (
        integrated_loudness(&paths, &quiet),
        integrated_loudness(&paths, &loud),
    );
    assert!(l0 - q0 > 20.0, "两段素材响度差别很大：{q0} / {l0}");
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.volume-pitch", json!({"targetLufs": -18})),
        &[video_task("q", &quiet), video_task("l", &loud)],
    );
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    for (id, source) in [("q", &quiet), ("l", &loud)] {
        let output = &done_outputs(&c, id)[0];
        assert_close(
            integrated_loudness(&paths, output),
            -18.0,
            1.0,
            "统一后的响度",
        );
        assert_eq!(
            video_packets_md5(&paths, output),
            video_packets_md5(&paths, source),
            "视频直接复制"
        );
        let info = probe(&paths, output);
        assert_eq!(info.audio[0].sample_rate, 44_100, "采样率还原");
        assert_close(info.duration, 3.0, 0.1, "时长");
    }
    assert!(
        all_files(&dir.join("tmp")).is_empty(),
        "测量结果等临时文件已清理"
    );
}

#[test]
fn video_without_audio_fails_clearly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("volume-silent");
    let input = dir.join("silent.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.volume-pitch", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    let message = format!("{:?}", c.final_state("t1"));
    assert!(message.contains("没有音轨"), "{message}");
    assert!(all_files(&out).is_empty());
}
