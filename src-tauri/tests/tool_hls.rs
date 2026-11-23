//! 集成测试：`tool.hls` M3U8 切片——每个视频一个文件夹，播放列表有效（分片存在、相对路径、总时长等于源视频、
//! 可被 ffprobe 读取），视频直接复制。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use common::*;
use serde_json::json;
use tool_support::*;

/// 解析播放列表：（各分片时长，分片文件名，是否点播且结束）。
fn parse_playlist(text: &str) -> (Vec<f64>, Vec<String>, bool) {
    assert!(text.starts_with("#EXTM3U"), "{text}");
    let mut durations = Vec::new();
    let mut files = Vec::new();
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let d: f64 = rest
                .trim_end_matches(',')
                .split(',')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            durations.push(d);
            files.push(lines.next().expect("EXTINF 后应是分片文件名").to_owned());
        }
    }
    let vod =
        text.contains("#EXT-X-PLAYLIST-TYPE:VOD") && text.trim_end().ends_with("#EXT-X-ENDLIST");
    (durations, files, vod)
}

#[test]
fn each_video_becomes_a_folder_with_a_valid_playlist() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("hls");
    let a = dir.join("src/clip.mp4");
    let b = dir.join("src/other.mkv");
    // 每秒一个关键帧
    for (file, seconds) in [(&a, 5), (&b, 3)] {
        make_lavfi(
            &paths,
            file,
            &format!("testsrc2=s=320x240:r=25:d={seconds}"),
            Some(&format!("sine=duration={seconds}")),
            &["-g", "25", "-keyint_min", "25", "-sc_threshold", "0"],
        );
    }
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.hls", json!({"segmentSeconds": 2})),
        &[video_task("t1", &a), video_task("t2", &b)],
    );
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    let folder = &done_outputs(&c, "t1")[0];
    assert_eq!(folder, &out.join("clip"));
    assert!(folder.is_dir());
    assert_eq!(done_outputs(&c, "t2"), [out.join("other")]);

    for (folder, seconds, pieces) in [(out.join("clip"), 5.0, 3), (out.join("other"), 3.0, 2)] {
        let playlist = folder.join("index.m3u8");
        let text = std::fs::read_to_string(&playlist).unwrap();
        let (durations, files, vod) = parse_playlist(&text);
        assert!(vod, "点播列表且有结束标记：{text}");
        assert_eq!(files.len(), pieces, "{text}");
        assert_close(durations.iter().sum(), seconds, 0.1, "分片时长之和");
        for name in &files {
            assert!(!name.contains(['/', '\\']), "分片应为相对路径：{name}");
            let piece: PathBuf = folder.join(name);
            assert!(piece.is_file(), "分片不存在：{}", piece.display());
            assert_eq!(codecs(&paths, &piece), ["h264", "aac"], "直接复制");
        }
        // 文件夹里只有列表与分片
        assert_eq!(all_files(&folder).len(), files.len() + 1);
        // 播放列表可被 ffprobe 读取
        let info = probe(&paths, &playlist);
        assert_close(info.duration, seconds, 0.15, "播放列表总时长");
        assert_eq!(info.video.as_ref().map(|v| v.width), Some(320));
    }
    // 没有残留的临时文件夹（以点开头的 .part）
    let leftovers: Vec<PathBuf> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(all_files(&dir.join("tmp")).is_empty());
}

#[test]
fn existing_folder_is_never_overwritten() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("hls-exists");
    let a = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &a,
        &Clip {
            seconds: 2.0,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    std::fs::create_dir_all(out.join("clip")).unwrap();
    std::fs::write(out.join("clip/keep.txt"), b"x").unwrap();
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.hls", json!({})),
        &[video_task("t1", &a)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(done_outputs(&c, "t1"), [out.join("clip (1)")]);
    assert!(Path::new(&out.join("clip/keep.txt")).is_file());
    assert!(out.join("clip (1)/index.m3u8").is_file());
}
