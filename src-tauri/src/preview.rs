//! 预览（`pipe.preview` 与单模块预览）和取帧（区域框选用）。
//!
//! - [`render_preview`]：把一个视频按当前配方（或只开某一个模块）生成低清片段 / 单帧；
//! - [`extract_frame`]：取源视频某一时刻的一帧（未经流水线处理），供区域框选等使用；
//! - [`PreviewDir`]：预览文件所在的临时目录（每个进程一个），用完即删，退出时整体清理。
//!
//! 这里不依赖 Tauri，命令封装见 `commands::preview`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::{self, path_arg, CancelToken, EncoderCaps, FfmpegPaths, MediaInfo, RunError};
use crate::model::Recipe;
use crate::model::Settings;
use crate::pipeline::{
    self, BuildRequest, PreviewOptions, PreviewOutput, PreviewRegion, MODULE_IDS, PREVIEW_EXCLUDED,
};

/// 预览画面的默认最大高度。
pub const DEFAULT_PREVIEW_HEIGHT: u32 = 480;
/// 取帧图片的最大宽度（更大的画面会等比缩小；返回的仍是源尺寸）。
pub const FRAME_MAX_WIDTH: u32 = 1920;

/// 临时目录名前缀：`<系统临时目录>/cliprecipe-preview/<进程号>`。
const PREVIEW_DIR_NAME: &str = "cliprecipe-preview";
/// 目录里最多保留的文件数，超过时删除最旧的。
const MAX_FILES: usize = 40;

/// 预览文件所在的临时目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewDir {
    root: PathBuf,
    dir: PathBuf,
}

impl Default for PreviewDir {
    fn default() -> Self {
        Self::for_process()
    }
}

impl PreviewDir {
    /// 本进程使用的目录（在系统临时目录下）。
    pub fn for_process() -> Self {
        Self::under(&std::env::temp_dir(), std::process::id())
    }

    /// 指定上级目录与进程号（测试用）。
    pub fn under(temp: &Path, pid: u32) -> Self {
        let root = temp.join(PREVIEW_DIR_NAME);
        let dir = root.join(pid.to_string());
        Self { root, dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// 生成一个新的文件路径（不创建文件），必要时创建目录并清理过多的旧文件。
    pub fn new_file(&self, prefix: &str, ext: &str) -> Result<PathBuf, String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("无法创建预览目录 {}：{e}", self.dir.display()))?;
        self.prune(MAX_FILES.saturating_sub(1));
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        Ok(self.dir.join(format!("{prefix}-{stamp}-{n}.{ext}")))
    }

    /// 是否是本目录下的文件（只允许删除这些）。
    pub fn contains(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| crate::jobs::output::same_path(parent, &self.dir))
    }

    /// 删除一个预览文件；不在预览目录里的路径会被拒绝。
    pub fn release(&self, path: &Path) -> Result<(), String> {
        if !self.contains(path) {
            return Err("只能删除预览目录中的文件".into());
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法删除预览文件：{e}")),
        }
    }

    /// 只保留最新的 `keep` 个文件（删除失败的忽略，例如仍在播放中）。
    fn prune(&self, keep: usize) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                meta.is_file()
                    .then(|| (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), e.path()))
            })
            .collect();
        if files.len() <= keep {
            return;
        }
        files.sort();
        let excess = files.len() - keep;
        for (_, path) in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// 删除本进程的整个预览目录（退出时调用）。
    pub fn remove_all(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// 删除其它进程留下、且超过 `max_age` 未修改的预览目录（异常退出的残留）。
    pub fn cleanup_stale(&self, max_age: Duration) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        let now = SystemTime::now();
        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.dir || !path.is_dir() {
                continue;
            }
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age >= max_age);
            if old {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

/// 生成预览时参与的配方：
///
/// - `only_module` 为 None（整条流水线预览）：关闭 [`PREVIEW_EXCLUDED`] 中的模块；
/// - 指定某个模块：只开这一个（不论它在配方里是否开启）。
///
/// 裂变次数总是视为 1。
pub fn preview_recipe(recipe: &Recipe, only_module: Option<&str>) -> Result<Recipe, String> {
    let mut preview = recipe.clone();
    preview.fission = 1;
    match only_module {
        None => {
            for id in PREVIEW_EXCLUDED {
                preview.set_module_enabled(id, false)?;
            }
        }
        Some(id) => {
            if !MODULE_IDS.contains(&id) {
                return Err(format!("未知模块：{id}"));
            }
            if PREVIEW_EXCLUDED.contains(&id) {
                return Err("该模块不支持预览".into());
            }
            for other in MODULE_IDS {
                preview.set_module_enabled(other, other == id)?;
            }
        }
    }
    Ok(preview)
}

/// 一次预览请求。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSpec {
    pub path: String,
    /// 已探测的媒体信息；没有时现场探测
    #[serde(default)]
    pub info: Option<MediaInfo>,
    pub recipe: Recipe,
    pub settings: Settings,
    /// 只预览这一个模块（功能 ID）；None 表示整条流水线
    #[serde(default)]
    pub only_module: Option<String>,
    pub output: PreviewOutput,
    /// 随机种子；None 时随机
    #[serde(default)]
    pub seed: Option<u64>,
    /// 队列序号（从 1 起），用于素材“依次取”
    #[serde(default)]
    pub number: usize,
    /// 画面最大高度；0 表示默认值
    #[serde(default)]
    pub max_height: u32,
    /// 队列项关联的字幕（预览时同样烧录；封面不参与预览）
    #[serde(default)]
    pub subtitle: Option<String>,
}

