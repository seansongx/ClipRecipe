//! 集成测试：模块在构建时登记的临时文件（`BuildContext::temp_file`）在任务 / 预览结束时一定被删除——
//! 成功、构建失败、ffmpeg 运行失败、取消都一样。借用 `pipe.text` 的长文本（写进临时文件用 `textfile=`）。
//!
//! 本文件只有一个测试函数，依次跑各个场景：同一进程里没有别的测试在创建构建临时目录，
//! 所以“本进程的临时目录都已删除”可以直接检查。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cliprecipe_lib::ffmpeg::{CancelToken, EncoderCaps, FfmpegPaths};
use cliprecipe_lib::jobs::{self, JobEvent, LogLevel, RunConfig, TaskSpec, TaskState};
use cliprecipe_lib::model::{Recipe, Settings};
use cliprecipe_lib::pipeline::modules::text::{TextItem, TextSource, MAX_INLINE_TEXT};
use cliprecipe_lib::pipeline::{temp, PreviewOutput};
use cliprecipe_lib::preview::{render_preview, PreviewDir, PreviewSpec};
use common::*;

/// 本进程的构建临时目录。
fn leftovers() -> Vec<PathBuf> {
    let prefix = format!("{}-", std::process::id());
    std::fs::read_dir(temp::temp_root())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// 只开文本模块、内容长到必须走临时文件的配方。
fn long_text_recipe() -> Recipe {
    let long = vec!["很长的文本，第二部分, 100%"; 300].join("\n");
    assert!(long.len() > MAX_INLINE_TEXT);
    let mut recipe = Recipe::disabled();
    recipe.modules.text.enabled = true;
    recipe.modules.text.items = vec![TextItem {
        enabled: true,
        source: TextSource::Custom,
        custom_text: long,
        ..TextItem::default()
    }];
    recipe
}

fn config(paths: &FfmpegPaths, recipe: Recipe, out: &Path) -> RunConfig {
    RunConfig {
        paths: paths.clone(),
        encoders: EncoderCaps::software_only(),
        recipe,
        settings: Settings::default(),
        output_dir: Some(out.to_path_buf()),
        seed: 5,
    }
}

fn task(input: &Path, info: Option<cliprecipe_lib::ffmpeg::MediaInfo>) -> Vec<TaskSpec> {
    vec![TaskSpec {
        id: "t".into(),
        path: input.to_string_lossy().into_owned(),
        source_root: None,
        info,
        number: 0,
    }]
}

fn final_state(c: &Collector) -> TaskState {
    match c.final_state("t") {
        Some(JobEvent::TaskState { state, .. }) => state,
        other => panic!("没有最终状态：{other:?}"),
    }
}

fn used_textfile(c: &Collector) -> bool {
    c.snapshot().iter().any(|e| {
        matches!(e, JobEvent::Log { level: LogLevel::Debug, message, .. }
            if message.contains(":textfile=") && message.contains(temp::TEMP_DIR_NAME))
    })
}

#[test]
fn build_temp_files_are_removed_however_the_task_ends() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("build-temp");
    let input = dir.join("src/in.mp4");
    let clip = Clip {
        width: 320,
        height: 240,
        fps: 25,
        seconds: 2.0,
        audio: true,
    };
    make_clip(&paths, &input, &clip);
    assert!(leftovers().is_empty());

    // 1. 成功：命令用的是临时文件，任务结束后删除
    let c = collector();
    let summary = jobs::start(
        config(&paths, long_text_recipe(), &dir.join("out1")),
        task(&input, None),
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    assert!(used_textfile(&c), "长文本应改用 textfile=");
    assert!(leftovers().is_empty(), "成功后残留：{:?}", leftovers());

    // 2. 构建失败：文本模块已写好临时文件，排在后面的背景音参数无效
    let mut recipe = long_text_recipe();
    recipe.modules.bgm.enabled = true; // 没有选择音乐 → 构建失败
    let c = collector();
    let summary = jobs::start(
        config(&paths, recipe, &dir.join("out2")),
        task(&input, None),
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(leftovers().is_empty(), "构建失败后残留：{:?}", leftovers());

    // 3. ffmpeg 运行失败：媒体信息事先给出，文件内容却已损坏
    let broken = dir.join("src/broken.mp4");
    std::fs::copy(&input, &broken).unwrap();
    let info = probe(&paths, &broken);
    std::fs::write(&broken, b"not a video at all").unwrap();
    let c = collector();
    let summary = jobs::start(
        config(&paths, long_text_recipe(), &dir.join("out3")),
        task(&broken, Some(info)),
        Arc::clone(&c.sink),
    )
    .wait();
    assert_eq!(summary.failed, 1, "{:#?}", c.snapshot());
    assert!(used_textfile(&c));
    assert!(leftovers().is_empty(), "运行失败后残留：{:?}", leftovers());
    assert!(dir.file_names("out3").is_empty());

    // 4. 取消：运行中临时文件存在，取消后删除
    let slow = dir.join("src/slow.mp4");
    make_clip(
        &paths,
        &slow,
        &Clip {
            width: 1280,
            height: 720,
            fps: 30,
            seconds: 30.0,
            audio: true,
        },
    );
    let c = collector();
    let handle = jobs::start(
        config(&paths, long_text_recipe(), &dir.join("out4")),
        task(&slow, None),
        Arc::clone(&c.sink),
    );
    c.wait_for(60, |e| matches!(e, JobEvent::Progress { .. }));
    let during = leftovers();
    assert_eq!(during.len(), 1, "运行中应有一个临时目录：{during:?}");
    assert!(during[0].join("text-1.txt").is_file());
    assert!(handle.cancel_task("t"));
    let summary = handle.wait();
    assert_eq!(summary.cancelled, 1, "{:#?}", c.snapshot());
    assert_eq!(final_state(&c), TaskState::Cancelled);
    assert!(leftovers().is_empty(), "取消后残留：{:?}", leftovers());

    // 5. 预览：同样在结束时删除
    let preview_dir = PreviewDir::under(dir.path(), 1);
    let result = render_preview(
        &paths,
        &PreviewSpec {
            path: input.to_string_lossy().into_owned(),
            info: None,
            recipe: long_text_recipe(),
            settings: Settings::default(),
            only_module: Some("pipe.text".into()),
            output: PreviewOutput::Clip {
                start: 0.0,
                duration: 1.0,
            },
            seed: Some(1),
            number: 1,
            max_height: 240,
            subtitle: None,
        },
        &preview_dir,
        &CancelToken::new(),
        |_| {},
    )
    .unwrap();
    assert!(Path::new(&result.path).is_file());
    assert!(leftovers().is_empty(), "预览后残留：{:?}", leftovers());
}
