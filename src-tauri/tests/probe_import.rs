//! 集成测试：媒体探测、导入、硬件编码器探测。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use cliprecipe_lib::ffmpeg::encoders::{self, test_encoder};
use cliprecipe_lib::ffmpeg::{VideoCodec, VideoEncoder};
use cliprecipe_lib::jobs::import;
use common::*;

#[test]
fn probe_reads_video_and_audio_streams() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("probe");
    let file = dir.join("clip.mp4");
    make_clip(
        &paths,
        &file,
        &Clip {
            width: 320,
            height: 240,
            fps: 25,
            seconds: 2.0,
            audio: true,
        },
    );

    let info = probe(&paths, &file);
    let video = info.video.as_ref().expect("应有视频流");
    assert_eq!((video.width, video.height), (320, 240));
    assert_eq!(video.codec, "h264");
    assert!((video.fps - 25.0).abs() < 0.01, "fps = {}", video.fps);
    assert_eq!(video.rotation, 0);
    assert!(
        (info.duration - 2.0).abs() < 0.15,
        "duration = {}",
        info.duration
    );
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].codec, "aac");
    assert!(info.subtitles.is_empty());
    assert!(info.size > 0);
    assert!(info.video_bit_rate().is_some());
}

#[test]
fn probe_reports_missing_audio_and_errors() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("probe-noaudio");
    let file = dir.join("silent.mp4");
    make_clip(
        &paths,
        &file,
        &Clip {
            audio: false,
            ..Clip::default()
        },
    );
    assert!(probe(&paths, &file).audio.is_empty());

    let bogus = dir.join("bogus.mp4");
    std::fs::write(&bogus, b"not a video").unwrap();
    let err = cliprecipe_lib::ffmpeg::probe(&paths.ffprobe, &bogus).unwrap_err();
    assert!(err.contains("无法读取媒体信息"), "{err}");
}

#[test]
fn import_expands_folders_probes_and_reports_progress() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("import");
    make_clip(&paths, &dir.join("in/a.mp4"), &Clip::default());
    make_clip(&paths, &dir.join("in/sub/b.mp4"), &Clip::default());
    std::fs::write(dir.join("in/notes.txt"), b"x").unwrap();
    std::fs::write(dir.join("in/broken.mkv"), b"x").unwrap();

    let calls = AtomicUsize::new(0);
    let result = import::import(
        &paths.ffprobe,
        &[dir.join("in"), dir.join("in/a.mp4")],
        true,
        |done, total| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert!(done <= total);
            assert_eq!(total, 3);
        },
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(result.items.len(), 3, "a.mp4 去重，notes.txt 被过滤");
    let ok: Vec<&import::ImportedItem> =
        result.items.iter().filter(|i| i.error.is_none()).collect();
    assert_eq!(ok.len(), 2);
    assert!(ok.iter().all(|i| i.info.as_ref().unwrap().video.is_some()));
    assert!(ok.iter().all(|i| i.source_root.is_some()));
    let broken = result
        .items
        .iter()
        .find(|i| i.path.ends_with("broken.mkv"))
        .unwrap();
    assert!(broken.info.is_none() && broken.error.is_some());

    let flat = import::import(&paths.ffprobe, &[dir.join("in")], false, |_, _| {});
    assert_eq!(flat.items.len(), 2, "不递归时不含 sub/b.mp4");
}

#[test]
fn encoder_detection_runs_and_software_fallback_works() {
    let paths = ffmpeg_paths();
    // 软件编码器必须可用，否则流水线无法回退
    assert!(test_encoder(
        &paths.ffmpeg,
        VideoEncoder::software(VideoCodec::H264)
    ));
    let caps = encoders::cached(&paths.ffmpeg);
    // 缓存命中应与首次结果一致
    assert_eq!(encoders::cached(&paths.ffmpeg), caps);
    let chosen = caps.select(VideoCodec::H264, true);
    assert!(
        test_encoder(&paths.ffmpeg, chosen),
        "选中的编码器 {} 应能试编码",
        chosen.name()
    );
    println!("可用硬件编码器：{:?}", caps.names());
}
