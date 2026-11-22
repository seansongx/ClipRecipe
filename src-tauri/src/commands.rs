//! 暴露给前端的命令。新增命令后记得在 `lib.rs` 的 `generate_handler!` 中注册。

use serde::Serialize;

use crate::ffmpeg::{self, FfmpegStatus};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    cpu_cores: usize,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "ClipRecipe",
        version: env!("CARGO_PKG_VERSION"),
        cpu_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
    }
}

/// 检测 FFmpeg 是否可用。会启动子进程，所以放到阻塞线程里执行。
#[tauri::command]
pub async fn ffmpeg_status() -> FfmpegStatus {
    tauri::async_runtime::spawn_blocking(ffmpeg::status)
        .await
        .unwrap_or_else(|e| FfmpegStatus::unavailable(format!("检测任务异常：{e}")))
}
