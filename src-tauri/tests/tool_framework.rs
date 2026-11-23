//! 集成测试：工具框架本身——多进一出、多步（分析 → 中间文件 → 拼接）、分步进度、取消与临时文件清理。
//!
//! 这里的 `SceneConcat` 也是“多进一出 + 先分析后处理 + 临时文件”类工具（合并、重组、静音移除…）的写法示例：
//! 1. `plan` 用 `plan::all_videos_as_one` 把全部视频合成一个工作项；
//! 2. 每个输入先 `ctx.analyze_input`（场景检测），再 `ctx.run_encode` 统一规格写进 `ctx.temp_file`；
//! 3. `media::write_concat_list` + concat 分离器把中间文件无损拼成一个输出。

mod common;
mod tool_support;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cliprecipe_lib::ffmpeg::path_arg;
use cliprecipe_lib::jobs::{JobEvent, TaskState};
use cliprecipe_lib::tools::analysis::SceneDetect;
use cliprecipe_lib::tools::context::Encoding;
use cliprecipe_lib::tools::media::{args, secs, write_concat_list};
use cliprecipe_lib::tools::{
    plan, OutputSpec, PlanRequest, Tool, ToolContext, ToolError, WorkItem,
};
use common::*;
use tool_support::*;

#[derive(Default)]
struct SceneConcat {
    /// 记录用过的临时目录与检测到的切换点，供测试断言
    temp_dirs: Mutex<Vec<PathBuf>>,
    cuts: Mutex<Vec<Vec<f64>>>,
}

impl Tool for SceneConcat {
    fn id(&self) -> &'static str {
        "tool.test-scene-concat"
    }

    fn plan(&self, req: &PlanRequest) -> Result<Vec<WorkItem>, String> {
        plan::all_videos_as_one(req, "拼接")
    }

    fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
        let n = ctx.inputs().len();
        // 每个输入：分析 1 份 + 编码 2 份；最后拼接 1 份
        let mut weights = Vec::new();
        for _ in 0..n {
            weights.extend([1.0, 2.0]);
        }
        weights.push(1.0);
        ctx.set_steps(&weights);
        self.temp_dirs.lock().unwrap().push(ctx.temp_dir()?);

        let mut parts = Vec::new();
        let mut total = 0.0;
        for i in 0..n {
            let cuts = ctx.analyze_input(i, &SceneDetect::default())?;
            self.cuts.lock().unwrap().push(cuts);
            let info = ctx.info(i)?;
            let input = path_arg(ctx.input_path(i)?)?;
            let part = ctx.temp_file("part.mp4")?;
            let part_arg = path_arg(&part)?;
            ctx.run_encode("统一规格", info.duration, |enc| {
                let mut a = args(&[
                    "-i",
                    &input,
                    "-vf",
                    "scale=320:240,fps=25",
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0",
                ]);
                a.extend(enc.video_args("mp4"));
                a.extend(Encoding::audio_args(2));
                a.extend(args(&["-ar", "48000", "-ac", "2", &part_arg]));
                Ok(a)
            })?;
            total += info.duration;
            parts.push(part);
        }
        let list = ctx.temp_file("list.txt")?;
        write_concat_list(&list, &parts)?;
        let out = ctx.output(&OutputSpec::file("mp4").stem("merged"))?;
        ctx.run_ffmpeg(
            "拼接",
            args(&[
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                &path_arg(&list)?,
                "-c",
                "copy",
                "-t",
                &secs(total),
                &path_arg(&out)?,
            ]),
            total,
        )
    }
}

