//! 导入与媒体探测。

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::blocking;
use crate::ffmpeg::{self, MediaInfo};
use crate::jobs::import::{self, ImportResult};
use crate::jobs::sidecar::SidecarOptions;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportProgress {
    done: usize,
    total: usize,
}

/// 展开文件 / 文件夹并探测媒体信息；过程中推送 `import-progress` 事件。
/// `sidecars` 指定是否自动关联同名字幕 / 封面（`set.auto-subtitle` / `set.auto-cover`）。
#[tauri::command]
pub async fn import_paths(
    app: AppHandle,
    paths: Vec<String>,
    recursive: bool,
    sidecars: Option<SidecarOptions>,
) -> Result<ImportResult, String> {
    blocking(move || {
        let ffprobe = ffmpeg::locate().ffprobe;
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let options = sidecars.unwrap_or_default();
        import::import_with(&ffprobe, &paths, recursive, options, |done, total| {
            let _ = app.emit("import-progress", ImportProgress { done, total });
        })
    })
    .await
}

#[tauri::command]
pub async fn probe_media(path: String) -> Result<MediaInfo, String> {
    blocking(move || ffmpeg::probe(&ffmpeg::locate().ffprobe, &PathBuf::from(path))).await?
}
