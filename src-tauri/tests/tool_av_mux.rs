//! 集成测试：`tool.av-mux` 音视频合成——按文件名 / 顺序 / 随机配对；替换原声与混合（用音调检测区分）；
//! 时长跟随视频（音频循环），视频直接复制。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::jobs::RunSummary;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

/// 带 440Hz 原声的视频。
fn video(paths: &FfmpegPaths, out: &Path, seconds: f64) {
    make_clip(
        paths,
        out,
        &Clip {
            seconds,
            ..Clip::default()
        },
    );
}

fn run(dir: &TempDir, audio: &Path, videos: &Path, params: Value) -> (RunSummary, Collector) {
    let paths = ffmpeg_paths();
    let mut folders = BTreeMap::new();
    folders.insert("audio".to_owned(), audio.to_path_buf());
    folders.insert("video".to_owned(), videos.to_path_buf());
    let c = collector();
    let summary = start_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.av-mux", params),
        &[],
        &folders,
        &c,
    )
    .wait();
    (summary, c)
}

#[test]
fn pairs_by_name_and_replaces_the_original_sound() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("av-mux-name");
    let (adir, vdir) = (dir.join("audio"), dir.join("video"));
    video(&paths, &vdir.join("a.mp4"), 3.0);
    video(&paths, &vdir.join("b.mp4"), 2.0);
    video(&paths, &vdir.join("c.mp4"), 2.0);
    tone(&paths, &adir.join("A.m4a"), 1000, 1.0);
    tone(&paths, &adir.join("b.mp3"), 1000, 4.0);
    tone(&paths, &adir.join("z.wav"), 1000, 1.0);

    let (summary, c) = run(&dir, &adir, &vdir, json!({}));
    assert_eq!(
        summary.done,
        2,
        "c.mp4 没有同名音频，不处理：{:#?}",
        c.snapshot()
    );
    let out = dir.join("out");
    assert_eq!(done_outputs(&c, "tool.av-mux#1"), [out.join("a.mp4")]);
    assert_eq!(done_outputs(&c, "tool.av-mux#2"), [out.join("b.mp4")]);
    for (name, seconds) in [("a", 3.0), ("b", 2.0)] {
        let file = out.join(format!("{name}.mp4"));
        let s = streams(&paths, &file);
        assert_close(s.video, seconds, 0.05, "视频时长不变");
        assert_close(
            s.audio.unwrap(),
            seconds,
            0.06,
            "音频时长跟随视频（短则循环、长则截断）",
        );
        assert_eq!(
            video_packets_md5(&paths, &file),
            video_packets_md5(&paths, &vdir.join(format!("{name}.mp4"))),
            "视频直接复制"
        );
        let (new, old) = (tone_db(&paths, &file, 1000), tone_db(&paths, &file, 440));
        assert!(new > -30.0, "新音频应清晰可闻：{new}");
        assert!(old < -38.0, "原声应被替换：{old}");
    }
    // 循环：a 的音频只有 1 秒，第 2.5 秒仍有声音
    assert!(tone_db_at(&paths, &out.join("a.mp4"), 2.2, 0.6) > -30.0);
}

fn tone_db_at(paths: &FfmpegPaths, file: &Path, start: f64, len: f64) -> f64 {
    let dir = TempDir::new("av-mux-cut");
    let cut = dir.join("cut.wav");
    let output = ffmpeg::command(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            &start.to_string(),
            "-t",
            &len.to_string(),
            "-i",
        ])
        .arg(file)
        .args(["-map", "0:a:0"])
        .arg(&cut)
        .output()
        .unwrap();
    assert!(output.status.success());
    tone_db(paths, &cut, 1000)
}

#[test]
fn keeping_the_original_mixes_both_sounds() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("av-mux-mix");
    let (adir, vdir) = (dir.join("audio"), dir.join("video"));
    video(&paths, &vdir.join("clip.mp4"), 2.0);
    tone(&paths, &adir.join("clip.mp3"), 1000, 2.0);
    let (summary, c) = run(
        &dir,
        &adir,
        &vdir,
        json!({"keepOriginal": true, "originalVolume": 100, "audioVolume": 100}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let file = &done_outputs(&c, "tool.av-mux#1")[0];
    assert!(tone_db(&paths, file, 1000) > -30.0, "新音频");
    assert!(tone_db(&paths, file, 440) > -30.0, "原声保留");
    assert_close(
        stream_duration(&paths, file, "a:0"),
        2.0,
        0.06,
        "时长跟随视频",
    );
}

#[test]
fn pairs_by_order_or_randomly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("av-mux-order");
    let (adir, vdir) = (dir.join("audio"), dir.join("video"));
    video(&paths, &vdir.join("x.mp4"), 1.0);
    video(&paths, &vdir.join("y.mp4"), 1.0);
    for name in ["1.mp3", "2.mp3", "3.mp3"] {
        tone(&paths, &adir.join(name), 1000, 1.0);
    }
    let (summary, c) = run(&dir, &adir, &vdir, json!({"pairing": "order"}));
    assert_eq!(
        summary.done,
        2,
        "两个视频，多出的音频忽略：{:#?}",
        c.snapshot()
    );

    let dir2 = TempDir::new("av-mux-random");
    let (summary, c) = run(&dir2, &adir, &vdir, json!({"pairing": "random"}));
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    let outputs: Vec<PathBuf> = ["tool.av-mux#1", "tool.av-mux#2"]
        .iter()
        .flat_map(|id| done_outputs(&c, id))
        .collect();
    for file in &outputs {
        assert!(tone_db(&paths, file, 1000) > -30.0);
    }
}
