//! 工具调度：校验输入、规划工作项，按并行数执行，推送与批量处理相同的事件（[`JobEvent`]）。
//!
//! - 任务状态 / 进度事件的 `task_id` 是工作项 ID；进度事件的 `round` / `rounds` 表示第几步 / 共几步；
//! - 工作项成功时输出改为最终文件名，失败或取消时删除全部临时输出；临时目录总会删除；
//! - 停止：取消运行中的工作项，未开始的标记为已取消。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::ffmpeg::{CancelToken, EncoderCaps, FfmpegPaths};
use crate::jobs::{EventSink, JobEvent, LogLevel, RunSummary, TaskSpec, TaskState};
use crate::model::Settings;
use crate::pipeline::modules::BitrateParams;

use super::context::ToolContext;
use super::plan::{PlanRequest, WorkItem};
use super::{Tool, ToolError};

/// 临时目录：`<临时目录>/cliprecipe-tools/<进程号>/<序号>-<编号>/`。
pub const TEMP_DIR_NAME: &str = "cliprecipe-tools";
/// 启动时清理其它进程超过这么久的残留临时目录。
const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);

/// 一次工具运行的配置。
#[derive(Debug, Clone)]
pub struct ToolRunConfig {
    pub paths: FfmpegPaths,
    pub encoders: EncoderCaps,
    pub settings: Settings,
    /// None 表示输出到输入文件所在目录
    pub output_dir: Option<PathBuf>,
    /// 流水线的码率设置（`pipe.bitrate`）：工具只沿用它和输出位置
    pub bitrate: BitrateParams,
    /// 基础种子；每个工作项用 基础种子 + 序号 派生
    pub seed: u64,
    /// 临时目录的上级目录；None 为系统临时目录
    pub temp_root: Option<PathBuf>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一次运行中各工作项共享的状态。
pub(crate) struct RunShared {
    pub config: ToolRunConfig,
    pub tool: Arc<dyn Tool>,
    sink: Arc<dyn EventSink>,
    max_number: usize,
    pending: Mutex<VecDeque<WorkItem>>,
    running: Mutex<HashMap<String, CancelToken>>,
    /// 本次运行已占用的输出路径（[`super::output::key`]）
    pub reserved: Mutex<HashSet<String>>,
    summary: Mutex<RunSummary>,
    stop: CancelToken,
    pub warned_same_dir: AtomicBool,
    /// 临时文件、临时输出名用的递增编号
    pub unique: AtomicU64,
}

impl RunShared {
    pub fn emit(&self, event: JobEvent) {
        self.sink.emit(event);
    }

    fn log(&self, level: LogLevel, task_id: Option<&str>, message: impl Into<String>) {
        self.emit(JobEvent::Log {
            level,
            task_id: task_id.map(str::to_owned),
            message: message.into(),
        });
    }

    /// 序号补零的依据（最大序号）。
    pub fn total_numbers(&self) -> usize {
        self.max_number
    }

    /// 本进程的临时目录。
    pub fn temp_root(&self) -> PathBuf {
        temp_root_for(self.config.temp_root.as_deref()).join(std::process::id().to_string())
    }

