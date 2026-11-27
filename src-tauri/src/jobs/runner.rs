//! 调度器：按并行数运行任务，推送状态 / 进度 / 日志，支持整体停止、单个取消与插队。
//! 硬件编码失败时自动改用软件编码重试一次。
//!
//! 模块在构建时登记的临时文件（`BuildContext::temp_file`）由 `BuiltCommand::temp_files` 持有，
//! 每轮结束（成功、失败、取消）时随构建结果一起删除；硬件编码重试时旧构建的临时文件在重建后删除。
//!
//! 裂变（`pipe.fission`）：配方的裂变次数为 N 时，每个任务依次生成 N 个输出，
//! 第 r 次使用 [`fission_seed`]`(任务种子, r)`，输出路径见 [`plan_output`]。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use serde::Serialize;

use super::finish::{default_dispose, finish_task};
use super::output::{path_key, plan_output, BatchTime, FissionSlot, OutputPlan, OutputRequest};
use super::{EventSink, JobEvent, LogLevel, TaskAttachments, TaskSpec, TaskState};
use crate::ffmpeg::{self, CancelToken, EncoderCaps, FfmpegPaths, MediaInfo, RunError};
use crate::model::rng::{fission_seed, mix_seed};
use crate::model::{Recipe, Settings};
use crate::pipeline::{self, BuildRequest, BuiltCommand, Sidecar};

/// 一次运行的配置。
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub paths: FfmpegPaths,
    pub encoders: EncoderCaps,
    pub recipe: Recipe,
    pub settings: Settings,
    /// None 表示输出到源文件所在目录
    pub output_dir: Option<PathBuf>,
    /// 基础种子；每个任务用 基础种子 + 队列序号 派生
    pub seed: u64,
}

/// 删除源文件的函数（替换回收站 / 永久删除，测试用）。
pub type DisposeFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// 一次运行的附加信息（不在 [`RunConfig`] 里，以免改动所有构造它的地方）。
#[derive(Default, Clone)]
pub struct RunExtras {
    /// 按任务 ID 关联的字幕 / 封面
    pub attachments: HashMap<String, TaskAttachments>,
    /// 替换“删除源视频”的实际操作（None 时按设置移入回收站或永久删除）
    pub dispose: Option<DisposeFn>,
    /// 本批开始处理的本地时间（输出文件名格式的 `{date}` / `{time}`）；None 时取开始时的 UTC 时间
    pub started_at: Option<BatchTime>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub done: usize,
    pub failed: usize,
    pub cancelled: usize,
}

struct Shared {
    config: RunConfig,
    extras: RunExtras,
    sink: Arc<dyn EventSink>,
    total: usize,
    /// 本批开始处理的时间，同一批的所有任务相同
    started: BatchTime,
    pending: Mutex<VecDeque<TaskSpec>>,
    running: Mutex<HashMap<String, CancelToken>>,
    reserved: Mutex<HashSet<String>>,
    summary: Mutex<RunSummary>,
    stop: CancelToken,
    warned_same_dir: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn emit(&self, event: JobEvent) {
        self.sink.emit(event);
    }

