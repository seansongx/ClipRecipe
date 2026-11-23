//! 构建期间模块登记的临时文件 / 目录（见 [`BuildContext::temp_file`](super::BuildContext::temp_file)、
//! [`BuildContext::register_temp`](super::BuildContext::register_temp)）。
//!
//! - 每次构建最多一个临时目录 `<系统临时目录>/cliprecipe-build/<进程号>-<序号>`，第一次需要时才创建；
//! - 登记的路径归 [`TempFiles`] 所有：构建失败时随构建上下文一起删除；构建成功时转交给
//!   [`BuiltCommand::temp_files`](super::BuiltCommand::temp_files)，最后一个持有者（调度器的一轮任务、
//!   一次预览、一次“查看命令”）结束时删除——不论成功、失败、取消还是 panic；
//! - 调度器启动时清理其它进程留下、超过一天的残留（异常退出时可能出现）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 临时目录名：`<系统临时目录>/cliprecipe-build`。
pub const TEMP_DIR_NAME: &str = "cliprecipe-build";
/// 超过这个时间的其它进程残留会被清理。
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);

/// 全部构建临时目录的上级目录。
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join(TEMP_DIR_NAME)
}

/// 本进程的目录名前缀（`<进程号>-`）。
fn own_prefix() -> String {
    format!("{}-", std::process::id())
}

/// 一次构建登记的临时路径；丢弃时删除（目录整体删除）。
#[derive(Debug, Default)]
pub struct TempFiles {
    paths: Vec<PathBuf>,
    /// 本次构建的临时目录（已创建时）
    dir: Option<PathBuf>,
}

impl TempFiles {
    /// 登记一个路径（文件或目录）。
    pub fn register(&mut self, path: PathBuf) {
        if !self.paths.contains(&path) {
            self.paths.push(path);
        }
    }

    /// 本次构建的临时目录，第一次调用时创建并登记。
    pub fn dir(&mut self) -> Result<&Path, String> {
        if self.dir.is_none() {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = temp_root().join(format!("{}{n}", own_prefix()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("无法创建临时目录 {}：{e}", dir.display()))?;
            self.register(dir.clone());
            self.dir = Some(dir);
        }
        Ok(self.dir.as_deref().unwrap_or(Path::new("")))
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// 构建结果持有的临时文件（可克隆；最后一个克隆丢弃时删除）。
#[derive(Debug, Clone, Default)]
pub struct TempGuard(Arc<TempFiles>);

impl TempGuard {
    pub fn new(files: TempFiles) -> Self {
        Self(Arc::new(files))
    }

    /// 登记的全部路径。
    pub fn paths(&self) -> &[PathBuf] {
        self.0.paths()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl PartialEq for TempGuard {
    fn eq(&self, other: &Self) -> bool {
        self.paths() == other.paths()
    }
}

/// 删除其它进程留下、超过 `max_age` 的构建临时目录。
pub fn cleanup_stale(max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(temp_root()) else {
        return;
    };
    let own = own_prefix();
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&own) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if old {
            let path = entry.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_paths_are_removed_with_the_last_holder() {
        let mut files = TempFiles::default();
        let dir = files.dir().unwrap().to_path_buf();
        assert!(dir.is_dir());
        assert!(dir.starts_with(temp_root()));
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        // 目录只创建一次
        assert_eq!(files.dir().unwrap(), dir);
        let outside = std::env::temp_dir().join(format!("cr-temp-unit-{}.txt", std::process::id()));
        std::fs::write(&outside, "x").unwrap();
        files.register(outside.clone());
        files.register(outside.clone());
        assert_eq!(files.paths(), [dir.clone(), outside.clone()]);

        let guard = TempGuard::new(files);
        let copy = guard.clone();
        assert_eq!(guard, copy);
        drop(guard);
        assert!(dir.is_dir() && outside.is_file(), "还有持有者时不删除");
        drop(copy);
        assert!(!dir.exists() && !outside.exists());
    }

    #[test]
    fn stale_leftovers_of_other_processes_are_removed() {
        std::fs::create_dir_all(temp_root()).unwrap();
        let pid = std::process::id();
        // 不可能的进程号 0：一个两天前的残留、一个刚生成的（可能是另一个正在运行的进程）
        let stale = temp_root().join(format!("0-stale-{pid}"));
        let fresh = temp_root().join(format!("0-fresh-{pid}"));
        std::fs::write(&stale, "x").unwrap();
        std::fs::write(&fresh, "x").unwrap();
        let two_days = std::time::SystemTime::now() - Duration::from_secs(2 * 24 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(two_days)
            .unwrap();
        cleanup_stale(STALE_AFTER);
        assert!(!stale.exists());
        assert!(fresh.exists());
        std::fs::remove_file(fresh).unwrap();
    }

    #[test]
    fn nothing_is_created_until_needed() {
        let files = TempFiles::default();
        assert!(files.is_empty());
        assert!(TempGuard::new(files).paths().is_empty());
    }
}
