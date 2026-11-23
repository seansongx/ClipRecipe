//! 集成测试：`tool.subtitle-extract` 字幕提取——mkv 两条文字字幕（含中文、带语言标记）、mp4 mov_text、
//! 按语言选择、没有字幕的视频跳过。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::jobs::JobEvent;
use common::*;
use serde_json::json;
use tool_support::*;

const CHINESE: &str = "1\n00:00:00,500 --> 00:00:01,500\n你好，世界\n\n2\n00:00:02,000 --> 00:00:02,800\n第二句字幕\n";
const ENGLISH: &str = "1\n00:00:00,200 --> 00:00:01,200\nHello subtitles\n";

/// 3 秒视频 + 两条字幕（中文 chi、英文 eng）。`container` 为 mkv 或 mp4（mp4 转成 mov_text）。
fn make_subtitled(paths: &FfmpegPaths, dir: &Path, name: &str) -> PathBuf {
    let zh = dir.join("zh.srt");
    let en = dir.join("en.srt");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(&zh, CHINESE).unwrap();
    std::fs::write(&en, ENGLISH).unwrap();
    let out = dir.join(name);
    let sub_codec = if name.ends_with(".mp4") {
        "mov_text"
    } else {
        "srt"
    };
    let output = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x240:r=25:d=3"])
        .arg("-i")
        .arg(&zh)
        .arg("-i")
        .arg(&en)
        .args(["-map", "0:v", "-map", "1:s", "-map", "2:s"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:s", sub_codec])
        .args([
            "-metadata:s:s:0",
            "language=chi",
            "-metadata:s:s:1",
            "language=eng",
        ])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_file(&zh).unwrap();
    std::fs::remove_file(&en).unwrap();
    out
}

fn cue_texts(srt: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(srt).unwrap();
    text.replace("\r\n", "\n")
        .split("\n\n")
        .filter_map(|block| {
            let lines: Vec<&str> = block.lines().collect();
            lines
                .iter()
                .position(|l| l.contains("-->"))
                .map(|i| lines[i + 1..].join("\n"))
        })
        .filter(|t| !t.is_empty())
        .collect()
}

#[test]
fn mkv_text_tracks_are_exported_with_language_names() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("subx-mkv");
    let input = make_subtitled(&paths, &dir.join("src"), "movie.mkv");
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.subtitle-extract", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(
        outputs,
        [
            out.join("movie_track1_chi.srt"),
            out.join("movie_track2_eng.srt")
        ]
    );
    assert_eq!(cue_texts(&outputs[0]), ["你好，世界", "第二句字幕"]);
    assert_eq!(cue_texts(&outputs[1]), ["Hello subtitles"]);
    let first = std::fs::read_to_string(&outputs[0]).unwrap();
    assert!(first.contains("00:00:00,500 --> 00:00:01,500"), "{first}");
}

#[test]
fn ass_format_and_language_selection() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("subx-select");
    let input = make_subtitled(&paths, &dir.join("src"), "movie.mkv");
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-extract",
            json!({"tracks": "selected", "selection": "zh", "textFormat": "ass"}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [out.join("movie_track1_chi.ass")]);
    let ass = std::fs::read_to_string(&outputs[0]).unwrap();
    assert!(
        ass.contains("[Events]") && ass.contains("你好，世界"),
        "{ass}"
    );
}

#[test]
fn mp4_mov_text_becomes_srt() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("subx-mp4");
    let input = make_subtitled(&paths, &dir.join("src"), "clip.mp4");
    assert_eq!(codecs(&paths, &input), ["h264", "mov_text", "mov_text"]);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-extract",
            json!({"tracks": "selected", "selection": "2", "textFormat": "original"}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [out.join("clip_track2_eng.srt")]);
    assert_eq!(cue_texts(&outputs[0]), ["Hello subtitles"]);
}

#[test]
fn video_without_subtitles_is_skipped_with_a_note() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("subx-none");
    let input = dir.join("plain.mp4");
    make_clip(&paths, &input, &Clip::default());
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.subtitle-extract", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "跳过不算失败：{:#?}", c.snapshot());
    assert!(done_outputs(&c, "t1").is_empty());
    assert!(all_files(&out).is_empty());
    let noted = c
        .snapshot()
        .iter()
        .any(|e| matches!(e, JobEvent::Log { message, .. } if message.contains("没有内嵌字幕轨")));
    assert!(noted, "处理记录里说明跳过原因");
}

#[test]
fn invalid_selection_fails_without_output() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("subx-invalid");
    let input = make_subtitled(&paths, &dir.join("src"), "movie.mkv");
    let out = dir.join("out");
    let (summary, _) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-extract",
            json!({"tracks": "selected", "selection": "0"}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(all_files(&out).is_empty());
}
