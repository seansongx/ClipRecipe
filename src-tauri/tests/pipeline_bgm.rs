//! 集成测试：`pipe.bgm` 真实运行 ffmpeg。
//!
//! 测试视频的原声是 440 Hz 正弦波（见 `common::make_clip`），背景音乐用 1000 Hz 正弦波，
//! 输出后再用带通滤波 + `volumedetect` 分别测量两个频率的音量，确认音乐确实被混入、原声确实被静音。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{self, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{AssetSource, PickOrder, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::BgmParams;
use common::*;

const MUSIC_HZ: u32 = 1000;
const ORIGINAL_HZ: u32 = 440;
/// 时长允许的误差（秒）：AAC 帧约 21 ms，容器时长还会取整
const DURATION_TOLERANCE: f64 = 0.12;
/// 低于此值视为没有该频率的声音（dB）
const SILENT_DB: f64 = -50.0;
/// 高于此值视为明显有声（dB）。lavfi sine 默认振幅 1/8，平均约 -21 dB
const AUDIBLE_DB: f64 = -35.0;

/// 用 lavfi 生成一段正弦波音乐。
fn make_music(paths: &FfmpegPaths, out: &Path, hz: u32, seconds: f64) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let codec = match out.extension().and_then(|e| e.to_str()) {
        Some("mp3") => "libmp3lame",
        _ => "aac",
    };
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
            "sine=frequency={hz}:duration={seconds}:sample_rate=44100"
        ))
        .args(["-c:a", codec])
        .arg(out)
        .output()
        .expect("无法启动 ffmpeg");
    assert!(
        output.status.success(),
        "生成测试音乐失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn file(path: &Path) -> AssetSource {
    AssetSource::File {
        path: path.to_string_lossy().into_owned(),
    }
}

/// 只启用背景音模块跑一次，返回输出文件。
fn run_bgm(paths: &FfmpegPaths, dir: &TempDir, input: &Path, params: BgmParams) -> PathBuf {
    let mut recipe = Recipe::disabled();
    recipe.modules.bgm = BgmParams {
        enabled: true,
        ..params
    };
    let out_dir = dir.join("out");
    let _ = std::fs::remove_dir_all(&out_dir);
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(out_dir),
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

/// 输出必须恰好一条音频流，且时长与源视频一致。
fn assert_single_audio_and_duration(paths: &FfmpegPaths, out: &Path, expected: f64) {
    let info = probe(paths, out);
    assert_eq!(info.audio.len(), 1, "音频流数量");
    assert!(info.video.is_some());
    assert!(
        (info.duration - expected).abs() < DURATION_TOLERANCE,
        "输出时长 {}，应为 {expected}",
        info.duration
    );
    let audio = stream_duration(paths, out, "a:0");
    assert!(
        (audio - expected).abs() < DURATION_TOLERANCE,
        "音频流时长 {audio}，应为 {expected}"
    );
}

/// 测量某个频率附近的平均音量（dB）。`window` 为 `(起点, 长度)`。
fn mean_volume(paths: &FfmpegPaths, file: &Path, hz: u32, window: Option<(f64, f64)>) -> f64 {
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args(["-hide_banner", "-nostats", "-i"]).arg(file);
    // 输出端的 -ss/-t 作用在滤镜之后，所以用 atrim 在滤镜里截取时段
    let trim = window
        .map(|(start, len)| format!("atrim=start={start}:duration={len},"))
        .unwrap_or_default();
    // 两级窄带通：440 Hz 与 1000 Hz 相互衰减 40 dB 以上
    cmd.args([
        "-vn",
        "-af",
        &format!("{trim}bandpass=f={hz}:t=q:w=10,bandpass=f={hz}:t=q:w=10,volumedetect"),
        "-f",
        "null",
        "-",
    ]);
    let output = cmd.output().expect("无法启动 ffmpeg");
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{log}");
    log.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .map(|v| {
            v.trim()
                .trim_end_matches("dB")
                .trim()
                .parse::<f64>()
                .unwrap()
        })
        .unwrap_or_else(|| panic!("volumedetect 没有输出：{log}"))
}

fn clip(seconds: f64, audio: bool) -> Clip {
    Clip {
        width: 160,
        height: 120,
        seconds,
        audio,
        ..Clip::default()
    }
}

#[test]
fn duration_follows_video_for_long_and_short_music() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-duration");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &clip(3.0, true));
    let short = dir.join("music/short.m4a");
    let long = dir.join("music/long.mp3");
    make_music(&paths, &short, MUSIC_HZ, 1.0);
    make_music(&paths, &long, MUSIC_HZ, 6.0);

    for (music, loop_music) in [
        (&short, true),
        (&short, false),
        (&long, true),
        (&long, false),
    ] {
        let out = run_bgm(
            &paths,
            &dir,
            &input,
            BgmParams {
                source: file(music),
                loop_music,
                ..BgmParams::default()
            },
        );
        assert_single_audio_and_duration(&paths, &out, 3.0);
    }

    // 短音乐循环：最后一秒仍有音乐；不循环：最后一秒只剩原声
    let looped = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            source: file(&short),
            music_volume: Range::fixed(100.0),
            fade_in: 0.0,
            fade_out: 0.0,
            ..BgmParams::default()
        },
    );
    let tail = Some((2.0, 0.8));
    assert!(mean_volume(&paths, &looped, MUSIC_HZ, tail) > AUDIBLE_DB);
    let once = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            source: file(&short),
            music_volume: Range::fixed(100.0),
            loop_music: false,
            fade_in: 0.0,
            fade_out: 0.0,
            ..BgmParams::default()
        },
    );
    assert!(mean_volume(&paths, &once, MUSIC_HZ, tail) < SILENT_DB);
    assert!(mean_volume(&paths, &once, ORIGINAL_HZ, tail) > AUDIBLE_DB);
}

