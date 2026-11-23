//! 集成测试：`tool.folder-merge` 文件夹合并——每个子文件夹合成一个、每 N 个一组、连接文件名。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn clip(paths: &FfmpegPaths, out: &Path, seconds: f64) {
    make_clip(
        paths,
        out,
        &Clip {
            seconds,
            ..Clip::default()
        },
    );
}

fn run(dir: &TempDir, root: &Path, params: Value) -> (cliprecipe_lib::jobs::RunSummary, Collector) {
    let paths = ffmpeg_paths();
    let mut folders = BTreeMap::new();
    folders.insert("source".to_owned(), root.to_path_buf());
    let c = collector();
    let summary = start_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.folder-merge", params),
        &[],
        &folders,
        &c,
    )
    .wait();
    (summary, c)
}

fn names(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn each_subfolder_becomes_one_video() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("folder-merge-sub");
    let root = dir.join("库");
    clip(&paths, &root.join("top.mp4"), 1.0);
    clip(&paths, &root.join("第一组/1.mp4"), 1.0);
    clip(&paths, &root.join("第一组/2.mp4"), 1.5);
    clip(&paths, &root.join("第一组/更深/3.mp4"), 0.5);
    clip(&paths, &root.join("第二组/4.mp4"), 2.0);
    clip(&paths, &root.join("第二组/5.mp4"), 1.0);

    let (summary, c) = run(&dir, &root, json!({}));
    assert_eq!(summary.done, 3, "{:#?}", c.snapshot());
    let out = dir.join("out");
    let mut outputs = all_files(&out);
    outputs.sort();
    assert_eq!(names(&outputs), ["库.mp4", "第一组.mp4", "第二组.mp4"]);
    assert_close(
        video_duration(&paths, &out.join("第一组.mp4")),
        3.0,
        0.05,
        "子文件夹含更深层（设置包含子目录）",
    );
    assert_close(
        video_duration(&paths, &out.join("第二组.mp4")),
        3.0,
        0.05,
        "第二组",
    );
    assert_close(
        video_duration(&paths, &out.join("库.mp4")),
        1.0,
        0.05,
        "顶层只有一个视频",
    );
    assert_eq!(codecs(&paths, &out.join("第二组.mp4")), ["h264", "aac"]);
    assert!(all_files(&dir.join("tmp")).is_empty());
}

#[test]
fn every_n_files_with_joined_names() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("folder-merge-n");
    let root = dir.join("src");
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        clip(
            &paths,
            &root.join(format!("{name}.mp4")),
            1.0 + i as f64 * 0.5,
        );
    }
    let (summary, c) = run(
        &dir,
        &root,
        json!({"grouping": "everyN", "groupSize": 2, "joinNames": true, "mode": "reencode"}),
    );
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    let out = dir.join("out");
    assert_eq!(
        done_outputs(&c, "tool.folder-merge#1"),
        [out.join("a b.mp4")]
    );
    assert_eq!(done_outputs(&c, "tool.folder-merge#2"), [out.join("c.mp4")]);
    assert_close(
        video_duration(&paths, &out.join("a b.mp4")),
        2.5,
        0.08,
        "第一组",
    );
}
