//! 应用内下载 FFmpeg 的清单（`src-tauri/resources/ffmpeg-manifest.json`，编译时内嵌）。
//!
//! 清单由 `npm run sync-ffmpeg-manifest` 从开发依赖 `@seankit/ffmpeg`、`@seankit/ffprobe` 的 `manifest.json`
//! 合并生成并提交；`cargo test` 检查它与 node_modules 里安装的版本一致。程序只会下载这里列出的文件
//! （地址、大小、SHA-256 都固定在程序里），运行时不从网络获取清单。
//!
//! 每个工具（ffmpeg / ffprobe）的清单：`platforms`（键为 Node.js 风格的 `win32-x64` 等）列出 npm 包 tgz 的
//! `urls`（npmmirror 在前，然后是 npmjs，内容相同）、`size`、`sha256`，包内可执行文件 `binary`
//! （在 tgz 中位于 `package/` 下）及其 `binarySha256` / `binarySize`，对应的 Rust 目标三元组 `rustTargets`；
//! `aliases` 把没有单独构建的平台指向别的平台（如 `win32-arm64` → `win32-x64`）。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 内嵌的清单原文。
pub const EMBEDDED: &str = include_str!("../../resources/ffmpeg-manifest.json");

/// 编译目标三元组（build.rs 写入）。
pub const TARGET: &str = env!("CLIPRECIPE_TARGET");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub ffmpeg_version: String,
    pub package_version: String,
    pub tools: Tools,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tools {
    pub ffmpeg: ToolManifest,
    pub ffprobe: ToolManifest,
}

/// 一个 npm 包（`@seankit/ffmpeg` 或 `@seankit/ffprobe`）的 manifest.json。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolManifest {
    pub tool: String,
    pub ffmpeg_version: String,
    pub package_version: String,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    pub platforms: BTreeMap<String, PlatformEntry>,
}

/// 某个平台的 npm 包。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformEntry {
    pub package: String,
    pub version: String,
    #[serde(default)]
    pub file: String,
    /// tgz 的字节数
    pub size: u64,
    /// tgz 的 SHA-256（小写十六进制）
    pub sha256: String,
    #[serde(default)]
    pub integrity: String,
    /// 包内可执行文件的路径（tgz 中在 `package/` 下）
    pub binary: String,
    pub binary_sha256: String,
    pub binary_size: u64,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub rust_targets: Vec<String>,
    pub urls: Vec<String>,
}

impl PlatformEntry {
    /// tgz 中可执行文件的完整条目名。
    pub fn tar_entry(&self) -> String {
        format!("package/{}", self.binary)
    }
}

/// 哪些下载地址可以访问。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlPolicy {
    /// 正式：只允许 HTTPS（跳转到 http 也拒绝）。
    HttpsOnly,
    /// 只用于测试（单元测试的本机假服务器、带 `e2e-hooks` 特性的端到端测试构建）：
    /// 另外允许 `http://` 访问本机地址。正式构建里不存在这个取值。
    #[cfg(any(test, feature = "e2e-hooks"))]
    AllowLoopbackHttp,
}

impl UrlPolicy {
    pub fn check(self, url: &str) -> Result<(), String> {
        let Some((scheme, host)) = crate::net::scheme_and_host(url) else {
            return Err(format!("无效的下载地址：{url}"));
        };
        match scheme.as_str() {
            "https" => Ok(()),
            #[cfg(any(test, feature = "e2e-hooks"))]
            "http" if self == UrlPolicy::AllowLoopbackHttp && crate::net::is_loopback(&host) => {
                Ok(())
            }
            _ => {
                let _ = host;
                Err(format!("只允许 HTTPS 下载地址：{url}"))
            }
        }
    }

    pub fn https_only(self) -> bool {
        self == UrlPolicy::HttpsOnly
    }
}

/// 当前平台在清单中的条目（ffmpeg 与 ffprobe 必须是同一个平台）。
#[derive(Debug, Clone, Copy)]
pub struct Platform<'a> {
    pub key: &'a str,
    pub ffmpeg: &'a PlatformEntry,
    pub ffprobe: &'a PlatformEntry,
}

impl Platform<'_> {
    /// 两个 tgz 的总字节数。
    pub fn download_size(&self) -> u64 {
        self.ffmpeg.size + self.ffprobe.size
    }
}

