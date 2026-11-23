//! 独立工具（`tool.*`）：开始、停止、取消单项。进度与日志通过 `tool-event` 事件推送
//! （内容同 `job-event` 的 [`JobEvent`]；任务 ID 为工作项 ID，进度的 `round`/`rounds` 为第几步/共几步）。
//! 同一时间只运行一个工具；与批量处理互不影响。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};

use super::{blocking, AppState};
use crate::ffmpeg::{self, encoders, CancelToken, EncoderCaps};
use crate::jobs::{JobEvent, TaskSpec};
use crate::model::rng::random_seed;
use crate::model::Settings;
use crate::pipeline::modules::BitrateParams;
use crate::preview::PreviewResult;
use crate::tools::preview::{render_tool_preview as render_tool_preview_impl, ToolPreviewSpec};
use crate::tools::{self, runner, PlannedItem, ToolRunConfig};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartToolRequest {
    /// 工具 ID，如 `tool.remux`
    tool: String,
    /// 工具参数（前端 features/tools/<工具>.tsx 的参数）；null 为默认
    #[serde(default)]
    params: Value,
    /// 队列中要处理的视频（声明了 videos 输入的工具）
    #[serde(default)]
    videos: Vec<TaskSpec>,
    /// 文件夹输入：键为工具声明的文件夹键
    #[serde(default)]
    folders: BTreeMap<String, String>,
    settings: Settings,
    #[serde(default)]
    output_dir: String,
    /// 流水线的码率设置（`pipe.bitrate`），工具沿用它
    #[serde(default)]
    bitrate: Option<BitrateParams>,
    #[serde(default)]
    seed: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartToolResponse {
    seed: u64,
    /// 规划出的工作项（事件里的任务 ID 即这些 ID）
    items: Vec<PlannedItem>,
}

fn is_running(state: &AppState) -> Result<bool, String> {
    Ok(state
        .tool_run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|r| !r.is_finished()))
}

/// 开始运行一个工具。已有工具在运行时返回错误。
#[tauri::command]
pub async fn start_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartToolRequest,
) -> Result<StartToolResponse, String> {
    if is_running(&state)? {
        return Err("已有工具在运行".into());
    }
    let tool = tools::create(&request.tool, request.params)?;
    let folders: BTreeMap<String, PathBuf> = request
        .folders
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(k, v)| (k, PathBuf::from(v.trim())))
        .collect();
    let settings = request.settings;
    let videos = request.videos;
    let plan_tool = Arc::clone(&tool);
    let plan_settings = settings.clone();
    // 规划与执行用同一个种子（规划阶段的打乱 / 随机配对也可复现）
    let seed = request.seed.unwrap_or_else(random_seed);
    let items = blocking(move || {
        runner::plan_seeded(plan_tool.as_ref(), &videos, &folders, &plan_settings, seed)
    })
    .await??;

    let paths = ffmpeg::locate();
    let gpu_settings = settings.clone();
    let ffmpeg_path = paths.ffmpeg.clone();
    let encoders = blocking(move || {
        if gpu_settings.gpu {
            encoders::cached(&ffmpeg_path)
        } else {
            EncoderCaps::software_only()
        }
    })
    .await?;
    let output_dir = request.output_dir.trim();
    let config = ToolRunConfig {
        paths,
        encoders,
        settings,
        output_dir: (!output_dir.is_empty()).then(|| PathBuf::from(output_dir)),
        bitrate: request.bitrate.unwrap_or_default(),
        seed,
        temp_root: None,
    };
    let summaries = items.iter().map(|i| i.summary()).collect();
    let sink = move |event: JobEvent| {
        let _ = app.emit("tool-event", event);
    };

    let mut run = state.tool_run.lock().map_err(|e| e.to_string())?;
    if run.as_ref().is_some_and(|r| !r.is_finished()) {
        return Err("已有工具在运行".into());
    }
    *run = Some(runner::start(config, tool, items, Arc::new(sink)));
    Ok(StartToolResponse {
        seed,
        items: summaries,
    })
}

/// 停止当前工具运行。
#[tauri::command]
pub fn stop_tool(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(run) = state.tool_run.lock().map_err(|e| e.to_string())?.as_ref() {
        run.stop();
    }
    Ok(())
}

/// 取消一个工作项；返回是否找到。
#[tauri::command]
pub fn cancel_tool_item(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state
        .tool_run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|run| run.cancel_item(&id)))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewProgress {
    percent: f64,
}

/// 工具预览（[`crate::tools::Tool::supports_preview`] 的工具）：第一个选中视频的低清片段经工具处理后
/// 放进预览目录。与流水线预览共用取消令牌（新的预览取消旧的，`cancel_preview` 取消）与进度事件
/// `preview-progress`；结果同 `render_preview`，用完由前端 `release_preview_file` 删除。
#[tauri::command]
pub async fn render_tool_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: ToolPreviewSpec,
) -> Result<PreviewResult, String> {
    let token = CancelToken::new();
    {
        let mut current = state.preview.lock().map_err(|e| e.to_string())?;
        if let Some(old) = current.replace(token.clone()) {
            old.cancel();
        }
    }
    let dir = state.preview_dir.clone();
    blocking(move || {
        let paths = ffmpeg::locate();
        render_tool_preview_impl(&paths, &spec, &dir, &token, |percent| {
            let _ = app.emit("preview-progress", PreviewProgress { percent });
        })
        .map_err(|e| e.to_string())
    })
    .await?
}

/// 是否有工具在运行。
#[tauri::command]
pub fn tool_running(state: State<'_, AppState>) -> Result<bool, String> {
    is_running(&state)
}
