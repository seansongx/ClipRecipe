//! 集成测试：`tool.remux` 无损转换——只换封装，视频数据包原样复制。

mod common;
mod tool_support;

use std::time::Instant;

use cliprecipe_lib::jobs::RunSummary;
use common::*;
use serde_json::json;
use tool_support::*;

#[test]
fn mp4_to_mkv_copies_streams_without_reencoding() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remux");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 3.0,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let tool = registered("tool.remux", json!({"container": "mkv"}));
    let started = Instant::now();
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        tool,
        &[video_task("t1", &input)],
    );
    let elapsed = started.elapsed();
    assert_eq!(
        summary,
        RunSummary {
            done: 1,
            failed: 0,
            cancelled: 0
        }
    );
    let outputs = done_outputs(&c, "t1");
    assert_eq!(outputs, [out.join("clip.mkv")]);
    let output = &outputs[0];
    assert_eq!(codecs(&paths, output), ["h264", "aac"]);
    assert_eq!(
        video_packets_md5(&paths, output),
        video_packets_md5(&paths, &input),
        "视频数据包应原样复制"
    );
    // mkv 没有逐流的时长 / 帧数字段，比较容器时长
    assert_close(
        video_duration(&paths, output),
        video_duration(&paths, &input),
        0.05,
        "视频时长",
    );
    // 复制很快：远小于重编码所需时间（3 秒 320x240 的视频复制只需几十毫秒）
    assert!(elapsed.as_secs_f64() < 5.0, "耗时 {elapsed:?}");
    assert_eq!(
        all_files(&out),
        [out.join("clip.mkv")],
        "没有残留的临时文件"
    );
}

#[test]
fn same_container_in_source_folder_never_overwrites_the_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remux-same");
    let input = dir.join("clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let mut config = tool_config(&paths, &dir.join("unused"), &dir.join("tmp"));
    config.output_dir = None; // 输出到源目录
    let (summary, c) = run_tool(
        config,
        registered("tool.remux", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(done_outputs(&c, "t1"), [dir.join("clip (1).mp4")]);
    assert_eq!(codecs(&paths, &dir.join("clip (1).mp4")), ["h264", "aac"]);
}

#[test]
fn mkv_with_pcm_audio_to_mp4_converts_only_the_audio() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("remux-pcm");
    let input = dir.join("pcm.mkv");
    make_lavfi(
        &paths,
        &input,
        "testsrc2=s=320x240:r=25:d=2",
        Some("sine=frequency=330:duration=2"),
        &["-c:a", "pcm_s16le"],
    );
    assert_eq!(codecs(&paths, &input), ["h264", "pcm_s16le"]);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.remux", json!({"container": "mp4"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let output = out.join("pcm.mp4");
    assert_eq!(codecs(&paths, &output), ["h264", "aac"]);
    assert_eq!(
        video_packets_md5(&paths, &output),
        video_packets_md5(&paths, &input)
    );

    // 关闭“转为 AAC”时失败，且不留下任何文件
    let strict_out = dir.join("strict");
    let (summary, _) = run_tool(
        tool_config(&paths, &strict_out, &dir.join("tmp")),
        registered(
            "tool.remux",
            json!({"container": "mp4", "convertIncompatibleAudio": false}),
        ),
        &[video_task("t2", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(all_files(&strict_out).is_empty());
}
