//! 任务执行：开始、停止、取消、命令预览、输出目录检查。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::{blocking, AppState};
use crate::ffmpeg::{self, encoders, EncoderCaps, MediaInfo};
use crate::jobs::output::{plan_output, same_path, FissionSlot, OutputRequest};
use crate::jobs::{self, JobEvent, RunConfig, RunExtras, TaskAttachments, TaskSpec};
use crate::model::rng::{fission_seed, mix_seed, random_seed};
use crate::model::{Recipe, Settings};
use crate::pipeline::{self, BuildRequest, Sidecar};

fn output_dir(dir: &str) -> Option<PathBuf> {
    let dir = dir.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

fn encoder_caps(settings: &Settings, ffmpeg_path: &Path) -> EncoderCaps {
    if settings.gpu {
        encoders::cached(ffmpeg_path)
    } else {
        EncoderCaps::software_only()
    }
}

/// 前端提交的一条任务：[`TaskSpec`] 加上关联的字幕 / 封面。
#[derive(Deserialize)]
pub struct StartTask {
    #[serde(flatten)]
    spec: TaskSpec,
    #[serde(flatten)]
    attachments: TaskAttachments,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    tasks: Vec<StartTask>,
    recipe: Recipe,
    settings: Settings,
    #[serde(default)]
    output_dir: String,
    /// 不指定时随机生成；返回值里带回实际使用的种子
    #[serde(default)]
    seed: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    seed: u64,
}

/// 开始处理。已有运行未结束时返回错误。
#[tauri::command]
pub async fn start_jobs(
    app: AppHandle,
    state: State<'_, AppState>,
    request: StartRequest,
) -> Result<StartResponse, String> {
    {
        let run = state.run.lock().map_err(|e| e.to_string())?;
        if run.as_ref().is_some_and(|r| !r.is_finished()) {
            return Err("已有任务在处理中".into());
        }
    }
    if request.tasks.is_empty() {
        return Err("没有需要处理的任务".into());
    }
    let paths = ffmpeg::locate();
    let settings = request.settings.clone();
    let ffmpeg_path = paths.ffmpeg.clone();
    let encoders = blocking(move || encoder_caps(&settings, &ffmpeg_path)).await?;
    let seed = request.seed.unwrap_or_else(random_seed);
    let config = RunConfig {
        paths,
        encoders,
        recipe: request.recipe,
        settings: request.settings,
        output_dir: output_dir(&request.output_dir),
        seed,
    };
    let sink = move |event: JobEvent| {
        let _ = app.emit("job-event", event);
    };
    let mut extras = RunExtras::default();
    let mut tasks = Vec::with_capacity(request.tasks.len());
    for task in request.tasks {
        if !task.attachments.is_empty() {
            extras
                .attachments
                .insert(task.spec.id.clone(), task.attachments);
        }
        tasks.push(task.spec);
    }

    let mut run = state.run.lock().map_err(|e| e.to_string())?;
    if run.as_ref().is_some_and(|r| !r.is_finished()) {
        return Err("已有任务在处理中".into());
    }
    *run = Some(jobs::start_with(config, tasks, extras, Arc::new(sink)));
    Ok(StartResponse { seed })
}

/// 停止全部任务。
#[tauri::command]
pub fn stop_jobs(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(run) = state.run.lock().map_err(|e| e.to_string())?.as_ref() {
        run.stop();
    }
    Ok(())
}

/// 取消单个任务；返回是否找到。
#[tauri::command]
pub fn cancel_task(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state
        .run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|run| run.cancel_task(&id)))
}

/// 插队：把等待中的任务移到最前面；返回是否找到。
#[tauri::command]
pub fn prioritize_task(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state
        .run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|run| run.prioritize(&id)))
}

#[tauri::command]
pub fn jobs_running(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state
        .run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|r| !r.is_finished()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    path: String,
    #[serde(default)]
    info: Option<MediaInfo>,
    recipe: Recipe,
    settings: Settings,
    #[serde(default)]
    output_dir: String,
    #[serde(default)]
    seed: Option<u64>,
    /// 队列序号（从 1 起）
    #[serde(default)]
    number: usize,
    /// 关联的字幕 / 封面
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    cover: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    command_line: String,
    args: Vec<String>,
    output: String,
    encoder: String,
    warnings: Vec<String>,
}

/// 生成某个视频将要执行的 ffmpeg 命令（不执行），用于检查与排错。
#[tauri::command]
pub async fn preview_command(request: PreviewRequest) -> Result<PreviewResponse, String> {
    blocking(move || {
        let paths = ffmpeg::locate();
        let input = PathBuf::from(&request.path);
        let info = match request.info {
            Some(info) => info,
            None => ffmpeg::probe(&paths.ffprobe, &input)?,
        };
        let number = request.number.max(1);
        let out_dir = output_dir(&request.output_dir);
        let plan = plan_output(
            &OutputRequest {
                input: &input,
                source_root: None,
                output_dir: out_dir.as_deref(),
                settings: &request.settings,
                number,
                total: number,
                fission: FissionSlot {
                    round: 1,
                    total: request.recipe.fission_count(),
                },
            },
            Path::exists,
            |_| false,
        )?;
        let encoders = encoder_caps(&request.settings, &paths.ffmpeg);
        let seed = mix_seed(request.seed.unwrap_or(0), number as u64);
        let probe = |p: &Path| ffmpeg::probe(&paths.ffprobe, p);
        let built = pipeline::build(&BuildRequest {
            input: &input,
            output: &plan.path,
            info: &info,
            recipe: &request.recipe,
            settings: &request.settings,
            encoders: &encoders,
            seed: fission_seed(seed, 1),
            task_index: number - 1,
            force_software: false,
            probe: Some(&probe),
            sidecar: Sidecar {
                subtitle: request.subtitle.as_deref().map(Path::new),
                cover: request.cover.as_deref().map(Path::new),
            },
        })?;
        Ok(PreviewResponse {
            command_line: built.command_line(&paths.ffmpeg.to_string_lossy()),
            encoder: built.encoder.name().to_owned(),
            args: built.args,
            output: plan.path.to_string_lossy().into_owned(),
            warnings: built.warnings,
        })
    })
    .await?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDirCheck {
    exists: bool,
    /// 与其中某个源文件所在目录相同
    same_as_source: bool,
}

/// 检查输出目录：是否存在、是否与源文件目录相同（界面据此给出警告）。
#[tauri::command]
pub async fn check_output_dir(
    output_dir: String,
    sources: Vec<String>,
) -> Result<OutputDirCheck, String> {
    blocking(move || {
        let dir = PathBuf::from(output_dir.trim());
        OutputDirCheck {
            exists: dir.is_dir(),
            same_as_source: sources
                .iter()
                .filter_map(|s| Path::new(s).parent())
                .any(|parent| same_path(parent, &dir)),
        }
    })
    .await
}
