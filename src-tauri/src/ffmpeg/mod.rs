//! FFmpeg 的定位与调用。
//!
//! - [`locate`]：找到 ffmpeg / ffprobe 可执行文件（[`locate_detailed`] 另外说明来源）。
//! - [`probe`]：用 ffprobe 读取媒体信息（[`MediaInfo`]）。
//! - [`run`]：运行 ffmpeg，解析 `-progress` 输出，支持取消。
//! - [`encoders`]：探测可用的硬件编码器并选择视频编码器。
//! - [`manifest`] / [`managed`]：应用内下载的 FFmpeg（内嵌的下载清单、下载校验安装）。

pub mod encoders;
pub mod managed;
pub mod manifest;
pub mod probe;
pub mod run;

use std::ffi::OsString;
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

/// 实际使用的 FFmpeg 从哪里找到。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FfmpegSource {
    /// 环境变量 `CLIPRECIPE_FFMPEG`
    Env,
    /// 应用内下载的副本（[`managed`]）
    Managed,
    /// 与本程序放在一起
    Bundled,
    /// PATH
    Path,
    /// 开发构建：仓库 node_modules 里的 @seankit 包
    Dev,
    /// 没找到（返回命令名，运行时报错）
    Missing,
}

/// 找到的托管副本（不论是否正在使用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCopy {
    pub version: String,
    pub dir: PathBuf,
    /// 比本程序固定的版本旧（仍会使用，可以更新）
    pub outdated: bool,
}

/// [`locate_detailed`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub paths: FfmpegPaths,
    pub source: FfmpegSource,
    pub managed: Option<ManagedCopy>,
}

/// 查找时可用的各个位置（由环境决定；测试中直接构造）。
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// `CLIPRECIPE_FFMPEG`
    pub env_ffmpeg: Option<OsString>,
    /// 托管副本根目录与本程序固定的 FFmpeg 版本
    pub managed_root: Option<PathBuf>,
    pub pinned_version: String,
    /// 本程序所在目录
    pub exe_dir: Option<PathBuf>,
    /// 是否查找 PATH
    pub system_path: bool,
    /// 开发构建的 node_modules 二进制（只在调试构建与测试中有值）
    pub dev: Option<FfmpegPaths>,
}

/// 端到端测试：设为 1 时不查找程序目录、PATH 与 node_modules（模拟“没有 FFmpeg”的电脑）。只在 `e2e-hooks` 构建中存在。
#[cfg(feature = "e2e-hooks")]
pub const E2E_ISOLATE_ENV: &str = "CLIPRECIPE_E2E_ISOLATE_FFMPEG";

impl Search {
    /// 按当前环境：环境变量、托管目录、程序目录、PATH、（调试构建）node_modules。
    pub fn from_env() -> Self {
        let pinned_version = manifest::active()
            .map(|(m, _)| m.ffmpeg_version.clone())
            .unwrap_or_default();
        #[allow(unused_mut)]
        let mut search = Self {
            env_ffmpeg: std::env::var_os(FFMPEG_ENV).filter(|v| !v.is_empty()),
            managed_root: managed::root(),
            pinned_version,
            exe_dir: current_exe_dir(),
            system_path: true,
            dev: dev_binaries(),
        };
        #[cfg(feature = "e2e-hooks")]
        if std::env::var_os(E2E_ISOLATE_ENV).is_some_and(|v| v == "1") {
            search.exe_dir = None;
            search.system_path = false;
            search.dev = None;
        }
        search
    }
}

/// 定位 ffmpeg 与 ffprobe，见 [`locate_detailed`]。
pub fn locate() -> FfmpegPaths {
    locate_detailed().paths
}

