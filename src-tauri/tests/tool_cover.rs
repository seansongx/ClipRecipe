//! 集成测试：`tool.cover` 封面处理——指定图片 / 视频画面 / 截图嵌入（mp4 封面流、mkv 封面附件），
//! 截图文件名样式，视频数据包不变，封面确实被识别且画面正确。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use common::*;
use serde_json::json;
use tool_support::*;

/// 3 秒视频：0～1 秒红、1～2 秒绿、2～3 秒蓝，带正弦音频。
fn make_rgb_clip(paths: &FfmpegPaths, out: &Path) {
    make_lavfi(
        paths,
        out,
        "color=c=red:s=320x240:r=25:d=1[a];color=c=lime:s=320x240:r=25:d=1[b];\
         color=c=blue:s=320x240:r=25:d=1[c];[a][b][c]concat=n=3",
        Some("sine=frequency=440:duration=3"),
        &[],
    );
}

/// 纯色图片。
fn make_image(paths: &FfmpegPaths, out: &Path, color: &str) {
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
        .arg(format!("color=c={color}:s=160x120"))
        .args(["-frames:v", "1", "-update", "1"])
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 一个文件（或其中某条流）第一帧的平均颜色。
fn mean_color(paths: &FfmpegPaths, file: &Path, map: Option<&str>) -> [i32; 3] {
    let mut cmd = ffmpeg::command(&paths.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(file);
    if let Some(map) = map {
        cmd.args(["-map", map]);
    }
    let output = cmd
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=1:1:flags=area,format=rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 3);
    [0, 1, 2].map(|i| i32::from(output.stdout[i]))
}

fn assert_color(actual: [i32; 3], expected: [i32; 3], what: &str) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= 12,
            "{what}：实际 {actual:?}，期望 {expected:?}"
        );
    }
}

/// 封面流（attached_pic）的序号与编码。
fn cover_streams(paths: &FfmpegPaths, file: &Path) -> Vec<String> {
    let output = ffmpeg::command(&paths.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_name:stream_disposition=attached_pic",
            "-of",
            "csv=p=0",
        ])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| l.trim_end().ends_with(",1"))
        .map(|l| l.trim().to_owned())
        .collect()
}

#[test]
fn chosen_image_is_embedded_into_mp4_without_reencoding() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("cover-image");
    let input = dir.join("src/clip.mp4");
    make_rgb_clip(&paths, &input);
    let image = dir.join("art.png");
    make_image(&paths, &image, "magenta");
    let before = std::fs::read(&input).unwrap();
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.cover",
            json!({"mode": "embed", "source": "image", "image": image.to_string_lossy()}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [out.join("clip_cover.mp4")]);
    let output = &outputs[0];
    assert!(probe(&paths, output).has_cover);
    assert_eq!(
        cover_streams(&paths, output),
        ["2,png,1"],
        "png 直接复制为封面流"
    );
    assert_color(
        mean_color(&paths, output, Some("0:v:1")),
        [255, 0, 255],
        "封面画面",
    );
    assert_eq!(
        video_packets_md5(&paths, output),
        video_packets_md5(&paths, &input),
        "视频数据包不变"
    );
    assert_eq!(codecs(&paths, output), ["h264", "aac", "png"]);
    assert_eq!(std::fs::read(&input).unwrap(), before, "源文件不变");
    assert!(all_files(&dir.join("tmp")).is_empty());
}

#[test]
fn frame_cover_is_attached_to_mkv() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("cover-mkv");
    let mp4 = dir.join("src/raw.mp4");
    make_rgb_clip(&paths, &mp4);
    let input = dir.join("src/movie.mkv");
    let remux = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&mp4)
        .args(["-c", "copy"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(remux.status.success());
    let mut config = tool_config(&paths, &dir.join("out"), &dir.join("tmp"));
    config.settings.target_format = cliprecipe_lib::model::TargetFormat::Source;
    let (summary, c) = run_tool(
        config,
        registered("tool.cover", json!({"source": "frame", "position": "1.5"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [dir.join("out/movie_cover.mkv")]);
    let output = &outputs[0];
    assert!(probe(&paths, output).has_cover, "mkv 附件被识别为封面");
    assert_eq!(cover_streams(&paths, output), ["2,mjpeg,1"]);
    assert_color(
        mean_color(&paths, output, Some("0:v:1")),
        [0, 255, 0],
        "1.5 秒处的绿色画面",
    );
    assert_eq!(
        video_packets_md5(&paths, output),
        video_packets_md5(&paths, &input)
    );
}

#[test]
fn capture_saves_named_images_and_can_embed_them() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("cover-capture");
    let a = dir.join("src/a.mp4");
    let b = dir.join("src/b.mp4");
    make_rgb_clip(&paths, &a);
    make_rgb_clip(&paths, &b);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.cover",
            json!({
                "mode": "capture",
                "position": "90%",
                "namePattern": "{index}-{name}-封面",
                "imageFormat": "png",
                "alsoEmbed": true
            }),
        ),
        &[video_task("a", &a), video_task("b", &b)],
    );
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    let first = done_outputs(&c, "a");
    assert_eq!(first, [out.join("1-a-封面.png"), out.join("a_cover.mp4")]);
    let second = done_outputs(&c, "b");
    assert_eq!(second, [out.join("2-b-封面.png"), out.join("b_cover.mp4")]);
    assert_color(
        mean_color(&paths, &first[0], None),
        [0, 0, 255],
        "90% 处的蓝色画面",
    );
    assert_color(
        mean_color(&paths, &first[1], Some("0:v:1")),
        [0, 0, 255],
        "嵌入的就是截图",
    );
    assert_eq!(
        video_packets_md5(&paths, &first[1]),
        video_packets_md5(&paths, &a)
    );

    // 只截图，不嵌入：jpg，默认名字
    let out2 = dir.join("out2");
    let (summary, c) = run_tool(
        tool_config(&paths, &out2, &dir.join("tmp")),
        registered(
            "tool.cover",
            json!({"mode": "capture", "position": "0:00.5"}),
        ),
        &[video_task("a", &a)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let only = done_outputs(&c, "a");
    assert_eq!(only, [out2.join("a_cover.jpg")]);
    assert_color(
        mean_color(&paths, &only[0], None),
        [255, 0, 0],
        "0.5 秒处的红色画面",
    );
}

#[test]
fn same_name_image_is_used_and_missing_one_fails_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("cover-same");
    let with = dir.join("src/with.mp4");
    let without = dir.join("src/without.mp4");
    make_rgb_clip(&paths, &with);
    make_rgb_clip(&paths, &without);
    make_image(&paths, &dir.join("src/with.jpg"), "yellow");
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.cover", json!({"source": "sameName"})),
        &[video_task("w", &with), video_task("n", &without)],
    );
    assert_eq!(
        (summary.done, summary.failed),
        (1, 1),
        "{:#?}",
        c.snapshot()
    );
    let outputs = done_outputs(&c, "w");
    assert_eq!(outputs, [out.join("with_cover.mp4")]);
    assert_eq!(cover_streams(&paths, &outputs[0]), ["2,mjpeg,1"]);
    assert_color(
        mean_color(&paths, &outputs[0], Some("0:v:1")),
        [255, 255, 0],
        "同名图片",
    );
    let failure = format!("{:?}", c.final_state("n"));
    assert!(failure.contains("同名封面"), "{failure}");
    assert_eq!(
        all_files(&out),
        [out.join("with_cover.mp4")],
        "失败的不留输出"
    );
}
