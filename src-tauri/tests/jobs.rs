//! 集成测试：流水线 + 调度器端到端真实运行 ffmpeg。

mod common;

use std::path::Path;
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{
    encoders, EncoderCaps, FfmpegPaths, HwVendor, VideoCodec, VideoEncoder,
};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{OnExists, Range, Recipe, Settings};
use cliprecipe_lib::pipeline::modules::BitrateMode;
use common::*;

fn config(paths: &FfmpegPaths, recipe: Recipe, settings: Settings, out: &Path) -> RunConfig {
    RunConfig {
        paths: paths.clone(),
        // 测试机不一定有 GPU：固定用软件编码，结果可复现
        encoders: EncoderCaps::software_only(),
        recipe,
        settings,
        output_dir: Some(out.to_path_buf()),
        seed: 7,
    }
}

fn task(id: &str, path: &Path) -> TaskSpec {
    TaskSpec {
        id: id.into(),
        path: path.to_string_lossy().into_owned(),
        source_root: None,
        info: None,
        number: 0,
    }
}

fn output_of(event: Option<JobEvent>) -> String {
    match event {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            output: Some(output),
            ..
        }) => output,
        other => panic!("任务应完成，实际：{other:?}"),
    }
}

#[test]
fn fps_and_bitrate_job_produces_expected_output() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fps-bitrate");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 320,
            height: 240,
            fps: 25,
            seconds: 2.0,
            audio: true,
        },
    );

    let mut recipe = Recipe::disabled();
    recipe.modules.fps.enabled = true;
    recipe.modules.fps.fps = Range::fixed(15.0);
    recipe.modules.bitrate.enabled = true;
    recipe.modules.bitrate.mode = BitrateMode::Fixed;
    recipe.modules.bitrate.kbps = Range::fixed(300);

    let c = collector();
    let out_dir = dir.join("out");
    let handle = jobs::start(
        config(&paths, recipe, Settings::default(), &out_dir),
        vec![task("t1", &input)],
        Arc::clone(&c.sink),
    );
    let summary = handle.wait();
    assert_eq!(
        summary,
        RunSummary {
            done: 1,
            failed: 0,
            cancelled: 0
        },
        "{:#?}",
        c.snapshot()
    );

    let output = output_of(c.final_state("t1"));
    assert!(output.ends_with("clip.mp4"), "{output}");
    let info = probe(&paths, Path::new(&output));
    let video = info.video.as_ref().unwrap();
    assert!((video.fps - 15.0).abs() < 0.01, "输出帧率 {}", video.fps);
    assert_eq!((video.width, video.height), (320, 240));
    assert_eq!(video.codec, "h264");
    assert!(
        (info.duration - 2.0).abs() < 0.2,
        "输出时长 {}",
        info.duration
    );
    assert_eq!(info.audio.len(), 1);
    // 300 kbit/s 目标码率：容许较大误差，只排除明显没生效的情况
    let kbps = info.video_bit_rate().unwrap() / 1000;
    assert!(kbps < 600, "输出视频码率 {kbps} kbit/s");

    // 事件顺序：等待 → 运行 → 进度 → 完成；最后是 RunFinished
    let events = c.snapshot();
    let states: Vec<TaskState> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::TaskState { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [TaskState::Waiting, TaskState::Running, TaskState::Done]
    );
    assert!(events
        .iter()
        .any(|e| matches!(e, JobEvent::Progress { percent, .. } if *percent >= 100.0)));
    assert!(matches!(
        events.last(),
        Some(JobEvent::RunFinished { done: 1, .. })
    ));
    // 没有残留临时文件
    assert_eq!(dir.file_names("out"), ["clip.mp4"]);
}

#[test]
fn stop_and_cancel_task_leave_no_output() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("cancel");
    // 足够长，保证取消时还在编码
    let long = Clip {
        width: 1280,
        height: 720,
        fps: 30,
        seconds: 40.0,
        audio: true,
    };
    let a = dir.join("src/a.mp4");
    let b = dir.join("src/b.mp4");
    make_clip(&paths, &a, &long);
    std::fs::copy(&a, &b).unwrap();

    let mut recipe = Recipe::disabled();
    recipe.modules.fps.enabled = true;
    recipe.modules.fps.fps = Range::fixed(60.0);
    let settings = Settings {
        parallel: 1,
        ..Settings::default()
    };
    let c = collector();
    let handle = jobs::start(
        config(&paths, recipe, settings, &dir.join("out")),
        vec![task("a", &a), task("b", &b)],
        Arc::clone(&c.sink),
    );

    // a 开始出进度后：先取消等待中的 b，再整体停止
    c.wait_for(
        30,
        |e| matches!(e, JobEvent::Progress { task_id, .. } if task_id == "a"),
    );
    assert!(handle.cancel_task("b"));
    assert!(!handle.cancel_task("nope"));
    handle.stop();
    let summary = handle.wait();

    assert_eq!(
        summary,
        RunSummary {
            done: 0,
            failed: 0,
            cancelled: 2
        },
        "{:#?}",
        c.snapshot()
    );
    for id in ["a", "b"] {
        assert!(matches!(
            c.final_state(id),
            Some(JobEvent::TaskState {
                state: TaskState::Cancelled,
                ..
            })
        ));
    }
    assert!(
        dir.file_names("out").is_empty(),
        "取消后不应留下文件：{:?}",
        dir.file_names("out")
    );
}

