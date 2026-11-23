//! 任务成功后的收尾：输出移回源位置（`set.move-back`）、删除源视频（`set.delete-source`）。
//!
//! 安全规则（任何一步失败都不会丢数据）：
//! - 只在任务的**全部**输出都已写完（调度器已把临时文件改名为最终文件）之后执行；失败 / 取消的任务不会进入这里；
//! - 先校验每个输出都存在且大小不为 0，校验不通过时什么也不做；
//! - 任何输出移动失败时不删除源文件；
//! - 永远不会删除与某个输出路径相同的源文件；移回源位置时也不会覆盖源文件本身——
//!   只有同时开启了“删除源视频”，才会在源文件被删除（默认移入回收站）**之后**用输出占用它的名字；
//! - 删除源文件默认移入回收站（`delete_to_trash`），可改为永久删除。
//!
//! 文件系统以外的副作用（回收站）通过 [`Disposer`] 注入，测试中可替换。

use std::path::{Path, PathBuf};

use super::output::same_path;
use super::LogLevel;
use crate::fsutil;
use crate::model::{OnExists, Settings};

/// 删除源文件的方式（移入回收站或永久删除）。
pub type Disposer<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// 按设置删除：移入回收站，或永久删除。
pub fn default_dispose(path: &Path, to_trash: bool) -> Result<(), String> {
    if to_trash {
        trash::delete(path).map_err(|e| format!("无法移入回收站：{e}"))
    } else {
        std::fs::remove_file(path).map_err(|e| format!("无法删除：{e}"))
    }
}

/// 收尾的结果：最终的输出路径与日志。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinishReport {
    pub outputs: Vec<String>,
    pub logs: Vec<(LogLevel, String)>,
    /// 源文件已被删除（或移入回收站）
    pub source_removed: bool,
}

impl FinishReport {
    fn log(&mut self, level: LogLevel, message: impl Into<String>) {
        self.logs.push((level, message.into()));
    }
}

/// 输出文件是否完整：存在、是文件、大小不为 0。
fn verified(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// `dir` 下与 `name` 不冲突的文件名：`名称 (n).扩展名`。`taken` 判断某个路径是否不能用。
fn renamed(dir: &Path, name: &Path, taken: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let stem = name.file_stem()?.to_string_lossy().into_owned();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !taken(p))
}

