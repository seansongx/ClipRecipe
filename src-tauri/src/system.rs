//! 与操作系统打交道的功能：全部完成后关机的倒计时（`set.after-all`）、检查更新与自动更新（`set.auto-update`）。
//!
//! 真正产生系统副作用的调用（关机、打开网页）都收在很小的函数里，由调用方（`commands::system`）传入；
//! 这里的逻辑（倒计时、取消、版本比较、响应解析）可以在测试中替换掉副作用后验证。
//! 测试绝不会真的关机、打开网页或访问网络。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

/// 关机前的倒计时秒数。
pub const SHUTDOWN_COUNTDOWN_SECS: u32 = 30;

/// 可取消的倒计时：每秒（`step`）回调一次剩余秒数，归零时执行 `fire`；取消后不再执行。
pub struct Countdown {
    cancelled: Arc<AtomicBool>,
    handle: Option<JoinHandle<bool>>,
}

impl Countdown {
    /// 开始倒计时。`tick(剩余秒数)` 在开始时与每过一个 `step` 调用；归零后调用 `fire` 一次。
    pub fn start(
        seconds: u32,
        step: Duration,
        tick: impl Fn(u32) + Send + 'static,
        fire: impl FnOnce() + Send + 'static,
    ) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let handle = std::thread::spawn(move || {
            for remaining in (1..=seconds).rev() {
                if flag.load(Ordering::SeqCst) {
                    return false;
                }
                tick(remaining);
                // 分小段睡眠，取消后尽快结束
                let slices = 10u32;
                for _ in 0..slices {
                    if flag.load(Ordering::SeqCst) {
                        return false;
                    }
                    std::thread::sleep(step / slices);
                }
            }
            if flag.load(Ordering::SeqCst) {
                return false;
            }
            tick(0);
            fire();
            true
        });
        Self {
            cancelled,
            handle: Some(handle),
        }
    }

    /// 取消（已经执行过 `fire` 时无效）。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().map_or(true, JoinHandle::is_finished)
    }

    /// 等待结束，返回是否执行了 `fire`。
    pub fn wait(mut self) -> bool {
        self.handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or(false)
    }
}

impl Drop for Countdown {
    fn drop(&mut self) {
        // 丢弃即取消：不会在没人管的情况下关机
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// 本系统的关机命令（程序与参数）。
pub fn shutdown_command() -> (&'static str, Vec<&'static str>) {
    if cfg!(windows) {
        ("shutdown", vec!["/s", "/t", "0"])
    } else if cfg!(target_os = "macos") {
        (
            "osascript",
            vec!["-e", "tell application \"System Events\" to shut down"],
        )
    } else {
        ("systemctl", vec!["poweroff"])
    }
}

/// 立即关机（真正的系统调用，只在倒计时结束时由命令层调用）。
pub fn shutdown_now() -> Result<(), String> {
    let (program, args) = shutdown_command();
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let status = command
        .status()
        .map_err(|e| format!("无法执行关机命令：{e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("关机命令失败（{status}）"))
    }
}

// ---------------------------------------------------------------- 检查更新

/// 开源仓库（检查更新与“使用文档”入口）。
pub const REPO_URL: &str = "https://github.com/seansongx/ClipRecipe";
/// GitHub 公开 API：最新发布（不能自动更新时的检查方式）。
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/seansongx/ClipRecipe/releases/latest";
/// `tauri.conf.json` 中 `plugins.updater.pubkey` 的占位值。项目所有者按 docs/RELEASING.md 生成签名密钥并填入公钥之前，
/// 构建出的程序不做自动更新（只提示并打开发布页）。
pub const UPDATER_PUBKEY_PLACEHOLDER: &str =
    "REPLACE_WITH_UPDATER_PUBLIC_KEY (see docs/RELEASING.md)";

/// 检查更新的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    /// 最新发布的版本号（去掉开头的 v）
    pub latest: String,
    /// 是否比当前版本新
    pub newer: bool,
    /// 发布页地址
    pub url: String,
    /// 更新说明（发布说明；没有时为空）
    pub notes: Option<String>,
    /// 能否在程序内下载并安装（否则只能打开发布页）
    pub auto: bool,
    /// 不能自动更新的原因（`auto` 为 false 且有新版本时显示给用户）
    pub manual_reason: Option<String>,
}

/// 解析版本号 `v1.2.3` / `1.2.3-beta.1` → `[1, 2, 3]`（预发布部分忽略，缺少的段为 0）。
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let core = text
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    let mut parts = [0u64; 3];
    let mut count = 0;
    for (i, piece) in core.split('.').enumerate() {
        if i >= 3 {
            break;
        }
        parts[i] = piece.parse().ok()?;
        count += 1;
    }
    (count > 0).then_some(parts)
}

/// `latest` 是否比 `current` 新。无法解析时视为不新。
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// 只允许打开本项目的 GitHub 页面。
pub fn is_project_url(url: &str) -> bool {
    url == REPO_URL || url.starts_with(&format!("{REPO_URL}/"))
}

/// 某个版本的发布页（tag 统一为 `v<版本>`，见 docs/RELEASING.md）。
pub fn release_page(version: &str) -> String {
    format!(
        "{REPO_URL}/releases/tag/v{}",
        version.trim().trim_start_matches(['v', 'V'])
    )
}

/// 发布说明：去掉首尾空白，空的视为没有。
pub fn clean_notes(notes: Option<&str>) -> Option<String> {
    notes
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
}

/// 解析 GitHub “最新发布” API 的响应（不能自动更新时使用，结果的 `auto` 为 false）。
pub fn parse_release(json: &str, current: &str) -> Result<UpdateInfo, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("无法解析发布信息：{e}"))?;
    let tag = value["tag_name"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("发布信息中没有版本号")?;
    let url = value["html_url"]
        .as_str()
        .filter(|u| is_project_url(u))
        .map_or_else(|| format!("{REPO_URL}/releases/latest"), str::to_owned);
    let latest = tag.trim_start_matches(['v', 'V']).to_owned();
    Ok(UpdateInfo {
        current: current.to_owned(),
        newer: is_newer(&latest, current),
        latest,
        url,
        notes: clean_notes(value["body"].as_str()),
        auto: false,
        manual_reason: None,
    })
}