/// 定位 ffmpeg 与 ffprobe 并说明来源。
///
/// 顺序：
/// 1. 环境变量 `CLIPRECIPE_FFMPEG`（明确指定，总是优先；ffprobe 取同目录下的 `ffprobe`，没有则用 PATH 里的）；
/// 2. 应用内下载的副本（本程序固定的版本；只有更旧的版本时也使用，并标记为可更新）；
/// 3. 本程序可执行文件所在目录（ffmpeg 与 ffprobe 都在时）；
/// 4. PATH；
/// 5. 只在调试构建与测试中：仓库 `node_modules/@seankit/ffmpeg-<平台>/bin` 与 `ffprobe-<平台>/bin`
///    （`npm install` 后即可运行程序与测试，不必设置环境变量）。
///
/// 找不到时仍返回命令名 `ffmpeg` / `ffprobe`，启动失败时再报错，方便用户之后补装。
/// 每次调用都重新查找（不缓存），下载或删除托管副本后立即生效。
pub fn locate_detailed() -> Located {
    locate_in(&Search::from_env())
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// 调试构建：仓库 node_modules 里与本平台对应的 @seankit 包（两个包、两个目录）。
#[cfg(debug_assertions)]
fn dev_binaries() -> Option<FfmpegPaths> {
    let platform = manifest::embedded().ok()?.current_platform().ok()?;
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("node_modules")
        .join("@seankit");
    let package = |tool: &str, binary: &str| {
        binary
            .split('/')
            .fold(base.join(format!("{tool}-{}", platform.key)), |p, part| {
                p.join(part)
            })
    };
    let ffmpeg = package("ffmpeg", &platform.ffmpeg.binary);
    let ffprobe = package("ffprobe", &platform.ffprobe.binary);
    (ffmpeg.is_file() && ffprobe.is_file()).then_some(FfmpegPaths { ffmpeg, ffprobe })
}

#[cfg(not(debug_assertions))]
fn dev_binaries() -> Option<FfmpegPaths> {
    None
}

/// 按 [`Search`] 查找（顺序见 [`locate_detailed`]）。
pub fn locate_in(search: &Search) -> Located {
    let managed = search
        .managed_root
        .as_deref()
        .and_then(|root| managed::find(root, &search.pinned_version));
    let managed_copy = managed.as_ref().map(|(copy, outdated)| ManagedCopy {
        version: copy.version.clone(),
        dir: copy.dir.clone(),
        outdated: *outdated,
    });
    let found = |paths: FfmpegPaths, source: FfmpegSource| Located {
        paths,
        source,
        managed: managed_copy.clone(),
    };
    let which_if = |name: &str| search.system_path.then(|| which(name)).flatten();

    if let Some(ffmpeg) = &search.env_ffmpeg {
        let ffmpeg = PathBuf::from(ffmpeg);
        let ffprobe = sibling_ffprobe(&ffmpeg)
            .or_else(|| which_if("ffprobe"))
            .unwrap_or_else(|| PathBuf::from("ffprobe"));
        return found(FfmpegPaths { ffmpeg, ffprobe }, FfmpegSource::Env);
    }
    if let Some((copy, _)) = &managed {
        return found(copy.paths.clone(), FfmpegSource::Managed);
    }
    if let Some(dir) = &search.exe_dir {
        let ffmpeg = dir.join(format!("ffmpeg{EXE_SUFFIX}"));
        let ffprobe = dir.join(format!("ffprobe{EXE_SUFFIX}"));
        if ffmpeg.is_file() && ffprobe.is_file() {
            return found(FfmpegPaths { ffmpeg, ffprobe }, FfmpegSource::Bundled);
        }
    }
    if let Some(ffmpeg) = which_if("ffmpeg") {
        let ffprobe = sibling_ffprobe(&ffmpeg)
            .or_else(|| which_if("ffprobe"))
            .unwrap_or_else(|| PathBuf::from("ffprobe"));
        return found(FfmpegPaths { ffmpeg, ffprobe }, FfmpegSource::Path);
    }
    if let Some(dev) = &search.dev {
        return found(dev.clone(), FfmpegSource::Dev);
    }
    found(
        FfmpegPaths {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: which_if("ffprobe").unwrap_or_else(|| PathBuf::from("ffprobe")),
        },
        FfmpegSource::Missing,
    )
}

fn sibling_ffprobe(ffmpeg: &Path) -> Option<PathBuf> {
    ffmpeg
        .parent()
        .map(|dir| dir.join(format!("ffprobe{EXE_SUFFIX}")))
        .filter(|p| p.is_file())
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
    /// `ffprobe -version` 的首行
    pub ffprobe_version: Option<String>,
    pub error: Option<String>,
    /// 实际使用的 FFmpeg 从哪里找到
    pub source: FfmpegSource,
    /// 当前 FFmpeg 是否有 `whisper` 滤镜（`tool.subtitle-asr` 的内置引擎）
    pub whisper: bool,
    /// 应用内下载
    pub managed: ManagedStatus,
}

/// 应用内下载的状态（设置页、缺少 FFmpeg 时的提示）。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedStatus {
    /// 本平台能否下载
    pub supported: bool,
    /// 不能下载的原因
    pub unsupported_reason: Option<String>,
    /// 本程序固定的 FFmpeg 版本与 npm 包版本
    pub pinned_version: String,
    pub package_version: String,
    /// 需要下载的字节数（ffmpeg + ffprobe 两个压缩包）
    pub download_size: u64,
    /// 已下载的副本（不论是否正在使用）
    pub installed_version: Option<String>,
    pub installed_dir: Option<String>,
    /// 已下载的副本比固定版本旧，可以更新
    pub update_available: bool,
    /// 托管副本的根目录（没有时不能下载）
    pub root: Option<String>,
}