#[test]
fn silent_source_gets_music_track() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-silent");
    let input = dir.join("src/silent.mp4");
    make_clip(&paths, &input, &clip(2.0, false));
    assert!(probe(&paths, &input).audio.is_empty());
    // 目录模式：目录里只有一首可用的音乐
    let music_dir = dir.join("music");
    make_music(&paths, &music_dir.join("tone.mp3"), MUSIC_HZ, 5.0);
    std::fs::write(music_dir.join("notes.txt"), b"not music").unwrap();

    let out = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            source: AssetSource::Directory {
                path: music_dir.to_string_lossy().into_owned(),
                recursive: false,
                order: PickOrder::Random,
            },
            music_volume: Range::fixed(100.0),
            ..BgmParams::default()
        },
    );
    assert_single_audio_and_duration(&paths, &out, 2.0);
    assert!(mean_volume(&paths, &out, MUSIC_HZ, None) > AUDIBLE_DB);
}

#[test]
fn music_is_really_mixed_and_original_can_be_muted() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-mix");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &clip(2.0, true));
    let music = dir.join("music/tone.m4a");
    make_music(&paths, &music, MUSIC_HZ, 4.0);
    let base = BgmParams {
        source: file(&music),
        fade_in: 0.0,
        fade_out: 0.0,
        ..BgmParams::default()
    };

    // 原声 + 音乐：两个频率都在
    let mixed = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            music_volume: Range::fixed(100.0),
            ..base.clone()
        },
    );
    assert_single_audio_and_duration(&paths, &mixed, 2.0);
    let mixed_music = mean_volume(&paths, &mixed, MUSIC_HZ, None);
    let mixed_original = mean_volume(&paths, &mixed, ORIGINAL_HZ, None);
    assert!(mixed_music > AUDIBLE_DB, "{mixed_music}");
    assert!(mixed_original > AUDIBLE_DB, "{mixed_original}");

    // 原声静音：只剩音乐
    let music_only = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            music_volume: Range::fixed(100.0),
            original_volume: Range::fixed(0.0),
            ..base.clone()
        },
    );
    assert_single_audio_and_duration(&paths, &music_only, 2.0);
    assert!(mean_volume(&paths, &music_only, MUSIC_HZ, None) > AUDIBLE_DB);
    assert!(mean_volume(&paths, &music_only, ORIGINAL_HZ, None) < SILENT_DB);

    // 音乐音量 25%（-12 dB）：比 100% 时明显小
    let quiet = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            music_volume: Range::fixed(25.0),
            ..base.clone()
        },
    );
    let quiet_music = mean_volume(&paths, &quiet, MUSIC_HZ, None);
    assert!(
        (mixed_music - quiet_music - 12.0).abs() < 2.0,
        "100%: {mixed_music} dB, 25%: {quiet_music} dB"
    );

    // 两路都为 0：整段静音，但仍有一条音频流且时长不变
    let silence = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            music_volume: Range::fixed(0.0),
            original_volume: Range::fixed(0.0),
            ..base
        },
    );
    assert_single_audio_and_duration(&paths, &silence, 2.0);
    assert!(mean_volume(&paths, &silence, MUSIC_HZ, None) < SILENT_DB);
    assert!(mean_volume(&paths, &silence, ORIGINAL_HZ, None) < SILENT_DB);
}

