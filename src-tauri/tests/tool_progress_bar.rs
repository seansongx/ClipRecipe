//! 集成测试：`tool.progress-bar` 进度条——在 25% / 50% / 75% 时长处数进度条上已播放部分的像素，
//! 圆角、顶部位置、章节分隔线与中文标题、跟随进度的指示器。素材是纯灰色画面，便于按颜色分类像素。

mod common;
mod tool_support;

use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::{json, Value};
use tool_support::*;

fn make_gray(paths: &FfmpegPaths, out: &Path, (w, h): (u32, u32), audio: bool) {
    let pix = if w % 2 == 1 || h % 2 == 1 {
        "yuv444p"
    } else {
        "yuv420p"
    };
    make_lavfi(
        paths,
        out,
        &format!("color=c=0x808080:s={w}x{h}:r=25:d=4,format={pix}"),
        audio.then_some("sine=frequency=440:duration=4"),
        &["-pix_fmt", pix],
    );
}

fn is_red(p: &[u8; 3]) -> bool {
    p[0] > 150 && p[1] < 90 && p[2] < 90
}
fn is_black(p: &[u8; 3]) -> bool {
    p.iter().all(|c| *c < 40)
}
fn is_white(p: &[u8; 3]) -> bool {
    p.iter().all(|c| *c > 200)
}
fn is_green(p: &[u8; 3]) -> bool {
    p[1] > 150 && p[0] < 100 && p[2] < 100
}
fn count(pixels: &[[u8; 3]], f: fn(&[u8; 3]) -> bool) -> usize {
    pixels.iter().filter(|p| f(p)).count()
}

fn run(dir: &TempDir, input: &Path, params: Value) -> PathBuf {
    let paths = ffmpeg_paths();
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.progress-bar", params),
        &[video_task("t1", input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs.len(), 1);
    assert!(all_files(&dir.join("tmp")).is_empty(), "临时文件已清理");
    outputs[0].clone()
}

/// 红色已播放、黑色（不透明）未播放、粗 5%。
fn solid(extra: Value) -> Value {
    let mut params = json!({
        "barColor": "#ff0000",
        "backgroundColor": "#000000",
        "backgroundOpacity": 100,
        "height": 5
    });
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    params
}

#[test]
fn fill_fraction_follows_playback() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("progress-bar-fill");
    let input = dir.join("src/gray.mp4");
    make_gray(&paths, &input, (320, 240), true);
    let out = run(&dir, &input, solid(json!({})));
    // 粗 12 像素，位于底部 y 228..240
    for (t, expected) in [(1.0, 80.0), (2.0, 160.0), (3.0, 240.0)] {
        let pixels = frame_row(&paths, &out, t, 234);
        assert_close(
            count(&pixels, is_red) as f64,
            expected,
            3.0,
            &format!("{t} 秒的已播放宽度"),
        );
        assert_close(
            count(&pixels, is_black) as f64,
            320.0 - expected,
            3.0,
            "未播放部分",
        );
    }
    // 进度条之外不变
    assert!(
        count(&frame_row(&paths, &out, 2.0, 220), |p| p[0].abs_diff(128)
            < 10)
            == 320
    );
    let s = streams(&paths, &out);
    assert_eq!(s.frames, 100);
    assert_close(s.audio.unwrap(), 4.0, 0.1, "音频保留");
}

#[test]
fn rounded_bar_keeps_fraction_and_cuts_corners() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("progress-bar-rounded");
    let input = dir.join("src/gray.mp4");
    make_gray(&paths, &input, (320, 240), true);
    let out = run(&dir, &input, solid(json!({"rounded": true})));
    for (t, expected) in [(1.0, 80.0), (2.0, 160.0), (3.0, 240.0)] {
        let pixels = frame_row(&paths, &out, t, 234);
        assert_close(
            count(&pixels, is_red) as f64,
            expected,
            3.0,
            &format!("{t} 秒的已播放宽度"),
        );
    }
    // 左上、右上角是透明的（露出灰色画面），中线两端仍在进度条内
    let top = frame_row(&paths, &out, 2.0, 228);
    assert!(
        top[0][0].abs_diff(128) < 20 && top[319][0].abs_diff(128) < 20,
        "{:?} {:?}",
        top[0],
        top[319]
    );
    let middle = frame_row(&paths, &out, 2.0, 234);
    assert!(is_red(&middle[1]) && is_black(&middle[318]));
}