#[test]
fn naming_collisions_rename_or_overwrite() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("naming");
    let clip = Clip {
        seconds: 1.0,
        ..Clip::default()
    };
    // 两个不同目录下的同名视频，并行处理到同一个输出目录
    let a1 = dir.join("src1/a.mp4");
    let a2 = dir.join("src2/a.mkv");
    make_clip(&paths, &a1, &clip);
    make_clip(&paths, &a2, &clip);
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("a.mp4"), b"existing").unwrap();

    let settings = Settings {
        parallel: 2,
        ..Settings::default()
    };
    let c = collector();
    let summary = jobs::start(
        config(&paths, Recipe::disabled(), settings.clone(), &out),
        vec![task("1", &a1), task("2", &a2)],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.done, 2, "{:#?}", c.snapshot());
    assert_eq!(dir.file_names("out"), ["a (1).mp4", "a (2).mp4", "a.mp4"]);
    assert_eq!(
        std::fs::read(out.join("a.mp4")).unwrap(),
        b"existing",
        "重命名模式不动已有文件"
    );

    // 覆盖模式：替换已有文件
    let overwrite = Settings {
        on_exists: OnExists::Overwrite,
        parallel: 1,
        ..settings
    };
    let c = collector();
    let summary = jobs::start(
        config(&paths, Recipe::disabled(), overwrite, &out),
        vec![task("3", &a1)],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(output_of(c.final_state("3")).ends_with("a.mp4"));
    assert_eq!(dir.file_names("out"), ["a (1).mp4", "a (2).mp4", "a.mp4"]);
    assert!(
        probe(&paths, &out.join("a.mp4")).video.is_some(),
        "已被有效视频覆盖"
    );
}

#[test]
fn output_next_to_source_never_overwrites_source_and_prefixes_numbers() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("same-dir");
    let src = dir.join("src/clip_raw.mp4");
    make_clip(
        &paths,
        &src,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    let original = std::fs::read(&src).unwrap();
    let settings = Settings {
        on_exists: OnExists::Overwrite,
        number_prefix: true,
        name_strip: cliprecipe_lib::model::NameStrip {
            text: "_raw".into(),
            regex: false,
        },
        ..Settings::default()
    };
    let mut cfg = config(&paths, Recipe::disabled(), settings, &dir.join("unused"));
    cfg.output_dir = None;
    let c = collector();
    let summary = jobs::start(cfg, vec![task("x", &src)], Arc::clone(&c.sink)).wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(dir.file_names("src"), ["1_clip.mp4", "clip_raw.mp4"]);
    assert_eq!(std::fs::read(&src).unwrap(), original, "源文件不能被改动");
    assert!(c
        .snapshot()
        .iter()
        .any(|e| matches!(e, JobEvent::Log { message, .. } if message.contains("输出目录与源文件目录相同"))));
}

#[test]
fn unusable_hardware_encoder_falls_back_to_software() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("hw-fallback");
    let src = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &src,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    // 假装探测到了 AMF / QSV / NVENC：没有对应硬件时运行会失败，应自动用 libx264 重试
    let mut cfg = config(
        &paths,
        Recipe::default(),
        Settings::default(),
        &dir.join("out"),
    );
    let hw = encoders::detect(&paths.ffmpeg);
    let fake = [HwVendor::Amf, HwVendor::Qsv, HwVendor::Nvenc]
        .into_iter()
        .map(|v| VideoEncoder::hardware(VideoCodec::H264, v))
        .find(|e| !hw.hardware.contains(e));
    let Some(fake) = fake else {
        println!("本机三种硬件编码器都可用，跳过回退测试");
        return;
    };
    cfg.encoders = EncoderCaps {
        hardware: vec![fake],
    };
    let c = collector();
    let summary = jobs::start(cfg, vec![task("hw", &src)], Arc::clone(&c.sink)).wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(c.snapshot().iter().any(|e| matches!(
        e,
        JobEvent::Log { message, .. } if message.contains("改用软件编码重试")
    )));
    let info = probe(&paths, &dir.join("out/clip.mp4"));
    assert!(
        (info.video.unwrap().fps - 30.0).abs() < 0.01,
        "默认配方帧率 30"
    );
}

#[test]
fn broken_input_fails_with_message() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("broken");
    let bogus = dir.join("src/bogus.mp4");
    std::fs::create_dir_all(bogus.parent().unwrap()).unwrap();
    std::fs::write(&bogus, b"definitely not a video").unwrap();
    let c = collector();
    let summary = jobs::start(
        config(
            &paths,
            Recipe::default(),
            Settings::default(),
            &dir.join("out"),
        ),
        vec![
            task("bad", &bogus),
            task("missing", &dir.join("src/none.mp4")),
        ],
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.failed, 2);
    match c.final_state("bad") {
        Some(JobEvent::TaskState {
            state: TaskState::Failed,
            error: Some(error),
            ..
        }) => assert!(error.contains("无法读取媒体信息"), "{error}"),
        other => panic!("{other:?}"),
    }
    assert!(dir.file_names("out").is_empty());
}
