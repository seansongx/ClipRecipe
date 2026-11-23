//! 集成测试：`tool.subtitle-erase` 字幕擦除——先用 drawtext 烧录白色文字，再用三种方式擦除：
//! 区域内的文字像素消失、区域外不变、时段限制生效。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::pipeline::escape_filter_path;
use cliprecipe_lib::pipeline::modules::text::default_font;
use common::*;
use serde_json::json;
use tool_support::*;

const BACKGROUND: [i32; 3] = [0x33, 0x66, 0x99];

/// 3 秒 320x240 纯色视频，底部（y=192 起）烧录白色文字，带音频。
fn make_captioned(paths: &FfmpegPaths, out: &Path) {
    let font = default_font().expect("测试需要一个系统字体");
    let graph = format!(
        "color=c=0x336699:s=320x240:r=25:d=3,drawtext=fontfile={}:text='HELLO 123 字幕':fontsize=24:fontcolor=white:x=(w-tw)/2:y=192",
        escape_filter_path(&font.to_string_lossy())
    );
    make_lavfi(
        paths,
        out,
        &graph,
        Some("sine=frequency=440:duration=3"),
        &[],
    );
}

/// 第 `time` 秒那一帧中区域 (x, y, w, h) 里接近白色的像素个数。
fn white_pixels(
    paths: &FfmpegPaths,
    file: &Path,
    time: f64,
    (x, y, w, h): (u32, u32, u32, u32),
) -> usize {
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
        .arg(format!("crop={w}:{h}:{x}:{y},format=rgb24"))
        .args(["-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), (w * h * 3) as usize);
    output
        .stdout
        .chunks_exact(3)
        .filter(|p| p.iter().all(|c| *c > 200))
        .count()
}

const BAND: (u32, u32, u32, u32) = (0, 186, 320, 40);
const ABOVE: (u32, u32, u32, u32) = (0, 0, 320, 150);

fn erase(
    paths: &FfmpegPaths,
    dir: &TempDir,
    input: &Path,
    name: &str,
    params: serde_json::Value,
) -> std::path::PathBuf {
    let out = dir.join(name);
    let (summary, c) = run_tool(
        tool_config(paths, &out, &dir.join("tmp")),
        registered("tool.subtitle-erase", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    outputs.into_iter().next().unwrap()
}

#[test]
fn every_mode_removes_the_caption_and_leaves_the_rest() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("erase-modes");
    let input = dir.join("src/captioned.mp4");
    make_captioned(&paths, &input);
    let burned = white_pixels(&paths, &input, 1.0, BAND);
    assert!(burned > 150, "测试素材里应有文字：{burned}");

    for (mode, extra) in [
        ("delogo", json!({})),
        ("blur", json!({"blurStrength": 20})),
        ("fill", json!({"fillColor": "#336699"})),
    ] {
        let mut params = json!({"mode": mode});
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let output = erase(&paths, &dir, &input, &format!("out-{mode}"), params);
        assert_eq!(output.file_name().unwrap(), "captioned_erased.mp4");
        for t in [0.5, 2.5] {
            let left = white_pixels(&paths, &output, t, BAND);
            assert!(
                left <= 2,
                "{mode}：{t} 秒字幕区域仍有 {left} 个白色像素（原来 {burned}）"
            );
        }
        let band = avg_rgb(&paths, &output, 1.0, BAND);
        if mode != "blur" {
            for (a, e) in band.iter().zip(BACKGROUND) {
                assert!((a - e).abs() <= 10, "{mode}：区域填成背景色，实际 {band:?}");
            }
        }
        let (before, after) = (
            avg_rgb(&paths, &input, 1.0, ABOVE),
            avg_rgb(&paths, &output, 1.0, ABOVE),
        );
        for (a, b) in before.iter().zip(after) {
            assert!(
                (a - b).abs() <= 3,
                "{mode}：区域外不变：{before:?} → {after:?}"
            );
        }
        let info = probe(&paths, &output);
        let video = info.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (320, 240));
        assert_close(info.duration, 3.0, 0.1, "时长不变");
        assert_eq!(info.audio[0].codec, "aac");
    }
    assert!(all_files(&dir.join("tmp")).is_empty());
}

#[test]
fn time_range_and_custom_regions_are_respected() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("erase-range");
    let input = dir.join("src/captioned.mp4");
    make_captioned(&paths, &input);
    let burned = white_pixels(&paths, &input, 2.5, BAND);
    let output = erase(
        &paths,
        &dir,
        &input,
        "out",
        json!({
            "area": "regions",
            "regions": [{"x": 20, "y": 184, "width": 280, "height": 44}],
            "mode": "fill",
            "fillColor": "#000000",
            "ranges": "0-1.5"
        }),
    );
    assert!(
        white_pixels(&paths, &output, 0.5, BAND) <= 2,
        "时段内已擦除"
    );
    let late = white_pixels(&paths, &output, 2.5, BAND);
    assert!(
        late as f64 > burned as f64 * 0.8,
        "时段外保留文字：{late}（原来 {burned}）"
    );
    let early = avg_rgb(&paths, &output, 0.5, (30, 190, 260, 30));
    assert!(early.iter().all(|c| *c < 20), "黑色遮盖：{early:?}");
}

#[test]
fn region_outside_the_frame_fails_without_output() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("erase-invalid");
    let input = dir.join("captioned.mp4");
    make_captioned(&paths, &input);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered(
            "tool.subtitle-erase",
            json!({"area": "regions", "regions": [{"x": 400, "y": 10, "width": 50, "height": 50}]}),
        ),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(format!("{:?}", c.final_state("t1")).contains("之外"));
    assert!(all_files(&out).is_empty());
}