#[test]
fn delay_and_fades() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-delay");
    let input = dir.join("src/silent.mp4");
    make_clip(&paths, &input, &clip(4.0, false));
    let music = dir.join("music/tone.m4a");
    make_music(&paths, &music, MUSIC_HZ, 10.0);

    let out = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            source: file(&music),
            music_volume: Range::fixed(100.0),
            delay: Range::fixed(1.5),
            fade_in: 0.5,
            fade_out: 1.0,
            ..BgmParams::default()
        },
    );
    assert_single_audio_and_duration(&paths, &out, 4.0);
    // 延迟期间无声；淡入后到淡出前为满音量；最后的淡出段比满音量小
    let before = mean_volume(&paths, &out, MUSIC_HZ, Some((0.0, 1.3)));
    let full = mean_volume(&paths, &out, MUSIC_HZ, Some((2.1, 0.8)));
    let fading = mean_volume(&paths, &out, MUSIC_HZ, Some((3.5, 0.45)));
    assert!(before < SILENT_DB, "延迟期间 {before} dB");
    assert!(full > AUDIBLE_DB, "满音量 {full} dB");
    assert!(fading < full - 4.0, "淡出 {fading} dB，满音量 {full} dB");
}

#[test]
fn missing_music_fails_the_task() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-missing");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &clip(1.0, true));
    let mut recipe = Recipe::disabled();
    recipe.modules.bgm = BgmParams {
        enabled: true,
        source: file(&dir.join("music/none.mp3")),
        ..BgmParams::default()
    };
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths: paths.clone(),
            encoders: EncoderCaps::software_only(),
            recipe,
            settings: Settings::default(),
            output_dir: Some(dir.join("out")),
            seed: 7,
        },
        vec![TaskSpec {
            id: "m".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    match c.final_state("m") {
        Some(JobEvent::TaskState {
            error: Some(error), ..
        }) => assert!(error.contains("不存在"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert!(dir.file_names("out").is_empty());
}

#[test]
fn play_once_short_music_fades_out_before_it_ends() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("bgm-once-fade");
    let input = dir.join("src/v.mp4");
    make_clip(&paths, &input, &clip(5.0, false));
    let music = dir.join("music/short.m4a");
    make_music(&paths, &music, MUSIC_HZ, 2.0);
    let out = run_bgm(
        &paths,
        &dir,
        &input,
        BgmParams {
            source: file(&music),
            loop_music: false,
            fade_in: 0.0,
            fade_out: 1.0,
            ..BgmParams::default()
        },
    );
    assert_single_audio_and_duration(&paths, &out, 5.0);
    // 音乐 0～2 秒：1～2 秒淡出（以前只在视频结尾 4～5 秒淡出，音乐会戛然而止）
    let full = mean_volume(&paths, &out, MUSIC_HZ, Some((0.2, 0.6)));
    let ending = mean_volume(&paths, &out, MUSIC_HZ, Some((1.7, 0.25)));
    assert!(full > AUDIBLE_DB, "满音量 {full} dB");
    assert!(
        ending < full - 8.0,
        "音乐结尾应已淡出：{ending} dB，满音量 {full} dB"
    );
}
