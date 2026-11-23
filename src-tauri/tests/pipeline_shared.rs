//! 集成测试：流水线的公共处理——奇数尺寸取偶、AAC 码率（单声道死循环问题）、ffmpeg 卡死检测。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cliprecipe_lib::ffmpeg::{self, CancelToken, EncoderCaps, FfmpegPaths, RunError};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::adjust::Tweak;
use common::*;

/// 跑一个任务，超过 `limit` 还没结束就停止并判失败（防止卡死的测试拖住整个测试集）。
fn run_with_deadline(
    paths: &FfmpegPaths,
    recipe: Recipe,
    input: &Path,
    out: &Path,
    limit: Duration,
) -> (RunSummary, Collector) {
    let c = collector();
    let handle = jobs::start(
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
    );
    let deadline = Instant::now() + limit;
    while !handle.is_finished() {
        if Instant::now() > deadline {
            handle.stop();
            let _ = handle.wait();
            panic!("任务超过 {limit:?} 仍未结束（卡死）：{:#?}", c.snapshot());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (handle.wait(), c)
}

fn output(c: &Collector) -> PathBuf {
    match c.final_state("t") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => PathBuf::from(output),
        other => panic!("任务应完成，实际：{other:?}\n{:#?}", c.snapshot()),
    }
}

fn odd_input(paths: &FfmpegPaths, dir: &TempDir, audio: bool) -> PathBuf {
    let input = dir.join("src/odd.mp4");
    make_clip(
        paths,
        &input,
        &Clip {
            width: 321,
            height: 241,
            fps: 25,
            seconds: 1.0,
            audio,
        },
    );
    let info = probe(paths, &input);
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height), (321, 241), "测试素材应为奇数尺寸");
    input
}

fn assert_even_output(paths: &FfmpegPaths, recipe: Recipe, name: &str, audio: bool) {
    let dir = TempDir::new(name);
    let input = odd_input(paths, &dir, audio);
    let (summary, c) = run_with_deadline(
        paths,
        recipe,
        &input,
        &dir.join("out"),
        Duration::from_secs(60),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let info = probe(paths, &output(&c));
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height), (320, 240));
    assert_eq!(v.pix_fmt, "yuv420p");
    assert_close(info.duration, 1.0, 0.1, "时长");
    assert_eq!(info.audio.len(), usize::from(audio));
}

#[test]
fn odd_sized_source_without_modules_is_rounded_to_even() {
    assert_even_output(&ffmpeg_paths(), Recipe::disabled(), "odd-none", true);
}

#[test]
fn odd_sized_source_with_adjust_only() {
    let mut recipe = Recipe::disabled();
    recipe.modules.adjust.enabled = true;
    recipe.modules.adjust.brightness = Tweak {
        enabled: true,
        value: Range::fixed(0.1),
    };
    recipe.modules.adjust.black_white = true;
    assert_even_output(&ffmpeg_paths(), recipe, "odd-adjust", false);
}

#[test]
fn odd_sized_source_with_hflip_only() {
    let mut recipe = Recipe::disabled();
    recipe.modules.rotate_flip.enabled = true;
    recipe.modules.rotate_flip.hflip = true;
    assert_even_output(&ffmpeg_paths(), recipe, "odd-hflip", true);
}

/// 单声道：1 秒数字静音后接 2 秒纯音，PCM 音频（放不进 mp4，必须重新编码为 AAC）。
fn make_silence_then_tone(paths: &FfmpegPaths, out: &Path) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let result = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=25:duration=3",
            "-filter_complex",
            "anullsrc=cl=mono:r=44100:d=1[s];sine=d=2:sample_rate=44100,aformat=channel_layouts=mono[t];\
             [s][t]concat=n=2:v=0:a=1[a]",
            "-map",
            "0:v",
            "-map",
            "[a]",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(out)
        .output()
        .expect("无法启动 ffmpeg");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn mono_silence_then_tone_does_not_hang_aac() {
    // FFmpeg 9.1 自带的 AAC 编码器在单声道 192k 下遇到这种音频会死循环；流水线按声道数限制码率
    let paths = ffmpeg_paths();
    let dir = TempDir::new("aac-mono");
    let input = dir.join("src/mono.mov");
    make_silence_then_tone(&paths, &input);
    assert_eq!(probe(&paths, &input).audio[0].channels, 1);
    let (summary, c) = run_with_deadline(
        &paths,
        Recipe::disabled(),
        &input,
        &dir.join("out"),
        Duration::from_secs(60),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = output(&c);
    let info = probe(&paths, &out);
    assert_eq!(info.audio[0].codec, "aac");
    assert_eq!(info.audio[0].channels, 1);
    let s = streams(&paths, &out);
    assert_close(s.audio.unwrap(), 3.0, 0.1, "音频时长");
    assert!(c.snapshot().iter().any(|e| matches!(
        e,
        JobEvent::Log { message, .. } if message.contains("-b:a 96k")
    )));
}

#[test]
fn stalled_ffmpeg_is_killed_by_watchdog() {
    // 让 ffmpeg 在一个没有人连接的 TCP 端口上等待输入：既不退出也没有任何进展
    let paths = ffmpeg_paths();
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-f",
        "mpegts",
        "-i",
        &format!("tcp://127.0.0.1:{port}?listen=1"),
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let started = Instant::now();
    let result = ffmpeg::run_with_stall_timeout(
        &paths.ffmpeg,
        &args,
        &CancelToken::new(),
        Duration::from_secs(2),
        |_| {},
    );
    let elapsed = started.elapsed();
    match result {
        Err(err @ RunError::Stalled { .. }) => {
            assert!(err.to_string().contains("卡死"), "{err}");
        }
        other => panic!("应被判定为卡死，实际：{other:?}"),
    }
    assert!(elapsed < Duration::from_secs(20), "用时 {elapsed:?}");
}