/// 访问 GitHub API 取最新发布（只发一个不带任何身份信息的 GET）。
/// `fetch(url) -> 响应文本` 由调用方提供：正式为 [`http_get`]，测试中替换。
pub fn check_update(
    current: &str,
    fetch: impl Fn(&str) -> Result<String, String>,
) -> Result<UpdateInfo, String> {
    let body = fetch(LATEST_RELEASE_API)?;
    parse_release(&body, current)
}

/// 最简单的 HTTPS GET：GitHub 要求带 User-Agent，这里只写程序名；10 秒超时；离线时返回错误。
/// 代理按环境变量（`HTTPS_PROXY` / `NO_PROXY` 等），与下载 FFmpeg 相同，见 [`crate::net`]。
pub fn http_get(url: &str) -> Result<String, String> {
    let response = crate::net::agent(url, crate::net::Timeouts::SHORT, true)
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "还没有发布任何版本".to_owned(),
            ureq::Error::Status(code, _) => format!("服务器返回 {code}"),
            ureq::Error::Transport(t) => format!("无法连接（{t}）"),
        })?;
    response
        .into_string()
        .map_err(|e| format!("读取响应失败：{e}"))
}

// ---------------------------------------------------------------- 自动更新（tauri-plugin-updater）

/// 当前程序的安装方式（打包工具写进程序里，见 `tauri::utils::platform::bundle_type`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Windows NSIS 安装包（发布的格式）
    Nsis,
    /// Windows MSI（不发布，但更新插件支持）
    Msi,
    /// Linux AppImage
    AppImage,
    /// Linux deb
    Deb,
    /// Linux rpm
    Rpm,
    /// macOS 应用包（不发布）
    MacApp,
    /// 没有经过打包：开发版、`cargo run`、直接运行编译出的可执行文件
    Unbundled,
}

impl InstallKind {
    pub fn current() -> Self {
        use tauri::utils::config::BundleType;
        match tauri::utils::platform::bundle_type() {
            Some(BundleType::Nsis) => Self::Nsis,
            Some(BundleType::Msi) => Self::Msi,
            Some(BundleType::AppImage) => Self::AppImage,
            Some(BundleType::Deb) => Self::Deb,
            Some(BundleType::Rpm) => Self::Rpm,
            Some(BundleType::App | BundleType::Dmg) => Self::MacApp,
            None => Self::Unbundled,
        }
    }
}

/// `plugins.updater.pubkey` 是否已经换成真正的公钥（不是空的，也不是 [`UPDATER_PUBKEY_PLACEHOLDER`]）。
pub fn pubkey_configured(pubkey: Option<&str>) -> bool {
    pubkey
        .map(str::trim)
        .is_some_and(|k| !k.is_empty() && k != UPDATER_PUBKEY_PLACEHOLDER)
}

