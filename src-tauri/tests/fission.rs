//! 集成测试：裂变（pipe.fission）—— 一个视频生成 N 个参数不同的输出，放在正确的目录里。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::EncoderCaps;
use cliprecipe_lib::jobs::{self, JobEvent, RunConfig, RunSummary, TaskSpec, TaskState};
use cliprecipe_lib::model::{Range, Recipe, Settings};
use common::*;

fn run(recipe: Recipe, settings: Settings, input: &Path, out: &Path) -> (RunSummary, Collector) {
    let paths = ffmpeg_paths();
    let c = collector();
    let summary = jobs::start(
        RunConfig {
            paths,
            encoders: EncoderCaps::software_only(),
            recipe,
            settings,
            output_dir: Some(out.to_path_buf()),
            seed: 11,
        },
        vec![TaskSpec {
            id: "f".into(),
            path: input.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }],
        Arc::clone(&c.sink),
    )
    .wait();
    (summary, c)
}

/// 帧率在大范围内随机：不同轮次几乎不可能取到同一个值。
fn fission_recipe(n: u32) -> Recipe {
    let mut recipe = Recipe::disabled();
    recipe.fission = n;
    recipe.modules.fps.enabled = true;
    recipe.modules.fps.fps = Range::new(8.0, 30.0);
    recipe
}

fn done_outputs(c: &Collector) -> Vec<PathBuf> {
    match c.final_state("f") {
        Some(JobEvent::TaskState {
            state: TaskState::Done,
            outputs,
            output,
            ..
        }) => {
            assert_eq!(output.as_ref(), outputs.last(), "output 为最后一个");
            outputs.into_iter().map(PathBuf::from).collect()
        }
        other => panic!("任务应完成，实际：{other:?}\n{:#?}", c.snapshot()),
    }
}

#[test]
fn fission_produces_distinct_outputs_in_video_folder() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fission");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let (summary, c) = run(fission_recipe(3), Settings::default(), &input, &out);
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());

    let outputs = done_outputs(&c);
    let folder = out.join("clip");
    assert_eq!(
        outputs,
        [
            folder.join("clip_1.mp4"),
            folder.join("clip_2.mp4"),
            folder.join("clip_3.mp4")
        ]
    );
    assert_eq!(
        dir.file_names("out/clip"),
        ["clip_1.mp4", "clip_2.mp4", "clip_3.mp4"],
        "没有残留临时文件"
    );
    let mut fps: Vec<f64> = outputs
        .iter()
        .map(|p| probe(&paths, p).video.unwrap().fps)
        .collect();
    for f in &fps {
        assert!((7.9..=30.1).contains(f), "帧率 {f} 应在范围内");
    }
    fps.sort_by(f64::total_cmp);
    fps.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    assert_eq!(fps.len(), 3, "三次裂变的随机帧率应各不相同：{fps:?}");

    // 进度覆盖全部轮次，最后到 100%
    let events = c.snapshot();
    let rounds: Vec<(usize, usize)> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress { round, rounds, .. } => Some((*round, *rounds)),
            _ => None,
        })
        .collect();
    assert!(rounds.iter().all(|(_, total)| *total == 3));
    assert!(rounds.iter().any(|(r, _)| *r == 3));
    let percents: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress { percent, .. } => Some(*percent),
            _ => None,
        })
        .collect();
    assert!(
        percents.windows(2).all(|w| w[1] >= w[0] - 1e-9),
        "进度不应倒退：{percents:?}"
    );
    assert_eq!(percents.last().copied(), Some(100.0));
    // 第一轮结束时整体进度约为 1/3
    assert!(percents.iter().any(|p| (*p - 100.0 / 3.0).abs() < 0.01));
}

#[test]
fn fission_folder_puts_each_round_in_its_own_folder() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fission-folder");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            audio: false,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let settings = Settings {
        fission_folder: true,
        ..Settings::default()
    };
    let (summary, c) = run(fission_recipe(2), settings, &input, &out);
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c);
    assert_eq!(
        outputs,
        [
            out.join("clip").join("第 1 次裂变").join("clip.mp4"),
            out.join("clip").join("第 2 次裂变").join("clip.mp4"),
        ]
    );
    for output in &outputs {
        assert!(probe(&paths, output).video.is_some());
    }
}

#[test]
fn single_fission_matches_plain_run() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fission-one");
    let input = dir.join("src/clip.mp4");
    make_clip(
        &paths,
        &input,
        &Clip {
            seconds: 1.0,
            audio: false,
            ..Clip::default()
        },
    );
    let out = dir.join("out");
    let (summary, c) = run(fission_recipe(1), Settings::default(), &input, &out);
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert_eq!(done_outputs(&c), [out.join("clip.mp4")]);
}