/// Rust 的 `std::env::consts::{OS, ARCH}` → Node.js 风格的平台键（`win32-x64`、`darwin-arm64`…）。
pub fn platform_key(os: &str, arch: &str) -> String {
    let os = match os {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("{os}-{arch}")
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 包内路径必须是普通的相对路径（不含 `..`、绝对路径、反斜杠、盘符）。
pub fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// 版本号会作为目录名，只允许字母、数字与 `.-_`。
fn is_safe_version(version: &str) -> bool {
    !version.is_empty()
        && !version.starts_with('.')
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

impl ToolManifest {
    fn validate(&self, name: &str, version: &str, policy: UrlPolicy) -> Result<(), String> {
        if self.tool != name {
            return Err(format!("清单中 {name} 的 tool 字段为 {}", self.tool));
        }
        if self.ffmpeg_version != version {
            return Err(format!(
                "{name} 的 FFmpeg 版本（{}）与清单（{version}）不一致",
                self.ffmpeg_version
            ));
        }
        if self.platforms.is_empty() {
            return Err(format!("{name} 的清单没有任何平台"));
        }
        for (alias, target) in &self.aliases {
            if !self.platforms.contains_key(target) {
                return Err(format!("{name} 的别名 {alias} 指向不存在的平台 {target}"));
            }
        }
        for (key, entry) in &self.platforms {
            let what = format!("{name}（{key}）");
            if !is_sha256(&entry.sha256) || !is_sha256(&entry.binary_sha256) {
                return Err(format!("{what} 的 SHA-256 格式不正确"));
            }
            if entry.size == 0 || entry.binary_size == 0 {
                return Err(format!("{what} 的大小为 0"));
            }
            if !is_safe_relative(&entry.binary) {
                return Err(format!("{what} 的可执行文件路径不安全：{}", entry.binary));
            }
            let file = entry.binary.rsplit('/').next().unwrap_or_default();
            if file.trim_end_matches(".exe") != name {
                return Err(format!(
                    "{what} 的可执行文件名不是 {name}：{}",
                    entry.binary
                ));
            }
            if entry.urls.is_empty() {
                return Err(format!("{what} 没有下载地址"));
            }
            for url in &entry.urls {
                policy.check(url).map_err(|e| format!("{what}：{e}"))?;
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// 解析并校验清单。
    pub fn parse(text: &str, policy: UrlPolicy) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|e| format!("FFmpeg 下载清单无法解析：{e}"))?;
        manifest.validate(policy)?;
        Ok(manifest)
    }

    pub fn validate(&self, policy: UrlPolicy) -> Result<(), String> {
        if !is_safe_version(&self.ffmpeg_version) {
            return Err(format!(
                "清单中的 FFmpeg 版本号无效：{}",
                self.ffmpeg_version
            ));
        }
        self.tools
            .ffmpeg
            .validate("ffmpeg", &self.ffmpeg_version, policy)?;
        self.tools
            .ffprobe
            .validate("ffprobe", &self.ffmpeg_version, policy)?;
        Ok(())
    }

    /// 找当前平台的条目：先按编译目标三元组匹配 `rustTargets`，再按平台键，最后按别名。
    pub fn resolve(&self, target: &str, os: &str, arch: &str) -> Result<Platform<'_>, String> {
        let key = platform_key(os, arch);
        let find = |tool: &ToolManifest| -> Option<String> {
            if let Some((k, _)) = tool
                .platforms
                .iter()
                .find(|(_, e)| !target.is_empty() && e.rust_targets.iter().any(|t| t == target))
            {
                return Some(k.clone());
            }
            if tool.platforms.contains_key(&key) {
                return Some(key.clone());
            }
            tool.aliases
                .get(&key)
                .filter(|k| tool.platforms.contains_key(*k))
                .cloned()
        };
        let unsupported = || {
            format!(
                "当前平台（{key}）没有可下载的 FFmpeg。请自行安装 FFmpeg 并加入 PATH，或设置环境变量 CLIPRECIPE_FFMPEG 指向 ffmpeg 可执行文件"
            )
        };
        let (Some(a), Some(b)) = (find(&self.tools.ffmpeg), find(&self.tools.ffprobe)) else {
            return Err(unsupported());
        };
        if a != b {
            return Err(unsupported());
        }
        let (key, ffmpeg) = self
            .tools
            .ffmpeg
            .platforms
            .get_key_value(&a)
            .expect("已找到");
        let ffprobe = &self.tools.ffprobe.platforms[&b];
        Ok(Platform {
            key: key.as_str(),
            ffmpeg,
            ffprobe,
        })
    }

    /// 本程序所在平台的条目。
    pub fn current_platform(&self) -> Result<Platform<'_>, String> {
        self.resolve(TARGET, std::env::consts::OS, std::env::consts::ARCH)
    }
}

/// 内嵌的清单（解析一次）。
pub fn embedded() -> Result<&'static Manifest, String> {
    static CACHE: OnceLock<Result<Manifest, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| Manifest::parse(EMBEDDED, UrlPolicy::HttpsOnly))
        .as_ref()
        .map_err(Clone::clone)
}

