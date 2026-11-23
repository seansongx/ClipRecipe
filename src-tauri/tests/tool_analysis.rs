//! 集成测试：分析助手（场景切换、静音、黑边、静止画面）对已知内容的合成视频给出正确结果。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::{CancelToken, FfmpegPaths, MediaInfo};
use cliprecipe_lib::tools::analysis::{
    run_analysis, Analysis, BlackBarDetect, CropRect, FreezeDetect, Interval, SceneDetect,
    SilenceDetect,
};
use cliprecipe_lib::tools::ToolError;
use common::*;
use tool_support::*;

fn analyze<A: Analysis>(paths: &FfmpegPaths, file: &Path, analysis: &A) -> (A::Output, MediaInfo) {
    let info = probe(paths, file);
    let mut last = 0.0;
    let out = run_analysis(
        &paths.ffmpeg,
        file,
        &info,
        analysis,
        &CancelToken::new(),
        |p| {
            last = p.out_time;
        },
    )
    .unwrap_or_else(|e| panic!("{} 失败：{e}", analysis.name()));
    assert!(last > 0.0, "应有进度输出");
    (out, info)
}

fn assert_interval(actual: &Interval, start: f64, end: f64, tolerance: f64) {
    assert_close(actual.start, start, tolerance, "区间开始");
    assert_close(actual.end, end, tolerance, "区间结束");
}

#[test]
fn scene_cuts_are_found_at_hard_cuts() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("an-scene");
    let clip = dir.join("scenes.mp4");
    make_scene_clip(&paths, &clip, 2.0);
    let (cuts, _) = analyze(&paths, &clip, &SceneDetect::default());
    assert_eq!(cuts.len(), 2, "{cuts:?}");
    assert_close(cuts[0], 2.0, 0.05, "第一个切换点");
    assert_close(cuts[1], 4.0, 0.05, "第二个切换点");

    // 最小间隔大于片段长度时只保留第一个
    let (sparse, _) = analyze(
        &paths,
        &clip,
        &SceneDetect {
            min_gap: 2.5,
            ..SceneDetect::default()
        },
    );
    assert!(sparse.is_empty(), "离开头 / 结尾都不足 2.5 秒：{sparse:?}");
    // 阈值很高时没有切换点
    let (none, _) = analyze(
        &paths,
        &clip,
        &SceneDetect {
            threshold: 0.95,
            ..SceneDetect::default()
        },
    );
    assert!(none.is_empty(), "{none:?}");
}

#[test]
fn silence_intervals_match_the_gaps() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("an-silence");
    let clip = dir.join("gaps.mp4");
    // 1～2 秒静音，4.5 秒到结尾（6 秒）静音
    make_lavfi(
        &paths,
        &clip,
        "testsrc2=s=160x120:r=25:d=6",
        Some("aevalsrc='if(between(t,1,2)+gte(t,4.5),0,0.5*sin(440*2*PI*t))':d=6:s=48000"),
        &[],
    );
    let (silences, _) = analyze(&paths, &clip, &SilenceDetect::default());
    assert_eq!(silences.len(), 2, "{silences:?}");
    assert_interval(&silences[0], 1.0, 2.0, 0.1);
    assert_interval(&silences[1], 4.5, 6.0, 0.15);

    // 没有音频时报错（而不是给出空结果）
    let mute = dir.join("mute.mp4");
    make_lavfi(&paths, &mute, "testsrc2=s=160x120:r=25:d=1", None, &[]);
    let info = probe(&paths, &mute);
    let err = run_analysis(
        &paths.ffmpeg,
        &mute,
        &info,
        &SilenceDetect::default(),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(err, ToolError::Failed(ref m) if m.contains("没有音频")),
        "{err:?}"
    );
}

#[test]
fn black_bars_are_measured() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("an-bars");
    let boxed = dir.join("letterbox.mp4");
    // 320x180 的画面放在 320x240 中间（上下各 30 像素黑边）
    make_lavfi(
        &paths,
        &boxed,
        "testsrc2=s=320x180:r=25:d=2,pad=320:240:0:30",
        None,
        &[],
    );
    let (rect, _) = analyze(&paths, &boxed, &BlackBarDetect::default());
    assert_eq!(
        rect,
        Some(CropRect {
            x: 0,
            y: 30,
            width: 320,
            height: 180
        })
    );
    let pillar = dir.join("pillarbox.mp4");
    make_lavfi(
        &paths,
        &pillar,
        "testsrc2=s=240x240:r=25:d=2,pad=320:240:40:0",
        None,
        &[],
    );
    let (rect, _) = analyze(&paths, &pillar, &BlackBarDetect::default());
    let rect = rect.expect("应检测到左右黑边");
    assert_eq!((rect.width, rect.height), (240, 240));
    assert_close(f64::from(rect.x), 40.0, 2.0, "左边黑边");

    let full = dir.join("full.mp4");
    make_lavfi(&paths, &full, "testsrc2=s=320x240:r=25:d=2", None, &[]);
    let (none, _) = analyze(&paths, &full, &BlackBarDetect::default());
    assert_eq!(none, None);
}

#[test]
fn frozen_sections_are_found() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("an-freeze");
    let clip = dir.join("freeze.mp4");
    // 0～1 秒运动，1～3 秒定格（复制最后一帧），3～4 秒运动
    make_lavfi(
        &paths,
        &clip,
        "testsrc2=s=320x240:r=25:d=1,tpad=stop_mode=clone:stop_duration=2[a];\
         testsrc2=s=320x240:r=25:d=1[b];[a][b]concat",
        None,
        &[],
    );
    let (frozen, info) = analyze(&paths, &clip, &FreezeDetect::default());
    assert_close(info.duration, 4.0, 0.1, "测试视频时长");
    assert_eq!(frozen.len(), 1, "{frozen:?}");
    assert_interval(&frozen[0], 1.0, 3.0, 0.1);

    let (strict, _) = analyze(
        &paths,
        &clip,
        &FreezeDetect {
            min_duration: 2.5,
            ..FreezeDetect::default()
        },
    );
    assert!(strict.is_empty(), "定格只有 2 秒：{strict:?}");
}

#[test]
fn analysis_can_be_cancelled() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("an-cancel");
    let clip = dir.join("clip.mp4");
    make_scene_clip(&paths, &clip, 1.0);
    let info = probe(&paths, &clip);
    let token = CancelToken::new();
    token.cancel();
    let result = run_analysis(
        &paths.ffmpeg,
        &clip,
        &info,
        &SceneDetect::default(),
        &token,
        |_| {},
    );
    assert_eq!(result.unwrap_err(), ToolError::Cancelled);
}