/// 对一个成功的任务执行收尾。`outputs` 为调度器给出的全部最终输出。
pub fn finish_task(
    source: &Path,
    outputs: &[String],
    settings: &Settings,
    dispose: Disposer,
) -> FinishReport {
    let mut report = FinishReport {
        outputs: outputs.to_vec(),
        ..FinishReport::default()
    };
    if !settings.move_back && !settings.delete_source {
        return report;
    }
    if outputs.is_empty() {
        return report;
    }
    if let Some(bad) = outputs.iter().find(|o| !verified(Path::new(o))) {
        report.log(
            LogLevel::Error,
            format!("输出文件不完整（不存在或大小为 0）：{bad}。已跳过移回源位置 / 删除源视频"),
        );
        return report;
    }

    let mut move_failed = false;
    // 想要占用源文件名字的输出：等源文件删除后再移过去
    let mut replace_source: Option<usize> = None;
    if settings.move_back {
        let Some(dir) = source.parent() else {
            report.log(LogLevel::Error, "无法确定源文件所在目录，未移回");
            return report;
        };
        for i in 0..report.outputs.len() {
            let current = PathBuf::from(&report.outputs[i]);
            let Some(name) = current.file_name() else {
                continue;
            };
            let desired = dir.join(name);
            if same_path(&desired, &current) {
                continue;
            }
            let target = if same_path(&desired, source) {
                if settings.delete_source && replace_source.is_none() {
                    replace_source = Some(i);
                    continue;
                }
                renamed(dir, Path::new(name), |p| p.exists() || same_path(p, source))
            } else if desired.exists() && settings.on_exists == OnExists::Rename {
                renamed(dir, Path::new(name), |p| p.exists() || same_path(p, source))
            } else {
                Some(desired)
            };
            let Some(target) = target else {
                report.log(
                    LogLevel::Error,
                    format!("无法为 {} 找到可用的文件名", current.display()),
                );
                move_failed = true;
                continue;
            };
            match fsutil::move_file(&current, &target) {
                Ok(()) => {
                    report.log(
                        LogLevel::Info,
                        format!("已移回源位置：{}", target.display()),
                    );
                    report.outputs[i] = target.to_string_lossy().into_owned();
                }
                Err(e) => {
                    move_failed = true;
                    report.log(
                        LogLevel::Error,
                        format!(
                            "无法移回源位置（{}）：{e}；输出保留在 {}",
                            target.display(),
                            current.display()
                        ),
                    );
                }
            }
        }
    }

    if !settings.delete_source {
        return report;
    }
    if move_failed {
        report.log(
            LogLevel::Warn,
            format!("有输出未能移回源位置，未删除源视频：{}", source.display()),
        );
        return report;
    }
    if report
        .outputs
        .iter()
        .any(|o| same_path(Path::new(o), source))
    {
        report.log(
            LogLevel::Warn,
            format!("输出与源视频是同一个文件，未删除：{}", source.display()),
        );
        return report;
    }
    if !source.is_file() {
        report.log(
            LogLevel::Warn,
            format!("源视频已不存在：{}", source.display()),
        );
        return report;
    }
    match dispose(source) {
        Ok(()) => {
            report.source_removed = true;
            let how = if settings.delete_to_trash {
                "已移入回收站"
            } else {
                "已删除"
            };
            report.log(LogLevel::Info, format!("源视频{how}：{}", source.display()));
        }
        Err(e) => {
            report.log(
                LogLevel::Error,
                format!("删除源视频失败（{}）：{e}", source.display()),
            );
            return report;
        }
    }
    if let Some(i) = replace_source {
        let current = PathBuf::from(&report.outputs[i]);
        match fsutil::move_file(&current, source) {
            Ok(()) => {
                report.log(
                    LogLevel::Info,
                    format!("已移回源位置：{}", source.display()),
                );
                report.outputs[i] = source.to_string_lossy().into_owned();
            }
            Err(e) => report.log(
                LogLevel::Error,
                format!(
                    "无法移回源位置（{}）：{e}；输出保留在 {}",
                    source.display(),
                    current.display()
                ),
            ),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("cr-finish-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("out")).unwrap();
        Dir(d)
    }

    fn write(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// 记录并真的删除（模拟回收站）。
    fn remove_and_record(calls: &RefCell<Vec<PathBuf>>, p: &Path) -> Result<(), String> {
        calls.borrow_mut().push(p.to_path_buf());
        std::fs::remove_file(p).map_err(|e| e.to_string())
    }

    #[test]
    fn nothing_happens_when_disabled() {
        let d = dir("off");
        let src = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&out, "output");
        let never = |_: &Path| -> Result<(), String> { panic!("不应删除") };
        let r = finish_task(
            &src,
            &[out.to_string_lossy().into()],
            &Settings::default(),
            &never,
        );
        assert_eq!(r.outputs, [out.to_string_lossy()]);
        assert!(src.exists() && out.exists());
    }

    #[test]
    fn delete_source_after_verified_output() {
        let d = dir("delete");
        let src = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&out, "output");
        let settings = Settings {
            delete_source: true,
            ..Settings::default()
        };
        let calls = RefCell::new(Vec::new());
        let dispose = |p: &Path| remove_and_record(&calls, p);
        let r = finish_task(&src, &[out.to_string_lossy().into()], &settings, &dispose);
        assert!(r.source_removed);
        assert_eq!(*calls.borrow(), [src.as_path()]);
        assert!(!src.exists() && out.exists());
    }

    #[test]
    fn empty_or_missing_output_keeps_source() {
        let d = dir("empty");
        let src = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&out, "");
        let settings = Settings {
            delete_source: true,
            move_back: true,
            ..Settings::default()
        };
        let never = |_: &Path| -> Result<(), String> { panic!("不应删除") };
        let r = finish_task(&src, &[out.to_string_lossy().into()], &settings, &never);
        assert!(!r.source_removed);
        assert_eq!(read(&src), "source");
        assert!(out.exists(), "不完整的输出也不移动");
        assert_eq!(r.logs[0].0, LogLevel::Error);
        let missing = d.0.join("out").join("missing.mp4");
        let r = finish_task(&src, &[missing.to_string_lossy().into()], &settings, &never);
        assert!(!r.source_removed && src.exists());
    }

    #[test]
    fn dispose_failure_keeps_everything() {
        let d = dir("fail");
        let src = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&out, "output");
        let settings = Settings {
            delete_source: true,
            move_back: true,
            ..Settings::default()
        };
        let failing = |_: &Path| -> Result<(), String> { Err("回收站不可用".into()) };
        let r = finish_task(&src, &[out.to_string_lossy().into()], &settings, &failing);
        assert!(!r.source_removed);
        assert_eq!(read(&src), "source");
        // 输出名与源文件同名：源文件没删掉，所以输出仍在输出目录，没有覆盖源文件
        assert_eq!(r.outputs, [out.to_string_lossy()]);
        assert_eq!(read(&out), "output");
    }

    #[test]
    fn move_back_renames_or_overwrites_but_never_the_source() {
        let d = dir("move");
        let src = d.0.join("src").join("a.mkv");
        let other = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&other, "old");
        write(&out, "output");
        let rename = Settings {
            move_back: true,
            ..Settings::default()
        };
        let never = |_: &Path| -> Result<(), String> { panic!("不应删除") };
        let r = finish_task(&src, &[out.to_string_lossy().into()], &rename, &never);
        let moved = d.0.join("src").join("a (1).mp4");
        assert_eq!(r.outputs, [moved.to_string_lossy()]);
        assert_eq!(read(&moved), "output");
        assert_eq!(read(&other), "old");
        assert!(!out.exists());

        write(&out, "output 2");
        let overwrite = Settings {
            on_exists: OnExists::Overwrite,
            ..rename.clone()
        };
        let r = finish_task(&src, &[out.to_string_lossy().into()], &overwrite, &never);
        assert_eq!(r.outputs, [other.to_string_lossy()]);
        assert_eq!(read(&other), "output 2");
        assert_eq!(read(&src), "source");

        // 与源文件同名（同扩展名）：即使是覆盖模式，也不覆盖源文件
        let same = d.0.join("out").join("a.mkv");
        write(&same, "output 3");
        let r = finish_task(&src, &[same.to_string_lossy().into()], &overwrite, &never);
        assert_eq!(
            r.outputs,
            [d.0.join("src").join("a (1).mkv").to_string_lossy()]
        );
        assert_eq!(read(&src), "source");
    }

    #[test]
    fn move_back_with_delete_takes_over_the_source_name() {
        let d = dir("replace");
        let src = d.0.join("src").join("a.mp4");
        let out = d.0.join("out").join("a.mp4");
        write(&src, "source");
        write(&out, "output");
        let settings = Settings {
            move_back: true,
            delete_source: true,
            ..Settings::default()
        };
        let calls = RefCell::new(Vec::new());
        let dispose = |p: &Path| remove_and_record(&calls, p);
        let r = finish_task(&src, &[out.to_string_lossy().into()], &settings, &dispose);
        assert!(r.source_removed);
        assert_eq!(r.outputs, [src.to_string_lossy()]);
        assert_eq!(read(&src), "output");
        assert!(!out.exists());
    }

    #[test]
    fn output_equal_to_source_is_never_deleted() {
        let d = dir("same");
        let src = d.0.join("src").join("a.mp4");
        write(&src, "source");
        let settings = Settings {
            delete_source: true,
            ..Settings::default()
        };
        let never = |_: &Path| -> Result<(), String> { panic!("不应删除") };
        let r = finish_task(&src, &[src.to_string_lossy().into()], &settings, &never);
        assert!(!r.source_removed);
        assert_eq!(read(&src), "source");
    }

    #[test]
    fn rename_helper() {
        let taken = |p: &Path| p.ends_with("x (1).mp4");
        assert_eq!(
            renamed(Path::new("/d"), Path::new("x.mp4"), taken),
            Some(Path::new("/d").join("x (2).mp4"))
        );
    }
}
