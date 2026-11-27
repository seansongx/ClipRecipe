//! 工具预览：把第一个选中视频的一小段（低清）交给工具自己的 [`crate::tools::Tool::run`] 处理，
//! 结果放进预览目录（[`PreviewDir`]），在与流水线预览相同的预览窗口里播放。
//!
//! - 只有声明了 [`crate::tools::Tool::supports_preview`] 的工具可以预览（一进一出、处理队列视频的视频效果类工具）；
//! - 步骤：先把源视频的 `[start, start + duration)` 截成一个低清片段（libx264 ultrafast，高度不超过
//!   `max_height`，进度 0～20%），再用一个工作项运行工具（进度 20～100%），取它的第一个视频输出；
//! - 工具看到的输入就是这个片段：按时间点写的参数（如“第 12 秒”）超出片段时长时按工具自己的规则报错或忽略，
//!   百分比、按间隔的参数正常；
//! - 临时文件（片段、工具的临时目录与输出目录）放在 `<系统临时目录>/cliprecipe-tools/<进程号>/preview-<n>/`，
//!   成功、失败、取消都会删除；取消令牌被触发时停止工具运行。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::ffmpeg::{self, CancelToken, EncoderCaps, FfmpegPaths, MediaInfo};
use crate::fsutil;
use crate::jobs::{JobEvent, LogLevel, TaskState};
use crate::model::Settings;
use crate::pipeline::modules::BitrateParams;
use crate::preview::{PreviewDir, PreviewError, PreviewResult, DEFAULT_PREVIEW_HEIGHT};

use super::context::head_args;
use super::media::secs;
use super::plan::{ItemInput, WorkItem};
use super::runner::{self, ToolRunConfig, TEMP_DIR_NAME};

/// 预览片段的默认时长（秒）。
pub const DEFAULT_PREVIEW_SECONDS: f64 = 5.0;
/// 截取片段占总进度的比例（%）。
const CLIP_SHARE: f64 = 20.0;

/// 一次工具预览请求。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPreviewSpec {
    /// 工具 ID
    pub tool: String,
    /// 工具参数；null 为默认
    #[serde(default)]
    pub params: Value,
    /// 源视频
    pub path: String,
    #[serde(default)]
    pub info: Option<MediaInfo>,
    pub settings: Settings,
    /// 流水线的码率设置（`pipe.bitrate`），与正式运行相同
    #[serde(default)]
    pub bitrate: Option<BitrateParams>,
    /// 片段在源视频中的起点（秒）
    #[serde(default)]
    pub start: f64,
    /// 片段时长（秒）；0 为 [`DEFAULT_PREVIEW_SECONDS`]
    #[serde(default)]
    pub duration: f64,
    /// 画面最大高度；0 为默认值
    #[serde(default)]
    pub max_height: u32,
    #[serde(default)]
    pub seed: Option<u64>,
    /// 队列序号（从 1 起）
    #[serde(default)]
    pub number: usize,
    /// 临时目录的上级目录；None 为系统临时目录（测试用）
    #[serde(skip)]
    pub temp_root: Option<PathBuf>,
}

/// 预览的工作目录，释放时删除。
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn work_dir(root: Option<&Path>) -> Result<WorkDir, String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = root
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join(TEMP_DIR_NAME)
        .join(std::process::id().to_string())
        .join(format!("preview-{n}"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("无法创建临时目录 {}：{e}", dir.display()))?;
    Ok(WorkDir(dir))
}

/// 截取低清片段的参数（不含开头的通用参数）。
pub fn clip_args(
    input: &str,
    start: f64,
    duration: f64,
    max_height: u32,
    output: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    if start > 0.0 {
        args.extend(["-ss".to_owned(), secs(start)]);
    }
    args.extend([
        "-i".to_owned(),
        input.to_owned(),
        "-t".to_owned(),
        secs(duration),
        "-map".to_owned(),
        "0:v:0".to_owned(),
        "-map".to_owned(),
        "0:a:0?".to_owned(),
        "-vf".to_owned(),
        format!("scale=-2:'2*trunc(min(ih,{max_height})/2)',format=yuv420p"),
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-preset".to_owned(),
        "ultrafast".to_owned(),
        "-crf".to_owned(),
        "18".to_owned(),
        "-c:a".to_owned(),
        "aac".to_owned(),
        "-b:a".to_owned(),
        "192k".to_owned(),
        "-sn".to_owned(),
        "-dn".to_owned(),
        output.to_owned(),
    ]);
    args
}

fn is_video_file(path: &Path) -> bool {
    fsutil::has_extension(path, fsutil::VIDEO_EXTENSIONS)
}

