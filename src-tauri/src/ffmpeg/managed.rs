//! 应用内下载的 FFmpeg（“托管副本”）：下载、校验、安装、查找与删除。
//!
//! 只在用户点击“下载 FFmpeg”时运行。下载的内容完全由编译时内嵌的清单决定（[`super::manifest`]）：
//!
//! 1. 按清单中的顺序尝试 tgz 的下载地址（npmmirror → npmjs），一个失败就换下一个；正式构建只允许 HTTPS
//!    （跳转到 http 也拒绝），代理与超时见 [`crate::net`]（30 秒收不到数据判定卡住）。
//! 2. 边下载边写入 `<根目录>/.staging-*/` 下的临时文件并计算 SHA-256；字节数超过清单中的 `size` 立即停止，
//!    结束时必须正好相等。
//! 3. **先**核对 tgz 的 SHA-256，一致后才打开压缩包；只解出清单指定的那一个条目（`package/<binary>`），
//!    其它条目一律不写入磁盘；任何条目是绝对路径、含 `..`、是链接或特殊文件、大小超过可执行文件，整个压缩包都拒绝。
//! 4. 核对可执行文件的 SHA-256 与大小；Unix 上设置可执行权限。
//! 5. ffmpeg 与 ffprobe 都通过后写入 `receipt.json`（版本、来源地址、哈希、时间），把临时目录**整体改名**为
//!    `<根目录>/<FFmpeg 版本>/`：已有同版本时先把它移开、改名成功后再删除，失败则移回——中途失败、取消或进程被结束
//!    都不会破坏已有的副本。成功后删除其它（旧）版本。
//! 6. 失败或取消时删除临时目录；启动下一次安装时清理残留。
//!
//! 未经上述校验的文件不会出现在版本目录里，也就不会被 [`super::locate`] 找到、被执行。
//!
//! 根目录：设置了 `CLIPRECIPE_CONFIG_DIR`（端到端测试的隔离目录）时为 `<它>/ffmpeg`，否则为应用启动时登记的
//! `<应用本地数据目录>/ffmpeg`（[`set_default_root`]）；都没有时（如 `cargo test`）不使用托管副本。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::manifest::{Manifest, Platform, PlatformEntry, UrlPolicy};
use super::{CancelToken, FfmpegPaths, EXE_SUFFIX};
use crate::net::Timeouts;

#[cfg(all(feature = "e2e-hooks", not(debug_assertions)))]
compile_error!("e2e-hooks 特性只能用于调试构建（端到端测试），不能出现在正式发布的程序中");

/// 版本目录中的安装记录。
pub const RECEIPT_FILE: &str = "receipt.json";
const STAGING_PREFIX: &str = ".staging-";
const OLD_PREFIX: &str = ".old-";
const TRASH_PREFIX: &str = ".trash-";

// ---------------------------------------------------------------- 根目录

static DEFAULT_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// 应用启动时登记托管副本的根目录（`<应用本地数据目录>/ffmpeg`）。
pub fn set_default_root(root: PathBuf) {
    let _ = DEFAULT_ROOT.set(root);
}

/// 托管副本的根目录（见模块说明）。
pub fn root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(crate::commands::CONFIG_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("ffmpeg"));
    }
    DEFAULT_ROOT.get().cloned()
}

// ---------------------------------------------------------------- 安装记录与查找

/// `receipt.json`：安装了什么、从哪里下载、哈希与时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub ffmpeg_version: String,
    pub package_version: String,
    /// 清单中的平台键，如 `win32-x64`
    pub platform: String,
    /// 安装时间（Unix 秒）
    pub installed_at: u64,
    pub tools: Vec<ReceiptTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptTool {
    /// `ffmpeg` / `ffprobe`
    pub tool: String,
    pub package: String,
    pub version: String,
    /// 实际下载成功的地址
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub binary_sha256: String,
    pub binary_size: u64,
}

impl Receipt {
    fn tool(&self, name: &str) -> Option<&ReceiptTool> {
        self.tools.iter().find(|t| t.tool == name)
    }
}

/// 一个完整的托管副本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    pub dir: PathBuf,
    pub paths: FfmpegPaths,
    pub receipt: Receipt,
}

fn binary_name(tool: &str) -> String {
    format!("{tool}{EXE_SUFFIX}")
}