/// 不能在程序内自动更新的原因；`None` 表示可以。
///
/// 未配置公钥的构建无法校验下载的更新包；deb / rpm 交给系统包管理器（更新插件虽能调用 dpkg / rpm，但需要
/// 管理员权限，发布流程也只把 AppImage 写进 `latest.json`）；没有打包的程序没有可替换的安装。
pub fn manual_update_reason(kind: InstallKind, pubkey_ok: bool) -> Option<&'static str> {
    if !pubkey_ok {
        return Some("这个版本没有配置更新签名公钥，不能在程序内自动更新");
    }
    match kind {
        InstallKind::Nsis | InstallKind::Msi | InstallKind::AppImage | InstallKind::MacApp => None,
        InstallKind::Deb | InstallKind::Rpm => {
            Some("deb / rpm 安装的版本请到发布页下载新的安装包（AppImage 版可以在程序内自动更新）")
        }
        InstallKind::Unbundled => {
            Some("当前程序不是用安装包安装的（开发版或直接运行的可执行文件），不能在程序内自动更新")
        }
    }
}

/// 自动更新的阶段（以后端为准，前端按命令结果与事件显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdatePhase {
    /// 没有可自动安装的更新
    #[default]
    Idle,
    /// 检查到可自动安装的新版本，等待用户确认
    Available,
    /// 正在下载（可取消）
    Downloading,
    /// 下载并校验完成，正在安装（随后重启，不能取消）
    Installing,
}

/// 自动更新的状态流转：检查 → 可安装 → 下载（可取消；失败回到可安装）→ 安装（失败回到可安装）。
#[derive(Debug, Default)]
pub struct UpdateFlow {
    phase: UpdatePhase,
}

impl UpdateFlow {
    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    /// 检查完成：`installable` 表示有可自动安装的新版本。下载或安装中不改变状态并返回 false
    /// （调用方不要替换正在安装的更新）。
    pub fn checked(&mut self, installable: bool) -> bool {
        match self.phase {
            UpdatePhase::Downloading | UpdatePhase::Installing => false,
            UpdatePhase::Idle | UpdatePhase::Available => {
                self.phase = if installable {
                    UpdatePhase::Available
                } else {
                    UpdatePhase::Idle
                };
                true
            }
        }
    }

    /// 开始下载：只能从“可安装”开始。
    pub fn start_download(&mut self) -> Result<(), String> {
        match self.phase {
            UpdatePhase::Available => {
                self.phase = UpdatePhase::Downloading;
                Ok(())
            }
            UpdatePhase::Downloading | UpdatePhase::Installing => Err("已经在更新了".into()),
            UpdatePhase::Idle => Err("没有可以安装的更新，请先检查更新".into()),
        }
    }

    /// 取消下载（安装开始后不能取消），返回是否取消了。
    pub fn cancel(&mut self) -> bool {
        if self.phase == UpdatePhase::Downloading {
            self.phase = UpdatePhase::Available;
            return true;
        }
        false
    }

    /// 下载结束：成功（含签名校验）进入安装，失败回到可安装。已被取消时返回 false，调用方不要继续安装。
    pub fn download_finished(&mut self, ok: bool) -> bool {
        if self.phase != UpdatePhase::Downloading {
            return false;
        }
        self.phase = if ok {
            UpdatePhase::Installing
        } else {
            UpdatePhase::Available
        };
        true
    }

    /// 安装失败：回到可安装（用户可以重试，或改为打开发布页）。
    pub fn install_failed(&mut self) {
        if self.phase == UpdatePhase::Installing {
            self.phase = UpdatePhase::Available;
        }
    }
}

/// 下载进度：累计字节数，并决定何时向界面报告（总大小已知时百分比每变 1 报告一次，未知时每 1 MiB 一次）。
#[derive(Debug, Default)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    last_step: Option<u64>,
}

impl DownloadProgress {
    const UNKNOWN_STEP: u64 = 1 << 20;

    /// 收到一块数据，返回是否应该报告。
    pub fn add(&mut self, chunk: usize, total: Option<u64>) -> bool {
        self.downloaded += chunk as u64;
        self.total = total.filter(|t| *t > 0);
        let step = match self.total {
            Some(total) => self.downloaded.min(total) * 100 / total,
            None => self.downloaded / Self::UNKNOWN_STEP,
        };
        if self.last_step == Some(step) {
            return false;
        }
        self.last_step = Some(step);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 更新地址的顺序：先 xiaofany.com（主线路），再 GitHub 的 `latest.json`（xiaofany 连不上或出错时的备用）。
    #[test]
    fn updater_tries_xiaofany_before_github() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let endpoints: Vec<&str> = conf["plugins"]["updater"]["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            endpoints,
            [
                "https://xiaofany.com/api/update/cliprecipe/tauri/{{target}}/{{arch}}/{{current_version}}",
                format!("{REPO_URL}/releases/latest/download/latest.json").as_str(),
            ]
        );
    }