/// 预览结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    /// 生成的文件（mp4 或 jpg），位于预览目录
    pub path: String,
    /// 片段时长（秒）；单帧为 0
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub seed: u64,
    /// 参与预览的模块
    pub modules: Vec<String>,
    pub warnings: Vec<String>,
    /// 模块画在画面上的东西所在的区域（如水印），供界面直接在画面上操作；没有时为空
    pub regions: Vec<PreviewRegion>,
}

/// 预览失败的原因。
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::Cancelled => write!(f, "预览已取消"),
            PreviewError::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl From<String> for PreviewError {
    fn from(message: String) -> Self {
        PreviewError::Failed(message)
    }
}

impl From<RunError> for PreviewError {
    fn from(err: RunError) -> Self {
        match err {
            RunError::Cancelled => PreviewError::Cancelled,
            other => PreviewError::Failed(other.to_string()),
        }
    }
}

/// 生成预览。`on_progress` 收到 0～100 的进度（片段时）。
pub fn render_preview(
    paths: &FfmpegPaths,
    spec: &PreviewSpec,
    dir: &PreviewDir,
    token: &CancelToken,
    mut on_progress: impl FnMut(f64),
) -> Result<PreviewResult, PreviewError> {
    let input = PathBuf::from(&spec.path);
    if !input.is_file() {
        return Err(format!("源文件不存在：{}", spec.path).into());
    }
    let info = match &spec.info {
        Some(info) => info.clone(),
        None => ffmpeg::probe(&paths.ffprobe, &input)?,
    };
    let recipe = preview_recipe(&spec.recipe, spec.only_module.as_deref())?;
    let modules: Vec<String> = pipeline::ordered_modules(&recipe.modules)
        .iter()
        .filter(|m| m.enabled() && m.implemented())
        .map(|m| m.id().to_owned())
        .collect();
    let ext = match spec.output {
        PreviewOutput::Clip { .. } => "mp4",
        PreviewOutput::Frame { .. } => "jpg",
    };
    let output = dir.new_file("preview", ext)?;
    let seed = spec.seed.unwrap_or_else(crate::model::rng::random_seed);
    let number = spec.number.max(1);
    let max_height = if spec.max_height == 0 {
        DEFAULT_PREVIEW_HEIGHT
    } else {
        spec.max_height
    };
    let subtitle = spec
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let probe = |p: &Path| ffmpeg::probe(&paths.ffprobe, p);
    let built = pipeline::build_preview(
        &BuildRequest {
            input: &input,
            output: &output,
            info: &info,
            recipe: &recipe,
            settings: &spec.settings,
            encoders: &EncoderCaps::software_only(),
            seed: crate::model::rng::mix_seed(seed, number as u64),
            task_index: number - 1,
            force_software: true,
            probe: Some(&probe),
            sidecar: crate::pipeline::Sidecar {
                subtitle: subtitle.as_deref(),
                cover: None,
            },
        },
        &PreviewOptions {
            output: spec.output,
            max_height,
        },
    )?;
    let expected = built.expected_duration;
    let result = ffmpeg::run(&paths.ffmpeg, &built.args, token, |p| {
        if expected > 0.0 {
            on_progress((p.out_time / expected * 100.0).clamp(0.0, 100.0));
        }
    });
    if let Err(err) = result {
        let _ = std::fs::remove_file(&output);
        return Err(err.into());
    }
    if !output.is_file() {
        return Err("预览没有生成任何画面（可能超出了视频时长）"
            .to_owned()
            .into());
    }
    let probed = ffmpeg::probe(&paths.ffprobe, &output).ok();
    let (width, height) = probed
        .as_ref()
        .and_then(|i| i.video.as_ref())
        .map(|v| v.display_size())
        .unwrap_or((0, 0));
    let duration = match spec.output {
        PreviewOutput::Clip { .. } => probed.map_or(expected, |i| i.duration),
        PreviewOutput::Frame { .. } => 0.0,
    };
    Ok(PreviewResult {
        path: output.to_string_lossy().into_owned(),
        duration,
        width,
        height,
        seed,
        modules,
        warnings: built.warnings,
        regions: built.preview_regions,
    })
}

