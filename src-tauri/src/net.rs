//! 联网功能（检查更新、下载 FFmpeg）共用的 HTTP 客户端设置：代理与超时。
//!
//! **代理**：按惯例读取环境变量——`https://` 用 `HTTPS_PROXY`（再退到 `ALL_PROXY`），`http://` 用 `HTTP_PROXY`
//! （再退到 `ALL_PROXY`），大小写均可；`NO_PROXY` 中列出的主机（`*`、精确主机名、`.example.com` / `example.com`
//! 后缀）直连；本机地址（localhost、127.x、::1）总是直连。只支持 HTTP 代理（ureq 未启用 SOCKS），
//! 代理地址无法识别时直连。Windows 的“系统代理”（Internet 选项）不读取。
//!
//! 请求只带固定的 `User-Agent: ClipRecipe`，不带任何用户或设备标识。

use std::time::Duration;

/// 请求的 User-Agent（只写程序名）。
pub const USER_AGENT: &str = "ClipRecipe";

/// 超时设置。`read` 是“多久没有收到任何数据”就失败（下载卡住时据此报错），不是整个请求的总时长。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub connect: Duration,
    pub read: Duration,
}

impl Timeouts {
    /// 小请求（检查更新）：连接与读取都 10 秒。
    pub const SHORT: Self = Self {
        connect: Duration::from_secs(10),
        read: Duration::from_secs(10),
    };
    /// 大文件下载：连接 15 秒，30 秒收不到数据即判定卡住。
    pub const DOWNLOAD: Self = Self {
        connect: Duration::from_secs(15),
        read: Duration::from_secs(30),
    };
}

/// URL 的协议与主机（小写，不含端口与用户信息；IPv6 去掉方括号）。
pub fn scheme_and_host(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or_default();
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

/// 是否是本机地址。
pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn no_proxy_matches(host: &str, list: &str) -> bool {
    list.split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            // 去掉端口（IPv6 条目原样比较）
            let entry = if entry.matches(':').count() == 1 {
                entry.split(':').next().unwrap_or_default().to_owned()
            } else {
                entry.trim_matches(['[', ']']).to_owned()
            };
            let suffix = entry.trim_start_matches("*.").trim_start_matches('.');
            host == suffix || host.ends_with(&format!(".{suffix}"))
        })
}

/// 按环境变量决定 `url` 使用的代理地址（`None` 为直连）。`env` 读取环境变量（测试中替换）。
pub fn proxy_for(url: &str, env: impl Fn(&str) -> Option<String>) -> Option<String> {
    let var = |name: &str| {
        env(name)
            .or_else(|| env(&name.to_ascii_lowercase()))
            .filter(|v| !v.trim().is_empty())
    };
    let (scheme, host) = scheme_and_host(url)?;
    if is_loopback(&host) {
        return None;
    }
    if var("NO_PROXY").is_some_and(|list| no_proxy_matches(&host, &list)) {
        return None;
    }
    let specific = match scheme.as_str() {
        "https" => var("HTTPS_PROXY"),
        "http" => var("HTTP_PROXY"),
        _ => None,
    };
    specific.or_else(|| var("ALL_PROXY"))
}

/// 为 `url` 建立客户端：代理（见模块说明）、超时、User-Agent。`https_only` 时拒绝跳转到 http。
pub fn agent(url: &str, timeouts: Timeouts, https_only: bool) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(timeouts.connect)
        .timeout_read(timeouts.read)
        .timeout_write(timeouts.read)
        .user_agent(USER_AGENT)
        .https_only(https_only);
    if let Some(proxy) = proxy_for(url, |name| std::env::var(name).ok()) {
        if let Ok(proxy) = ureq::Proxy::new(proxy) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn hosts_are_parsed() {
        assert_eq!(
            scheme_and_host("https://user:pw@Registry.NPMjs.org:443/a?b"),
            Some(("https".into(), "registry.npmjs.org".into()))
        );
        assert_eq!(
            scheme_and_host("http://[::1]:8080/x"),
            Some(("http".into(), "::1".into()))
        );
        assert_eq!(scheme_and_host("no-scheme"), None);
        assert!(is_loopback("127.0.0.1") && is_loopback("127.5.0.1") && is_loopback("::1"));
        assert!(!is_loopback("10.0.0.1") && !is_loopback("localhost.evil.com"));
    }

    #[test]
    fn proxy_selection_follows_environment_conventions() {
        let e = env(&[
            ("HTTPS_PROXY", "http://127.0.0.1:7890"),
            ("http_proxy", "http://p:1"),
            (
                "NO_PROXY",
                "localhost,.internal.example, mirror.example:443",
            ),
        ]);
        assert_eq!(
            proxy_for("https://registry.npmjs.org/x", &e).as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            proxy_for("http://example.com/", &e).as_deref(),
            Some("http://p:1")
        );
        assert_eq!(proxy_for("https://a.internal.example/x", &e), None);
        assert_eq!(proxy_for("https://mirror.example/x", &e), None);
        // 本机地址总是直连
        assert_eq!(proxy_for("http://127.0.0.1:5000/x", &e), None);
        assert_eq!(proxy_for("http://localhost:5000/x", &e), None);
        // ALL_PROXY 兜底；NO_PROXY=* 全部直连
        let all = env(&[("ALL_PROXY", "http://all:3")]);
        assert_eq!(
            proxy_for("https://x.org", &all).as_deref(),
            Some("http://all:3")
        );
        let star = env(&[("HTTPS_PROXY", "http://p:1"), ("no_proxy", "*")]);
        assert_eq!(proxy_for("https://x.org", &star), None);
        assert_eq!(proxy_for("https://x.org", env(&[])), None);
    }
}
