//! 集成测试：`tool.subtitle-asr` 语音生成字幕。
//!
//! 参数校验类的测试总会运行。真正的识别需要 whisper.cpp 的 ggml 模型：设置环境变量
//! `CLIPRECIPE_WHISPER_MODEL` 指向模型文件（如 ggml-tiny.bin，从 huggingface.co/ggerganov/whisper.cpp 下载）
//! 后才运行，未设置时打印原因并直接返回。语音用 lavfi 的 `flite`（文字转语音）现场生成。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use cliprecipe_lib::jobs::JobEvent;
use common::*;
use serde_json::json;
use tool_support::*;

const MODEL_ENV: &str = "CLIPRECIPE_WHISPER_MODEL";

fn model() -> Option<PathBuf> {
    match std::env::var_os(MODEL_ENV) {
        Some(path) if Path::new(&path).is_file() => Some(PathBuf::from(path)),
        Some(path) => {
            println!(
                "跳过：{MODEL_ENV}={} 不是文件",
                PathBuf::from(path).display()
            );
            None
        }
        None => {
            println!("跳过：未设置 {MODEL_ENV}（指向 whisper.cpp 的 ggml 模型，如 ggml-tiny.bin），不运行真实的语音识别");
            None
        }
    }
}

/// 深蓝背景 + flite 合成的英文语音。
fn make_speech(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=0x203040:s=320x240:r=25:d=5",
        Some("flite=text='Hello world. This is a speech recognition test.':voice=slt,aresample=44100,apad=pad_dur=1"),
        &[],
    );
}

fn fake_model(dir: &TempDir, header: &[u8]) -> PathBuf {
    let name: String = header.iter().map(|b| format!("{b:02x}")).collect();
    let path = dir.join(&format!("models/ggml-{name}.bin"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = header.to_vec();
    bytes.extend([0u8; 64]);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// 第 `time` 秒那一帧中区域 (x, y, w, h) 里接近白色的像素个数。
fn white_pixels(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> usize {
    let output = cliprecipe_lib::ffmpeg::command(&paths.ffmpeg)
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
        .arg(format!("crop={w}:{h}:{x}:{y},format=rgb24"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(3)
        .filter(|p| p.iter().all(|c| *c > 200))
        .count()
}

fn failure(c: &Collector, id: &str) -> String {
    match c.final_state(id) {
        Some(JobEvent::TaskState { error, .. }) => error.unwrap_or_default(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn invalid_model_and_missing_paths_fail_with_clear_messages() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("asr-invalid");
    let input = dir.join("clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let out = dir.join("out");
    let not_ggml = fake_model(&dir, b"GGUF");
    let cases = [
        (json!({}), "请选择 whisper 模型文件"),
        (
            json!({"modelPath": dir.join("none.bin").to_string_lossy()}),
            "模型文件不存在",
        ),
        (
            json!({"modelPath": not_ggml.to_string_lossy()}),
            "不是 whisper.cpp 的 ggml 模型",
        ),
        (
            json!({"modelPath": fake_model(&dir, b"lmgg").to_string_lossy(), "engine": "cli"}),
            "whisper-cli",
        ),
        (
            json!({"modelPath": fake_model(&dir, b"lmgg").to_string_lossy(), "engine": "cli", "cliPath": dir.join("whisper-cli.exe").to_string_lossy()}),
            "whisper-cli 程序不存在",
        ),
        (
            json!({"modelPath": fake_model(&dir, b"lmgg").to_string_lossy(), "language": "chinese"}),
            "识别语言无效",
        ),
    ];
    for (params, expected) in cases {
        let (summary, c) = run_tool(
            tool_config(&paths, &out, &dir.join("tmp")),
            registered("tool.subtitle-asr", params.clone()),
            &[video_task("t1", &input)],
        );
        assert_eq!(summary.failed, 1, "{params}");
        let message = failure(&c, "t1");
        assert!(message.contains(expected), "{params}：{message}");
    }
    assert!(all_files(&out).is_empty());
}

#[test]
fn video_without_audio_fails() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("asr-silent");
    let input = dir.join("silent.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    let model = fake_model(&dir, b"lmgg");
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-asr",
            json!({"modelPath": model.to_string_lossy()}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(failure(&c, "t1").contains("没有音轨"));
    assert!(all_files(&out).is_empty());
}

/// 真实识别：内置 whisper 滤镜生成 SRT，并烧录进新视频。
#[test]
fn builtin_whisper_transcribes_speech_and_burns_subtitles() {
    let Some(model) = model() else { return };
    let paths = ffmpeg_paths();
    let dir = TempDir::new("asr-real");
    let input = dir.join("src/speech.mp4");
    make_speech(&paths, &input);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-asr",
            json!({
                "engine": "builtin",
                "modelPath": model.to_string_lossy(),
                "language": "en",
                "burn": true,
                "fontSize": 8,
                "fontColor": "#ffffff"
            }),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(
        outputs,
        [out.join("speech.srt"), out.join("speech_subtitled.mp4")]
    );
    let srt = std::fs::read_to_string(&outputs[0]).unwrap();
    println!("识别结果：\n{srt}");
    assert!(srt.starts_with("1\n"), "序号从 1 开始：{srt}");
    assert!(srt.contains(" --> "), "{srt}");
    let lower = srt.to_lowercase();
    assert!(
        lower.contains("hello") && lower.contains("test"),
        "识别出关键词：{srt}"
    );

    // 烧录：说话时画面下方出现白色文字，原视频没有
    let burned = &outputs[1];
    let info = probe(&paths, burned);
    assert_close(
        info.duration,
        probe(&paths, &input).duration,
        0.15,
        "时长不变",
    );
    let region = (0, 150, 320, 90);
    let before = white_pixels(&paths, &input, 1.0, region);
    let after = white_pixels(&paths, burned, 1.0, region);
    assert_eq!(before, 0, "原视频没有文字");
    assert!(after > 100, "字幕烧录进画面下方：{after} 个白色像素");
    assert_eq!(
        white_pixels(&paths, burned, 1.0, (0, 0, 320, 120)),
        0,
        "画面上半部分没有字幕"
    );
    assert!(all_files(&dir.join("tmp")).is_empty());
}

/// 真实识别：字幕放在源视频旁，已存在时加序号。
#[test]
fn srt_beside_the_source_does_not_overwrite_by_default() {
    let Some(model) = model() else { return };
    let paths = ffmpeg_paths();
    let dir = TempDir::new("asr-beside");
    let input = dir.join("src/talk.mp4");
    make_speech(&paths, &input);
    std::fs::write(dir.join("src/talk.srt"), "existing").unwrap();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-asr",
            json!({"modelPath": model.to_string_lossy(), "srtLocation": "source"}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(done_outputs(&c, "t1").is_empty(), "源视频旁的字幕不算输出");
    assert_eq!(
        std::fs::read_to_string(dir.join("src/talk.srt")).unwrap(),
        "existing"
    );
    let srt = std::fs::read_to_string(dir.join("src/talk (1).srt")).unwrap();
    assert!(srt.to_lowercase().contains("hello"), "{srt}");
    assert!(all_files(&out).is_empty());
}