/// 生成工具预览。`on_progress` 收到 0～100 的进度。
pub fn render_tool_preview(
    paths: &FfmpegPaths,
    spec: &ToolPreviewSpec,
    dir: &PreviewDir,
    token: &CancelToken,
    mut on_progress: impl FnMut(f64),
) -> Result<PreviewResult, PreviewError> {
    let tool = super::create(&spec.tool, spec.params.clone())?;
    if !tool.implemented() {
        return Err(format!("{}：{}", spec.tool, super::NOT_IMPLEMENTED).into());
    }
    if !tool.supports_preview() {
        return Err(format!("{} 不支持预览", spec.tool).into());
    }
    let input = PathBuf::from(&spec.path);
    if !input.is_file() {
        return Err(format!("源文件不存在：{}", spec.path).into());
    }
    let info = match &spec.info {
        Some(info) => info.clone(),
        None => ffmpeg::probe(&paths.ffprobe, &input)?,
    };
    if info.video.is_none() {
        return Err("文件中没有视频流，无法预览".to_owned().into());
    }
    let wanted = if spec.duration > 0.0 {
        spec.duration
    } else {
        DEFAULT_PREVIEW_SECONDS
    };
    let start = if info.duration > 0.0 {
        spec.start.clamp(0.0, (info.duration - 0.5).max(0.0))
    } else {
        spec.start.max(0.0)
    };
    let duration = if info.duration > 0.0 {
        wanted.min(info.duration - start)
    } else {
        wanted
    };
    if duration < 0.2 {
        return Err("预览片段太短".to_owned().into());
    }
    let max_height = if spec.max_height == 0 {
        DEFAULT_PREVIEW_HEIGHT
    } else {
        spec.max_height
    };

    let work = work_dir(spec.temp_root.as_deref())?;
    // 第 1 步：截取低清片段
    let name = input.file_stem().map_or_else(
        || "preview".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    let clip_dir = work.0.join("src");
    std::fs::create_dir_all(&clip_dir).map_err(|e| format!("无法创建临时目录：{e}"))?;
    let clip = clip_dir.join(format!("{name}.mp4"));
    let mut args = head_args("error");
    args.extend(clip_args(
        &ffmpeg::path_arg(&input)?,
        start,
        duration,
        max_height,
        &ffmpeg::path_arg(&clip)?,
    ));
    ffmpeg::run(&paths.ffmpeg, &args, token, |p| {
        on_progress((p.out_time / duration * CLIP_SHARE).clamp(0.0, CLIP_SHARE));
    })?;
    if !clip.is_file() {
        return Err("无法截取预览片段".to_owned().into());
    }

    // 第 2 步：用工具自己的运行逻辑处理片段
    let seed = spec.seed.unwrap_or_else(crate::model::rng::random_seed);
    let config = ToolRunConfig {
        paths: paths.clone(),
        encoders: EncoderCaps::software_only(),
        settings: spec.settings.clone(),
        output_dir: Some(work.0.join("out")),
        bitrate: spec.bitrate.clone().unwrap_or_default(),
        seed,
        temp_root: Some(work.0.join("tmp")),
    };
    let item = WorkItem::new(
        "preview",
        format!("{name}（预览）"),
        spec.number.max(1),
        vec![ItemInput::file(&clip)],
    );
    let (tx, rx) = mpsc::channel::<JobEvent>();
    let tx = Mutex::new(tx);
    let sink = move |event: JobEvent| {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(event);
        }
    };
    let handle = runner::start(config, tool, vec![item], Arc::new(sink));
    let mut warnings = Vec::new();
    let mut outcome: Option<(TaskState, Vec<String>, Option<String>)> = None;
    loop {
        if token.is_cancelled() {
            handle.stop();
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(JobEvent::Progress { percent, .. }) => {
                on_progress(CLIP_SHARE + percent.clamp(0.0, 100.0) * (100.0 - CLIP_SHARE) / 100.0);
            }
            Ok(JobEvent::Log {
                level: LogLevel::Warn,
                message,
                ..
            }) => warnings.push(message),
            Ok(JobEvent::TaskState {
                state,
                outputs,
                error,
                ..
            }) if matches!(
                state,
                TaskState::Done | TaskState::Failed | TaskState::Cancelled
            ) =>
            {
                outcome = Some((state, outputs, error));
            }
            Ok(JobEvent::RunFinished { .. }) => break,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if handle.is_finished() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.wait();
    if token.is_cancelled() {
        return Err(PreviewError::Cancelled);
    }
    let outputs = match outcome {
        Some((TaskState::Done, outputs, _)) => outputs,
        Some((TaskState::Failed, _, error)) => {
            return Err(error.unwrap_or_else(|| "工具运行失败".into()).into())
        }
        _ => return Err(PreviewError::Cancelled),
    };
    let produced = outputs
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file() && is_video_file(p))
        .ok_or_else(|| "该工具没有生成可预览的视频（例如没有检测到要处理的内容）".to_owned())?;
    let ext = produced.extension().map_or_else(
        || "mp4".to_owned(),
        |e| e.to_string_lossy().to_ascii_lowercase(),
    );
    let output = dir.new_file("tool", &ext)?;
    fsutil::move_file(&produced, &output).map_err(|e| format!("无法写入预览文件：{e}"))?;
    let probed = ffmpeg::probe(&paths.ffprobe, &output).ok();
    let (width, height) = probed
        .as_ref()
        .and_then(|i| i.video.as_ref())
        .map(|v| v.display_size())
        .unwrap_or((0, 0));
    on_progress(100.0);
    Ok(PreviewResult {
        path: output.to_string_lossy().into_owned(),
        duration: probed.map_or(duration, |i| i.duration),
        width,
        height,
        seed,
        modules: vec![spec.tool.clone()],
        warnings,
        regions: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_args_trim_and_shrink() {
        assert_eq!(
            clip_args("in.mp4", 2.5, 5.0, 480, "c.mp4").join(" "),
            "-ss 2.5 -i in.mp4 -t 5 -map 0:v:0 -map 0:a:0? \
             -vf scale=-2:'2*trunc(min(ih,480)/2)',format=yuv420p \
             -c:v libx264 -preset ultrafast -crf 18 -c:a aac -b:a 192k -sn -dn c.mp4"
        );
        assert!(clip_args("in.mp4", 0.0, 5.0, 360, "c.mp4")[0] == "-i");
    }

    #[test]
    fn spec_defaults() {
        let spec: ToolPreviewSpec = serde_json::from_value(serde_json::json!({
            "tool": "tool.camera-move",
            "path": "a.mp4",
            "settings": Settings::default(),
        }))
        .unwrap();
        assert_eq!((spec.start, spec.duration, spec.max_height), (0.0, 0.0, 0));
        assert!(spec.params.is_null() && spec.bitrate.is_none());
    }
}
