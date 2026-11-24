//! 集成测试：`tool.simple-mix` 简单混剪——副视频在各自时段叠加（铺满 / 居中，按像素检查）、字幕烧录（中文）、
//! 背景音频可闻，时长跟随主视频。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::jobs::RunSummary;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn run(dir: &TempDir, main: &Path, params: Value) -> (RunSummary, Collector) {
    let paths = ffmpeg_paths();
    let result = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.simple-mix", params),
        &[video_task("t1", main)],
    );
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    result
}

fn dir_source(path: &Path) -> Value {
    json!({"mode": "directory", "path": path.to_string_lossy(), "order": "sequential"})
}

fn file_source(path: &Path) -> Value {
    json!({"mode": "file", "path": path.to_string_lossy()})
}

fn blue(rgb: [i32; 3]) -> bool {
    rgb[2] > 180 && rgb[0] < 70
}

fn red(rgb: [i32; 3]) -> bool {
    rgb[0] > 180 && rgb[2] < 70
}

#[test]
fn clips_fill_the_frame_in_their_segments_and_duration_follows_main() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mix-fill");
    let main = dir.join("main.mp4");
    color_clip(&paths, &main, "red", 6.0, true);
    let clips = dir.join("clips");
    color_clip(&paths, &clips.join("blue.mp4"), "blue", 1.0, false);
    let (summary, c) = run(
        &dir,
        &main,
        json!({"clips": dir_source(&clips), "layout": "fill", "showSeconds": 1.5, "gapSeconds": 1}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "t1")[0];
    assert_eq!(out, &dir.join("out").join("main.mp4"));
    let s = streams(&paths, out);
    assert_close(s.video, 6.0, 0.06, "时长跟随主视频");
    assert_close(s.audio.unwrap(), 6.0, 0.06, "保留主视频声音");
    // 时段：[1, 2.5]、[3.5, 5]；副视频只有 1 秒，循环播放
    let full = (0, 0, 320, 240);
    for (t, overlay) in [
        (0.5, false),
        (1.2, true),
        (2.2, true),
        (3.0, false),
        (4.6, true),
        (5.6, false),
    ] {
        let rgb = avg_rgb(&paths, out, t, full);
        if overlay {
            assert!(blue(rgb), "{t} 秒应铺满副视频：{rgb:?}");
        } else {
            assert!(red(rgb), "{t} 秒应只有主视频：{rgb:?}");
        }
    }
}

#[test]
fn centred_clips_cover_the_middle_only() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mix-center");
    let main = dir.join("main.mp4");
    color_clip(&paths, &main, "red", 3.0, false);
    let clip = dir.join("blue.mp4");
    color_clip(&paths, &clip, "blue", 3.0, false);
    let (summary, c) = run(
        &dir,
        &main,
        json!({"clips": file_source(&clip), "layout": "center", "centerSize": 50, "showSeconds": 1, "gapSeconds": 1}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "t1")[0];
    // 居中 50%：160x120，位于 (80, 60)
    assert!(
        blue(avg_rgb(&paths, out, 1.5, (100, 80, 120, 80))),
        "中间是副视频"
    );
    assert!(
        red(avg_rgb(&paths, out, 1.5, (0, 0, 60, 50))),
        "角落仍是主视频"
    );
    assert!(
        red(avg_rgb(&paths, out, 0.5, (100, 80, 120, 80))),
        "间隔时只有主视频"
    );
    assert_eq!(
        stream_count(&paths, out, "a"),
        0,
        "主视频没有声音、未开背景音频"
    );
}

#[test]
fn subtitles_are_burned_in_with_chinese_text() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mix-subtitle");
    let main = dir.join("main.mp4");
    color_clip(&paths, &main, "gray", 4.0, true);
    let srt: PathBuf = dir.join("字幕 1.srt");
    std::fs::write(
        &srt,
        "1\n00:00:00,000 --> 00:00:02,000\n你好，世界！中文字幕测试\n\n",
    )
    .unwrap();
    let (summary, c) = run(
        &dir,
        &main,
        json!({"clipsEnabled": false, "subtitleEnabled": true, "subtitle": file_source(&srt)}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "t1")[0];
    // 字幕区（画面下方）在字幕时段内出现白色文字与黑色描边，字幕结束后消失；画面上方不受影响
    let bottom = (20, 180, 280, 60);
    let (white, dark) = contrast_pixels(&paths, out, 1.0, bottom);
    // 偏亮像素数随字体的笔画粗细变化（Linux 的 Noto Sans CJK 比 Windows 的微软雅黑细），所以“偏亮”只要求明显亮于
    // 灰色背景，阈值留出余量；没有字幕时两者都是 0（见下一条断言），所以仍能区分“画出了字”和“没画”。
    // 失败时打印字幕区的字符画与 libass 的字体日志（macOS 上不指定中文字体时曾全部画成方框：只有描边、没有亮像素）
    assert!(
        white > 15 && dark > 150,
        "字幕区应有文字：白 {white}，黑 {dark}\n{}",
        subtitle_diagnostics(&paths, out, &srt, bottom)
    );
    assert_eq!(
        contrast_pixels(&paths, out, 3.0, bottom),
        (0, 0),
        "字幕结束后消失"
    );
    assert_eq!(
        contrast_pixels(&paths, out, 1.0, (20, 10, 280, 60)),
        (0, 0),
        "画面上方没有字幕"
    );
}