impl ManagedStatus {
    fn for_located(located: &Located) -> Self {
        let root = managed::root();
        let mut status = ManagedStatus {
            root: root.as_ref().map(|r| r.to_string_lossy().into_owned()),
            installed_version: located.managed.as_ref().map(|c| c.version.clone()),
            installed_dir: located
                .managed
                .as_ref()
                .map(|c| c.dir.to_string_lossy().into_owned()),
            ..Self::default()
        };
        match manifest::active() {
            Ok((m, _)) => {
                status.pinned_version = m.ffmpeg_version.clone();
                status.package_version = m.package_version.clone();
                match m.current_platform() {
                    Ok(platform) => {
                        status.supported = root.is_some();
                        status.download_size = platform.download_size();
                        if root.is_none() {
                            status.unsupported_reason = Some("无法确定下载位置".into());
                        }
                    }
                    Err(e) => status.unsupported_reason = Some(e),
                }
            }
            Err(e) => status.unsupported_reason = Some(e),
        }
        status.update_available =
            status.supported && located.managed.as_ref().is_some_and(|c| c.outdated);
        status
    }
}

impl FfmpegStatus {
    pub fn unavailable(error: String) -> Self {
        let located = locate_detailed();
        Self {
            available: false,
            path: located.paths.ffmpeg.to_string_lossy().into_owned(),
            ffprobe_path: located.paths.ffprobe.to_string_lossy().into_owned(),
            version: None,
            ffprobe_version: None,
            error: Some(error),
            source: located.source,
            whisper: false,
            managed: ManagedStatus::for_located(&located),
        }
    }
}

/// 检测 ffmpeg 与 ffprobe 是否都能启动，并报告来源、应用内下载的状态。
pub fn status() -> FfmpegStatus {
    let located = locate_detailed();
    let paths = &located.paths;
    let mut status = FfmpegStatus {
        available: false,
        path: paths.ffmpeg.to_string_lossy().into_owned(),
        ffprobe_path: paths.ffprobe.to_string_lossy().into_owned(),
        version: None,
        ffprobe_version: None,
        error: None,
        source: located.source,
        whisper: false,
        managed: ManagedStatus::for_located(&located),
    };
    match version_line(&paths.ffmpeg) {
        Ok(line) => status.version = Some(line),
        Err(e) => {
            status.error = Some(format!("ffmpeg：{e}"));
            return status;
        }
    }
    match version_line(&paths.ffprobe) {
        Ok(line) => status.ffprobe_version = Some(line),
        Err(e) => {
            status.error = Some(format!("ffprobe：{e}"));
            return status;
        }
    }
    status.available = true;
    status.whisper = has_filter(&paths.ffmpeg, "whisper");
    status
}