/// 读取一个版本目录：有安装记录、版本与目录名一致、ffmpeg 与 ffprobe 都在且大小与记录相符才算完整。
pub fn read_installed(dir: &Path) -> Option<Installed> {
    let version = dir.file_name()?.to_str()?.to_owned();
    if version.starts_with('.') {
        return None;
    }
    let receipt: Receipt =
        serde_json::from_str(&std::fs::read_to_string(dir.join(RECEIPT_FILE)).ok()?).ok()?;
    if receipt.ffmpeg_version != version {
        return None;
    }
    let mut found = Vec::new();
    for tool in ["ffmpeg", "ffprobe"] {
        let path = dir.join(binary_name(tool));
        let size = std::fs::metadata(&path).ok().filter(|m| m.is_file())?.len();
        if size != receipt.tool(tool)?.binary_size {
            return None;
        }
        found.push(path);
    }
    let ffprobe = found.pop()?;
    let ffmpeg = found.pop()?;
    Some(Installed {
        version,
        dir: dir.to_path_buf(),
        paths: FfmpegPaths { ffmpeg, ffprobe },
        receipt,
    })
}

/// 根目录下全部完整的副本，版本从新到旧。
pub fn list_installed(root: &Path) -> Vec<Installed> {
    let mut list: Vec<Installed> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| read_installed(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|c| std::cmp::Reverse(version_key(&c.version)));
    list
}

fn version_key(version: &str) -> ([u64; 3], String) {
    (
        crate::system::parse_version(version).unwrap_or([0; 3]),
        version.to_owned(),
    )
}

/// `version` 是否比 `pinned` 旧。
pub fn is_older(version: &str, pinned: &str) -> bool {
    version_key(version).0 < version_key(pinned).0
}

/// 要使用的托管副本：优先当前程序固定的版本，否则用最新的一个（比固定版本旧时第二项为 true，即“可更新”）。
pub fn find(root: &Path, pinned: &str) -> Option<(Installed, bool)> {
    let mut list = list_installed(root);
    if let Some(i) = list.iter().position(|c| c.version == pinned) {
        return Some((list.swap_remove(i), false));
    }
    let newest = list.into_iter().next()?;
    let outdated = is_older(&newest.version, pinned);
    Some((newest, outdated))
}

// ---------------------------------------------------------------- 进度

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// 下载中
    Download,
    /// 解出并校验可执行文件
    Extract,
    /// 移入版本目录
    Install,
}

/// 下载进度（`ffmpeg-download` 事件）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub phase: Phase,
    /// 当前处理的工具：`ffmpeg` / `ffprobe`
    pub tool: String,
    /// 当前下载地址的主机名
    pub source: String,
    /// 两个压缩包合计已下载 / 总字节数
    pub downloaded: u64,
    pub total: u64,
    /// 最近几秒的平均速度（字节/秒）
    pub bytes_per_sec: f64,
}

/// 最近 3 秒的平均速度。
struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    fn new() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }

    fn record(&mut self, bytes: u64) -> f64 {
        let now = Instant::now();
        self.samples.push_back((now, bytes));
        while self.samples.len() > 2
            && now.duration_since(self.samples[0].0) > Duration::from_secs(3)
        {
            self.samples.pop_front();
        }
        let (t0, b0) = self.samples[0];
        let secs = now.duration_since(t0).as_secs_f64();
        if secs < 0.05 {
            0.0
        } else {
            bytes.saturating_sub(b0) as f64 / secs
        }
    }
}

// ---------------------------------------------------------------- 安装

/// 安装失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    Cancelled,
    Failed(String),
}

impl From<String> for InstallError {
    fn from(e: String) -> Self {
        InstallError::Failed(e)
    }
}

impl From<&str> for InstallError {
    fn from(e: &str) -> Self {
        InstallError::Failed(e.to_owned())
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Cancelled => f.write_str("已取消下载"),
            InstallError::Failed(e) => f.write_str(e),
        }
    }
}

/// 同一时间只允许一个安装 / 删除。
fn lock() -> Result<std::sync::MutexGuard<'static, ()>, InstallError> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.try_lock()
        .map_err(|_| "已有 FFmpeg 下载或删除正在进行".into())
}

/// 一次安装的全部输入。
pub struct Installer<'a> {
    pub root: PathBuf,
    pub manifest: &'a Manifest,
    pub platform: Platform<'a>,
    /// 正式为 [`UrlPolicy::HttpsOnly`]；只有测试能构造允许本机 http 的取值
    pub policy: UrlPolicy,
    pub timeouts: Timeouts,
    pub cancel: CancelToken,
}