/// 取帧结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameResult {
    /// jpg 文件，位于预览目录
    pub path: String,
    /// 源视频（旋转后）的显示宽高：区域坐标都以此为准
    pub width: u32,
    pub height: u32,
    /// 实际取帧的时间（秒）
    pub time: f64,
}

/// 取源视频 `time` 秒处的一帧（自动按旋转信息转正；过大时等比缩小）。
pub fn extract_frame(
    paths: &FfmpegPaths,
    input: &Path,
    info: Option<&MediaInfo>,
    time: f64,
    dir: &PreviewDir,
    token: &CancelToken,
) -> Result<FrameResult, PreviewError> {
    if !input.is_file() {
        return Err(format!("源文件不存在：{}", input.display()).into());
    }
    let probed;
    let info = match info {
        Some(info) => info,
        None => {
            probed = ffmpeg::probe(&paths.ffprobe, input)?;
            &probed
        }
    };
    let video = info
        .video
        .as_ref()
        .ok_or_else(|| "文件中没有视频流".to_owned())?;
    // 变形（非方形像素）源按方形像素显示：区域坐标与流水线（先换算为方形像素）一致
    let (width, height) = video.square_display_size();
    let anamorphic = video.sar_ratio().is_some();
    let time = if info.duration > 0.0 {
        time.clamp(0.0, (info.duration - 0.1).max(0.0))
    } else {
        time.max(0.0)
    };
    let output = dir.new_file("frame", "jpg")?;
    let args = frame_args(input, time, (width, height), anamorphic, &output)?;
    if let Err(err) = ffmpeg::run(&paths.ffmpeg, &args, token, |_| {}) {
        let _ = std::fs::remove_file(&output);
        return Err(err.into());
    }
    if !output.is_file() {
        return Err("无法从视频中取得画面".to_owned().into());
    }
    Ok(FrameResult {
        path: output.to_string_lossy().into_owned(),
        width,
        height,
        time,
    })
}