    #[test]
    fn countdown_ticks_and_fires() {
        let ticks = Arc::new(Mutex::new(Vec::new()));
        let fired = Arc::new(AtomicBool::new(false));
        let (t, f) = (Arc::clone(&ticks), Arc::clone(&fired));
        let countdown = Countdown::start(
            3,
            Duration::from_millis(10),
            move |s| t.lock().unwrap().push(s),
            move || f.store(true, Ordering::SeqCst),
        );
        assert!(countdown.wait());
        assert!(fired.load(Ordering::SeqCst));
        assert_eq!(*ticks.lock().unwrap(), [3, 2, 1, 0]);
    }

    #[test]
    fn cancelled_countdown_never_fires() {
        let fired = Arc::new(AtomicBool::new(false));
        let f = Arc::clone(&fired);
        let countdown = Countdown::start(
            30,
            Duration::from_millis(20),
            |_| {},
            move || f.store(true, Ordering::SeqCst),
        );
        std::thread::sleep(Duration::from_millis(30));
        countdown.cancel();
        assert!(!countdown.wait());
        assert!(!fired.load(Ordering::SeqCst));

        // 丢弃也等于取消
        let f = Arc::clone(&fired);
        drop(Countdown::start(
            1,
            Duration::from_millis(50),
            |_| {},
            move || f.store(true, Ordering::SeqCst),
        ));
        std::thread::sleep(Duration::from_millis(120));
        assert!(!fired.load(Ordering::SeqCst));
    }

