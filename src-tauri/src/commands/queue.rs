//! 队列：未完成任务的保存与恢复（`queue.persist`）、同名字幕 / 封面查找（`queue.sidecar`）。

use std::path::{Path, PathBuf};

use tauri::AppHandle;

use super::{blocking, config_dir};
use crate::ffmpeg;
use crate::jobs::persist::{self, RestoredQueue, SavedTask, QUEUE_FILE};
use crate::jobs::sidecar::{find_sidecars as find, SidecarOptions, Sidecars};

/// 保存未完成的任务（空列表删除保存的文件）。
#[tauri::command]
pub async fn save_queue(app: AppHandle, tasks: Vec<SavedTask>) -> Result<(), String> {
    let file = config_dir(&app)?.join(QUEUE_FILE);
    blocking(move || persist::save_queue(&file, &tasks)).await?
}

/// 读取上次保存的任务：丢弃已不存在的视频，缺少媒体信息的重新探测。
#[tauri::command]
pub async fn restore_queue(app: AppHandle) -> Result<RestoredQueue, String> {
    let file = config_dir(&app)?.join(QUEUE_FILE);
    blocking(move || {
        let ffprobe = ffmpeg::locate().ffprobe;
        persist::restore_queue(&file, |p: &Path| ffmpeg::probe(&ffprobe, p))
    })
    .await?
}

/// 查找一批视频的同名字幕 / 封面（队列菜单“全部重载”）。
#[tauri::command]
pub async fn find_sidecars(
    paths: Vec<String>,
    options: SidecarOptions,
) -> Result<Vec<Sidecars>, String> {
    blocking(move || {
        paths
            .iter()
            .map(|p| find(&PathBuf::from(p), options))
            .collect()
    })
    .await
}