/// 取帧命令：输入端快速定位，只输出一帧。
fn frame_args(
    input: &Path,
    time: f64,
    (width, height): (u32, u32),
    anamorphic: bool,
    output: &Path,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-ss",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push(pipeline::fmt_num(time));
    args.push("-i".into());
    args.push(path_arg(input)?);
    if width > FRAME_MAX_WIDTH {
        args.push("-vf".into());
        // 按显示比例（方形像素）算高度，变形源也能正确缩小
        let h = (f64::from(height) * f64::from(FRAME_MAX_WIDTH) / f64::from(width.max(1)) / 2.0)
            .round() as u32
            * 2;
        args.push(format!("scale={FRAME_MAX_WIDTH}:{},setsar=1", h.max(2)));
    } else if anamorphic {
        args.push("-vf".into());
        args.push(format!("scale={width}:{height},setsar=1"));
    }
    args.extend(
        ["-map", "0:v:0", "-frames:v", "1", "-q:v", "3", "-an"]
            .iter()
            .map(|s| (*s).to_owned()),
    );
    args.push(path_arg(output)?);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(recipe: &Recipe) -> Vec<&'static str> {
        pipeline::ordered_modules(&recipe.modules)
            .iter()
            .filter(|m| m.enabled())
            .map(|m| m.id())
            .collect()
    }

    #[test]
    fn whole_recipe_preview_excludes_speed_bgm_intro_fps_bitrate() {
        let mut recipe = Recipe {
            fission: 5,
            ..Recipe::default()
        };
        for id in [
            "pipe.crop",
            "pipe.speed",
            "pipe.bgm",
            "pipe.intro-outro",
            "pipe.bitrate",
        ] {
            recipe.set_module_enabled(id, true).unwrap();
        }
        let preview = preview_recipe(&recipe, None).unwrap();
        assert_eq!(enabled(&preview), ["pipe.crop"]);
        assert_eq!(preview.fission, 1);
    }

    #[test]
    fn single_module_preview_enables_only_that_module() {
        let recipe = Recipe::default(); // 只开了帧率
        let preview = preview_recipe(&recipe, Some("pipe.crop")).unwrap();
        assert_eq!(enabled(&preview), ["pipe.crop"]);
        assert!(preview_recipe(&recipe, Some("pipe.fps")).is_err());
        assert!(preview_recipe(&recipe, Some("pipe.nope")).is_err());
    }

    #[test]
    fn frame_args_seek_on_input_and_scale_large_sources() {
        let args = frame_args(
            Path::new("in.mp4"),
            1.5,
            (3840, 2160),
            false,
            Path::new("f.jpg"),
        )
        .unwrap();
        let line = args.join(" ");
        assert!(
            line.ends_with(
                "-ss 1.5 -i in.mp4 -vf scale=1920:1080,setsar=1 -map 0:v:0 -frames:v 1 -q:v 3 -an f.jpg"
            ),
            "{line}"
        );
        let small = frame_args(
            Path::new("in.mp4"),
            0.0,
            (1280, 720),
            false,
            Path::new("f.jpg"),
        )
        .unwrap();
        assert!(!small.contains(&"-vf".to_owned()));
        // 变形源（480x480、SAR 2:1）：换算为方形像素
        let wide = frame_args(
            Path::new("in.mp4"),
            0.0,
            (960, 480),
            true,
            Path::new("f.jpg"),
        )
        .unwrap();
        assert!(wide.join(" ").contains("-vf scale=960:480,setsar=1"));
    }

    #[test]
    fn preview_dir_files_prune_and_release() {
        let temp = std::env::temp_dir().join(format!("cr-preview-unit-{}", std::process::id()));
        let dir = PreviewDir::under(&temp, 1);
        let mut made = Vec::new();
        for _ in 0..(MAX_FILES + 5) {
            let file = dir.new_file("x", "jpg").unwrap();
            std::fs::write(&file, b"x").unwrap();
            made.push(file);
        }
        let count = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(count <= MAX_FILES, "旧文件被清理：{count}");
        let last = made.last().unwrap();
        assert!(dir.contains(last));
        dir.release(last).unwrap();
        assert!(!last.exists());
        assert!(dir.release(&temp.join("elsewhere.jpg")).is_err());

        // 其它进程的旧目录被清理，本进程的保留
        let other = PreviewDir::under(&temp, 2);
        std::fs::write(other.new_file("y", "jpg").unwrap(), b"y").unwrap();
        dir.cleanup_stale(Duration::ZERO);
        assert!(!other.path().exists());
        assert!(dir.path().exists());
        dir.remove_all();
        assert!(!dir.path().exists());
        std::fs::remove_dir_all(&temp).unwrap();
    }
}