/// 临时目录：离开作用域时删除（成功时已被改名，删除是空操作）。
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn unique_name(prefix: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{prefix}{}-{nanos}-{n}", std::process::id())
}

/// 清理残留：`.old-*` / `.trash-*` 总是删除；`.staging-*` 只删本进程的或一小时以前的（可能另一个实例正在下载）。
fn clean_leftovers(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let own = format!("{STAGING_PREFIX}{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > Duration::from_secs(3600));
        let remove = name.starts_with(OLD_PREFIX)
            || name.starts_with(TRASH_PREFIX)
            || (name.starts_with(STAGING_PREFIX) && (name.starts_with(&own) || stale));
        if remove {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn sha256_hex(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn host_of(url: &str) -> String {
    crate::net::scheme_and_host(url)
        .map(|(_, h)| h)
        .unwrap_or_else(|| url.to_owned())
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// 进度回报：合计字节、速度、节流（约每 100 毫秒一次）。
struct Reporter<'a> {
    callback: &'a mut dyn FnMut(&DownloadProgress),
    total: u64,
    done_before: u64,
    meter: SpeedMeter,
    last: Option<Instant>,
}

impl Reporter<'_> {
    fn report(&mut self, phase: Phase, tool: &str, source: &str, current: u64, force: bool) {
        let downloaded = self.done_before + current;
        let speed = self.meter.record(downloaded);
        let due = self
            .last
            .map_or(true, |t| t.elapsed() >= Duration::from_millis(100));
        if !force && !due {
            return;
        }
        self.last = Some(Instant::now());
        (self.callback)(&DownloadProgress {
            phase,
            tool: tool.to_owned(),
            source: source.to_owned(),
            downloaded,
            total: self.total,
            bytes_per_sec: speed,
        });
    }
}

impl Installer<'_> {
    /// 下载并安装（见模块说明）。`progress` 在调用线程中回调。
    pub fn install(
        &self,
        progress: &mut dyn FnMut(&DownloadProgress),
    ) -> Result<Installed, InstallError> {
        let _guard = lock()?;
        let version = self.manifest.ffmpeg_version.clone();
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("无法创建目录 {}：{e}", self.root.display()))?;
        clean_leftovers(&self.root);
        let staging = Staging(self.root.join(unique_name(STAGING_PREFIX)));
        std::fs::create_dir(&staging.0).map_err(|e| format!("无法创建临时目录：{e}"))?;

        let mut reporter = Reporter {
            callback: progress,
            total: self.platform.download_size(),
            done_before: 0,
            meter: SpeedMeter::new(),
            last: None,
        };
        let mut tools = Vec::new();
        for (tool, entry) in [
            ("ffmpeg", self.platform.ffmpeg),
            ("ffprobe", self.platform.ffprobe),
        ] {
            let tarball = staging.0.join(format!("{tool}.tgz"));
            let url = self.download(tool, entry, &tarball, &mut reporter)?;
            reporter.done_before += entry.size;
            reporter.report(Phase::Extract, tool, &host_of(&url), 0, true);
            let binary = staging.0.join(binary_name(tool));
            extract_binary(&tarball, entry, &binary, &self.cancel)?;
            let _ = std::fs::remove_file(&tarball);
            make_executable(&binary)?;
            tools.push(ReceiptTool {
                tool: tool.to_owned(),
                package: entry.package.clone(),
                version: entry.version.clone(),
                url,
                size: entry.size,
                sha256: entry.sha256.clone(),
                binary_sha256: entry.binary_sha256.clone(),
                binary_size: entry.binary_size,
            });
        }
        if self.cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        reporter.report(Phase::Install, "ffprobe", "", 0, true);
        let receipt = Receipt {
            ffmpeg_version: version.clone(),
            package_version: self.manifest.package_version.clone(),
            platform: self.platform.key.to_owned(),
            installed_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            tools,
        };
        let json = serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?;
        std::fs::write(staging.0.join(RECEIPT_FILE), json)
            .map_err(|e| format!("无法写入安装记录：{e}"))?;

        let target = self.root.join(&version);
        move_into_place(&staging.0, &target)?;
        remove_other_versions(&self.root, &version);
        read_installed(&target).ok_or_else(|| "安装后的文件不完整".into())
    }

    /// 按顺序尝试各个地址，返回成功的地址。
    fn download(
        &self,
        tool: &str,
        entry: &PlatformEntry,
        dest: &Path,
        reporter: &mut Reporter,
    ) -> Result<String, InstallError> {
        let mut errors = Vec::new();
        for url in &entry.urls {
            if self.cancel.is_cancelled() {
                return Err(InstallError::Cancelled);
            }
            if let Err(e) = self.policy.check(url) {
                errors.push(e);
                continue;
            }
            match self.fetch(tool, url, entry, dest, reporter) {
                Ok(()) => return Ok(url.clone()),
                Err(InstallError::Cancelled) => {
                    let _ = std::fs::remove_file(dest);
                    return Err(InstallError::Cancelled);
                }
                Err(InstallError::Failed(e)) => {
                    let _ = std::fs::remove_file(dest);
                    errors.push(format!("{}：{e}", host_of(url)));
                }
            }
        }
        Err(format!(
            "{tool} 下载失败（已尝试全部 {} 个地址）：\n{}",
            entry.urls.len(),
            errors.join("\n")
        )
        .into())
    }

    /// 从一个地址下载到 `dest`，校验大小与 SHA-256。
    fn fetch(
        &self,
        tool: &str,
        url: &str,
        entry: &PlatformEntry,
        dest: &Path,
        reporter: &mut Reporter,
    ) -> Result<(), InstallError> {
        let source = host_of(url);
        reporter.report(Phase::Download, tool, &source, 0, true);
        let response = crate::net::agent(url, self.timeouts, self.policy.https_only())
            .get(url)
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, _) => format!("服务器返回 {code}"),
                ureq::Error::Transport(t) => format!("无法连接（{t}）"),
            })?;
        if let Some(length) = response
            .header("Content-Length")
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            if length != entry.size {
                return Err(format!(
                    "文件大小不符：服务器报告 {length} 字节，应为 {} 字节",
                    entry.size
                )
                .into());
            }
        }
        let mut reader = response.into_reader();
        let mut file = File::create(dest).map_err(|e| format!("无法写入临时文件：{e}"))?;
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            if self.cancel.is_cancelled() {
                return Err(InstallError::Cancelled);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if is_timeout(&e) => {
                    return Err(format!(
                        "下载卡住：{} 秒内没有收到数据（已收到 {received} / {} 字节）",
                        self.timeouts.read.as_secs_f64(),
                        entry.size
                    )
                    .into())
                }
                Err(e) => {
                    return Err(
                        format!("下载中断（已收到 {received} / {} 字节）：{e}", entry.size).into(),
                    )
                }
            };
            received += n as u64;
            if received > entry.size {
                return Err(format!("服务器发送的数据超过应有的 {} 字节", entry.size).into());
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .map_err(|e| format!("无法写入临时文件：{e}"))?;
            reporter.report(Phase::Download, tool, &source, received, false);
        }
        if received != entry.size {
            return Err(
                format!("下载不完整：收到 {received} 字节，应为 {} 字节", entry.size).into(),
            );
        }
        file.sync_all()
            .map_err(|e| format!("无法写入临时文件：{e}"))?;
        drop(file);
        reporter.report(Phase::Download, tool, &source, received, true);
        let actual = sha256_hex(hasher);
        if actual != entry.sha256 {
            return Err(format!(
                "压缩包 SHA-256 校验失败（应为 {}，实际 {actual}），文件可能被篡改或损坏",
                entry.sha256
            )
            .into());
        }
        Ok(())
    }
}

