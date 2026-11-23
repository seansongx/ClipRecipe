//! 与操作系统打交道的功能：全部完成后关机的倒计时（`set.after-all`）、检查更新（`set.auto-update`）。
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
/// GitHub 公开 API：最新发布。
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/seansongx/ClipRecipe/releases/latest";

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

/// 解析 GitHub “最新发布” API 的响应。
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
pub fn http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", "ClipRecipe")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
}