/// 端到端测试：用这个环境变量指定的清单文件代替内嵌清单（允许本机 http）。只在 `e2e-hooks` 构建中存在。
#[cfg(feature = "e2e-hooks")]
pub const E2E_MANIFEST_ENV: &str = "CLIPRECIPE_E2E_FFMPEG_MANIFEST";

/// 实际使用的清单与地址策略。正式构建总是内嵌清单 + 只允许 HTTPS。
pub fn active() -> Result<(&'static Manifest, UrlPolicy), String> {
    #[cfg(feature = "e2e-hooks")]
    {
        static OVERRIDE: OnceLock<Option<Result<Manifest, String>>> = OnceLock::new();
        let loaded = OVERRIDE.get_or_init(|| {
            let path = std::env::var_os(E2E_MANIFEST_ENV).filter(|v| !v.is_empty())?;
            Some(
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("无法读取测试清单：{e}"))
                    .and_then(|text| Manifest::parse(&text, UrlPolicy::AllowLoopbackHttp)),
            )
        });
        if let Some(result) = loaded {
            return result
                .as_ref()
                .map(|m| (m, UrlPolicy::AllowLoopbackHttp))
                .map_err(Clone::clone);
        }
    }
    embedded().map(|m| (m, UrlPolicy::HttpsOnly))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_is_valid_and_https_only() {
        let m = embedded().unwrap();
        assert_eq!(m.tools.ffmpeg.ffmpeg_version, m.ffmpeg_version);
        for tool in [&m.tools.ffmpeg, &m.tools.ffprobe] {
            for entry in tool.platforms.values() {
                assert!(entry.urls.iter().all(|u| u.starts_with("https://")));
                // npmmirror 在前
                assert!(entry.urls[0].contains("npmmirror"), "{:?}", entry.urls);
            }
        }
    }

    #[test]
    fn platform_resolution_uses_targets_keys_and_aliases() {
        let m = embedded().unwrap();
        let win = m
            .resolve("x86_64-pc-windows-msvc", "windows", "x86_64")
            .unwrap();
        assert_eq!(win.key, "win32-x64");
        assert_eq!(win.ffmpeg.binary, "bin/ffmpeg.exe");
        assert_eq!(win.ffprobe.binary, "bin/ffprobe.exe");
        assert_eq!(win.download_size(), win.ffmpeg.size + win.ffprobe.size);
        // Windows on ARM：别名 / 目标三元组都指向 x64 构建
        assert_eq!(
            m.resolve("", "windows", "aarch64").unwrap().key,
            "win32-x64"
        );
        assert_eq!(
            m.resolve("aarch64-pc-windows-msvc", "windows", "aarch64")
                .unwrap()
                .key,
            "win32-x64"
        );
        assert_eq!(
            m.resolve("aarch64-apple-darwin", "macos", "aarch64")
                .unwrap()
                .key,
            "darwin-arm64"
        );
        assert_eq!(
            m.resolve("x86_64-unknown-linux-gnu", "linux", "x86_64")
                .unwrap()
                .key,
            "linux-x64"
        );
        // musl 目标不在清单里，按平台键找到 linux-x64
        assert_eq!(
            m.resolve("x86_64-unknown-linux-musl", "linux", "x86_64")
                .unwrap()
                .key,
            "linux-x64"
        );
        let err = m
            .resolve("riscv64gc-unknown-linux-gnu", "linux", "riscv64")
            .unwrap_err();
        assert!(
            err.contains("linux-riscv64") && err.contains("CLIPRECIPE_FFMPEG"),
            "{err}"
        );
        assert!(m.current_platform().is_ok(), "本机平台应在清单中");
    }

    fn sample() -> serde_json::Value {
        serde_json::from_str(EMBEDDED).unwrap()
    }

    #[test]
    fn manifest_validation_rejects_unsafe_content() {
        type Mutation = fn(&mut serde_json::Value);
        let cases: [(&str, Mutation); 7] = [
            ("http 地址", |v| {
                v["tools"]["ffmpeg"]["platforms"]["win32-x64"]["urls"][0] =
                    "http://registry.npmmirror.com/x.tgz".into()
            }),
            ("路径穿越", |v| {
                v["tools"]["ffmpeg"]["platforms"]["win32-x64"]["binary"] = "../evil.exe".into()
            }),
            ("绝对路径", |v| {
                v["tools"]["ffprobe"]["platforms"]["linux-x64"]["binary"] = "/bin/ffprobe".into()
            }),
            ("错误的哈希", |v| {
                v["tools"]["ffmpeg"]["platforms"]["win32-x64"]["sha256"] = "ABC".into()
            }),
            ("版本号做目录名", |v| {
                v["ffmpegVersion"] = "../9".into()
            }),
            ("工具版本不一致", |v| {
                v["tools"]["ffprobe"]["ffmpegVersion"] = "1.0".into()
            }),
            ("别名指向不存在的平台", |v| {
                v["tools"]["ffmpeg"]["aliases"]["win32-arm64"] = "nope".into()
            }),
        ];
        for (what, mutate) in cases {
            let mut v = sample();
            mutate(&mut v);
            assert!(
                Manifest::parse(&v.to_string(), UrlPolicy::HttpsOnly).is_err(),
                "{what} 应被拒绝"
            );
        }
        assert!(Manifest::parse("{", UrlPolicy::HttpsOnly).is_err());
        assert!(Manifest::parse(EMBEDDED, UrlPolicy::HttpsOnly).is_ok());
    }

    #[test]
    fn url_policy() {
        assert!(UrlPolicy::HttpsOnly.check("https://a.org/x").is_ok());
        assert!(UrlPolicy::HttpsOnly.check("http://127.0.0.1:1/x").is_err());
        assert!(UrlPolicy::HttpsOnly.check("ftp://a.org/x").is_err());
        assert!(UrlPolicy::AllowLoopbackHttp
            .check("http://127.0.0.1:1/x")
            .is_ok());
        assert!(UrlPolicy::AllowLoopbackHttp
            .check("http://example.com/x")
            .is_err());
        assert!(is_safe_relative("bin/ffmpeg.exe"));
        for bad in ["", "/x", "a/../b", "a\\b", "C:/x", "./x", "a//b"] {
            assert!(!is_safe_relative(bad), "{bad}");
        }
    }

    /// 提交的清单必须与 node_modules 里安装的 @seankit/ffmpeg、@seankit/ffprobe 一致
    /// （升级 npm 包后忘了运行 `npm run sync-ffmpeg-manifest` 时失败）。
    #[test]
    fn committed_manifest_matches_installed_npm_packages() {
        let modules =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules/@seankit");
        if !modules.join("ffmpeg/manifest.json").is_file() {
            println!(
                "跳过：{} 下没有 @seankit/ffmpeg（先运行 npm install），无法检查清单是否同步",
                modules.display()
            );
            return;
        }
        let committed = sample();
        for tool in ["ffmpeg", "ffprobe"] {
            let path = modules.join(tool).join("manifest.json");
            let installed: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("无法读取 {}：{e}", path.display())),
            )
            .unwrap();
            assert_eq!(
                committed["tools"][tool], installed,
                "src-tauri/resources/ffmpeg-manifest.json 与已安装的 @seankit/{tool} 不一致，请运行 npm run sync-ffmpeg-manifest"
            );
        }
        assert_eq!(
            committed["ffmpegVersion"],
            committed["tools"]["ffmpeg"]["ffmpegVersion"]
        );
    }
}
