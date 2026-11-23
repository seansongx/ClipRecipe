//! 暴露给前端的命令，按领域分文件。
//! 新增命令后在 `lib.rs` 的 `generate_handler!` 中注册，并在 `src/lib/backend.ts` 中封装。
//!
//! 事件：
//! - `job-event`：[`crate::jobs::JobEvent`]
//! - `import-progress`：`{ done, total }`
//! - `preview-progress`：`{ percent }`
//! - `tool-event`：独立工具运行的 [`crate::jobs::JobEvent`]（见 `commands::tools`）
//! - `shutdown-countdown`：`{ remaining }`；`shutdown-failed`：错误信息（见 `commands::system`）
//! - `ffmpeg-download`：应用内下载 FFmpeg 的进度 [`crate::ffmpeg::managed::DownloadProgress`]（见 `commands::ffmpeg`）

pub mod app;
pub mod ffmpeg;
pub mod fonts;
pub mod jobs;
pub mod media;
pub mod preview;
pub mod queue;
pub mod recipe;
pub mod settings;
pub mod system;
pub mod tools;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};

use crate::ffmpeg::CancelToken;
use crate::jobs::RunHandle;
use crate::preview::PreviewDir;

/// Tauri 托管的全局状态。
#[derive(Default)]
pub struct AppState {
    /// 当前（或最近一次）运行
    pub run: Mutex<Option<RunHandle>>,
    /// 正在生成的预览；开始新的预览时取消旧的
    pub preview: Mutex<Option<CancelToken>>,
    /// 预览与取帧的临时目录
    pub preview_dir: PreviewDir,
    /// 当前（或最近一次）独立工具运行（`tool-event` 事件）
    pub tool_run: Mutex<Option<crate::tools::ToolRunHandle>>,
    /// 正在进行的 FFmpeg 下载（`commands::ffmpeg`）
    pub ffmpeg_download: Mutex<Option<CancelToken>>,
}

/// 环境变量：覆盖配置目录（端到端测试用它隔离用户的设置与队列，见 `scripts/e2e-smoke.mjs`）。
pub const CONFIG_DIR_ENV: &str = "CLIPRECIPE_CONFIG_DIR";

/// 应用配置目录（设置、日志等）。设置了 [`CONFIG_DIR_ENV`] 时用它。
pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    app.path()
        .app_config_dir()
        .map_err(|e| format!("无法确定配置目录：{e}"))
}

/// 在阻塞线程中执行，避免卡住异步运行时。
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("后台任务异常：{e}"))
}