    fn log(&self, level: LogLevel, task_id: Option<&str>, message: impl Into<String>) {
        self.emit(JobEvent::Log {
            level,
            task_id: task_id.map(str::to_owned),
            message: message.into(),
        });
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

/// 正在进行的一次运行。
pub struct RunHandle {
    shared: Arc<Shared>,
    supervisor: Option<JoinHandle<RunSummary>>,
}

impl RunHandle {
    /// 停止：取消正在运行的任务，尚未开始的标记为已取消。
    pub fn stop(&self) {
        self.shared.stop.cancel();
        for token in lock(&self.shared.running).values() {
            token.cancel();
        }
    }

    /// 取消单个任务（等待中或运行中）。返回是否找到该任务。
    pub fn cancel_task(&self, id: &str) -> bool {
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

    /// 插队：把一个等待中的任务移到最前面。返回是否找到该任务。
    pub fn prioritize(&self, id: &str) -> bool {
        let mut pending = lock(&self.shared.pending);
        match pending.iter().position(|t| t.id == id) {
            Some(pos) => {
                if let Some(task) = pending.remove(pos) {
                    pending.push_front(task);
                }
                true
            }
            None => false,
        }
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

/// 开始运行一批任务。立即返回；所有任务先被标记为“等待中”。
pub fn start(config: RunConfig, tasks: Vec<TaskSpec>, sink: Arc<dyn EventSink>) -> RunHandle {
    start_with(config, tasks, RunExtras::default(), sink)
}

/// 同 [`start`]，带附加信息（关联的字幕 / 封面等）。
pub fn start_with(
    config: RunConfig,
    tasks: Vec<TaskSpec>,
    extras: RunExtras,
    sink: Arc<dyn EventSink>,
) -> RunHandle {
    // 本进程构建时登记的临时文件随每轮任务结束删除；这里只清理异常退出的其它进程的残留
    pipeline::temp::cleanup_stale(pipeline::temp::STALE_AFTER);
    let total = tasks.len();
    let workers = config.settings.effective_parallel().min(total.max(1));
    let tasks: VecDeque<TaskSpec> = tasks
        .into_iter()
        .enumerate()
        .map(|(i, mut t)| {
            if t.number == 0 {
                t.number = i + 1;
            }
            t
        })
        .collect();
    let started = extras.started_at.unwrap_or_else(BatchTime::now_utc);
    let shared = Arc::new(Shared {
        config,
        extras,
        sink,
        total,
        started,
        pending: Mutex::new(VecDeque::new()),
        running: Mutex::default(),
        reserved: Mutex::default(),
        summary: Mutex::default(),
        stop: CancelToken::new(),
        warned_same_dir: AtomicBool::new(false),
    });
    for task in &tasks {
        shared.emit(JobEvent::TaskState {
            task_id: task.id.clone(),
            state: TaskState::Waiting,
            output: None,
            error: None,
            outputs: Vec::new(),
        });
    }
    *lock(&shared.pending) = tasks;
    shared.log(
        LogLevel::Info,
        None,
        format!("开始处理 {total} 个任务，并行 {workers} 个"),
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
        // 停止后剩下的任务
        let leftover: Vec<TaskSpec> = lock(&shared.pending).drain(..).collect();
        for task in leftover {
            shared.set_state(&task.id, TaskState::Cancelled, Vec::new(), None);
        }
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

    RunHandle {
        shared,
        supervisor: Some(supervisor),
    }
}

fn worker(shared: &Shared) {
    loop {
        if shared.stop.is_cancelled() {
            return;
        }
        let Some(task) = lock(&shared.pending).pop_front() else {
            return;
        };
        let token = CancelToken::new();
        lock(&shared.running).insert(task.id.clone(), token.clone());
        // 先登记再检查，避免与 stop() 竞争时漏掉取消
        if shared.stop.is_cancelled() {
            token.cancel();
        }
        shared.set_state(&task.id, TaskState::Running, Vec::new(), None);
        let mut outputs = Vec::new();
        let outcome = run_task(shared, &task, &token, &mut outputs);
        lock(&shared.running).remove(&task.id);
        match outcome {
            Ok(()) => {
                finish(shared, &task, &mut outputs);
                let message = match outputs.as_slice() {
                    [single] => format!("完成：{single}"),
                    many => format!("完成：{} 个输出（裂变）\n{}", many.len(), many.join("\n")),
                };
                shared.log(LogLevel::Info, Some(&task.id), message);
                shared.set_state(&task.id, TaskState::Done, outputs, None);
            }
            Err(TaskError::Cancelled) => {
                shared.log(
                    LogLevel::Warn,
                    Some(&task.id),
                    format!("已取消：{}", task.path),
                );
                shared.set_state(&task.id, TaskState::Cancelled, outputs, None);
            }
            Err(TaskError::Failed(message)) => {
                shared.log(
                    LogLevel::Error,
                    Some(&task.id),
                    format!("失败：{} —— {message}", task.path),
                );
                shared.set_state(&task.id, TaskState::Failed, outputs, Some(message));
            }
        }
    }
}

/// 成功后的收尾：输出移回源位置、删除源视频（见 [`super::finish`]）。
fn finish(shared: &Shared, task: &TaskSpec, outputs: &mut Vec<String>) {
    let settings = &shared.config.settings;
    if !settings.move_back && !settings.delete_source {
        return;
    }
    let to_trash = settings.delete_to_trash;
    let fallback = move |p: &Path| default_dispose(p, to_trash);
    let dispose: &dyn Fn(&Path) -> Result<(), String> = match &shared.extras.dispose {
        Some(f) => f.as_ref(),
        None => &fallback,
    };
    let report = finish_task(Path::new(&task.path), outputs, settings, dispose);
    for (level, message) in report.logs {
        shared.log(level, Some(&task.id), message);
    }
    *outputs = report.outputs;
}

enum TaskError {
    Cancelled,
    Failed(String),
}

impl From<String> for TaskError {
    fn from(message: String) -> Self {
        TaskError::Failed(message)
    }
}

/// 释放输出路径占用的守卫。
struct Reservation<'a> {
    shared: &'a Shared,
    key: String,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        lock(&self.shared.reserved).remove(&self.key);
    }
}

/// 临时输出文件：同目录下以点开头，成功后改名为最终文件；失败 / 取消时删除。
fn temp_path(plan: &OutputPlan, number: usize) -> PathBuf {
    let dir = plan.path.parent().unwrap_or(Path::new(""));
    let stem = plan
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = plan
        .path
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mp4".into());
    dir.join(format!(".{stem}.cliprecipe-{number}.part.{ext}"))
}

/// 一个任务在各轮裂变之间共享的信息。
struct TaskRun<'a> {
    shared: &'a Shared,
    task: &'a TaskSpec,
    token: &'a CancelToken,
    input: PathBuf,
    info: MediaInfo,
    /// 硬件编码失败过一次后，后续轮次直接用软件编码
    force_software: bool,
}

/// 运行一个任务的全部裂变轮次；每完成一轮就把输出加入 `outputs`。
fn run_task(
    shared: &Shared,
    task: &TaskSpec,
    token: &CancelToken,
    outputs: &mut Vec<String>,
) -> Result<(), TaskError> {
    let config = &shared.config;
    let input = PathBuf::from(&task.path);
    if !input.is_file() {
        return Err(format!("源文件不存在：{}", task.path).into());
    }
    let info: MediaInfo = match &task.info {
        Some(info) => info.clone(),
        None => ffmpeg::probe(&config.paths.ffprobe, &input)?,
    };
    let mut run = TaskRun {
        shared,
        task,
        token,
        input,
        info,
        force_software: false,
    };

    let rounds = config.recipe.fission_count();
    let task_seed = mix_seed(config.seed, task.number as u64);
    for round in 1..=rounds {
        if token.is_cancelled() {
            return Err(TaskError::Cancelled);
        }
        let slot = FissionSlot {
            round,
            total: rounds,
        };
        let output = run
            .run_round(slot, fission_seed(task_seed, round))
            .map_err(|e| match e {
                TaskError::Failed(message) if slot.is_fission() => {
                    TaskError::Failed(format!("第 {round}/{rounds} 次裂变：{message}"))
                }
                other => other,
            })?;
        if slot.is_fission() {
            shared.log(
                LogLevel::Info,
                Some(&task.id),
                format!("第 {round}/{rounds} 次裂变完成：{output}"),
            );
        }
        outputs.push(output);
    }
    Ok(())
}

impl TaskRun<'_> {
    /// 运行一轮：规划输出路径、构建命令、执行（必要时软件编码重试）、改名为最终文件。
    fn run_round(&mut self, slot: FissionSlot, seed: u64) -> Result<String, TaskError> {
        let shared = self.shared;
        let task = self.task;
        let config = &shared.config;

        // 规划输出路径并占用
        let source_root = task.source_root.as_ref().map(PathBuf::from);
        let (plan, _reservation) = {
            let mut reserved = lock(&shared.reserved);
            let plan = plan_output(
                &OutputRequest {
                    input: &self.input,
                    source_root: source_root.as_deref(),
                    output_dir: config.output_dir.as_deref(),
                    settings: &config.settings,
                    number: task.number,
                    total: shared.total,
                    fission: slot,
                    started: shared.started,
                },
                Path::exists,
                |p| reserved.contains(&path_key(p)),
            )?;
            let key = path_key(&plan.path);
            reserved.insert(key.clone());
            (plan, Reservation { shared, key })
        };
        if plan.same_dir_as_source && !shared.warned_same_dir.swap(true, Ordering::SeqCst) {
            shared.log(
                LogLevel::Warn,
                None,
                "输出目录与源文件目录相同，输出文件会和源文件放在一起",
            );
        }
        if let Some(dir) = plan.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("无法创建输出目录 {}：{e}", dir.display()))?;
        }
        let temp = temp_path(&plan, task.number);

        let probe = |p: &Path| ffmpeg::probe(&config.paths.ffprobe, p);
        let attachments = shared.extras.attachments.get(&task.id);
        let subtitle = attachments
            .and_then(|a| a.subtitle.as_deref())
            .map(Path::new);
        let cover = attachments.and_then(|a| a.cover.as_deref()).map(Path::new);
        let build = |force_software: bool| -> Result<BuiltCommand, String> {
            pipeline::build(&BuildRequest {
                input: &self.input,
                output: &temp,
                info: &self.info,
                recipe: &config.recipe,
                settings: &config.settings,
                encoders: &config.encoders,
                seed,
                task_index: task.number.saturating_sub(1),
                force_software,
                probe: Some(&probe),
                sidecar: Sidecar { subtitle, cover },
            })
        };

        let mut built = build(self.force_software)?;
        for warning in &built.warnings {
            shared.log(LogLevel::Warn, Some(&task.id), warning.clone());
        }
        let mut result = execute(shared, task, self.token, &built, slot);
        if built.encoder.is_hardware() && matches!(result, Err(RunError::Failed { .. })) {
            shared.log(
                LogLevel::Warn,
                Some(&task.id),
                format!("硬件编码（{}）失败，改用软件编码重试", built.encoder.name()),
            );
            self.force_software = true;
            built = build(true)?;
            result = execute(shared, task, self.token, &built, slot);
        }

        match result {
            Ok(_) => {
                std::fs::rename(&temp, &plan.path).map_err(|e| {
                    let _ = std::fs::remove_file(&temp);
                    format!("无法写入输出文件 {}：{e}", plan.path.display())
                })?;
                shared.emit(JobEvent::Progress {
                    task_id: task.id.clone(),
                    percent: overall_percent(slot, 100.0),
                    out_time: built.expected_duration,
                    speed: 0.0,
                    fps: 0.0,
                    round: slot.round,
                    rounds: slot.total,
                });
                Ok(plan.path.to_string_lossy().into_owned())
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                match err {
                    RunError::Cancelled => Err(TaskError::Cancelled),
                    RunError::Failed {
                        ref stderr_tail, ..
                    }
                    | RunError::Stalled {
                        ref stderr_tail, ..
                    } => {
                        let tail = stderr_tail
                            .iter()
                            .rev()
                            .take(10)
                            .rev()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !tail.is_empty() {
                            shared.log(
                                LogLevel::Error,
                                Some(&task.id),
                                format!("ffmpeg 输出：\n{tail}"),
                            );
                        }
                        Err(TaskError::Failed(err.to_string()))
                    }
                    RunError::Spawn(_) => Err(TaskError::Failed(err.to_string())),
                }
            }
        }
    }
}

