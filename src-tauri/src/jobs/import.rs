//! 导入：把拖入 / 选择的文件和文件夹展开成视频文件列表（按扩展名、可递归、去重），
//! 再并行探测媒体信息。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use super::output::path_key;
use super::sidecar::{find_sidecars, SidecarOptions, Sidecars};
use crate::ffmpeg::{self, MediaInfo};
use crate::fsutil::{self, VIDEO_EXTENSIONS};

/// 展开后的一个视频文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpandedFile {
    pub path: PathBuf,
    /// 来自文件夹导入时为该文件夹
    pub source_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Expanded {
    pub files: Vec<ExpandedFile>,
    /// 被跳过的非视频文件数量
    pub skipped: usize,
    /// 无法访问的路径及原因
    pub errors: Vec<String>,
}

/// 展开路径：文件按扩展名过滤，文件夹列出其中视频（`recursive` 时含子目录），整体去重并保持顺序。
pub fn expand_paths(paths: &[PathBuf], recursive: bool) -> Expanded {
    let mut out = Expanded::default();
    let mut seen = HashSet::new();
    let mut push = |out: &mut Expanded, path: PathBuf, root: Option<PathBuf>| {
        if seen.insert(path_key(&path)) {
            out.files.push(ExpandedFile {
                path,
                source_root: root,
            });
        }
    };
    for path in paths {
        if path.is_dir() {
            match fsutil::list_files(path, recursive, VIDEO_EXTENSIONS) {
                Ok(files) => {
                    for file in files {
                        push(&mut out, file, Some(path.clone()));
                    }
                }
                Err(e) => out.errors.push(format!("{}：{e}", path.display())),
            }
        } else if path.is_file() {
            if fsutil::has_extension(path, VIDEO_EXTENSIONS) {
                push(&mut out, path.clone(), None);
            } else {
                out.skipped += 1;
            }
        } else {
            out.errors.push(format!("{}：路径不存在", path.display()));
        }
    }
    out
}

/// 导入的一项。探测失败时 `info` 为空、`error` 为原因。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedItem {
    pub path: String,
    pub source_root: Option<String>,
    pub info: Option<MediaInfo>,
    pub error: Option<String>,
    /// 自动关联的同名字幕 / 封面（`queue.sidecar`）
    #[serde(flatten)]
    pub sidecars: Sidecars,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub items: Vec<ImportedItem>,
    pub skipped: usize,
    pub errors: Vec<String>,
}

/// 展开并探测。`on_progress(done, total)` 在每个文件探测完成后调用（可能来自多个线程）。
pub fn import(
    ffprobe: &Path,
    paths: &[PathBuf],
    recursive: bool,
    on_progress: impl Fn(usize, usize) + Sync,
) -> ImportResult {
    import_with(
        ffprobe,
        paths,
        recursive,
        SidecarOptions::default(),
        on_progress,
    )
}

/// 同 [`import`]，并按 `sidecars` 自动关联同名字幕 / 封面。
pub fn import_with(
    ffprobe: &Path,
    paths: &[PathBuf],
    recursive: bool,
    sidecars: SidecarOptions,
    on_progress: impl Fn(usize, usize) + Sync,
) -> ImportResult {
    let expanded = expand_paths(paths, recursive);
    let total = expanded.files.len();
    let workers = crate::model::settings::cpu_cores()
        .clamp(1, 8)
        .min(total.max(1));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<ImportedItem>>> = Mutex::new(vec![None; total]);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(file) = expanded.files.get(i) else {
                    break;
                };
                let (info, error) = match ffmpeg::probe(ffprobe, &file.path) {
                    Ok(info) if info.video.is_some() => (Some(info), None),
                    Ok(info) => (Some(info), Some("文件中没有视频流".to_owned())),
                    Err(e) => (None, Some(e)),
                };
                let item = ImportedItem {
                    path: file.path.to_string_lossy().into_owned(),
                    source_root: file
                        .source_root
                        .as_ref()
                        .map(|r| r.to_string_lossy().into_owned()),
                    info,
                    error,
                    sidecars: find_sidecars(&file.path, sidecars),
                };
                results.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(item);
                on_progress(done.fetch_add(1, Ordering::SeqCst) + 1, total);
            });
        }
    });

    ImportResult {
        items: results
            .into_inner()
            .unwrap_or_else(|e| e.into_inner())
            .into_iter()
            .flatten()
            .collect(),
        skipped: expanded.skipped,
        errors: expanded.errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_filters_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("cr-import-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["b.mp4", "a.MKV", "readme.txt", "sub/c.mov"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let flat = expand_paths(
            &[dir.clone(), dir.join("b.mp4"), dir.join("readme.txt")],
            false,
        );
        let names: Vec<String> = flat
            .files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.MKV", "b.mp4"], "去重且按自然顺序");
        assert_eq!(flat.skipped, 1);
        assert!(flat
            .files
            .iter()
            .all(|f| f.source_root.as_deref() == Some(dir.as_path())));

        let deep = expand_paths(&[dir.clone(), dir.join("missing.mp4")], true);
        assert_eq!(deep.files.len(), 3);
        assert_eq!(deep.errors.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
