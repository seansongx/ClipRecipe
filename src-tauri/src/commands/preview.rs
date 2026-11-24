//! 预览与取帧（`pipe.preview`、单模块预览、区域框选），以及让 WebView 播放本地视频。
//!
//! WebView 通过 Tauri 的 asset 协议读取本地文件。配置里的静态范围为空；
//! 启动时只把本进程的预览目录加入范围，源视频在用户要播放时经 [`allow_media_file`] 单独加入。

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::{blocking, AppState};
use crate::ffmpeg::{self, CancelToken, MediaInfo};
use crate::fsutil::{has_extension, VIDEO_EXTENSIONS};
use crate::preview::{self, FrameResult, PreviewResult, PreviewSpec};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewProgress {
    percent: f64,
}

/// 生成预览片段 / 单帧。开始新的预览会取消尚未完成的旧预览。
#[tauri::command]
pub async fn render_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    spec: PreviewSpec,
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
        preview::render_preview(&paths, &spec, &dir, &token, |percent| {
            let _ = app.emit("preview-progress", PreviewProgress { percent });
        })
        .map_err(|e| e.to_string())
    })
    .await?
}

/// 取消正在生成的预览。
#[tauri::command]
pub fn cancel_preview(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(token) = state.preview.lock().map_err(|e| e.to_string())?.take() {
        token.cancel();
    }
    Ok(())
}

/// 取源视频 `time` 秒处的一帧（jpg，位于预览目录）。
#[tauri::command]
pub async fn extract_frame(
    state: State<'_, AppState>,
    path: String,
    info: Option<MediaInfo>,
    time: f64,
) -> Result<FrameResult, String> {
    let dir = state.preview_dir.clone();
    blocking(move || {
        let paths = ffmpeg::locate();
        preview::extract_frame(
            &paths,
            Path::new(&path),
            info.as_ref(),
            time,
            &dir,
            &CancelToken::new(),
        )
        .map_err(|e| e.to_string())
    })
    .await?
}

/// 删除不再需要的预览 / 取帧文件（只接受预览目录中的文件）。
#[tauri::command]
pub async fn release_preview_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let dir = state.preview_dir.clone();
    blocking(move || dir.release(Path::new(&path))).await?
}

/// 允许 WebView 通过 asset 协议读取某个视频文件（双击队列播放源视频时使用）。
/// 只接受存在的、扩展名为常见视频格式的文件。
#[tauri::command]
pub async fn allow_media_file(app: AppHandle, path: String) -> Result<String, String> {
    blocking(move || {
        let file = PathBuf::from(&path);
        if !file.is_file() {
            return Err(format!("文件不存在：{path}"));
        }
        if !has_extension(&file, VIDEO_EXTENSIONS) {
            return Err("只能播放视频文件".into());
        }
        app.asset_protocol_scope()
            .allow_file(&file)
            .map_err(|e| format!("无法访问该文件：{e}"))?;
        Ok(path)
    })
    .await?
}

/// 启动时：清理其它进程残留的预览目录，并把本进程的预览目录加入 asset 协议范围。
pub fn setup(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let dir = &state.preview_dir;
    dir.cleanup_stale(std::time::Duration::from_secs(24 * 3600));
    crate::pipeline::temp::cleanup_stale(crate::pipeline::temp::STALE_AFTER);
    std::fs::create_dir_all(dir.path()).map_err(|e| format!("无法创建预览目录：{e}"))?;
    app.asset_protocol_scope()
        .allow_directory(dir.path(), false)
        .map_err(|e| format!("无法设置预览目录访问权限：{e}"))
}

/// 退出时删除本进程的预览目录。
pub fn cleanup(app: &AppHandle) {
    app.state::<AppState>().preview_dir.remove_all();
}