/// 把某一轮的进度换算成整个任务的进度。
fn overall_percent(slot: FissionSlot, round_percent: f64) -> f64 {
    let total = slot.total.max(1) as f64;
    ((slot.round.saturating_sub(1) as f64 * 100.0 + round_percent) / total).clamp(0.0, 100.0)
}

fn execute(
    shared: &Shared,
    task: &TaskSpec,
    token: &CancelToken,
    built: &BuiltCommand,
    slot: FissionSlot,
) -> Result<Vec<String>, RunError> {
    let ffmpeg_path = &shared.config.paths.ffmpeg;
    let round = if slot.is_fission() {
        format!("（第 {}/{} 次裂变）", slot.round, slot.total)
    } else {
        String::new()
    };
    shared.log(
        LogLevel::Info,
        Some(&task.id),
        format!(
            "开始：{}{round}（编码器 {}）",
            task.path,
            built.encoder.name()
        ),
    );
    shared.log(
        LogLevel::Debug,
        Some(&task.id),
        built.command_line(&ffmpeg_path.to_string_lossy()),
    );
    let expected = built.expected_duration;
    ffmpeg::run(ffmpeg_path, &built.args, token, |p| {
        let percent = if expected > 0.0 {
            (p.out_time / expected * 100.0).clamp(0.0, 99.9)
        } else {
            0.0
        };
        shared.emit(JobEvent::Progress {
            task_id: task.id.clone(),
            percent: overall_percent(slot, percent).min(99.9),
            out_time: p.out_time,
            speed: p.speed,
            fps: p.fps,
            round: slot.round,
            rounds: slot.total,
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overall_percent_spans_all_rounds() {
        let slot = |round| FissionSlot { round, total: 4 };
        assert_eq!(overall_percent(slot(1), 0.0), 0.0);
        assert_eq!(overall_percent(slot(1), 100.0), 25.0);
        assert_eq!(overall_percent(slot(3), 50.0), 62.5);
        assert_eq!(overall_percent(slot(4), 100.0), 100.0);
        assert_eq!(overall_percent(FissionSlot::SINGLE, 42.0), 42.0);
    }
}
