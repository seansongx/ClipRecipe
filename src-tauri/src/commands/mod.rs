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
//! - `open-paths`：再次启动程序时命令行里的文件（路径数组，见 `commands::window`）
//! - `close-requested`：批量处理或工具正在运行时关闭主窗口 / 托盘“退出”。前端显示确认框时调用 `close_prompt_shown`，
//!   确认后调用 `quit_app`，继续处理时调用 `close_prompt_dismissed`；前端没有应答时，10 秒内再次关闭 / 退出就直接退出
//!   （见 `commands::system::should_confirm_quit`）

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
pub mod window;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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

/// 退出时等待工作线程结束的上限。
const STOP_WAIT: Duration = Duration::from_secs(3);
/// 预览没有可等待的线程句柄：给它两个取消轮询周期（`ffmpeg::run` 每 100 毫秒检查一次）结束 ffmpeg。
const PREVIEW_GRACE: Duration = Duration::from_millis(300);

/// 退出程序前停止全部工作（批量处理、独立工具、预览、FFmpeg 下载），并有上限地等待工作线程自己结束并回收 ffmpeg / whisper-cli 子进程
/// （否则进程退出后子进程成为孤儿继续运行，留下半成品输出与临时目录）。
pub fn stop_all_work(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    // 只在取锁的瞬间持有锁，等待时不持有
    if let Ok(run) = state.run.lock() {
        if let Some(run) = run.as_ref() {
            run.stop();
        }
    }
    if let Ok(run) = state.tool_run.lock() {
        if let Some(run) = run.as_ref() {
            run.stop();
        }
    }
    let had_preview = state
        .preview
        .lock()
        .ok()
        .and_then(|mut p| p.take())
        .map(|token| token.cancel())
        .is_some();
    if let Ok(download) = state.ffmpeg_download.lock() {
        if let Some(token) = download.as_ref() {
            token.cancel();
        }
    }

    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        let preview_done = !had_preview || elapsed >= PREVIEW_GRACE;
        if (preview_done && work_finished(&state)) || elapsed >= STOP_WAIT {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 批量处理、独立工具与 FFmpeg 下载都已结束（锁中毒时视为已结束，不阻塞退出）。
fn work_finished(state: &AppState) -> bool {
    let run_done = state
        .run
        .lock()
        .map_or(true, |r| r.as_ref().map_or(true, RunHandle::is_finished));
    let tool_done = state.tool_run.lock().map_or(true, |r| {
        r.as_ref()
            .map_or(true, crate::tools::ToolRunHandle::is_finished)
    });
    let download_done = state.ffmpeg_download.lock().map_or(true, |d| d.is_none());
    run_done && tool_done && download_done
}

/// 在阻塞线程中执行，避免卡住异步运行时。
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("后台任务异常：{e}"))
}