/// tar 条目名的安全检查：拒绝绝对路径、盘符、反斜杠、`..`。
fn check_entry_name(name: &str) -> Result<(), String> {
    let unsafe_name = name.starts_with('/')
        || name.contains(['\\', '\0'])
        || name.as_bytes().get(1) == Some(&b':')
        || name.split('/').any(|part| part == "..");
    if unsafe_name {
        Err(format!("压缩包包含不安全的路径（{name}），已拒绝"))
    } else {
        Ok(())
    }
}

/// 从已校验的 tgz 中只解出 `entry.tar_entry()` 这一个文件到 `out`，并校验其 SHA-256 与大小。
fn extract_binary(
    tarball: &Path,
    entry: &PlatformEntry,
    out: &Path,
    cancel: &CancelToken,
) -> Result<(), InstallError> {
    let expected = entry.tar_entry();
    let file = File::open(tarball).map_err(|e| format!("无法打开压缩包：{e}"))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file)));
    let bad = |e: std::io::Error| InstallError::Failed(format!("压缩包损坏：{e}"));
    let mut found = false;
    for item in archive.entries().map_err(bad)? {
        if cancel.is_cancelled() {
            return Err(InstallError::Cancelled);
        }
        let mut item = item.map_err(bad)?;
        let name = String::from_utf8(item.path_bytes().into_owned())
            .map_err(|_| "压缩包包含无法识别的文件名，已拒绝".to_owned())?;
        check_entry_name(&name)?;
        let kind = item.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(format!("压缩包包含链接或特殊文件（{name}），已拒绝").into());
        }
        let size = item.size();
        if size > entry.binary_size {
            return Err(format!("压缩包条目过大（{name}，{size} 字节），已拒绝").into());
        }
        if name.trim_start_matches("./") != expected {
            // 其它文件（package.json、LICENSE…）不写入磁盘
            continue;
        }
        if found {
            return Err(format!("压缩包中有重复的 {expected}，已拒绝").into());
        }
        found = true;
        if size != entry.binary_size {
            return Err(format!(
                "{expected} 的大小不符：{size} 字节，应为 {} 字节",
                entry.binary_size
            )
            .into());
        }
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(out)
            .map_err(|e| format!("无法写入 {}：{e}", out.display()))?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;
        let mut buf = vec![0u8; 256 * 1024];
        let mut limited = (&mut item).take(entry.binary_size + 1);
        loop {
            if cancel.is_cancelled() {
                return Err(InstallError::Cancelled);
            }
            let n = limited.read(&mut buf).map_err(bad)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > entry.binary_size {
                return Err(format!("{expected} 超过应有大小，已拒绝").into());
            }
            hasher.update(&buf[..n]);
            target
                .write_all(&buf[..n])
                .map_err(|e| format!("无法写入 {}：{e}", out.display()))?;
        }
        target
            .sync_all()
            .map_err(|e| format!("无法写入 {}：{e}", out.display()))?;
        let actual = sha256_hex(hasher);
        if written != entry.binary_size || actual != entry.binary_sha256 {
            return Err(format!(
                "可执行文件校验失败（{expected}：SHA-256 应为 {}，实际 {actual}）",
                entry.binary_sha256
            )
            .into());
        }
    }
    if !found {
        return Err(format!("压缩包里没有 {expected}").into());
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("无法设置可执行权限：{e}"))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// 把临时目录改名为版本目录；已有同名目录时先移开，成功后删除，失败则移回。
fn move_into_place(staging: &Path, target: &Path) -> Result<(), String> {
    let parent = target.parent().unwrap_or(Path::new("."));
    let old = if target.exists() {
        let old = parent.join(unique_name(OLD_PREFIX));
        std::fs::rename(target, &old).map_err(|e| {
            format!("无法替换已有的 FFmpeg（可能正在使用中，请等处理结束后再试）：{e}")
        })?;
        Some(old)
    } else {
        None
    };
    if let Err(e) = std::fs::rename(staging, target) {
        if let Some(old) = &old {
            let _ = std::fs::rename(old, target);
        }
        return Err(format!("无法完成安装：{e}"));
    }
    if let Some(old) = old {
        let _ = std::fs::remove_dir_all(old);
    }
    Ok(())
}

