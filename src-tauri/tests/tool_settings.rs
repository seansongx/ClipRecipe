//! 集成测试：独立工具对设置项的统一处理——变速适应（`set.speed-fit`，音视频合成、截取合并）与
//! 版权信息（`set.copyright`，由框架的 `run_ffmpeg` / `run_encode` 统一写入）。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cliprecipe_lib::ffmpeg::{self, FfmpegPaths};
use cliprecipe_lib::tools::ToolRunConfig;
use common::*;
use serde_json::json;
use tool_support::*;

fn config(paths: &FfmpegPaths, dir: &TempDir, speed_fit: bool, copyright: &str) -> ToolRunConfig {
    let mut config = tool_config(paths, &dir.join("out"), &dir.join("tmp"));
    config.settings.speed_fit = speed_fit;
    config.settings.copyright = copyright.into();
    config
}

fn format_tag(paths: &FfmpegPaths, file: &Path, tag: &str) -> String {
    let output = ffmpeg::command(&paths.ffprobe)
        .args(["-v", "error", "-show_entries"])
        .arg(format!("format_tags={tag}"))
        .args(["-of", "default=nw=1:nk=1"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn av_mux(dir: &TempDir, speed_fit: bool, copyright: &str) -> PathBuf {
    let paths = ffmpeg_paths();
    let (adir, vdir) = (dir.join("audio"), dir.join("video"));
    make_clip(
        &paths,
        &vdir.join("a.mp4"),
        &Clip {
            seconds: 2.0,
            ..Clip::default()
        },
    );
    tone(&paths, &adir.join("a.m4a"), 1000, 4.0);
    let mut folders = BTreeMap::new();
    folders.insert("audio".to_owned(), adir);
    folders.insert("video".to_owned(), vdir);
    let c = collector();
    let summary = start_tool(
        config(&paths, dir, speed_fit, copyright),
        registered("tool.av-mux", json!({})),
        &[],
        &folders,
        &c,
    )
    .wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    done_outputs(&c, "tool.av-mux#1").remove(0)
}

#[test]
fn av_mux_speed_fit_stretches_video_to_the_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("tool-speed-fit-av");
    let out = av_mux(&dir, true, "© 测试工作室");
    let s = streams(&paths, &out);
    assert_close(s.video, 4.0, 0.15, "视频放慢到音频时长");
    assert_close(s.audio.unwrap(), 4.0, 0.15, "新音频完整保留");
    assert_eq!(codecs(&paths, &out)[0], "h264", "变速需要重编码");
    // 重编码路径（run_encode）写入版权信息
    assert_eq!(format_tag(&paths, &out, "copyright"), "© 测试工作室");

    // 关闭时：时长跟随视频，视频直接复制
    let plain_dir = TempDir::new("tool-speed-fit-av-off");
    let plain = av_mux(&plain_dir, false, "");
    assert_close(streams(&paths, &plain).video, 2.0, 0.1, "不变速");
    assert_eq!(format_tag(&paths, &plain, "copyright"), "", "未设置时不写");
}

#[test]
fn clip_merge_speed_fit_hits_the_target_length() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("tool-speed-fit-clip");
    let root = dir.join("src");
    for name in ["a.mp4", "b.mp4"] {
        make_clip(
            &paths,
            &root.join(name),
            &Clip {
                seconds: 3.0,
                ..Clip::default()
            },
        );
    }
    let mut folders = BTreeMap::new();
    folders.insert("source".to_owned(), root);
    let c = collector();
    // 两段各 3 秒、目标 4 秒并“截去超出长度”：开启变速适应后整体加快到 4 秒，而不是截掉后 2 秒
    let summary = start_tool(
        config(&paths, &dir, true, ""),
        registered(
            "tool.clip-merge",
            json!({
                "segmentSeconds": {"min": 3.0, "max": 3.0},
                "outputMode": "single",
                "trimExcess": true,
                "targetSeconds": 4.0
            }),
        ),
        &[],
        &folders,
        &c,
    )
    .wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = done_outputs(&c, "tool.clip-merge#all").remove(0);
    let s = streams(&paths, &out);
    assert_close(s.format, 4.0, 0.15, "正好是目标时长");
    assert_close(s.audio.unwrap(), 4.0, 0.15, "音频同步变速");
    // 加快了 1.5 倍：帧率不变时帧数约为 4 秒的量
    assert_close(s.frames as f64, 100.0, 4.0, "帧数");
    assert!(
        log_messages(&c).iter().any(|m| m.contains("变速适应")),
        "{:?}",
        log_messages(&c)
    );
}

#[test]
fn stream_copy_tool_writes_copyright() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("tool-copyright-remux");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let (summary, c) = run_tool(
        config(&paths, &dir, false, "© 2026 ClipRecipe"),
        registered("tool.remux", json!({"container": "mkv"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let out = done_outputs(&c, "t1").remove(0);
    assert_eq!(format_tag(&paths, &out, "copyright"), "© 2026 ClipRecipe");
    assert_eq!(codecs(&paths, &out)[0], "h264", "仍是无损复制");
}
