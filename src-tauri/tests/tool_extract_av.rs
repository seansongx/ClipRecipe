//! 集成测试：`tool.extract-av` 音视频提取——音轨（全部 / 第一条，原格式复制或转 mp3）与纯视频。

mod common;
mod tool_support;

use std::path::Path;

use cliprecipe_lib::ffmpeg::FfmpegPaths;
use common::*;
use serde_json::json;
use tool_support::*;

/// 两条音轨（AAC 立体声 + AAC 单声道）的 3 秒视频。
fn make_two_track_clip(paths: &FfmpegPaths, out: &Path) {
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let output = cliprecipe_lib::ffmpeg::command(&paths.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i", "testsrc2=s=320x240:r=25:d=3"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3,aformat=channel_layouts=stereo",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=660:duration=3"])
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-c:a", "aac", "-shortest"])
        .arg(out)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn all_audio_tracks_are_copied_in_original_format() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("extract-all");
    let input = dir.join("src/movie.mp4");
    make_two_track_clip(&paths, &input);
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.extract-av", json!({})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(
        outputs,
        [out.join("movie_audio_1.m4a"), out.join("movie_audio_2.m4a")]
    );
    for (file, channels) in outputs.iter().zip([2, 1]) {
        let info = probe(&paths, file);
        assert!(info.video.is_none(), "{} 不应有视频", file.display());
        assert_eq!(info.audio.len(), 1);
        assert_eq!(info.audio[0].codec, "aac");
        assert_eq!(info.audio[0].channels, channels);
        assert_close(info.duration, 3.0, 0.1, "音频时长");
    }
}

#[test]
fn mp3_with_chosen_bitrate_and_video_only() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("extract-mp3");
    let input = dir.join("src/movie.mp4");
    make_two_track_clip(&paths, &input);
    let out = dir.join("out");
    let params = json!({"mode": "both", "tracks": "first", "audioFormat": "mp3", "mp3Kbps": 128});
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.extract-av", params),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "t1");
    assert_eq!(
        outputs,
        [out.join("movie_audio.mp3"), out.join("movie_video.mp4")]
    );

    let mp3 = probe(&paths, &outputs[0]);
    assert_eq!(mp3.audio.len(), 1);
    assert_eq!(mp3.audio[0].codec, "mp3");
    let kbps = mp3.audio[0].bit_rate.or(mp3.bit_rate).unwrap() / 1000;
    assert!((120..=136).contains(&kbps), "mp3 码率 {kbps}k");

    let video = &outputs[1];
    assert_eq!(codecs(&paths, video), ["h264"], "纯视频：只有一条视频流");
    assert_eq!(stream_count(&paths, video, "a"), 0);
    assert_eq!(
        video_packets_md5(&paths, video),
        video_packets_md5(&paths, &input),
        "视频直接复制"
    );
}

#[test]
fn extracting_audio_from_a_silent_video_fails_cleanly() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("extract-silent");
    let input = dir.join("silent.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let (summary, _) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        registered("tool.extract-av", json!({"mode": "audio"})),
        &[video_task("t1", &input)],
    );
    assert_eq!(summary.failed, 1);
    assert!(all_files(&out).is_empty());
}