/// 尽力删除除 `keep` 以外的版本目录（正在使用的删不掉，下次再试）。
fn remove_other_versions(root: &Path, keep: &str) {
    for copy in list_installed(root) {
        if copy.version != keep {
            let _ = remove_dir_safely(root, &copy.dir);
        }
    }
}

/// 先改名再删除：正在运行的程序在 Windows 上无法改名，这样不会删掉一半。
fn remove_dir_safely(root: &Path, dir: &Path) -> Result<(), String> {
    let trash = root.join(unique_name(TRASH_PREFIX));
    std::fs::rename(dir, &trash)
        .map_err(|e| format!("FFmpeg 可能正在使用中，请等处理结束后再删除（{e}）"))?;
    std::fs::remove_dir_all(&trash).map_err(|e| format!("删除失败：{e}"))
}

/// 删除根目录下全部托管副本（与残留的临时目录）。
pub fn remove_all(root: &Path) -> Result<usize, String> {
    let _guard = lock().map_err(|e| e.to_string())?;
    let copies = list_installed(root);
    for copy in &copies {
        remove_dir_safely(root, &copy.dir)?;
    }
    clean_leftovers(root);
    // 根目录空了就一起删除
    let _ = std::fs::remove_dir(root);
    Ok(copies.len())
}

#[cfg(test)]
mod tests;