/// `ffmpeg -h filter=<name>` 的输出里是否有这个滤镜。
pub fn help_has_filter(help: &str, name: &str) -> bool {
    help.contains(&format!("Filter {name}\n"))
        || help.contains(&format!("Filter {name}\r\n"))
        || help.contains(&format!("{name} AVOptions"))
}

/// 这个 ffmpeg 有没有某个滤镜（如 `whisper`：FFmpeg 8.0 起、编译时启用 whisper.cpp 才有）。
pub fn has_filter(ffmpeg: &Path, name: &str) -> bool {
    command(ffmpeg)
        .args(["-hide_banner", "-h"])
        .arg(format!("filter={name}"))
        .output()
        .map(|o| help_has_filter(&String::from_utf8_lossy(&o.stdout), name))
        .unwrap_or(false)
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

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cr-locate-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        /// 在子目录里放两个空的 ffmpeg / ffprobe。
        fn pair(&self, sub: &str) -> FfmpegPaths {
            let dir = self.0.join(sub);
            std::fs::create_dir_all(&dir).unwrap();
            let paths = FfmpegPaths {
                ffmpeg: dir.join(format!("ffmpeg{EXE_SUFFIX}")),
                ffprobe: dir.join(format!("ffprobe{EXE_SUFFIX}")),
            };
            std::fs::write(&paths.ffmpeg, b"").unwrap();
            std::fs::write(&paths.ffprobe, b"").unwrap();
            paths
        }

        /// 在托管根目录下放一个带安装记录的版本。
        fn managed(&self, version: &str) -> FfmpegPaths {
            let paths = self.pair(&format!("managed/{version}"));
            let tool = |name: &str| managed::ReceiptTool {
                tool: name.into(),
                package: format!("@seankit/{name}-x"),
                version: "1".into(),
                url: "https://example/x.tgz".into(),
                size: 1,
                sha256: "0".repeat(64),
                binary_sha256: "0".repeat(64),
                binary_size: 0,
            };
            let receipt = managed::Receipt {
                ffmpeg_version: version.into(),
                package_version: "1".into(),
                platform: "test".into(),
                installed_at: 0,
                tools: vec![tool("ffmpeg"), tool("ffprobe")],
            };
            std::fs::write(
                self.0
                    .join("managed")
                    .join(version)
                    .join(managed::RECEIPT_FILE),
                serde_json::to_string(&receipt).unwrap(),
            )
            .unwrap();
            paths
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn env_var_wins_and_ffprobe_falls_back_to_name() {
        let search = Search {
            env_ffmpeg: Some("Z:/nowhere/ffmpeg.exe".into()),
            ..Search::default()
        };
        let located = locate_in(&search);
        assert_eq!(located.source, FfmpegSource::Env);
        assert_eq!(located.paths.ffmpeg, PathBuf::from("Z:/nowhere/ffmpeg.exe"));
        // 同目录没有 ffprobe、不查 PATH：退回命令名
        assert_eq!(located.paths.ffprobe, PathBuf::from("ffprobe"));
    }

    #[test]
    fn ffprobe_is_taken_from_ffmpeg_directory() {
        let dir = Dir::new("sibling");
        let pair = dir.pair("bin");
        let search = Search {
            env_ffmpeg: Some(pair.ffmpeg.clone().into_os_string()),
            ..Search::default()
        };
        assert_eq!(locate_in(&search).paths, pair);
        let search = Search {
            exe_dir: Some(dir.0.join("bin")),
            ..Search::default()
        };
        let located = locate_in(&search);
        assert_eq!(
            (located.paths, located.source),
            (pair, FfmpegSource::Bundled)
        );
    }

    #[test]
    fn locate_precedence_env_managed_bundled_dev() {
        let dir = Dir::new("order");
        let env = dir.pair("env");
        let managed_pinned = dir.managed("9.0.2");
        let bundled = dir.pair("exe");
        let dev = FfmpegPaths {
            ffmpeg: dir.0.join("dev-ffmpeg/bin/ffmpeg"),
            ffprobe: dir.0.join("dev-ffprobe/bin/ffprobe"),
        };
        let mut search = Search {
            env_ffmpeg: Some(env.ffmpeg.clone().into_os_string()),
            managed_root: Some(dir.0.join("managed")),
            pinned_version: "9.0.2".into(),
            exe_dir: Some(dir.0.join("exe")),
            system_path: false,
            dev: Some(dev.clone()),
        };
        let located = locate_in(&search);
        assert_eq!((&located.paths, located.source), (&env, FfmpegSource::Env));
        // 环境变量优先时仍报告已下载的副本
        assert_eq!(
            located.managed.as_ref().map(|m| m.version.as_str()),
            Some("9.0.2")
        );

        search.env_ffmpeg = None;
        let located = locate_in(&search);
        assert_eq!(
            (&located.paths, located.source),
            (&managed_pinned, FfmpegSource::Managed)
        );
        assert!(!located.managed.unwrap().outdated);

        search.managed_root = None;
        let located = locate_in(&search);
        assert_eq!(
            (&located.paths, located.source),
            (&bundled, FfmpegSource::Bundled)
        );

        // 程序目录只有 ffmpeg 没有 ffprobe：不算
        std::fs::remove_file(&bundled.ffprobe).unwrap();
        let located = locate_in(&search);
        assert_eq!((&located.paths, located.source), (&dev, FfmpegSource::Dev));

        search.dev = None;
        let located = locate_in(&search);
        assert_eq!(located.source, FfmpegSource::Missing);
        assert_eq!(located.paths.ffmpeg, PathBuf::from("ffmpeg"));
    }

    #[test]
    fn older_managed_copy_is_used_but_flagged_and_pinned_one_wins() {
        let dir = Dir::new("versions");
        let old = dir.managed("8.1");
        let search = Search {
            managed_root: Some(dir.0.join("managed")),
            pinned_version: "9.0.2".into(),
            ..Search::default()
        };
        let located = locate_in(&search);
        assert_eq!(
            (&located.paths, located.source),
            (&old, FfmpegSource::Managed)
        );
        assert!(located.managed.as_ref().unwrap().outdated);

        // 不完整的版本目录（缺 ffprobe、没有安装记录、以点开头的临时目录）不算
        let broken = dir.managed("9.0.2");
        std::fs::remove_file(&broken.ffprobe).unwrap();
        dir.pair("managed/9.5");
        let staging = dir.managed(".staging-1");
        assert!(staging.ffmpeg.is_file());
        assert_eq!(locate_in(&search).paths, old);

        let pinned = dir.managed("9.0.2");
        let located = locate_in(&search);
        assert_eq!(located.paths, pinned);
        assert!(!located.managed.unwrap().outdated);
    }

    #[test]
    fn filter_help_parsing() {
        assert!(help_has_filter(
            "Filter whisper\n  Transcribe audio\n",
            "whisper"
        ));
        assert!(help_has_filter("whisper AVOptions:\n", "whisper"));
        assert!(!help_has_filter("Unknown filter 'whisper'.", "whisper"));
        assert!(!help_has_filter("Filter whisperx\n", "whisper"));
    }

    /// 调试构建与测试里，`npm install` 后能找到 node_modules 中的 ffmpeg 与 ffprobe（两个不同的目录）。
    #[test]
    fn dev_fallback_points_into_node_modules() {
        match dev_binaries() {
            Some(paths) => {
                let (a, b) = (
                    paths.ffmpeg.to_string_lossy(),
                    paths.ffprobe.to_string_lossy(),
                );
                assert!(a.contains("node_modules") && a.contains("ffmpeg-"), "{a}");
                assert!(b.contains("ffprobe-"), "{b}");
                assert_ne!(paths.ffmpeg.parent(), paths.ffprobe.parent());
            }
            None => println!("跳过：node_modules 里没有 @seankit 的本平台包（先运行 npm install）"),
        }
    }
}
