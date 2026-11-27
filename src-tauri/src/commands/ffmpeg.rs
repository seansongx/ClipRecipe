//! 应用内下载 FFmpeg（`ffmpeg::managed`）：开始 / 取消下载、删除已下载的副本、重新检测。
//!
//! 启动时缺少 FFmpeg 由前端自动调用一次（state/ffmpeg.ts 的 autoDownloadIfMissing），其余只在用户点击时调用；下载内容由编译时内嵌的清单决定。进度事件 `ffmpeg-download`（[`DownloadProgress`]）。
//! 下载与删除都在 Rust 中进行，网页本身不联网（CSP 不变），也不需要新增任何 capability。

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::{blocking, AppState};
use crate::ffmpeg::managed::{self, DownloadProgress, InstallError, Installer};
use crate::ffmpeg::{self, encoders, manifest, CancelToken, FfmpegStatus};
use crate::net::Timeouts;

/// 下载的结果。失败时命令返回错误信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOutcome {
    /// 用户取消
    pub cancelled: bool,
    /// 结束后的状态（成功时已改用新下载的副本）
    pub status: FfmpegStatus,
}

/// 新的 FFmpeg 生效：丢弃按路径缓存的编码器探测结果。
fn invalidate_caches() {
    encoders::clear_cache();
}

fn running_work(state: &AppState) -> Result<bool, String> {
    let jobs = state
        .run
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|r| !r.is_finished());
    Ok(jobs || super::tools::is_running(state)?)
}

/// 下载并安装本程序固定版本的 FFmpeg（ffmpeg + ffprobe）。
#[tauri::command]
pub async fn download_ffmpeg(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DownloadOutcome, String> {
    let cancel = {
        let mut slot = state.ffmpeg_download.lock().map_err(|e| e.to_string())?;
        if slot.is_some() {
            return Err("FFmpeg 正在下载".into());
        }
        let token = CancelToken::new();
        *slot = Some(token.clone());
        token
    };
    let result = blocking(move || -> Result<DownloadOutcome, String> {
        let root = managed::root().ok_or("无法确定 FFmpeg 的下载位置")?;
        let (manifest, policy) = manifest::active()?;
        let platform = manifest.current_platform()?;
        let installer = Installer {
            root,
            manifest,
            platform,
            policy,
            timeouts: Timeouts::DOWNLOAD,
            cancel,
        };
        let outcome = installer.install(&mut |p: &DownloadProgress| {
            let _ = app.emit("ffmpeg-download", p);
        });
        invalidate_caches();
        match outcome {
            Ok(_) => Ok(DownloadOutcome {
                cancelled: false,
                status: ffmpeg::status(),
            }),
            Err(InstallError::Cancelled) => Ok(DownloadOutcome {
                cancelled: true,
                status: ffmpeg::status(),
            }),
            Err(InstallError::Failed(e)) => Err(e),
        }
    })
    .await
    .and_then(|r| r);
    if let Ok(mut slot) = state.ffmpeg_download.lock() {
        *slot = None;
    }
    result
}

/// 取消正在进行的下载（网络卡住时最多等到读取超时才真正停止）。
#[tauri::command]
pub fn cancel_ffmpeg_download(state: State<'_, AppState>) -> Result<bool, String> {
    let slot = state.ffmpeg_download.lock().map_err(|e| e.to_string())?;
    Ok(slot.as_ref().map(CancelToken::cancel).is_some())
}

/// 删除应用内下载的 FFmpeg（处理或下载进行中时拒绝）。
#[tauri::command]
pub async fn remove_managed_ffmpeg(state: State<'_, AppState>) -> Result<FfmpegStatus, String> {
    if state
        .ffmpeg_download
        .lock()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("FFmpeg 正在下载，请先取消".into());
    }
    if running_work(&state)? {
        return Err("正在处理视频，请等处理结束后再删除 FFmpeg".into());
    }
    blocking(|| {
        let root = managed::root().ok_or("无法确定 FFmpeg 的下载位置")?;
        let result = managed::remove_all(&root);
        invalidate_caches();
        result.map(|_| ffmpeg::status())
    })
    .await?
}

/// 重新查找 FFmpeg（清除缓存的编码器探测结果）并返回状态。
#[tauri::command]
pub async fn redetect_ffmpeg() -> FfmpegStatus {
    blocking(|| {
        invalidate_caches();
        ffmpeg::status()
    })
    .await
    .unwrap_or_else(FfmpegStatus::unavailable)
}