/// 灰色背景（亮度约 128）上某区域里明显偏亮（> 170）与偏黑（< 50）的像素个数。
fn contrast_pixels(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> (usize, usize) {
    let pixels = gray_crop(paths, file, time, (x, y, w, h));
    let white = pixels.iter().filter(|p| **p > 170).count();
    let dark = pixels.iter().filter(|p| **p < 50).count();
    (white, dark)
}

/// 某一帧里一块区域的灰度像素（逐行，每像素一字节）。
fn gray_crop(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> Vec<u8> {
    let output = ffmpeg::command(&paths.ffmpeg)
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
        .arg(format!("crop={w}:{h}:{x}:{y},format=gray"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    output.stdout
}

/// 字幕断言失败时的排查信息：字幕区的字符画（`#` 偏黑、`o` 偏亮、`+` 略亮于背景、`.` 背景）、最亮像素的亮度，
/// 以及用同一个字幕文件单独跑一次 `subtitles` 滤镜时 libass 关于字体的日志。
fn subtitle_diagnostics(
    paths: &FfmpegPaths,
    file: &Path,
    srt: &Path,
    (x, y, w, h): (u32, u32, u32, u32),
) -> String {
    let pixels = gray_crop(paths, file, 1.0, (x, y, w, h));
    let brightest = pixels.iter().copied().max().unwrap_or(0);
    let mut text = format!("最亮像素 {brightest}；字幕区 {w}x{h}：\n");
    for row in pixels.chunks(w as usize) {
        text.extend(row.iter().map(|p| match p {
            0..=49 => '#',
            50..=140 => '.',
            141..=170 => '+',
            _ => 'o',
        }));
        text.push('\n');
    }
    let filter = format!(
        "subtitles={}",
        cliprecipe_lib::pipeline::escape_filter_path(&srt.to_string_lossy())
    );
    let log = ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "verbose", "-f", "lavfi", "-i"])
        .arg("color=c=gray:s=320x240:d=1")
        .args(["-vf", &filter, "-frames:v", "1", "-f", "null", "-"])
        .output()
        .unwrap();
    text.push_str("libass 日志：\n");
    for line in String::from_utf8_lossy(&log.stderr).lines() {
        let lower = line.to_lowercase();
        if ["subtitles", "font", "glyph", "libass"]
            .iter()
            .any(|key| lower.contains(key))
        {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

#[test]
fn background_audio_is_audible_and_mixed() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("mix-bgm");
    let silent = dir.join("silent.mp4");
    color_clip(&paths, &silent, "red", 3.0, false);
    let music = dir.join("music");
    tone(&paths, &music.join("1000.mp3"), 1000, 1.0);
    let (summary, c) = run(
        &dir,
        &silent,
        json!({"clipsEnabled": false, "bgmEnabled": true, "bgm": dir_source(&music)}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "t1")[0];
    assert!(tone_db(&paths, out, 1000) > -30.0, "背景音频可闻");
    assert_close(
        stream_duration(&paths, out, "a:0"),
        3.0,
        0.06,
        "背景音频循环到主视频结束",
    );

    let dir2 = TempDir::new("mix-bgm-mix");
    let main = dir2.join("main.mp4");
    color_clip(&paths, &main, "red", 2.0, true);
    let (summary, c) = run(
        &dir2,
        &main,
        json!({"clipsEnabled": false, "bgmEnabled": true, "bgm": dir_source(&music), "bgmVolume": 100, "originalVolume": 100}),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = &done_outputs(&c, "t1")[0];
    assert!(tone_db(&paths, out, 1000) > -30.0, "背景音频");
    assert!(tone_db(&paths, out, 440) > -30.0, "主视频原声");
}
