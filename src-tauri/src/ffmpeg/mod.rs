//! FFmpeg 的定位与调用。
//!
//! - [`locate`]：找到 ffmpeg / ffprobe 可执行文件。
//! - [`probe`]：用 ffprobe 读取媒体信息（[`MediaInfo`]）。
//! - [`run`]：运行 ffmpeg，解析 `-progress` 输出，支持取消。
//! - [`encoders`]：探测可用的硬件编码器并选择视频编码器。

pub mod encoders;
pub mod probe;
pub mod run;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

pub use encoders::{EncoderCaps, HwVendor, VideoCodec, VideoEncoder};
pub use probe::{probe, AudioStream, MediaInfo, SubtitleStream, VideoStream};
pub use run::{
    run, run_with_stall_timeout, run_with_stderr, CancelToken, Progress, RunError,
    DEFAULT_STALL_TIMEOUT,
};

/// 用环境变量指定 ffmpeg 可执行文件位置；ffprobe 会在它旁边查找。
pub const FFMPEG_ENV: &str = "CLIPRECIPE_FFMPEG";

const EXE_SUFFIX: &str = std::env::consts::EXE_SUFFIX;

/// 实际使用的 ffmpeg 与 ffprobe。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// 定位 ffmpeg 与 ffprobe。
///
/// 顺序：
/// 1. 环境变量 `CLIPRECIPE_FFMPEG`（ffprobe 取同目录下的 `ffprobe`，没有则用 PATH 里的）；
/// 2. 本程序可执行文件所在目录（便于随包分发）；
/// 3. PATH。
///
/// 找不到时仍返回命令名 `ffmpeg` / `ffprobe`，启动失败时再报错，方便用户之后补装。
pub fn locate() -> FfmpegPaths {
    let env = std::env::var_os(FFMPEG_ENV).filter(|v| !v.is_empty());
    locate_with(env.as_deref(), current_exe_dir().as_deref())
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

fn locate_with(env_ffmpeg: Option<&OsStr>, exe_dir: Option<&Path>) -> FfmpegPaths {
    if let Some(ffmpeg) = env_ffmpeg {
        let ffmpeg = PathBuf::from(ffmpeg);
        let sibling = ffmpeg
            .parent()
            .map(|dir| dir.join(format!("ffprobe{EXE_SUFFIX}")))
            .filter(|p| p.is_file());
        let ffprobe = sibling
            .or_else(|| which("ffprobe"))
            .unwrap_or_else(|| PathBuf::from("ffprobe"));
        return FfmpegPaths { ffmpeg, ffprobe };
    }
    if let Some(dir) = exe_dir {
        let ffmpeg = dir.join(format!("ffmpeg{EXE_SUFFIX}"));
        let ffprobe = dir.join(format!("ffprobe{EXE_SUFFIX}"));
        if ffmpeg.is_file() && ffprobe.is_file() {
            return FfmpegPaths { ffmpeg, ffprobe };
        }
    }
    FfmpegPaths {
        ffmpeg: which("ffmpeg").unwrap_or_else(|| PathBuf::from("ffmpeg")),
        ffprobe: which("ffprobe").unwrap_or_else(|| PathBuf::from("ffprobe")),
    }
}

/// 在 PATH 中查找可执行文件。
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(format!("{name}{EXE_SUFFIX}")))
        .find(|candidate| candidate.is_file())
}

/// 创建一个不会弹出控制台窗口、标准输入为空的子进程命令。
pub fn command(program: &Path) -> Command {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub available: bool,
    /// 实际使用的 ffmpeg（命令名或完整路径）
    pub path: String,
    /// 实际使用的 ffprobe
    pub ffprobe_path: String,
    /// `ffmpeg -version` 的首行
    pub version: Option<String>,
    pub error: Option<String>,
}

impl FfmpegStatus {
    pub fn unavailable(error: String) -> Self {
        let paths = locate();
        Self {
            available: false,
            path: paths.ffmpeg.to_string_lossy().into_owned(),
            ffprobe_path: paths.ffprobe.to_string_lossy().into_owned(),
            version: None,
            error: Some(error),
        }
    }
}

/// 检测 ffmpeg 与 ffprobe 是否都能启动。
pub fn status() -> FfmpegStatus {
    let paths = locate();
    let mut status = FfmpegStatus {
        available: false,
        path: paths.ffmpeg.to_string_lossy().into_owned(),
        ffprobe_path: paths.ffprobe.to_string_lossy().into_owned(),
        version: None,
        error: None,
    };
    match version_line(&paths.ffmpeg) {
        Ok(line) => status.version = Some(line),
        Err(e) => {
            status.error = Some(format!("ffmpeg：{e}"));
            return status;
        }
    }
    if let Err(e) = version_line(&paths.ffprobe) {
        status.error = Some(format!("ffprobe：{e}"));
        return status;
    }
    status.available = true;
    status
}

fn version_line(program: &Path) -> Result<String, String> {
    let output = command(program)
        .arg("-version")
        .output()
        .map_err(|e| format!("无法启动 {}（{e}）", program.display()))?;
    if !output.status.success() {
        return Err(format!("退出码异常：{}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned())
}

/// 把路径转成命令行参数；FFmpeg 参数统一用 `String` 便于断言与日志。
pub fn path_arg(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("路径包含无法识别的字符：{}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_wins_and_ffprobe_falls_back_to_name() {
        let paths = locate_with(Some(OsStr::new("Z:/nowhere/ffmpeg.exe")), None);
        assert_eq!(paths.ffmpeg, PathBuf::from("Z:/nowhere/ffmpeg.exe"));
        // 同目录没有 ffprobe：要么在 PATH 里找到，要么退回命令名
        assert!(paths.ffprobe.to_string_lossy().contains("ffprobe"));
    }

    #[test]
    fn ffprobe_is_taken_from_ffmpeg_directory() {
        let dir = std::env::temp_dir().join(format!("cr-locate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ffmpeg = dir.join(format!("ffmpeg{EXE_SUFFIX}"));
        let ffprobe = dir.join(format!("ffprobe{EXE_SUFFIX}"));
        std::fs::write(&ffmpeg, b"").unwrap();
        std::fs::write(&ffprobe, b"").unwrap();
        let paths = locate_with(Some(ffmpeg.as_os_str()), None);
        assert_eq!(paths.ffprobe, ffprobe);
        let from_exe_dir = locate_with(None, Some(&dir));
        assert_eq!(from_exe_dir, FfmpegPaths { ffmpeg, ffprobe });
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
