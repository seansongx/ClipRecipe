//! 文件系统小工具：按扩展名列目录、自然排序。

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// 常见视频扩展名（小写，不带点）。前端 `VIDEO_EXTENSIONS` 与此一致。
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "flv", "wmv", "webm", "ts", "m4v", "mpg", "mpeg", "3gp",
];

/// 扩展名（忽略大小写）是否在列表里；列表为空表示不限。
pub fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    if extensions.is_empty() {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| extensions.iter().any(|x| x.eq_ignore_ascii_case(ext)))
}

/// 列出目录下符合扩展名的文件，按自然顺序排序；`recursive` 时包含子目录。
/// 读不了的子目录会被跳过。
pub fn list_files(
    dir: &Path,
    recursive: bool,
    extensions: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(dir, recursive, extensions, &mut out, true)?;
    Ok(out)
}

fn walk(
    dir: &Path,
    recursive: bool,
    extensions: &[&str],
    out: &mut Vec<PathBuf>,
    top: bool,
) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if top => return Err(e),
        Err(_) => return Ok(()),
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            dirs.push(path);
        } else if has_extension(&path, extensions) {
            files.push(path);
        }
    }
    files.sort_by(|a, b| natural_cmp_path(a, b));
    out.extend(files);
    if recursive {
        dirs.sort_by(|a, b| natural_cmp_path(a, b));
        for sub in dirs {
            walk(&sub, recursive, extensions, out, false)?;
        }
    }
    Ok(())
}

/// 移动文件：先尝试改名（同一分区），失败时（如跨分区）复制后删除源文件。
/// 目标已存在时被替换。
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            if !from.is_file() {
                return Err(rename_error);
            }
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

fn natural_cmp_path(a: &Path, b: &Path) -> Ordering {
    natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())
}

/// 自然排序：`2.mp4` 排在 `10.mp4` 前面；字母忽略大小写。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na: String = std::iter::from_fn(|| a.next_if(char::is_ascii_digit)).collect();
                let nb: String = std::iter::from_fn(|| b.next_if(char::is_ascii_digit)).collect();
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["10.mp4", "2.mp4", "a1.mp4", "A02.mp4", "1.mp4"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["1.mp4", "2.mp4", "10.mp4", "a1.mp4", "A02.mp4"]);
    }

    #[test]
    fn extension_filter_ignores_case() {
        assert!(has_extension(Path::new("x/Y.MP4"), VIDEO_EXTENSIONS));
        assert!(!has_extension(Path::new("x/y.txt"), VIDEO_EXTENSIONS));
        assert!(has_extension(Path::new("x/y.txt"), &[]));
    }
}
