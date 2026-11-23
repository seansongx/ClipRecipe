//! 集成测试（M4）：任务成功后的收尾——删除源视频（`set.delete-source`）、输出移回源位置（`set.move-back`），
//! 经真实调度器与 ffmpeg 运行。失败、取消时源视频必须原样保留。
//! 回收站通过 `RunExtras::dispose` 替换（测试不碰系统回收站）；永久删除走真实的删除路径。

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cliprecipe_lib::ffmpeg::{EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunExtras, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{OnExists, Recipe, Settings};
use common::*;

fn config(paths: &FfmpegPaths, settings: Settings, out: &Path) -> RunConfig {
    RunConfig {
        paths: paths.clone(),
        encoders: EncoderCaps::software_only(),
        recipe: Recipe::disabled(),
        settings,
        output_dir: Some(out.to_path_buf()),
        seed: 9,
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

/// 记录被“移入回收站”的文件，并真的删除它（模拟回收站）。
fn recording_dispose() -> (Arc<Mutex<Vec<PathBuf>>>, RunExtras) {
    let calls: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let record = Arc::clone(&calls);
    let extras = RunExtras {
        dispose: Some(Arc::new(move |p: &Path| {
            record.lock().unwrap().push(p.to_path_buf());
            std::fs::remove_file(p).map_err(|e| e.to_string())
        })),
        ..RunExtras::default()
    };
    (calls, extras)
}

fn outputs_of(c: &Collector, id: &str) -> Vec<String> {
    match c.final_state(id) {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            outputs,
            ..
        }) => outputs,
        other => panic!("任务应完成：{other:?}"),
    }
}

fn run(config: RunConfig, tasks: Vec<TaskSpec>, extras: RunExtras) -> (RunSummary, Collector) {
    let c = collector();
    let summary = jobs::start_with(config, tasks, extras, c.sink.clone()).wait();
    (summary, c)
}

#[test]
fn delete_source_to_trash_after_success() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-delete");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let settings = Settings {
        delete_source: true,
        ..Settings::default()
    };
    let (calls, extras) = recording_dispose();
    let (summary, c) = run(
        config(&paths, settings, &dir.join("out")),
        vec![task("t1", &input)],
        extras,
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(
        *calls.lock().unwrap(),
        [input.as_path()],
        "默认移入回收站（这里替换为记录）"
    );
    assert!(!input.exists());
    let outputs = outputs_of(&c, "t1");
    assert!(Path::new(&outputs[0]).is_file());
    assert_close(
        probe(&paths, Path::new(&outputs[0])).duration,
        2.0,
        0.15,
        "输出完整",
    );
}

#[test]
fn permanent_delete_uses_real_removal() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-delete-permanent");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let settings = Settings {
        delete_source: true,
        delete_to_trash: false,
        ..Settings::default()
    };
    let (summary, c) = run(
        config(&paths, settings, &dir.join("out")),
        vec![task("t1", &input)],
        RunExtras::default(),
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(!input.exists(), "永久删除");
    assert!(Path::new(&outputs_of(&c, "t1")[0]).is_file());
}

#[test]
fn failed_task_keeps_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-delete-failed");
    // 不是视频的“视频”：探测失败，任务失败
    let input = dir.join("src/broken.mp4");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, b"not a video").unwrap();
    let settings = Settings {
        delete_source: true,
        move_back: true,
        ..Settings::default()
    };
    let (calls, extras) = recording_dispose();
    let (summary, c) = run(
        config(&paths, settings, &dir.join("out")),
        vec![task("t1", &input)],
        extras,
    );
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&input).unwrap(), b"not a video");
}

#[test]
fn cancelled_task_keeps_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-delete-cancel");
    let input = dir.join("src/long.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            width: 1280,
            height: 720,
            seconds: 30.0,
            ..Clip::default()
        },
    );
    let before = std::fs::metadata(&input).unwrap().len();
    let settings = Settings {
        delete_source: true,
        ..Settings::default()
    };
    let (calls, extras) = recording_dispose();
    let c = collector();
    let handle = jobs::start_with(
        config(&paths, settings, &dir.join("out")),
        vec![task("t1", &input)],
        extras,
        c.sink.clone(),
    );
    c.wait_for(
        60,
        |e| matches!(e, JobEvent::Progress { percent, .. } if *percent > 1.0),
    );
    assert!(handle.cancel_task("t1"));
    let summary = handle.wait();
    assert_eq!(summary.cancelled, 1, "{:#?}", c.snapshot());
    assert!(calls.lock().unwrap().is_empty(), "取消时不删除源视频");
    assert_eq!(std::fs::metadata(&input).unwrap().len(), before);
    assert!(dir.file_names("out").is_empty(), "也没有留下半截输出");
}

#[test]
fn move_back_renames_next_to_source_and_never_overwrites_it() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-move-back");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let source_bytes = std::fs::read(&input).unwrap();
    for on_exists in [OnExists::Rename, OnExists::Overwrite] {
        let settings = Settings {
            move_back: true,
            on_exists,
            ..Settings::default()
        };
        let (summary, c) = run(
            config(&paths, settings, &dir.join("out")),
            vec![task("t1", &input)],
            RunExtras::default(),
        );
        assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
        let output = PathBuf::from(&outputs_of(&c, "t1")[0]);
        assert_eq!(output.parent(), input.parent(), "输出移到源视频所在目录");
        assert_ne!(output, input, "{on_exists:?}：不会覆盖源视频本身");
        assert!(output.is_file());
        assert_eq!(
            std::fs::read(&input).unwrap(),
            source_bytes,
            "源视频未被改动"
        );
    }
    let names = dir.file_names("src");
    assert_eq!(names.len(), 3, "源视频 + 两次输出（重命名）：{names:?}");
    assert!(dir.file_names("out").is_empty(), "输出目录里不再保留文件");
}

#[test]
fn move_back_with_delete_replaces_the_source() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("m4-move-back-delete");
    let input = dir.join("src/clip.mp4");
    make_clip(&paths, &input, &Clip::default());
    let source_bytes = std::fs::read(&input).unwrap();
    let settings = Settings {
        move_back: true,
        delete_source: true,
        ..Settings::default()
    };
    let (calls, extras) = recording_dispose();
    let (summary, c) = run(
        config(&paths, settings, &dir.join("out")),
        vec![task("t1", &input)],
        extras,
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(*calls.lock().unwrap(), [input.as_path()]);
    assert_eq!(outputs_of(&c, "t1"), [input.to_string_lossy().into_owned()]);
    assert_ne!(
        std::fs::read(&input).unwrap(),
        source_bytes,
        "源位置现在是处理后的输出"
    );
    assert_close(probe(&paths, &input).duration, 2.0, 0.15, "输出完整");
}