#[test]
fn top_bar_with_margin_on_portrait_without_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("progress-bar-top");
    let input = dir.join("src/portrait.mp4");
    make_gray(&paths, &input, (180, 320), false);
    let out = run(
        &dir,
        &input,
        solid(json!({"position": "top", "margin": 10})),
    );
    // 粗 16 像素，y 32..48
    for (t, expected) in [(1.0, 45.0), (2.0, 90.0), (3.0, 135.0)] {
        let pixels = frame_row(&paths, &out, t, 40);
        assert_close(
            count(&pixels, is_red) as f64,
            expected,
            3.0,
            &format!("{t} 秒的已播放宽度"),
        );
    }
    assert_eq!(
        count(&frame_row(&paths, &out, 2.0, 28), is_black),
        0,
        "边距内没有进度条"
    );
    assert_eq!(stream_count(&paths, &out, "a"), 0);
}

#[test]
fn chapters_with_chinese_labels_and_gif_indicator_on_odd_size() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("progress-bar-chapters");
    let input = dir.join("src/odd.mp4");
    make_gray(&paths, &input, (321, 241), true);
    let dot = dir.join("assets/dot.gif");
    make_lavfi(
        &paths,
        &dot,
        "color=c=0x00ff00:s=10x10:r=10:d=0.5",
        None,
        &["-c:v", "gif", "-loop", "0"],
    );
    let plain = run(&dir, &input, solid(json!({})));
    let out = run(
        &dir,
        &input,
        solid(json!({
            "chapters": "0,第一章\n50%，第二章",
            "labelSize": 8,
            "showIndicator": true,
            "indicator": {"mode": "file", "path": dot},
            "indicatorSize": 10
        })),
    );
    let info = probe(&paths, &out);
    assert_eq!(info.video.as_ref().unwrap().display_size(), (320, 240));

    // 标题写在进度条上方（y 228 之上 6 像素起），两个章节各自居中：左半、右半都有白色文字
    let labels = |file: &Path| frame_pixels(&paths, file, 0.5, (0, 190, 320, 32));
    assert_eq!(count(&labels(&plain), is_white), 0, "没有章节时没有文字");
    let drawn = labels(&out);
    let left = drawn
        .iter()
        .enumerate()
        .filter(|(i, p)| i % 320 < 160 && is_white(p))
        .count();
    let right = drawn
        .iter()
        .enumerate()
        .filter(|(i, p)| i % 320 >= 160 && is_white(p))
        .count();
    assert!(left > 30 && right > 30, "标题像素：左 {left}、右 {right}");
    // 中文真正渲染成了字形：缺字时两个标题都是同样的“豆腐块”，字形不同则相对位置的像素图案不同
    let pattern = |from: usize, to: usize| {
        let points: Vec<(usize, usize)> = drawn
            .iter()
            .enumerate()
            .filter(|(i, p)| (from..to).contains(&(i % 320)) && is_white(p))
            .map(|(i, _)| (i % 320, i / 320))
            .collect();
        let (x0, y0) = (
            points.iter().map(|p| p.0).min().unwrap(),
            points.iter().map(|p| p.1).min().unwrap(),
        );
        points
            .iter()
            .map(|(x, y)| (x - x0, y - y0))
            .collect::<Vec<_>>()
    };
    assert_ne!(
        pattern(0, 160),
        pattern(160, 320),
        "“第一章”与“第二章”应画出不同的字形"
    );

    // 章节分隔线（x 159..161）在已播放部分上仍是背景色
    let bar = frame_row(&paths, &out, 3.0, 230);
    assert!(
        is_black(&bar[159]) && is_black(&bar[160]),
        "{:?}",
        &bar[157..163]
    );
    assert!(is_red(&bar[150]) && is_red(&bar[170]));

    // 指示器（24×24 绿色）跟着进度条前端走：1 秒在 x≈80，3 秒在 x≈240
    let green_at =
        |t: f64, x: u32| count(&frame_pixels(&paths, &out, t, (x - 4, 230, 8, 8)), is_green);
    assert!(green_at(1.0, 80) > 40, "1 秒时指示器在 80");
    assert_eq!(green_at(1.0, 240), 0);
    assert!(green_at(3.0, 240) > 40, "3 秒时指示器在 240");
    assert_eq!(green_at(3.0, 80), 0);
}

#[test]
fn invalid_chapter_config_fails_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("progress-bar-bad");
    let input = dir.join("src/gray.mp4");
    make_gray(&paths, &input, (320, 240), false);
    let (summary, c) = run_tool(
        tool_config(&paths, &dir.join("out"), &dir.join("tmp")),
        registered("tool.progress-bar", json!({"chapters": "abc,第一章"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(all_files(&dir.join("out")).is_empty());
    assert!(all_files(&dir.join("tmp")).is_empty());
}
