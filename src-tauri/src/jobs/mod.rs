//! 任务：导入、输出路径解析、调度执行与事件推送。
//!
//! 队列本身（顺序、选择、媒体信息）由前端维护；点“开始”时前端把要处理的任务
//! 连同配方和设置一起交给 [`runner::start`]，后端按并行数调度，并通过
//! [`EventSink`] 推送 [`JobEvent`]（Tauri 中为 `job-event` 事件）。

pub mod finish;
pub mod import;
pub mod output;
pub mod persist;
pub mod runner;
pub mod sidecar;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::MediaInfo;

pub use runner::{start, start_with, RunConfig, RunExtras, RunHandle, RunSummary};

/// 前端交给后端的一条任务。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSpec {
    /// 前端生成的唯一 ID，事件里原样带回
    pub id: String,
    pub path: String,
    /// 以文件夹方式导入时的那个文件夹
    #[serde(default)]
    pub source_root: Option<String>,
    /// 已探测的媒体信息；没有时运行前再探测
    #[serde(default)]
    pub info: Option<MediaInfo>,
    /// 队列序号（从 1 起）；0 表示按提交顺序
    #[serde(default)]
    pub number: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Waiting,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// 推送给前端的事件，JSON 以 `type` 区分。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobEvent {
    TaskState {
        task_id: String,
        state: TaskState,
        /// 完成时的输出文件（裂变时为最后一个）
        output: Option<String>,
        /// 失败原因
        error: Option<String>,
        /// 全部输出文件（裂变时有多个；失败 / 取消时为已完成的那几个）
        outputs: Vec<String>,
    },
    Progress {
        task_id: String,
        /// 整个任务（含全部裂变轮次）的进度，0～100
        percent: f64,
        /// 当前这一次的已输出时长（秒）
        out_time: f64,
        speed: f64,
        fps: f64,
        /// 当前第几次裂变（从 1 起）
        round: usize,
        /// 裂变总次数（不裂变为 1）
        rounds: usize,
    },
    Log {
        level: LogLevel,
        task_id: Option<String>,
        message: String,
    },
    RunFinished {
        done: usize,
        failed: usize,
        cancelled: usize,
    },
}

/// 事件接收者。Tauri 中转发为窗口事件，测试中收集到内存。
pub trait EventSink: Send + Sync {
    fn emit(&self, event: JobEvent);
}

impl<F: Fn(JobEvent) + Send + Sync> EventSink for F {
    fn emit(&self, event: JobEvent) {
        self(event)
    }
}

/// 队列项关联的附加文件（`queue.sidecar`）：烧录的字幕、嵌入的封面。
/// 不放进 [`TaskSpec`]（独立工具也使用它），开始处理时按任务 ID 另外传入（[`RunExtras::attachments`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskAttachments {
    pub subtitle: Option<String>,
    pub cover: Option<String>,
}

impl TaskAttachments {
    pub fn is_empty(&self) -> bool {
        self.subtitle.is_none() && self.cover.is_none()
    }
}
