//! FFmpeg 的定位与调用。
//!
//! 目前只做可用性检测；后续的媒体探测（ffprobe）、滤镜图生成、
//! 进度解析都放在这个模块下（体量大了再拆成目录）。

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde::Serialize;

/// 用环境变量覆盖 ffmpeg 可执行文件位置。
const FFMPEG_ENV: &str = "CLIPRECIPE_FFMPEG";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    available: bool,
    /// 实际使用的可执行文件（命令名或完整路径）
    path: String,
    /// `ffmpeg -version` 的首行
    version: Option<String>,
    error: Option<String>,
}

impl FfmpegStatus {
    pub fn unavailable(error: String) -> Self {
        Self {
            available: false,
            path: ffmpeg_path().to_string_lossy().into_owned(),
            version: None,
            error: Some(error),
        }
    }
}

/// ffmpeg 可执行文件：优先环境变量，否则依赖 PATH。
pub fn ffmpeg_path() -> PathBuf {
    std::env::var_os(FFMPEG_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

/// 创建一个不会弹出控制台窗口的子进程命令。
pub fn command(program: &PathBuf) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn status() -> FfmpegStatus {
    let path = ffmpeg_path();
    let output = match command(&path).arg("-version").output() {
        Ok(output) => output,
        Err(e) => return FfmpegStatus::unavailable(format!("无法启动 ffmpeg：{e}")),
    };
    if !output.status.success() {
        return FfmpegStatus::unavailable(format!("ffmpeg 退出码异常：{}", output.status));
    }
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::to_owned);
    FfmpegStatus {
        available: true,
        path: path.to_string_lossy().into_owned(),
        version,
        error: None,
    }
}