#[test]
fn many_to_one_multi_step_with_progress_and_cleanup() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fw-ok");
    let a = dir.join("src/a.mp4");
    let b = dir.join("src/b.mp4");
    make_scene_clip(&paths, &a, 1.5);
    make_scene_clip(&paths, &b, 1.0);
    let tool = Arc::new(SceneConcat::default());
    let out = dir.join("out");
    let (summary, c) = run_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        Arc::clone(&tool) as Arc<dyn Tool>,
        &[video_task("a", &a), video_task("b", &b)],
    );
    assert_eq!(summary.done, 1, "{:#?}", c.snapshot());
    let outputs = done_outputs(&c, "tool.test-scene-concat#all");
    assert_eq!(outputs, [out.join("merged.mp4")]);
    assert_close(video_duration(&paths, &outputs[0]), 7.5, 0.1, "拼接后时长");

    // 分析结果：a 在 1.5、3 秒，b 在 1、2 秒切换
    let cuts = tool.cuts.lock().unwrap().clone();
    assert_eq!(cuts.len(), 2);
    assert_eq!(cuts[0].len(), 2, "{cuts:?}");
    assert_close(cuts[1][1], 2.0, 0.05, "b 的第二个切换点");

    // 进度：共 5 步，按步推进、单调不减，最后 100
    let progress: Vec<(usize, usize, f64)> = c
        .snapshot()
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress {
                round,
                rounds,
                percent,
                ..
            } => Some((*round, *rounds, *percent)),
            _ => None,
        })
        .collect();
    assert!(
        progress.iter().all(|(_, steps, _)| *steps == 5),
        "{progress:?}"
    );
    assert!(progress
        .windows(2)
        .all(|w| w[1].2 >= w[0].2 - 1e-9 && w[1].0 >= w[0].0));
    assert_eq!(progress.last().map(|p| (p.0, p.2)), Some((5, 100.0)));

    // 临时目录已删除，输出目录里只有最终文件
    let temp_dirs = tool.temp_dirs.lock().unwrap().clone();
    assert_eq!(temp_dirs.len(), 1);
    assert!(temp_dirs[0].starts_with(dir.join("tmp")));
    assert!(!temp_dirs[0].exists(), "临时目录应已删除");
    assert_eq!(all_files(&out), [out.join("merged.mp4")]);
}

#[test]
fn cancelling_mid_plan_removes_temp_files_and_partial_outputs() {
    let paths = ffmpeg_paths();
    let dir = TempDir::new("fw-cancel");
    let mut videos = Vec::new();
    for name in ["a", "b", "c"] {
        let path = dir.join(&format!("src/{name}.mp4"));
        make_lavfi(
            &paths,
            &path,
            "testsrc2=s=640x480:r=25:d=20",
            Some("sine=duration=20"),
            &[],
        );
        videos.push(video_task(name, &path));
    }
    let tool = Arc::new(SceneConcat::default());
    let out = dir.join("out");
    let c = collector();
    let handle = start_tool(
        tool_config(&paths, &out, &dir.join("tmp")),
        Arc::clone(&tool) as Arc<dyn Tool>,
        &videos,
        &BTreeMap::new(),
        &c,
    );
    // 等到第一个输入的中间文件开始编码（第 2 步），此时临时目录里已有文件
    c.wait_for(
        120,
        |e| matches!(e, JobEvent::Progress { round, .. } if *round >= 2),
    );
    let temp_dirs = tool.temp_dirs.lock().unwrap().clone();
    assert_eq!(temp_dirs.len(), 1);
    assert!(temp_dirs[0].is_dir());
    handle.stop();
    let summary = handle.wait();
    assert_eq!(
        (summary.done, summary.cancelled),
        (0, 1),
        "{:#?}",
        c.snapshot()
    );
    assert!(matches!(
        c.final_state("tool.test-scene-concat#all"),
        Some(JobEvent::TaskState {
            state: TaskState::Cancelled,
            ..
        })
    ));
    assert!(!temp_dirs[0].exists(), "取消后临时目录应已删除");
    assert!(all_files(&out).is_empty(), "取消后不留输出");
    assert!(all_files(&dir.join("tmp")).is_empty());
    assert!(matches!(
        c.snapshot().last(),
        Some(JobEvent::RunFinished { cancelled: 1, .. })
    ));
}