    fn set_state(&self, id: &str, state: TaskState, outputs: Vec<String>, error: Option<String>) {
        {
            let mut summary = lock(&self.summary);
            match state {
                TaskState::Done => summary.done += 1,
                TaskState::Failed => summary.failed += 1,
                TaskState::Cancelled => summary.cancelled += 1,
                TaskState::Waiting | TaskState::Running => {}
            }
        }
        self.emit(JobEvent::TaskState {
            task_id: id.to_owned(),
            state,
            output: if state == TaskState::Done {
                outputs.last().cloned()
            } else {
                None
            },
            error,
            outputs,
        });
    }
}

fn temp_root_for(root: Option<&Path>) -> PathBuf {
    root.map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join(TEMP_DIR_NAME)
}

/// 删除其它进程留下的、超过一天的临时目录（异常退出时可能残留）。
pub fn cleanup_stale_temp(root: Option<&Path>) {
    let base = temp_root_for(root);
    let own = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(&base) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() == own {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// 正在进行的一次工具运行。
pub struct ToolRunHandle {
    shared: Arc<RunShared>,
    supervisor: Option<JoinHandle<RunSummary>>,
}

impl ToolRunHandle {
    /// 停止：取消运行中的工作项，未开始的标记为已取消。
    pub fn stop(&self) {
        self.shared.stop.cancel();
        for token in lock(&self.shared.running).values() {
            token.cancel();
        }
    }

    /// 取消单个工作项（等待中或运行中）。返回是否找到。
    pub fn cancel_item(&self, id: &str) -> bool {
        if let Some(token) = lock(&self.shared.running).get(id) {
            token.cancel();
            return true;
        }
        let removed = {
            let mut pending = lock(&self.shared.pending);
            let before = pending.len();
            pending.retain(|t| t.id != id);
            before != pending.len()
        };
        if removed {
            self.shared
                .set_state(id, TaskState::Cancelled, Vec::new(), None);
        }
        removed
    }

    pub fn tool_id(&self) -> &'static str {
        self.shared.tool.id()
    }

    pub fn is_finished(&self) -> bool {
        self.supervisor
            .as_ref()
            .map_or(true, JoinHandle::is_finished)
    }

    /// 等待全部结束。
    pub fn wait(mut self) -> RunSummary {
        self.supervisor
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_else(|| *lock(&self.shared.summary))
    }
}

/// 校验输入并规划工作项（不执行），种子为 0。需要与执行用同一个种子时用 [`plan_seeded`]。
pub fn plan(
    tool: &dyn Tool,
    videos: &[TaskSpec],
    folders: &BTreeMap<String, PathBuf>,
    settings: &Settings,
) -> Result<Vec<WorkItem>, String> {
    plan_seeded(tool, videos, folders, settings, 0)
}

/// 校验输入并规划工作项（不执行）；`seed` 为本次运行的种子（[`ToolRunConfig::seed`]），
/// 规划阶段的随机（打乱、随机配对）由它派生。
pub fn plan_seeded(
    tool: &dyn Tool,
    videos: &[TaskSpec],
    folders: &BTreeMap<String, PathBuf>,
    settings: &Settings,
    seed: u64,
) -> Result<Vec<WorkItem>, String> {
    if !tool.implemented() {
        return Err(format!("{}：{}", tool.id(), super::NOT_IMPLEMENTED));
    }
    let inputs = tool.inputs();
    let videos: &[TaskSpec] = if inputs.videos { videos } else { &[] };
    let known: HashSet<&str> = inputs.folders.iter().map(|f| f.key).collect();
    let folders: BTreeMap<String, PathBuf> = folders
        .iter()
        .filter(|(k, _)| known.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let items = tool.plan(&PlanRequest {
        tool_id: tool.id(),
        videos,
        folders: &folders,
        settings,
        seed,
    })?;
    if items.is_empty() {
        return Err("没有需要处理的内容".into());
    }
    let mut ids = HashSet::new();
    for item in &items {
        if item.inputs.is_empty() {
            return Err(format!("工作项“{}”没有输入", item.label));
        }
        if !ids.insert(item.id.as_str()) {
            return Err(format!("工作项 ID 重复：{}", item.id));
        }
    }
    Ok(items)
}

/// 开始运行。立即返回；所有工作项先被标记为“等待中”。
pub fn start(
    config: ToolRunConfig,
    tool: Arc<dyn Tool>,
    items: Vec<WorkItem>,
    sink: Arc<dyn EventSink>,
) -> ToolRunHandle {
    cleanup_stale_temp(config.temp_root.as_deref());
    let total = items.len();
    let workers = config.settings.effective_parallel().min(total.max(1));
    let max_number = items.iter().map(|i| i.number).max().unwrap_or(1);
    let shared = Arc::new(RunShared {
        config,
        tool,
        sink,
        max_number,
        pending: Mutex::new(VecDeque::new()),
        running: Mutex::default(),
        reserved: Mutex::default(),
        summary: Mutex::default(),
        stop: CancelToken::new(),
        warned_same_dir: AtomicBool::new(false),
        unique: AtomicU64::new(1),
    });
    for item in &items {
        shared.emit(JobEvent::TaskState {
            task_id: item.id.clone(),
            state: TaskState::Waiting,
            output: None,
            error: None,
            outputs: Vec::new(),
        });
    }
    *lock(&shared.pending) = items.into();
    shared.log(
        LogLevel::Info,
        None,
        format!("开始处理 {total} 项，并行 {workers} 个"),
    );

    let supervisor_shared = Arc::clone(&shared);
    let supervisor = std::thread::spawn(move || {
        let shared = supervisor_shared;
        let handles: Vec<JoinHandle<()>> = (0..workers)
            .map(|_| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || worker(&shared))
            })
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
        let leftover: Vec<WorkItem> = lock(&shared.pending).drain(..).collect();
        for item in leftover {
            shared.set_state(&item.id, TaskState::Cancelled, Vec::new(), None);
        }
        // 本进程的临时目录为空时一并删除
        let _ = std::fs::remove_dir(shared.temp_root());
        let summary = *lock(&shared.summary);
        shared.log(
            LogLevel::Info,
            None,
            format!(
                "全部结束：完成 {}，失败 {}，取消 {}",
                summary.done, summary.failed, summary.cancelled
            ),
        );
        shared.emit(JobEvent::RunFinished {
            done: summary.done,
            failed: summary.failed,
            cancelled: summary.cancelled,
        });
        summary
    });

    ToolRunHandle {
        shared,
        supervisor: Some(supervisor),
    }
}

fn worker(shared: &RunShared) {
    loop {
        if shared.stop.is_cancelled() {
            return;
        }
        let Some(item) = lock(&shared.pending).pop_front() else {
            return;
        };
        let token = CancelToken::new();
        lock(&shared.running).insert(item.id.clone(), token.clone());
        if shared.stop.is_cancelled() {
            token.cancel();
        }
        shared.set_state(&item.id, TaskState::Running, Vec::new(), None);
        let (outcome, outputs) = run_item(shared, &item, &token);
        lock(&shared.running).remove(&item.id);
        match outcome {
            Ok(()) => {
                let message = match outputs.as_slice() {
                    [] => "完成（没有输出文件）".to_owned(),
                    [single] => format!("完成：{single}"),
                    many => format!("完成：{} 个输出\n{}", many.len(), many.join("\n")),
                };
                shared.log(LogLevel::Info, Some(&item.id), message);
                shared.set_state(&item.id, TaskState::Done, outputs, None);
            }
            Err(ToolError::Cancelled) => {
                shared.log(
                    LogLevel::Warn,
                    Some(&item.id),
                    format!("已取消：{}", item.label),
                );
                shared.set_state(&item.id, TaskState::Cancelled, Vec::new(), None);
            }
            Err(ToolError::Failed(message)) => {
                shared.log(
                    LogLevel::Error,
                    Some(&item.id),
                    format!("失败：{} —— {message}", item.label),
                );
                shared.set_state(&item.id, TaskState::Failed, Vec::new(), Some(message));
            }
        }
    }
}

/// 执行一个工作项：成功时提交输出，否则删除临时输出；临时目录在上下文释放时删除。
fn run_item(
    shared: &RunShared,
    item: &WorkItem,
    token: &CancelToken,
) -> (Result<(), ToolError>, Vec<String>) {
    let mut ctx = ToolContext::new(shared, item, token.clone());
    let result = catch_unwind(AssertUnwindSafe(|| shared.tool.run(&mut ctx)))
        .unwrap_or_else(|_| Err(ToolError::Failed("工具内部错误".into())));
    let result = match result {
        Err(ToolError::Failed(_)) if token.is_cancelled() => Err(ToolError::Cancelled),
        Ok(()) if token.is_cancelled() => Err(ToolError::Cancelled),
        other => other,
    };
    match result {
        Ok(()) => match ctx.commit() {
            Ok(outputs) => (Ok(()), outputs),
            Err(e) => (Err(ToolError::Failed(e)), Vec::new()),
        },
        Err(e) => {
            ctx.discard();
            (Err(e), Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{FolderSlot, ToolInputs};
    use std::sync::mpsc;

    /// 测试用工具：不运行 ffmpeg，直接写输出 / 临时文件，可按名字失败。
    struct Fake {
        fail_on: &'static str,
    }

    impl Tool for Fake {
        fn id(&self) -> &'static str {
            "tool.fake"
        }
        fn inputs(&self) -> ToolInputs {
            const SLOTS: &[FolderSlot] = &[FolderSlot {
                key: "extra",
                label: "额外",
            }];
            ToolInputs::videos_and_folders(SLOTS)
        }
        fn run(&self, ctx: &mut ToolContext) -> Result<(), ToolError> {
            let temp = ctx.temp_file("scratch.txt")?;
            std::fs::write(&temp, b"x").map_err(|e| e.to_string())?;
            let out = ctx.output(&crate::tools::OutputSpec::file("txt").suffix("_a"))?;
            std::fs::write(&out, b"a").map_err(|e| e.to_string())?;
            let folder = ctx.output(&crate::tools::OutputSpec::folder().suffix("_dir"))?;
            std::fs::write(folder.join("f.txt"), b"f").map_err(|e| e.to_string())?;
            if ctx.item().label.contains(self.fail_on) {
                return Err("故意失败".into());
            }
            Ok(())
        }
    }

    fn setup(name: &str) -> (PathBuf, ToolRunConfig) {
        let dir = std::env::temp_dir().join(format!("cr-toolrun-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let config = ToolRunConfig {
            paths: FfmpegPaths {
                ffmpeg: "ffmpeg".into(),
                ffprobe: "ffprobe".into(),
            },
            encoders: EncoderCaps::software_only(),
            settings: Settings::default(),
            output_dir: Some(dir.join("out")),
            bitrate: BitrateParams::default(),
            seed: 1,
            temp_root: Some(dir.join("tmp")),
        };
        (dir, config)
    }

    fn task(id: &str, path: &Path) -> TaskSpec {
        std::fs::write(path, b"").unwrap();
        TaskSpec {
            id: id.into(),
            path: path.to_string_lossy().into_owned(),
            source_root: None,
            info: None,
            number: 0,
        }
    }

    #[test]
    fn outputs_are_committed_or_discarded_and_temp_is_removed() {
        let (dir, config) = setup("commit");
        let tasks = vec![
            task("ok", &dir.join("src/good.mp4")),
            task("bad", &dir.join("src/bad.mp4")),
        ];
        let tool: Arc<dyn Tool> = Arc::new(Fake { fail_on: "bad" });
        let items = plan(tool.as_ref(), &tasks, &BTreeMap::new(), &config.settings).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink = move |e: JobEvent| {
            let _ = lock(&tx).send(e);
        };
        let summary = start(config.clone(), tool, items, Arc::new(sink)).wait();
        assert_eq!((summary.done, summary.failed), (1, 1));
        let events: Vec<JobEvent> = rx.try_iter().collect();
        let done = events.iter().find_map(|e| match e {
            JobEvent::TaskState {
                task_id,
                state: TaskState::Done,
                outputs,
                ..
            } if task_id == "ok" => Some(outputs.clone()),
            _ => None,
        });
        let out = dir.join("out");
        assert_eq!(
            done.unwrap(),
            [
                out.join("good_a.txt").to_string_lossy().into_owned(),
                out.join("good_dir").to_string_lossy().into_owned()
            ]
        );
        assert!(out.join("good_dir").join("f.txt").is_file());
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["good_a.txt", "good_dir"], "失败项的临时输出已删除");
        let temp = dir.join("tmp").join(TEMP_DIR_NAME);
        assert!(
            !temp.join(std::process::id().to_string()).exists(),
            "临时目录已删除"
        );
        assert!(matches!(
            events.last(),
            Some(JobEvent::RunFinished {
                done: 1,
                failed: 1,
                ..
            })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plan_rejects_placeholders_and_empty_inputs() {
        let settings = Settings::default();
        struct Placeholder;
        impl Tool for Placeholder {
            fn id(&self) -> &'static str {
                "tool.placeholder"
            }
            fn implemented(&self) -> bool {
                false
            }
            fn run(&self, _: &mut ToolContext) -> Result<(), ToolError> {
                Err(super::super::NOT_IMPLEMENTED.into())
            }
        }
        let err = plan(&Placeholder, &[], &BTreeMap::new(), &settings).unwrap_err();
        assert!(err.contains("尚未实现"), "{err}");
        let fake = Fake { fail_on: "" };
        assert!(
            plan(&fake, &[], &BTreeMap::new(), &settings).is_err(),
            "没有视频"
        );
    }

    #[test]
    fn stale_temp_dirs_of_other_processes_are_removed() {
        let (dir, _) = setup("stale");
        let base = dir.join(TEMP_DIR_NAME);
        let own = base.join(std::process::id().to_string());
        std::fs::create_dir_all(&own).unwrap();
        cleanup_stale_temp(Some(&dir));
        assert!(own.exists(), "本进程的目录保留");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