    #[test]
    fn shutdown_command_shape() {
        let (program, args) = shutdown_command();
        assert!(!program.is_empty() && !args.is_empty());
        if cfg!(windows) {
            assert_eq!((program, args), ("shutdown", vec!["/s", "/t", "0"]));
        }
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(parse_version("0.2"), Some([0, 2, 0]));
        assert_eq!(parse_version("1.0.0-beta.1"), Some([1, 0, 0]));
        assert_eq!(parse_version("abc"), None);
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn release_parsing_and_url_restriction() {
        let json = r#"{"tag_name":"v0.3.0","html_url":"https://github.com/seansongx/ClipRecipe/releases/tag/v0.3.0"}"#;
        let info = parse_release(json, "0.1.0").unwrap();
        assert_eq!(info.latest, "0.3.0");
        assert!(info.newer);
        assert!(info.url.ends_with("/tag/v0.3.0"));
        // 不是本项目的地址时改用固定的发布页
        let evil = r#"{"tag_name":"v9","html_url":"https://example.com/x"}"#;
        assert_eq!(
            parse_release(evil, "0.1.0").unwrap().url,
            format!("{REPO_URL}/releases/latest")
        );
        assert!(parse_release("{}", "0.1.0").is_err());
        assert!(is_project_url(REPO_URL));
        assert!(!is_project_url(
            "https://github.com/seansongx/ClipRecipe.evil.com"
        ));
    }

    #[test]
    fn check_update_uses_injected_fetch_and_reports_offline() {
        let ok = check_update("0.1.0", |url| {
            assert_eq!(url, LATEST_RELEASE_API);
            Ok(r#"{"tag_name":"0.1.0","html_url":"https://github.com/seansongx/ClipRecipe/releases/tag/0.1.0"}"#.into())
        })
        .unwrap();
        assert!(!ok.newer);
        let offline = check_update("0.1.0", |_| Err("无法连接".into()));
        assert_eq!(offline.unwrap_err(), "无法连接");
    }

    #[test]
    fn release_notes_and_pages() {
        let json = r#"{"tag_name":"v0.2.0","html_url":"https://github.com/seansongx/ClipRecipe/releases/tag/v0.2.0","body":"\n  ### 新增\n- 自动更新\n "}"#;
        let info = parse_release(json, "0.1.0").unwrap();
        assert_eq!(info.notes.as_deref(), Some("### 新增\n- 自动更新"));
        assert!(!info.auto, "GitHub API 的结果只用于打开发布页");
        assert_eq!(info.manual_reason, None);
        let blank = r#"{"tag_name":"v0.2.0","body":"   "}"#;
        assert_eq!(parse_release(blank, "0.1.0").unwrap().notes, None);
        assert_eq!(
            release_page("0.2.0"),
            format!("{REPO_URL}/releases/tag/v0.2.0")
        );
        assert_eq!(release_page("v0.2.0"), release_page("0.2.0"));
        assert!(is_project_url(&release_page("1.0.0")));
    }

    #[test]
    fn pubkey_placeholder_is_not_configured() {
        assert!(!pubkey_configured(None));
        assert!(!pubkey_configured(Some("")));
        assert!(!pubkey_configured(Some("  ")));
        assert!(!pubkey_configured(Some(UPDATER_PUBKEY_PLACEHOLDER)));
        assert!(!pubkey_configured(Some(&format!(
            " {UPDATER_PUBKEY_PLACEHOLDER}\n"
        ))));
        assert!(pubkey_configured(Some(
            "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk="
        )));
    }

    #[test]
    fn manual_fallback_decision() {
        use InstallKind::*;
        // 配置了公钥：安装包与 AppImage 可以自动更新，deb / rpm 与未打包的程序回退到发布页
        for kind in [Nsis, Msi, AppImage, MacApp] {
            assert_eq!(manual_update_reason(kind, true), None, "{kind:?}");
        }
        for kind in [Deb, Rpm, Unbundled] {
            assert!(manual_update_reason(kind, true).is_some(), "{kind:?}");
        }
        // 没有公钥时一律回退，并说明是公钥的原因
        for kind in [Nsis, Msi, AppImage, MacApp, Deb, Rpm, Unbundled] {
            assert!(manual_update_reason(kind, false).unwrap().contains("公钥"));
        }
        // 测试程序没有经过打包。macOS 上 Tauri 分辨不出是否打包，一律报告为应用包
        let expected = if cfg!(target_os = "macos") {
            MacApp
        } else {
            Unbundled
        };
        assert_eq!(InstallKind::current(), expected);
    }

    #[test]
    fn update_flow_happy_path() {
        let mut flow = UpdateFlow::default();
        assert_eq!(flow.phase(), UpdatePhase::Idle);
        assert!(flow.start_download().is_err(), "没有检查到更新时不能下载");
        assert!(flow.checked(true));
        assert_eq!(flow.phase(), UpdatePhase::Available);
        flow.start_download().unwrap();
        assert_eq!(flow.phase(), UpdatePhase::Downloading);
        assert!(flow.start_download().is_err(), "不能重复下载");
        assert!(!flow.checked(false), "下载中再次检查不改变状态");
        assert_eq!(flow.phase(), UpdatePhase::Downloading);
        assert!(flow.download_finished(true));
        assert_eq!(flow.phase(), UpdatePhase::Installing);
        assert!(!flow.cancel(), "安装开始后不能取消");
        assert!(!flow.checked(true));
        assert_eq!(flow.phase(), UpdatePhase::Installing);
    }

    #[test]
    fn update_flow_cancel_and_failures() {
        let mut flow = UpdateFlow::default();
        flow.checked(true);
        flow.start_download().unwrap();
        assert!(flow.cancel());
        assert_eq!(flow.phase(), UpdatePhase::Available);
        assert!(!flow.cancel());
        // 取消后才结束的下载不能继续安装
        assert!(!flow.download_finished(true));
        assert_eq!(flow.phase(), UpdatePhase::Available);
        // 下载或校验失败：回到可安装，可以重试
        flow.start_download().unwrap();
        assert!(flow.download_finished(false));
        assert_eq!(flow.phase(), UpdatePhase::Available);
        // 安装失败：回到可安装
        flow.start_download().unwrap();
        flow.download_finished(true);
        flow.install_failed();
        assert_eq!(flow.phase(), UpdatePhase::Available);
        // 再次检查发现已是最新：回到空闲
        assert!(flow.checked(false));
        assert_eq!(flow.phase(), UpdatePhase::Idle);
        flow.install_failed();
        assert_eq!(flow.phase(), UpdatePhase::Idle);
    }

    #[test]
    fn download_progress_reports_sparingly() {
        let mut p = DownloadProgress::default();
        assert!(p.add(10, Some(1000)), "第一块总要报告");
        assert!(!p.add(5, Some(1000)), "仍是 1%");
        assert!(p.add(5, Some(1000)));
        assert_eq!((p.downloaded, p.total), (20, Some(1000)));
        let reports = (0..98).filter(|_| p.add(10, Some(1000))).count();
        assert_eq!(reports, 98);
        assert_eq!(p.downloaded, 1000);
        // 超过声明的大小也不超过 100%
        assert!(!p.add(10, Some(1000)));

        let mut unknown = DownloadProgress::default();
        assert!(unknown.add(1000, None));
        assert!(!unknown.add(1000, Some(0)), "大小为 0 视为未知");
        assert_eq!(unknown.total, None);
        assert!(unknown.add(1 << 20, None));
    }
}
