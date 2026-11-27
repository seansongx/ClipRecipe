//! 应用与运行环境。

use serde::Serialize;

use super::blocking;
use crate::ffmpeg::{self, encoders, FfmpegStatus, VideoCodec};
use crate::model::settings::cpu_cores;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    cpu_cores: usize,
    /// 编译目标的操作系统：`windows` / `macos` / `linux`（前端的平台来源，见 src/lib/platform.ts）
    platform: &'static str,
    /// 操作系统名称与版本、CPU 架构（反馈问题时附带的环境信息）
    os: String,
    arch: &'static str,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "ClipRecipe",
        version: env!("CARGO_PKG_VERSION"),
        cpu_cores: cpu_cores(),
        platform: std::env::consts::OS,
        os: crate::system::os_description(),
        arch: std::env::consts::ARCH,
    }
}

/// 检测 FFmpeg / FFprobe 是否可用。
#[tauri::command]
pub async fn ffmpeg_status() -> FfmpegStatus {
    blocking(ffmpeg::status)
        .await
        .unwrap_or_else(FfmpegStatus::unavailable)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    /// 试编码成功的硬件编码器，如 `h264_nvenc`
    hardware: Vec<String>,
    /// 开启 GPU 时实际会用的 H.264 / H.265 编码器
    h264: String,
    hevc: String,
    /// 装了显卡驱动、但硬件编码器试编码全部失败的厂商（如“NVIDIA（NVENC）”），通常是驱动版本过低
    failed_vendors: Vec<String>,
}

/// 探测硬件编码器（结果在进程内缓存；`refresh` 为 true 时重新探测）。
#[tauri::command]
pub async fn encoder_info(refresh: bool) -> Result<EncoderInfo, String> {
    blocking(move || {
        let paths = ffmpeg::locate();
        let caps = if refresh {
            encoders::refresh(&paths.ffmpeg)
        } else {
            encoders::cached(&paths.ffmpeg)
        };
        EncoderInfo::of(&caps)
    })
    .await
}

/// 已经探测过的编码器信息（不触发探测；还没探测过时为 null）。状态栏据此显示实际会用的编码器。
#[tauri::command]
pub async fn encoder_info_cached() -> Result<Option<EncoderInfo>, String> {
    blocking(|| encoders::peek(&ffmpeg::locate().ffmpeg).map(|caps| EncoderInfo::of(&caps))).await
}

impl EncoderInfo {
    fn of(caps: &encoders::EncoderCaps) -> Self {
        EncoderInfo {
            hardware: caps.names(),
            h264: caps.select(VideoCodec::H264, true).name().to_owned(),
            hevc: caps.select(VideoCodec::Hevc, true).name().to_owned(),
            failed_vendors: encoders::failed_vendors(caps, encoders::driver_present)
                .iter()
                .map(|v| v.label().to_owned())
                .collect(),
        }
    }
}
